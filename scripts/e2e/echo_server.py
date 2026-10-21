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
import os
import signal
import sys
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from socketserver import TCPServer


def report_error(error: BaseException) -> None:
    """Expose useful failure codes without paths, request data or markers."""
    code = getattr(error, "errno", None)
    code = code if isinstance(code, int) else 0
    print(f"echo fixture error: {type(error).__name__} errno={code}", file=sys.stderr, flush=True)


class EchoHTTPServer(ThreadingHTTPServer):
    def server_bind(self) -> None:
        # HTTPServer.server_bind calls getfqdn before listen. This loopback
        # fixture needs no reverse DNS and must work with an offline resolver.
        TCPServer.server_bind(self)
        self.server_name = "localhost"
        self.server_port = self.server_address[1]

    def handle_error(self, request: object, client_address: object) -> None:
        error = sys.exc_info()[1]
        if error is not None:
            report_error(error)


def write_ready(path: Path, port: int) -> None:
    # Exclusive creation rejects stale files and symlinks. The observer waits
    # for a complete port line; only this already-listening process owns it.
    descriptor = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
    with os.fdopen(descriptor, "w", encoding="ascii") as opened:
        if os.name != "nt":
            os.fchmod(opened.fileno(), 0o600)
        opened.write(f"{port}\n")
        opened.flush()


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
    parser.add_argument("--ready-file", type=Path, help="exclusively create a private file containing the actual listening port")
    args = parser.parse_args()

    def stop(_signum: int, _frame: object) -> None:
        # serve_forever runs on this same thread. shutdown() would wait for
        # itself and deadlock the E2E cleanup; finally closes the socket.
        raise KeyboardInterrupt

    server = None
    ready_created = False
    status = 0
    try:
        if not 0 <= args.port <= 65535:
            raise ValueError("invalid fixture port")
        server = EchoHTTPServer((args.bind, args.port), EchoHandler)
        server.marker = args.marker  # type: ignore[attr-defined]
        server.daemon_threads = True
        signal.signal(signal.SIGTERM, stop)
        signal.signal(signal.SIGINT, stop)
        if args.ready_file is not None:
            write_ready(args.ready_file, server.server_port)
            ready_created = True
        print("echo fixture ready", flush=True)
        server.serve_forever(poll_interval=0.2)
    except KeyboardInterrupt:
        pass
    except Exception as error:
        report_error(error)
        status = 1
    finally:
        try:
            if server is not None:
                server.server_close()
            if ready_created:
                args.ready_file.unlink(missing_ok=True)
        except Exception as error:
            report_error(error)
            status = 1
    return status


if __name__ == "__main__":
    sys.exit(main())
