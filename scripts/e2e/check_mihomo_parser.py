#!/usr/bin/env python3
"""Check real provider-generated inbounds with the pinned official Mihomo parser.

This optional integration gate opens only a temporary loopback provider API.
Mihomo runs exclusively with -t; no proxy listener is started. All credentials,
certificates and generated configuration are synthetic and removed on exit.

Examples:
  python scripts/e2e/check_mihomo_parser.py --download-core
  python scripts/e2e/check_mihomo_parser.py --mihomo-archive target/mihomo.zip
  python scripts/e2e/check_mihomo_parser.py --mihomo-bin /trusted/path/mihomo
"""

from __future__ import annotations

import argparse
import gzip
import hashlib
import io
import json
import os
from pathlib import Path
import platform
import re
import shutil
import socket
import subprocess
import sys
import tempfile
import time
import urllib.error
import urllib.parse
import urllib.request
import uuid
import zipfile


ROOT = Path(__file__).resolve().parents[2]
PROTOCOLS = ("vless", "vmess", "trojan", "ss", "hysteria2", "tuic", "anytls", "socks5")


def clean_environment() -> dict[str, str]:
    return {key: value for key, value in os.environ.items() if not key.upper().startswith("VPN233_")}


def official_asset() -> tuple[str, str, str]:
    config_source = (ROOT / "provider-server/src/config.rs").read_text(encoding="utf-8")
    version = re.search(r'DEFAULT_MIHOMO_VERSION: &str = "([^"]+)"', config_source)
    if not version:
        raise RuntimeError("Cannot read the pinned Mihomo version")
    machine = platform.machine().lower()
    architecture = "arm64" if machine in ("arm64", "aarch64") else "amd64-compatible"
    if machine not in ("amd64", "x86_64", "arm64", "aarch64"):
        raise RuntimeError("No pinned Mihomo asset for this architecture")
    operating_system = {"Windows": "windows", "Linux": "linux", "Darwin": "darwin"}.get(platform.system())
    if not operating_system:
        raise RuntimeError("No pinned Mihomo asset for this operating system")
    extension = "zip" if operating_system == "windows" else "gz"
    asset = f"mihomo-{operating_system}-{architecture}-{version[1]}.{extension}"
    script_source = (ROOT / "provider-server/src/scripts.rs").read_text(encoding="utf-8")
    digests = dict(re.findall(r'"(mihomo-[^"]+)"\s*,\s*"([a-f0-9]{64})"', script_source))
    if asset not in digests:
        raise RuntimeError("The generated script has no official checksum for this platform")
    url = f"https://github.com/MetaCubeX/mihomo/releases/download/{version[1]}/{asset}"
    return asset, digests[asset], url


def trusted_core(arguments: argparse.Namespace, work: Path) -> Path:
    if arguments.mihomo_bin:
        binary = Path(arguments.mihomo_bin).resolve(strict=True)
        if not binary.is_file():
            raise RuntimeError("The explicitly trusted Mihomo binary is not a file")
        return binary
    asset, expected, url = official_asset()
    if arguments.mihomo_archive:
        archive = Path(arguments.mihomo_archive).resolve(strict=True)
    elif arguments.download_core:
        cache = Path(arguments.cache_dir).expanduser()
        cache.mkdir(parents=True, exist_ok=True)
        archive = cache / asset
        if not archive.exists():
            partial = cache / f".{asset}.{uuid.uuid4().hex}.part"
            try:
                download_official(url, partial)
                if hashlib.sha256(partial.read_bytes()).hexdigest() != expected:
                    raise RuntimeError("Downloaded official Mihomo archive SHA-256 mismatch")
                os.replace(partial, archive)
            finally:
                partial.unlink(missing_ok=True)
    else:
        raise RuntimeError("Supply --mihomo-archive, --mihomo-bin, or opt in with --download-core")
    archive_bytes = archive.read_bytes()
    if hashlib.sha256(archive_bytes).hexdigest() != expected:
        raise RuntimeError("Official Mihomo archive SHA-256 mismatch; refusing execution")
    binary = work / ("mihomo.exe" if platform.system() == "Windows" else "mihomo")
    if asset.endswith(".zip"):
        with zipfile.ZipFile(io.BytesIO(archive_bytes)) as package:
            executables = [entry for entry in package.infolist() if not entry.is_dir() and entry.filename.endswith(".exe")]
            if len(executables) != 1:
                raise RuntimeError("Official archive must contain exactly one executable")
            # Extract bytes to a fixed path, never trust archive entry paths.
            binary.write_bytes(package.read(executables[0]))
    else:
        binary.write_bytes(gzip.decompress(archive_bytes))
        binary.chmod(0o700)
    print(f"Official archive SHA-256 verified: {asset}")
    return binary


