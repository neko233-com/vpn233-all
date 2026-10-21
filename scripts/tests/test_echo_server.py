"""Real loopback echo fixture checks; all payloads are synthetic."""
import http.client
import contextlib
import io
import json
import os
import runpy
import stat
import subprocess
import sys
import tempfile
import time
import unittest
from pathlib import Path
from unittest.mock import patch

ECHO_PATH = Path(__file__).resolve().parents[1] / "e2e" / "echo_server.py"
ECHO = runpy.run_path(str(ECHO_PATH))


class EchoServerTests(unittest.TestCase):
    def test_listener_never_uses_reverse_dns(self):
        with patch("socket.getfqdn", side_effect=AssertionError("fixture must not resolve hostnames")):
            server = ECHO["EchoHTTPServer"](("127.0.0.1", 0), ECHO["EchoHandler"])
            try:
                self.assertGreater(server.server_port, 0)
                self.assertEqual("localhost", server.server_name)
                connection = http.client.HTTPConnection("127.0.0.1", server.server_port, timeout=2)
                connection.connect()
                connection.close()
            finally:
                server.server_close()

    def test_port_zero_publishes_private_ready_file_and_serves_http(self):
        with tempfile.TemporaryDirectory(prefix="vpn233-echo-test-") as scratch:
            ready = Path(scratch) / "echo.ready"
            process = subprocess.Popen(
                [sys.executable, str(ECHO_PATH), "--port", "0", "--ready-file", str(ready), "--marker", "change-me-echo-fixture"],
                stdout=subprocess.PIPE, stderr=subprocess.PIPE,
            )
            try:
                deadline = time.monotonic() + 5
                while time.monotonic() < deadline:
                    if ready.is_file() and ready.stat().st_size:
                        break
                    if process.poll() is not None:
                        self.fail("echo process exited before readiness")
                    time.sleep(0.02)
                self.assertTrue(ready.is_file(), "echo process did not publish readiness")
                port = int(ready.read_text(encoding="ascii"))
                self.assertGreater(port, 0)
                if os.name != "nt":
                    self.assertEqual(0o600, stat.S_IMODE(ready.stat().st_mode))
                connection = http.client.HTTPConnection("127.0.0.1", port, timeout=3)
                try:
                    connection.request("GET", "/fixture/test")
                    response = connection.getresponse()
                    self.assertEqual(200, response.status)
                    payload = json.loads(response.read())
                    self.assertEqual("change-me-echo-fixture", payload["marker"])
                    self.assertEqual("/fixture/test", payload["path"])
                    connection.request("HEAD", "/fixture/head")
                    response = connection.getresponse()
                    self.assertEqual(200, response.status)
                    self.assertEqual(b"", response.read())
                finally:
                    connection.close()
            finally:
                process.terminate()
                try:
                    output, errors = process.communicate(timeout=3)
                except subprocess.TimeoutExpired:
                    process.kill()
                    process.communicate()
                    self.fail("echo shutdown hung")
            if os.name != "nt":
                self.assertEqual(0, process.returncode)
                self.assertFalse(ready.exists())
            self.assertNotIn(b"change-me-echo-fixture", output + errors)

    def test_invalid_ready_files_fail_without_overwriting_or_leaking(self):
        with tempfile.TemporaryDirectory(prefix="vpn233-echo-invalid-") as scratch:
            root = Path(scratch)
            existing = root / "existing.ready"
            existing.write_text("retained synthetic content", encoding="ascii")
            cases = [root, root / "missing" / "echo.ready", existing]
            if os.name != "nt":
                link = root / "symlink.ready"
                link.symlink_to(existing)
                cases.append(link)
            for ready in cases:
                with self.subTest(case=ready.name):
                    result = subprocess.run(
                        [sys.executable, str(ECHO_PATH), "--port", "0", "--ready-file", str(ready), "--marker", "change-me-private-marker"],
                        capture_output=True, timeout=5,
                    )
                    self.assertEqual(1, result.returncode)
                    self.assertRegex(result.stderr.decode("ascii"), r"^echo fixture error: [A-Za-z]+ errno=\d+\s*$")
                    self.assertNotIn(b"change-me-private-marker", result.stdout + result.stderr)
                    self.assertNotIn(str(root).encode(), result.stdout + result.stderr)
            self.assertEqual("retained synthetic content", existing.read_text(encoding="ascii"))

    def test_invalid_port_reports_only_error_type_and_code(self):
        result = subprocess.run(
            [sys.executable, str(ECHO_PATH), "--port", "65536", "--marker", "change-me-private-marker"],
            capture_output=True, timeout=5,
        )
        self.assertEqual(1, result.returncode)
        self.assertEqual(b"echo fixture error: ValueError errno=0\n", result.stderr.replace(b"\r\n", b"\n"))
        self.assertNotIn(b"change-me-private-marker", result.stdout + result.stderr)

    def test_cleanup_failure_reports_no_private_path_or_traceback(self):
        with tempfile.TemporaryDirectory(prefix="vpn233-echo-cleanup-") as scratch:
            ready = Path(scratch) / "echo.ready"
            output, errors = io.StringIO(), io.StringIO()
            with patch("sys.argv", [str(ECHO_PATH), "--port", "0", "--ready-file", str(ready)]), \
                    patch("signal.signal"), \
                    patch.object(ECHO["EchoHTTPServer"], "serve_forever"), \
                    patch.object(Path, "unlink", side_effect=PermissionError(13, "change-me-private-error", str(ready))), \
                    contextlib.redirect_stdout(output), contextlib.redirect_stderr(errors):
                self.assertEqual(1, ECHO["main"]())
            self.assertEqual("echo fixture error: PermissionError errno=13\n", errors.getvalue())
            self.assertNotIn("change-me-private-error", output.getvalue() + errors.getvalue())
            self.assertNotIn(str(ready), output.getvalue() + errors.getvalue())


if __name__ == "__main__":
    unittest.main()
