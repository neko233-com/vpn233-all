#!/usr/bin/env python3
"""Exercise the production SH downloader with real, loopback-only HTTP transfers."""
from __future__ import annotations

import hashlib
import os
import re
import shutil
import socket
import subprocess
import tempfile
import threading
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from socketserver import TCPServer


ROOT = Path(__file__).resolve().parents[2]
BASH = shutil.which("bash")
assert BASH, "Bash is required for the SH HTTP integration tests"
BODY = bytes(range(256)) * 2048
DIGEST = hashlib.sha256(BODY).hexdigest()
CHUNK = 65536
REQUESTS: list[tuple[str, int | None]] = []
REQUEST_PATHS: list[str] = []
ASSETS = [f"vpn233-rust-{platform}.{extension}" for platform, extension in [
    ("linux-x86_64", "tar.gz"), ("windows-x86_64", "zip"),
    ("macos-x86_64", "tar.gz"), ("macos-aarch64", "tar.gz"),
]]
MANIFESTS = {
    "manifest-missing": f"{DIGEST}  unrelated.bin\n".encode(),
    "manifest-invalid": "".join(f"invalid-digest  {name}\n" for name in ASSETS).encode(),
    "manifest-duplicate": "".join(f"{DIGEST}  {name}\n{DIGEST}  {name}\n" for name in ASSETS).encode(),
}


class LocalServer(ThreadingHTTPServer):
    def server_bind(self) -> None:
        # HTTPServer.server_bind calls getfqdn, which can stall offline macOS.
        TCPServer.server_bind(self)
        self.server_name = "localhost"
        self.server_port = self.server_address[1]