def download_official(url: str, destination: Path) -> None:
    """Bounded official download; retain TLS verification in both implementations."""
    git_curl = Path(os.environ.get("ProgramFiles", "C:/Program Files")) / "Git/mingw64/bin/curl.exe"
    curl = str(git_curl) if os.name == "nt" and git_curl.is_file() else shutil.which("curl")
    if curl:
        options = {"creationflags": subprocess.CREATE_NO_WINDOW} if os.name == "nt" else {}
        for _ in range(2):
            result = subprocess.run(
                [curl, "--fail", "--location", "--silent", "--show-error", "--connect-timeout", "10",
                 "--max-time", "90", "--continue-at", "-", "--output", str(destination), url],
                capture_output=True, timeout=100, **options,
            )
            if result.returncode == 0:
                return
        raise RuntimeError("Official Mihomo download failed after two bounded TLS-verified attempts")
    request = urllib.request.Request(url, headers={"User-Agent": "vpn233-public-parser-check"})
    started = time.monotonic()
    with urllib.request.urlopen(request, timeout=20) as response, destination.open("wb") as output:
        while chunk := response.read(1024 * 1024):
            output.write(chunk)
            if time.monotonic() - started > 120:
                raise RuntimeError("Official Mihomo download exceeded the time limit")


def request_json(base: str, path: str, *, token: str = "", payload: dict | None = None) -> dict:
    headers = {"Content-Type": "application/json"}
    if token:
        headers["Authorization"] = f"Bearer {token}"
    body = json.dumps(payload).encode() if payload is not None else None
    request = urllib.request.Request(base + path, data=body, headers=headers)
    # Loopback HTTP deliberately bypasses any user-configured outward proxy.
    opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))
    with opener.open(request, timeout=2) as response:
        return json.load(response)


def openssl_binary() -> str:
    candidate = shutil.which("openssl")
    if candidate:
        return candidate
    bundled = Path(os.environ.get("ProgramFiles", "C:/Program Files")) / "Git/usr/bin/openssl.exe"
    if bundled.is_file():
        return str(bundled)
    raise RuntimeError("OpenSSL is required to create a synthetic one-day certificate")


def run_parser(core: Path, configuration: Path, work: Path, name: str) -> None:
    options = {"creationflags": subprocess.CREATE_NO_WINDOW} if os.name == "nt" else {}
    result = subprocess.run(
        [str(core), "-t", "-d", str(work), "-f", str(configuration)], env=clean_environment(),
        capture_output=True, text=True, timeout=30, **options,
    )
    if result.returncode:
        diagnostic = (result.stdout + result.stderr).replace(str(work), "<temporary>")
        raise RuntimeError(f"Mihomo rejected {name}: {diagnostic[:1600]}")
    print(f"PASS official Mihomo -t: {name}")


