"""Actual loopback HTTP/PowerShell archive verification, without external traffic."""
from __future__ import annotations

import contextlib
import hashlib
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import io
import os
from pathlib import Path
import shutil
import socket
from socketserver import TCPServer
import subprocess
import tempfile
import threading
import time
import unittest
import zipfile

ROOT = Path(__file__).resolve().parents[2]
SHELL = shutil.which("powershell" if os.name == "nt" else "pwsh")
PAYLOAD = b"synthetic archive fixture\x00" + bytes(range(256)) * 512


class OfflineServer(ThreadingHTTPServer):
    daemon_threads = True

    def server_bind(self):
        TCPServer.server_bind(self)
        self.server_name = "localhost"
        self.server_port = self.server_address[1]


class Handler(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"

    def log_message(self, *args):
        return

    def do_GET(self):
        route = self.path.split("/")[1]
        request_range = self.headers.get("Range")
        self.server.requests.append((route, request_range, self.headers.get("Accept-Encoding")))
        payload = self.server.payload
        if self.path.endswith("/SHA256SUMS"):
            body = (hashlib.sha256(payload).hexdigest() + "  vpn233-rust-windows-x86_64.zip\n").encode()
            return self.send_bytes(200, body)
        offset = int(request_range.removeprefix("bytes=").removesuffix("-")) if request_range else 0
        try:
            if route == "wrong":
                return self.send_bytes(200, b"synthetic wrong archive")
            if route == "encoded":
                return self.send_bytes(200, payload, encoding="gzip")
            if route == "oversized":
                return self.send_bytes(200, b"x", declared=1073741825, cut=True)
            if route == "unavailable":
                return self.send_bytes(500, b"synthetic HTTP failure")
            if route == "stall":
                self.send_response(200)
                self.send_header("Content-Length", str(len(payload)))
                self.send_header("Connection", "close")
                self.end_headers()
                self.server.stopping.wait(5)
                return
            if route == "trickle":
                self.send_response(206 if request_range else 200)
                self.send_header("Content-Length", str(len(payload) - offset))
                if request_range:
                    self.send_header("Content-Range", f"bytes {offset}-{len(payload)-1}/{len(payload)}")
                self.end_headers()
                for byte in payload[offset:]:
                    self.wfile.write(bytes([byte]))
                    self.wfile.flush()
                    if self.server.stopping.wait(0.04):
                        return
                return
            if route in ("resume", "reset", "chunked", "wrongrange", "wronglength", "changedtotal", "alwaysshort"):
                if not request_range:
                    return self.send_bytes(200, payload[:4093], declared=len(payload), cut=True)
                if route == "reset":
                    return self.send_bytes(200, payload)
                first = 0 if route == "wrongrange" else offset
                total = len(payload) + (1 if route == "changedtotal" else 0)
                body = payload[offset:]
                if route == "alwaysshort":
                    body = body[:4093]
                return self.send_bytes(206, body, content_range=f"bytes {first}-{len(payload)-1}/{total}",
                                       declared=len(payload)-offset-(1 if route == "wronglength" else 0),
                                       cut=route == "alwaysshort", chunked=route == "chunked")
            if route == "resetafterfull":
                return self.send_bytes(200, payload, declared=len(payload)+16, cut=True)
            return self.send_bytes(200, payload)
        except (OSError, ValueError):
            return

    def send_bytes(self, code, body, *, declared=None, content_range=None, cut=False, chunked=False, encoding=None):
        self.send_response(code)
        self.send_header("Connection", "close")
        if chunked:
            self.send_header("Transfer-Encoding", "chunked")
        else:
            self.send_header("Content-Length", str(len(body) if declared is None else declared))
        if content_range:
            self.send_header("Content-Range", content_range)
        if encoding:
            self.send_header("Content-Encoding", encoding)
        self.end_headers()
        if chunked:
            self.wfile.write(f"{len(body):x}\r\n".encode()+body+b"\r\n0\r\n\r\n")
        else:
            self.wfile.write(body)
        self.wfile.flush()
        if cut:
            self.connection.shutdown(socket.SHUT_RDWR)
            self.connection.close()


@contextlib.contextmanager
def server(payload=PAYLOAD):
    instance = OfflineServer(("127.0.0.1", 0), Handler)
    instance.payload, instance.requests, instance.stopping = payload, [], threading.Event()
    thread = threading.Thread(target=instance.serve_forever, daemon=True)
    thread.start()
    try:
        yield instance
    finally:
        instance.stopping.set()
        instance.shutdown()
        instance.server_close()
        thread.join(2)


@unittest.skipUnless(SHELL, "PowerShell runtime unavailable")
class MirrorHTTPTests(unittest.TestCase):
    def run_transfer(self, routes, *, digest=None, seconds=4, succeeds=True, payload=PAYLOAD, installer=False, verified=True):
        with tempfile.TemporaryDirectory(prefix="vpn233-http-mirror-test-") as scratch, server(payload) as fixture:
            directory = Path(scratch)
            destination = directory / "asset.bin"
            destination.write_bytes(b"synthetic prior destination")
            home = directory / "home"
            home.mkdir()
            cache = home / "mirror-cache"
            cache.write_text("release http://127.0.0.1/remembered/\n")
            previous_cache = cache.read_bytes()
            prefixes = [f"http://127.0.0.1:{fixture.server_port}/{route}/" for route in routes]
            environment = {k: v for k, v in os.environ.items() if not k.upper().startswith("VPN233_")}
            environment.update(VPN233_HOME=str(home), VPN233_GH_MIRRORS="|".join(prefixes), VPN233_GH_MAX_TIME=str(seconds))
            environment["VPN233_TEST_DIGEST"] = hashlib.sha256(payload).hexdigest() if digest is None else digest
            if os.name == "nt":
                environment["PSModulePath"] = str(Path(os.environ["SystemRoot"]) / "System32/WindowsPowerShell/v1.0/Modules")
            script = directory / "receive.ps1"
            script.write_text("""param([string]$Library,[string]$Destination,[switch]$NoHash)
$ErrorActionPreference='Stop'
. $Library
if($NoHash){Receive-Vpn233ReleaseAsset fixture.bin $Destination '2.1.0' -Quiet -WarningAction SilentlyContinue}
else{Receive-Vpn233ReleaseAsset fixture.bin $Destination '2.1.0' -ExpectedSha256 $env:VPN233_TEST_DIGEST -Quiet -WarningAction SilentlyContinue}
""", encoding="utf-8")
            if installer:
                command = [SHELL, "-NoProfile", "-ExecutionPolicy", "Bypass", "-File", str(ROOT / "install.ps1"), "-Release", "-Version", "2.1.0", "-Prefix", str(directory / "installed"), "-NoStart", "-Mirror", "auto"]
            else:
                command = [SHELL, "-NoProfile", "-File", str(script), "-Library", str(ROOT / "scripts/lib/mirrors.ps1"), "-Destination", str(destination)]
                if not verified:
                    command.append("-NoHash")
            started = time.monotonic()
            result = subprocess.run(command, capture_output=True, env=environment, timeout=25)
            elapsed = time.monotonic() - started
            self.assertEqual(succeeds, result.returncode == 0, "controlled loopback transfer result differs; no private output printed")
            self.assertNotIn(prefixes[0].encode(), result.stdout + result.stderr)
            self.assertFalse(list(directory.glob("*.part")))
            if succeeds:
                if installer:
                    self.assertTrue((directory / "installed/bin/vpn233-provider-server.exe").is_file())
                else:
                    self.assertEqual(payload, destination.read_bytes())
                self.assertEqual("release " + prefixes[-1], cache.read_text().strip())
            else:
                self.assertEqual(b"synthetic prior destination", destination.read_bytes())
                self.assertEqual(previous_cache, cache.read_bytes())
            return list(fixture.requests), elapsed

    def test_truncated_response_resumes_with_range(self):
        requests, _ = self.run_transfer(["resume"])
        self.assertEqual([None, "bytes=4093-"], [r[1] for r in requests])
        self.assertTrue(all(r[2] == "identity" for r in requests))

    def test_ignored_range_restarts_instead_of_appending(self):
        requests, _ = self.run_transfer(["reset"])
        self.assertEqual([None, "bytes=4093-"], [r[1] for r in requests])

    def test_chunked_range_is_valid_without_content_length(self):
        requests, _ = self.run_transfer(["chunked"])
        self.assertEqual([None, "bytes=4093-"], [r[1] for r in requests])

    def test_complete_correct_hash_survives_peer_reset(self):
        requests, _ = self.run_transfer(["resetafterfull"])
        self.assertEqual(1, len(requests))

    def test_wrong_hash_and_malformed_ranges_fall_back(self):
        for route in ("wrong", "wrongrange", "wronglength", "changedtotal", "encoded", "oversized"):
            with self.subTest(route=route):
                requests, _ = self.run_transfer([route, "good"])
                self.assertEqual("good", requests[-1][0])
                self.assertEqual(1 if route in ("wrong", "encoded", "oversized") else 2, len([r for r in requests if r[0] == route]))

    def test_no_hash_mode_does_not_resume_failed_raw_downloads(self):
        requests, _ = self.run_transfer(["unavailable", "good"], verified=False)
        self.assertEqual([None, None], [r[1] for r in requests])

    def test_three_attempts_preserve_prior_output_and_cache(self):
        requests, _ = self.run_transfer(["alwaysshort"], succeeds=False)
        self.assertEqual([None, "bytes=4093-", "bytes=8186-"], [r[1] for r in requests])

    def test_stall_and_trickle_share_one_source_deadline(self):
        for route in ("stall", "trickle"):
            with self.subTest(route=route):
                requests, elapsed = self.run_transfer([route], seconds=1, succeeds=False)
                self.assertEqual(1, len(requests))
                self.assertLess(elapsed, 4, "one source budget must not be multiplied by three attempts")

    def test_invalid_hash_has_no_http_or_file_side_effect(self):
        for digest in ("not-a-sha256", "a"*64+"\n", "a"*64+"\r\n", " "+"a"*64):
            with self.subTest(invalid_form=repr(digest[:3]+digest[-2:])):
                requests, _ = self.run_transfer(["good"], digest=digest, succeeds=False)
                self.assertEqual([], requests)

    def test_real_installer_passes_manifest_hash_before_archive_selection(self):
        output = io.BytesIO()
        with zipfile.ZipFile(output, "w") as archive:
            for name in ("vpn233-provider-server", "vpn233-subscribe-server", "vpn233-client"):
                archive.writestr("bin/" + name + ".exe", b"synthetic executable fixture")
            for name in ("provider-server", "subscribe-server", "vpn233-client"):
                archive.writestr("examples/" + name + ".yaml", (ROOT / name / "config.example.yaml").read_bytes())
            archive.writestr("version.txt", "2.1.0\n")
            archive.writestr("admin-web/dist/index.html", "<!doctype html><title>synthetic fixture</title>")
        requests, _ = self.run_transfer(["wrong", "good"], payload=output.getvalue(), installer=True)
        self.assertTrue(any(route == "good" for route, _, _ in requests))


if __name__ == "__main__":
    unittest.main()
