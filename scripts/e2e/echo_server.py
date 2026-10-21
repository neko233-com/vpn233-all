#!/usr/bin/env python3
"""Tiny HTTP echo server used by the end-to-end suites.

It answers every GET request with a deterministic JSON document so the test
suites can prove that a request really travelled through the SOCKS5/HTTP proxy
implemented by vpn233-client. The response never reflects request headers into
logs and contains no user data.

Usage:
    python3 scripts/e2e/echo_server.py --port 18080 [--marker vpn233-e2e-echo]

Exit codes: 0 on normal shutdown (SIGTERM/SIGINT), 2 on usage errors.
"""

from __future__ import annotations

import argparse
import json
import signal
import sys
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer


class EchoHandler(BaseHTTPRequestHandler):
    server_version = "vpn233-e2e-echo/1.0"
    protocol_version = "HTTP/1.1"

    def do_GET(self) -> None:  # noqa: N802 - http.server naming
        payload = {
            "marker": self.server.marker,  # type: ignore[attr-defined]
            "method": "GET",
            "path": self.path,
            "host": self.headers.get("Host", ""),
        }
        body = (json.dumps(payload, sort_keys=True) + "\n").encode("utf-8")
        self.send_response(200)
        self.send_header("Content-Type", "application/json; charset=utf-8")
        self.send_header("Content-Length", str(len(body)))
        self.send_header("Cache-Control", "no-store")
        self.end_headers()
        self.wfile.write(body)

    def do_HEAD(self) -> None:  # noqa: N802 - http.server naming
        self.send_response(200)
        self.send_header("Content-Type", "application/json; charset=utf-8")
        self.send_header("Content-Length", "0")
        self.end_headers()

    def log_message(self, fmt: str, *args: object) -> None:
        # Keep stdout/stderr quiet; the caller only needs the server to run.
        return


def main() -> int:
    parser = argparse.ArgumentParser(description="VPN233 end-to-end echo server")
    parser.add_argument("--port", type=int, default=18080)
    parser.add_argument("--bind", default="127.0.0.1")
    parser.add_argument("--marker", default="vpn233-e2e-echo")
    args = parser.parse_args()

    server = ThreadingHTTPServer((args.bind, args.port), EchoHandler)
    server.marker = args.marker  # type: ignore[attr-defined]
    server.daemon_threads = True

    def stop(_signum: int, _frame: object) -> None:
        # serve_forever runs on this same thread. shutdown() would wait for
        # itself and deadlock the E2E cleanup; finally closes the socket.
        raise SystemExit(0)

    signal.signal(signal.SIGTERM, stop)
    signal.signal(signal.SIGINT, stop)
    print(f"echo server listening on http://{args.bind}:{args.port}", flush=True)
    try:
        server.serve_forever(poll_interval=0.2)
    except KeyboardInterrupt:
        return 0
    finally:
        server.server_close()
    return 0


if __name__ == "__main__":
    sys.exit(main())
