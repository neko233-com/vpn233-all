#!/usr/bin/env python3
"""Verify release checksums, exact asset/member allowlists and private data.

No archive is extracted or executed. --partial permits one platform for local
packaging tests; the release workflow always verifies all four platforms.
"""
from __future__ import annotations

import argparse
import hashlib
import posixpath
import re
import runpy
import stat
import sys
import tarfile
import zipfile
from pathlib import Path, PurePosixPath
from urllib.parse import unquote

PRIVACY = runpy.run_path(str(Path(__file__).with_name("check-privacy.py")))
PLATFORMS = ("linux-x86_64", "windows-x86_64", "macos-x86_64", "macos-aarch64")
BINARIES = ("vpn233-provider-server", "vpn233-subscribe-server", "vpn233-client")
OPERATORS = tuple(name + suffix for name in ("install", "deploy", "update", "uninstall", "start", "stop") for suffix in (".sh", ".ps1"))
COMMON = {
    "version.txt", "README.md", "README.zh-CN.md", "LICENSE", "SECURITY.md", "CONTRIBUTING.md", "CHANGELOG.md", "CODE_OF_CONDUCT.md", "admin-web/dist/index.html",
    "docs/getting-started.md", "docs/deployment.md", "docs/mirrors.md", "docs/api.md", "docs/index.md", "docs/privacy.md", "docs/protocols.md", "docs/testing.md", "docs/troubleshooting.md", "docs/_config.yml",
    "examples/provider-server.yaml", "examples/subscribe-server.yaml", "examples/vpn233-client.yaml",
    "scripts/lib/mirrors.sh", "scripts/lib/mirrors.ps1", "scripts/lib/services.sh", "scripts/lib/layout.sh", "scripts/lib/layout.ps1", *OPERATORS,
}
WEB_SUFFIXES = {".html", ".js", ".css", ".svg", ".png", ".jpg", ".jpeg", ".webp", ".ico", ".woff", ".woff2", ".ttf", ".txt"}


def inspect_archive(archive: Path, version: str) -> list[str]:
    name = archive.name
    platform = name.removeprefix("vpn233-rust-").removesuffix(".tar.gz").removesuffix(".zip")
    if platform not in PLATFORMS:
        return [f"{name}: unsupported platform"]
    suffix = ".exe" if platform.startswith("windows-") else ""
    required = COMMON | {"bin/" + binary + suffix for binary in BINARIES}
    found: set[str] = set()
    markdown: dict[str, str] = {}
    problems: list[str] = []
    root = "vpn233-rust-" + platform

    def inspect(member: str, data: bytes | None, regular: bool = True) -> None:
        path = PurePosixPath(member)
        if "\\" in member or path.is_absolute() or ".." in path.parts or not path.parts or path.parts[0] != root:
            problems.append(f"{name}: unsafe archive member path")
            return
        if not regular:
            problems.append(f"{name}: links/devices are forbidden")
            return
        if data is None:
            return
        relative = "/".join(path.parts[1:])
        if relative in found:
            problems.append(f"{name}: duplicate archive member")
        found.add(relative)
        public_asset = relative.startswith("admin-web/dist/assets/") and path.suffix.lower() in WEB_SUFFIXES
        if relative not in required and not public_asset:
            problems.append(f"{name}: non-allowlisted member {relative}")
        if PRIVACY["sensitive_path"](relative):
            problems.append(f"{name}: sensitive-file-path {relative}")
        for rule in sorted(PRIVACY["content_rules"](data)):
            problems.append(f"{name}: {rule} in {relative} (value redacted)")
        if relative == "version.txt" and data.decode("ascii", "replace").strip() != version:
            problems.append(f"{name}: version.txt does not match the release version")
        if relative.endswith(".md"):
            markdown[relative] = data.decode("utf-8", "replace")

    if name.endswith(".zip"):
        with zipfile.ZipFile(archive) as opened:
            for member in opened.infolist():
                mode = member.external_attr >> 16
                regular = not stat.S_ISLNK(mode) and (not stat.S_IFMT(mode) or stat.S_ISREG(mode) or stat.S_ISDIR(mode))
                if member.file_size > 256 * 1024 * 1024:
                    problems.append(f"{name}: oversized member")
                else:
                    inspect(member.filename, None if member.is_dir() else opened.read(member), regular)
    else:
        with tarfile.open(archive, "r:gz") as opened:
            for member in opened:
                if member.size > 256 * 1024 * 1024:
                    problems.append(f"{name}: oversized member")
                else:
                    handle = opened.extractfile(member) if member.isfile() else None
                    inspect(member.name, handle.read() if handle else None, member.isfile() or member.isdir())
    for missing in sorted(required - found):
        problems.append(f"{name}: missing {missing}")
    for source, text in markdown.items():
        for target in re.findall(r"\]\(([^()\s]+)\)", text):
            if target.startswith(("http://", "https://", "mailto:", "tel:", "#")):
                continue
            clean = unquote(target.split("#", 1)[0].split("?", 1)[0])
            resolved = posixpath.normpath(posixpath.join(posixpath.dirname(source), clean))
            if clean and resolved not in found:
                problems.append(f"{name}: broken bundle link from {source} (target redacted)")
    return problems


def verify(directory: Path, version: str, partial: bool = False) -> list[str]:
    problems: list[str] = []
    if not re.fullmatch(r"\d+\.\d+\.\d+", version):
        return ["invalid semantic release version"]
    expected = {f"vpn233-rust-{platform}.{extension}" for platform in PLATFORMS for extension in ("tar.gz", "zip")}
    present = {path.name for path in directory.iterdir() if path.is_file()}
    archives = present & expected
    if partial:
        if not archives:
            problems.append("no platform archives found")
        expected = archives
    if present != expected | {"SHA256SUMS", "version.txt"}:
        problems.append("release asset set differs from the exact allowlist")
    manifest = directory / "SHA256SUMS"
    if not manifest.is_file():
        return problems + ["SHA256SUMS missing"]
    entries: dict[str, str] = {}
    for line in manifest.read_text(encoding="utf-8").splitlines():
        match = re.fullmatch(r"([a-fA-F0-9]{64}) [ *]([^/\\]+)", line)
        if not match:
            problems.append("invalid SHA256SUMS entry")
            continue
        digest, name = match.groups()
        if name in entries:
            problems.append("duplicate SHA256SUMS entry")
        entries[name] = digest.lower()
    if set(entries) != expected:
        problems.append("SHA256SUMS must cover each expected archive exactly once")
    if not (directory / "version.txt").is_file() or (directory / "version.txt").read_text().strip() != version:
        problems.append("release version.txt mismatch")
    for name in sorted(archives):
        archive = directory / name
        digest = hashlib.sha256(archive.read_bytes()).hexdigest()
        if entries.get(name) != digest:
            problems.append(f"{name}: SHA256 mismatch")
            continue
        problems.extend(inspect_archive(archive, version))
    return problems


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("directory", type=Path)
    parser.add_argument("version")
    parser.add_argument("--partial", action="store_true")
    args = parser.parse_args()
    try:
        problems = verify(args.directory, args.version.removeprefix("v"), args.partial)
    except (OSError, ValueError, tarfile.TarError, zipfile.BadZipFile):
        problems = ["release validation could not complete (archive contents redacted)"]
    for problem in problems:
        print("FAIL " + problem)
    print(f"release assets: {'FAILED' if problems else 'PASSED'} ({len(problems)} finding(s))")
    return 1 if problems else 0


if __name__ == "__main__":
    sys.exit(main())