class Handler(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"

    def log_message(self, *_args: object) -> None:
        pass

    def do_GET(self) -> None:
        route = self.path.split("/", 2)[1]
        range_header = self.headers.get("Range")
        match = re.fullmatch(r"bytes=(\d+)-", range_header or "")
        offset = int(match.group(1)) if match else None
        REQUESTS.append((route, offset))
        REQUEST_PATHS.append(self.path)
        body = BODY
        start = offset or 0
        status = 206 if offset is not None else 200
        if route in MANIFESTS:
            body = MANIFESTS[route]
        elif route == "wrong":
            body = b"wrong complete response"
            status = 200
            start = 0
        elif route == "no-range" and offset is not None:
            status = 200
            start = 0
        elif route == "bad-position" and offset is not None:
            start = 0
        elif route == "changed" and offset is None:
            body = b"x" * len(BODY)
        if route == "range-416" and offset is not None:
            self.send_response(416)
            self.send_header("Content-Length", "0")
            self.send_header("Content-Range", f"bytes */{len(BODY)}")
            self.send_header("Connection", "close")
            self.end_headers()
            self.close_connection = True
            return
        data = body[start:]
        chunked = route == "chunked" and offset is not None
        self.send_response(status)
        if chunked:
            self.send_header("Transfer-Encoding", "chunked")
        else:
            self.send_header("Content-Length", str(len(data) + (1 if route == "complete-eof" else 0)))
        self.send_header("Connection", "close")
        if status == 206 and route != "missing-range":
            self.send_header("Content-Range", f"bytes {start}-{len(body)-1}/{len(body)}")
        self.end_headers()
        truncated = route in {"resume", "chunked", "no-range", "bad-position", "missing-range", "changed", "bounded", "range-416"} and offset is None
        if route == "bounded" or (route == "resume-twice" and (offset or 0) < CHUNK * 2):
            truncated = True
        if route == "slow":
            truncated = True
            time.sleep(1.2)
        try:
            if chunked:
                self.wfile.write(f"{len(data):x}\r\n".encode() + data + b"\r\n0\r\n\r\n")
            else:
                self.wfile.write(data[:CHUNK] if truncated else data)
            self.wfile.flush()
        except (BrokenPipeError, ConnectionResetError):
            pass
        if truncated:
            try:
                self.connection.shutdown(socket.SHUT_WR)
            except OSError:
                pass
        self.close_connection = True


def main() -> None:
    server = LocalServer(("127.0.0.1", 0), Handler)
    worker = threading.Thread(target=server.serve_forever, daemon=True)
    worker.start()
    passed = 0
    try:
        with tempfile.TemporaryDirectory(prefix="vpn233-mirror-http-") as temporary:
            workspace = Path(temporary)

            def check(label: str, routes: list[str], expected: str = DIGEST, *, success: bool = True, prior_cache: bool = False, budget: str = "5") -> list[tuple[str, int | None]]:
                nonlocal passed
                REQUESTS.clear()
                REQUEST_PATHS.clear()
                case = workspace / str(passed)
                case.mkdir()
                destination = case / "artifact.bin"
                destination.write_bytes(b"previous verified destination")
                cache = case / "cache"
                environment = {key: value for key, value in os.environ.items() if key.lower() not in {
                    "http_proxy", "https_proxy", "all_proxy", "no_proxy", "vpn233_gh_mirror", "vpn233_gh_mirrors",
                    "vpn233_mirror_mode", "vpn233_no_mirror", "vpn233_home",
                }}
                prefixes = [f"http://127.0.0.1:{server.server_port}/{route}/" for route in routes]
                previous_cache = "raw https://previous.example.invalid/\nrelease " + prefixes[0] + "\n"
                if prior_cache:
                    cache.mkdir()
                    (cache / "mirror-cache").write_text(previous_cache)
                environment.update({
                    "VPN233_HOME": cache.as_posix(), "VPN233_GH_MIRRORS": "|".join(prefixes),
                    "VPN233_GH_MAX_TIME": budget, "VPN233_GH_CONNECT_TIMEOUT": "2",
                    "VPN233_TEST_LIBRARY": (ROOT / "scripts/lib/mirrors.sh").as_posix(),
                    "VPN233_TEST_DEST": destination.as_posix(), "VPN233_TEST_DIGEST": expected,
                    "NO_PROXY": "127.0.0.1,localhost", "no_proxy": "127.0.0.1,localhost",
                })
                result = subprocess.run([
                    BASH, "-c", 'set -Eeuo pipefail; . "$VPN233_TEST_LIBRARY"; '
                    'vpn233_download_release_asset artifact.bin "$VPN233_TEST_DEST" 2.1.0 0 "$VPN233_TEST_DIGEST"',
                ], env=environment, capture_output=True, timeout=45)
                flags = [pattern.decode() for pattern in (b"No such file", b"not found", b"Wsl", b"WSL", b"Syntax error") if pattern in result.stderr]
                assert (result.returncode == 0) is success, f"{label}: exit={result.returncode}, requests={len(REQUESTS)}, categories={flags}"
                assert b"http://" not in result.stdout + result.stderr and b"https://" not in result.stdout + result.stderr, label + ": URL leaked"
                assert not list(case.glob("*.part.*")), label + ": partial file was retained across sources"
                if success:
                    assert destination.read_bytes() == BODY, label + ": incorrect destination"
                    assert (cache / "mirror-cache").read_text().strip() == "release " + prefixes[-1], label + ": unverified source cached"
                else:
                    assert destination.read_bytes() == b"previous verified destination", label + ": old destination modified"
                    if prior_cache:
                        assert (cache / "mirror-cache").read_text() == previous_cache, label + ": previous cache was changed"
                    else:
                        assert not (cache / "mirror-cache").exists(), label + ": failed source cached"
                passed += 1
                print("[PASS] " + label, flush=True)
                return list(REQUESTS)

            requests = check("SH real HTTP truncated response resumes using matching 206", ["resume"])
            assert requests == [("resume", None), ("resume", CHUNK)], "resume must request the actual partial offset"
            requests = check("SH real HTTP two interrupted transfers resume within three requests", ["resume-twice"])
            assert requests == [("resume-twice", None), ("resume-twice", CHUNK), ("resume-twice", CHUNK * 2)], "bounded resume offsets differ"
            requests = check("SH real HTTP valid chunked 206 resumes without Content-Length", ["chunked"])
            assert requests == [("chunked", None), ("chunked", CHUNK)], "chunked response resume offset differs"
            requests = check("SH complete expected digest is accepted after a late transfer error", ["complete-eof"])
            assert requests == [("complete-eof", None)], "already verified bytes must not trigger an unnecessary Range request"
            for route, label in [
                ("no-range", "ignored Range 200"),
                ("bad-position", "mismatched Content-Range 206"),
                ("missing-range", "missing Content-Range 206"),
                ("range-416", "unsatisfied Range 416"),
                ("changed", "changed same-source bytes"),
                ("wrong", "complete wrong-hash 200"),
            ]:
                requests = check("SH real HTTP " + label + " falls back to a fresh verified source", [route, "good"])
                assert requests[-1] == ("good", None), "fallback reused another source's partial offset"
                if route == "wrong":
                    assert requests == [("wrong", None), ("good", None)], "complete wrong content must not be resumed"
            requests = check("SH real HTTP repeated truncation stops after three requests and preserves destination", ["bounded"], success=False)
            assert requests == [("bounded", None), ("bounded", CHUNK), ("bounded", CHUNK * 2)], "retry budget is not bounded to three"
            started = time.monotonic()
            requests = check("SH same-source retries share one configured network time budget", ["slow"], success=False, budget="2")
            elapsed = time.monotonic() - started
            assert len(requests) <= 3 and elapsed < 3.2, f"source time budget was repeated: requests={len(requests)}, elapsed={elapsed:.3f}s"
            requests = check("SH real HTTP hash failure preserves destination without caching", ["wrong"], success=False)
            assert requests == [("wrong", None)], "wrong complete content must not be retried"
            check("SH failed verified download preserves an existing cache and destination", ["wrong"], success=False, prior_cache=True)
            requests = check("SH no-hash downloads do not resume truncated responses", ["resume", "good"], "")
            assert requests == [("resume", None), ("good", None)], "unverified download sent a Range request"
            for invalid in ("invalid", DIGEST + " ", "0" * 63, "g" * 64):
                requests = check("SH invalid expected digest fails before any request or cache write", ["good"], invalid, success=False)
                assert not requests, "invalid expected digest reached the network"
            requests = check("SH sub-millisecond budget fails before curl can interpret it as unlimited", ["good"], success=False, budget="0.0001")
            assert not requests, "unsafe timeout reached the network"
            # Exercise the actual installer contract: malformed/missing expected
            # hashes are rejected before any archive request or install mutation.
            for route in MANIFESTS:
                REQUESTS.clear()
                REQUEST_PATHS.clear()
                prefix = workspace / route
                environment = {key: value for key, value in os.environ.items() if not key.startswith("VPN233_") and key.lower() not in {
                    "http_proxy", "https_proxy", "all_proxy", "no_proxy",
                }}
                environment.update({
                    "VPN233_HOME": (workspace / (route + "-cache")).as_posix(),
                    "XDG_CONFIG_HOME": (workspace / (route + "-xdg")).as_posix(),
                    "NO_PROXY": "127.0.0.1,localhost", "no_proxy": "127.0.0.1,localhost",
                })
                result = subprocess.run([
                    BASH, (ROOT / "install.sh").as_posix(), "--release", "--version", "2.1.0", "--no-start", "--yes",
                    "--prefix", prefix.as_posix(), "--mirror", f"http://127.0.0.1:{server.server_port}/{route}/",
                ], env=environment, capture_output=True, timeout=30)
                assert result.returncode != 0 and not prefix.exists(), "invalid manifest reached installation mutation"
                assert len(REQUEST_PATHS) == 1 and REQUEST_PATHS[0].endswith("/SHA256SUMS"), "archive was requested before expected hash validation"
                passed += 1
                print("[PASS] SH installer " + route + " rejects before archive request or installation", flush=True)
        print(f"test-mirror-http.py: {passed} passed")
    finally:
        server.shutdown()
        server.server_close()
        worker.join(timeout=5)


if __name__ == "__main__":
    main()