def check_generated_configuration(provider_binary: Path, core: Path, work: Path) -> None:
    environment = clean_environment()
    with socket.socket() as listener:
        listener.bind(("127.0.0.1", 0))
        api_port = listener.getsockname()[1]
    admin_secret = f"synthetic-parser-{uuid.uuid4()}"
    provider_config = work / "provider-fixture.yaml"
    provider_config.write_text(
        f'listen_addr: "127.0.0.1"\nlisten_port: {api_port}\n'
        f'admin_user: "parser-probe"\nadmin_password: "{admin_secret}"\n'
        'default_node_ip: "203.0.113.10"\ndefault_port_base: 22000\n'
        'auto_detect_public_ip: false\nregistry_url: ""\nprovider_access_token: ""\n',
        encoding="utf-8",
    )
    provider_config.chmod(0o600)
    options = {"creationflags": subprocess.CREATE_NO_WINDOW} if os.name == "nt" else {}
    with (work / "provider-synthetic.log").open("wb") as provider_log:
        provider = subprocess.Popen(
            [str(provider_binary), "--config", str(provider_config)], cwd=work,
            env=environment, stdout=provider_log, stderr=subprocess.STDOUT, **options,
        )
        try:
            base = f"http://127.0.0.1:{api_port}"
            for _ in range(60):
                if provider.poll() is not None:
                    raise RuntimeError("Synthetic provider exited before becoming ready")
                try:
                    request_json(base, "/api/v1/health")
                    break
                except (urllib.error.URLError, TimeoutError):
                    time.sleep(0.1)
            else:
                raise RuntimeError("Synthetic loopback provider did not become ready")
            login = request_json(base, "/api/v1/login", payload={"username": "parser-probe", "password": admin_secret})
            query = urllib.parse.urlencode({
                "node_name": "parser-probe", "node_ip": "203.0.113.10", "port_base": 22000,
                "uuid": "123e4567-e89b-42d3-a456-426614174000", "password": "synthetic-inbound-parser-password",
            })
            generated = request_json(base, "/api/v1/generate?" + query, token=login["token"])
            if tuple(port["protocol"] for port in generated["node"]["ports"]) != PROTOCOLS:
                raise RuntimeError("Provider did not generate all eight expected inbound protocols")
            match = re.search(r"(?ms)^render_config\(\) \{\r?\n  cat <<VPN233_YAML\r?\n(.*?)\r?\nVPN233_YAML\r?\n\}", generated["sh"])
            if not match:
                raise RuntimeError("Provider bash output has no configuration heredoc")
            certificate = work / "synthetic-certificate.pem"
            private_key = work / "synthetic-private-key.pem"
            certificate_result = subprocess.run(
                [openssl_binary(), "req", "-x509", "-newkey", "rsa:2048", "-sha256", "-days", "1", "-nodes",
                 "-keyout", str(private_key), "-out", str(certificate), "-subj", "/CN=example.invalid"],
                env=environment, capture_output=True, timeout=30, **options,
            )
            if certificate_result.returncode:
                raise RuntimeError("OpenSSL failed to create the synthetic certificate")
            yaml = match[1].replace("$TLS_CERT", certificate.as_posix()).replace("$TLS_KEY", private_key.as_posix())
            header = yaml.split("listeners:", 1)[0] + "listeners:\n"
            blocks = re.findall(r"(?ms)^  - name:.*?(?=^  - name:|\Z)", yaml)
            if len(blocks) != 8:
                raise RuntimeError("Generated configuration does not contain eight listener blocks")
            checks = [("all-eight", yaml)] + list(zip(PROTOCOLS, (header + block for block in blocks), strict=True))
            for name, configuration in checks:
                path = work / f"{name}.yaml"
                path.write_text(configuration, encoding="utf-8")
                path.chmod(0o600)
                run_parser(core, path, work, name)
        finally:
            if provider.poll() is None:
                provider.terminate()
                try:
                    provider.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    provider.kill()
                    provider.wait(timeout=5)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    default_provider = ROOT / "target/release" / ("vpn233-provider-server.exe" if os.name == "nt" else "vpn233-provider-server")
    parser.add_argument("--provider-bin", default=str(default_provider))
    trust = parser.add_mutually_exclusive_group()
    trust.add_argument("--mihomo-bin", help="Explicitly trusted local Mihomo executable")
    trust.add_argument("--mihomo-archive", help="Official platform archive; checked against the pinned SHA-256")
    trust.add_argument("--download-core", action="store_true", help="Download the public pinned official release and verify SHA-256")
    default_cache = Path(os.environ.get("XDG_CACHE_HOME", Path.home() / ".cache")) / "vpn233/mihomo"
    parser.add_argument("--cache-dir", default=str(default_cache), help="Cache only official public archives; SHA-256 is rechecked on every use")
    parser.add_argument("--core-fixtures-dir", help="Also parse each synthetic *.yaml fixture exported by the core-library example")
    arguments = parser.parse_args()
    provider = Path(arguments.provider_bin).resolve(strict=True)
    with tempfile.TemporaryDirectory(prefix="vpn233-mihomo-parser-") as temporary:
        work = Path(temporary)
        core = trusted_core(arguments, work)
        check_generated_configuration(provider, core, work)
        if arguments.core_fixtures_dir:
            fixtures = sorted(Path(arguments.core_fixtures_dir).resolve(strict=True).glob("*.yaml"))
            if not fixtures:
                raise RuntimeError("The core fixture directory contains no *.yaml fixtures")
            for fixture in fixtures:
                run_parser(core, fixture, work, "core/" + fixture.stem)
    print("Official parser: combined configuration and all eight generated inbounds passed; no proxy listeners started.")
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except (OSError, RuntimeError, subprocess.SubprocessError, urllib.error.URLError) as failure:
        print(f"Mihomo parser check failed: {failure}", file=sys.stderr)
        raise SystemExit(1)
