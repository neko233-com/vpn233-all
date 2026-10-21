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
import struct
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
VC_RUNTIME = re.compile(r"(?:vcruntime.*|msvcp.*|msvcr(?:\d.*|td)|concrt\d.*|vcomp\d.*|vccorlib\d.*|mfcm?\d.*|atl\d.*)\.dll", re.IGNORECASE)


def pe_imports(data: bytes) -> set[str]:
    """Read bounded AMD64 PE32+ import tables without loading the executable.

    Layouts: https://learn.microsoft.com/windows/win32/debug/pe-format
    Both ordinary and delay-loaded imports must meet the runtime dependency gate.
    """
    def unpack(layout: str, offset: int) -> tuple:
        size = struct.calcsize(layout)
        if offset < 0 or offset + size > len(data):
            raise ValueError("invalid PE")
        return struct.unpack_from(layout, data, offset)

    if len(data) < 64 or data[:2] != b"MZ":
        raise ValueError("invalid PE")
    pe = unpack("<I", 0x3C)[0]
    if pe < 64 or data[pe:pe + 4] != b"PE\0\0":
        raise ValueError("invalid PE")
    machine, count, _, _, _, optional_size, flags = unpack("<HHIIIHH", pe + 4)
    optional = pe + 24
    sections_start = optional + optional_size
    if machine != 0x8664 or not 1 <= count <= 96 or not flags & 2 or flags & 0x2000:
        raise ValueError("invalid PE")
    if optional_size < 112 or sections_start + count * 40 > len(data) or unpack("<H", optional)[0] != 0x20B:
        raise ValueError("invalid PE")
    entry = unpack("<I", optional + 16)[0]
    headers = unpack("<I", optional + 60)[0]
    directory_count = unpack("<I", optional + 108)[0]
    if not 2 <= directory_count <= 16 or 112 + directory_count * 8 > optional_size:
        raise ValueError("invalid PE")
    if not sections_start + count * 40 <= headers <= len(data):
        raise ValueError("invalid PE")
    sections: list[tuple[int, int, int, int]] = []
    virtual_ranges = [(0, headers)]
    raw_ranges = [(0, headers)]
    executable_entry = False
    for number in range(count):
        start = sections_start + number * 40
        virtual_size, rva, raw_size, raw = unpack("<IIII", start + 8)
        section_flags = unpack("<I", start + 36)[0]
        span = max(virtual_size, raw_size)
        if not span or rva + span > 0x100000000 or raw + raw_size > len(data):
            raise ValueError("invalid PE")
        if any(rva < end and rva + span > begin for begin, end in virtual_ranges):
            raise ValueError("invalid PE")
        if raw_size and any(raw < end and raw + raw_size > begin for begin, end in raw_ranges):
            raise ValueError("invalid PE")
        virtual_ranges.append((rva, rva + span))
        if raw_size:
            raw_ranges.append((raw, raw + raw_size))
        sections.append((rva, span, raw, raw_size))
        executable_entry |= bool(section_flags & 0x20000000 and rva <= entry < rva + raw_size)
    if not executable_entry:
        raise ValueError("invalid PE")

    def locate(rva: int, size: int) -> tuple[int, int]:
        if rva < 0 or size < 1 or rva + size > 0x100000000:
            raise ValueError("invalid PE")
        if rva + size <= headers:
            return rva, headers
        for base, _, raw, raw_size in sections:
            if base <= rva and rva + size <= base + raw_size:
                return raw + rva - base, raw + raw_size
        raise ValueError("invalid PE")

    def dll_name(rva: int) -> str:
        start, end = locate(rva, 1)
        terminator = data.find(b"\0", start, min(end, start + 261))
        if terminator < 0:
            raise ValueError("invalid PE")
        value = data[start:terminator].decode("ascii")
        if not re.fullmatch(r"[A-Za-z0-9_.-]+\.dll", value, re.IGNORECASE):
            raise ValueError("invalid PE")
        return value.lower()

    imports: set[str] = set()
    for index, width in ((1, 20), (13, 32)):
        if index >= directory_count:
            continue
        rva, size = unpack("<II", optional + 112 + index * 8)
        if rva == size == 0:
            continue
        if not rva or size < width or size > 1024 * 1024:
            raise ValueError("invalid PE")
        start, _ = locate(rva, size)
        for offset in range(0, size - width + 1, width):
            descriptor = unpack("<" + "I" * (width // 4), start + offset)
            if not any(descriptor):
                break
            if index == 13:
                # Modern PE32+ delay descriptors use RVAs; reject legacy pointer
                # encodings instead of silently overlooking their dependencies.
                if descriptor[0] != 1:
                    raise ValueError("invalid PE")
                name = descriptor[1]
            else:
                name = descriptor[3]
            imports.add(dll_name(name))
        else:
            raise ValueError("invalid PE")
    return imports


def windows_binary_rules(data: bytes) -> set[str]:
    try:
        imports = pe_imports(data)
    except (ValueError, struct.error, UnicodeError):
        return {"windows-invalid-amd64-pe"}
    return {"windows-extra-vc-runtime"} if any(VC_RUNTIME.fullmatch(name) for name in imports) else set()


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
        if suffix and relative in {"bin/" + binary + suffix for binary in BINARIES}:
            for rule in sorted(windows_binary_rules(data)):
                problems.append(f"{name}: {rule} in {relative} (dependency redacted)")
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
