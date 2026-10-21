#!/usr/bin/env python3
"""Offline privacy gate. Reports paths/rule names, never matching values.

Default: all Git candidates (tracked + nonignored untracked), plus the index.
--history also examines all reachable Git blobs. Ignored local configuration is
deliberately not read or printed. Use --artifacts for a generated public tree.
This complements gitleaks; it does not replace its credential rules.
"""
from __future__ import annotations

import argparse
import ipaddress
import re
import subprocess
import sys
import xml.etree.ElementTree as ET
from pathlib import Path, PurePosixPath

ROOT = Path(__file__).resolve().parents[1]
RULES = {
    "github-token": re.compile(rb"(?:gh[pousr]_[A-Za-z0-9]{20,}|github_pat_[A-Za-z0-9_]{20,})"),
    "aws-access-key": re.compile(rb"(?:AKIA|ASIA)[A-Z0-9]{16}"),
    "slack-token": re.compile(rb"xox[baprs]-[A-Za-z0-9-]{10,}"),
    "private-key": re.compile(rb"-----BEGIN (?:[A-Z0-9]+ )*PRIVATE KEY-----"),
    "machine-home-path": re.compile(rb"(?:[A-Za-z]:[\\/](?:Users|Documents and Settings)[\\/][^\s\\/'\"<>]+|/(?:home|Users)/[^\s/'\"<>]+)"),
    "credential-in-url": re.compile(rb"https?://[^\s/@:'\"]+:[^\s/@'\"]+@(?P<host>[A-Za-z0-9.-]+)"),
}
IPV4 = re.compile(rb"(?<![\w.])(?:\d{1,3}\.){3}\d{1,3}(?![\w.])")
IPV6 = re.compile(rb"(?<![A-Za-z0-9:])(?:[A-Fa-f0-9]{1,4}:){2,}[A-Fa-f0-9:]+(?![A-Za-z0-9:])")
DNS_IPS = {"1.1.1.1", "1.0.0.1", "8.8.8.8", "8.8.4.4", "9.9.9.9", "149.112.112.112", "223.5.5.5", "223.6.6.6", "119.29.29.29", "114.114.114.114", "114.114.115.115"}
DNS_IPV6 = {"2606:4700:4700::1111", "2606:4700:4700::1001", "2001:4860:4860::8888", "2001:4860:4860::8844"}
DOC_NETWORKS = tuple(ipaddress.ip_network(n) for n in ("192.0.2.0/24", "198.51.100.0/24", "203.0.113.0/24", "198.18.0.0/15"))
FORBIDDEN_DIRS = {"config", "data", "runtime", "logs", "backups", "secrets", ".ssh"}
FORBIDDEN_SUFFIXES = {".pem", ".key", ".ppk", ".p12", ".pfx", ".log", ".session"}
SVG_NAMESPACE = "http://www.w3.org/2000/svg"
PATH_TOKEN = re.compile(r"[MmZzLlHhVvCcSsQqTtAa]|[+-]?(?:\d+(?:\.\d*)?|\.\d+)(?:[eE][+-]?\d+)?")


def valid_svg_path(value: str) -> bool:
    """Validate command arity and numeric tokens before excluding geometry."""
    arities = {"m": 2, "l": 2, "h": 1, "v": 1, "c": 6, "s": 4, "q": 4, "t": 2, "a": 7, "z": 0}
    tokens = []
    offset = 0
    for match in PATH_TOKEN.finditer(value):
        if not re.fullmatch(r"[\s,]*", value[offset:match.start()]):
            return False
        tokens.append(match.group())
        offset = match.end()
    if not re.fullmatch(r"[\s,]*", value[offset:]) or not tokens or tokens[0] not in ("M", "m"):
        return False
    index = 0
    while index < len(tokens):
        command = tokens[index].lower()
        if command not in arities:
            return False
        index += 1
        start = index
        while index < len(tokens) and tokens[index].lower() not in arities:
            index += 1
        count = index - start
        arity = arities[command]
        if (arity == 0 and count != 0) or (arity != 0 and (count == 0 or count % arity != 0)):
            return False
        if command == "a":
            for position in range(start, index, arity):
                if tokens[position + 3] not in ("0", "1") or tokens[position + 4] not in ("0", "1"):
                    return False
    return True


def without_svg_geometry(data: bytes) -> bytes:
    if not data.lstrip().startswith(b"<"):
        return data
    try:
        root = ET.fromstring(data)
    except ET.ParseError:
        return data
    if root.tag not in ("svg", "{" + SVG_NAMESPACE + "}svg"):
        return data
    for node in root.iter():
        if node.tag in ("path", "{" + SVG_NAMESPACE + "}path") and "d" in node.attrib and valid_svg_path(node.attrib["d"]):
            node.attrib["d"] = ""
    return ET.tostring(root, encoding="utf-8")


def git(*args: str) -> bytes:
    return subprocess.run(["git", "-C", str(ROOT), *args], check=True, stdout=subprocess.PIPE, stderr=subprocess.PIPE).stdout


def git_objects(oids: list[str]) -> dict[str, tuple[str, bytes]]:
    """One batch process keeps quick/history scans fast on Windows too."""
    if not oids:
        return {}
    output = subprocess.run(["git", "-C", str(ROOT), "cat-file", "--batch"], input=("\n".join(oids) + "\n").encode("ascii"), check=True, stdout=subprocess.PIPE, stderr=subprocess.PIPE).stdout
    result = {}
    offset = 0
    for _ in oids:
        end = output.index(b"\n", offset)
        oid, kind, size_text = output[offset:end].split()
        size = int(size_text)
        offset = end + 1
        result[oid.decode("ascii")] = (kind.decode("ascii"), output[offset:offset + size])
        offset += size + 1
    return result


def sensitive_path(name: str) -> bool:
    path = PurePosixPath(name.replace("\\", "/"))
    return (
        any(part.lower() in FORBIDDEN_DIRS for part in path.parts[:-1])
        or path.name in {"config.yaml", "config.json", "links.txt", "node-manifest.json", ".vpn233-layout"}
        or (path.name.startswith(".env") and path.name != ".env.example")
        or path.suffix.lower() in FORBIDDEN_SUFFIXES
        or path.name.endswith((".install.sh", ".install.ps1", ".credentials", ".secret"))
    )


def content_rules(data: bytes) -> set[str]:
    found = set()
    for name, expression in RULES.items():
        for match in expression.finditer(data):
            # Literal example.invalid hosts are nonresolving test/doc fixtures;
            # do not exempt a file or any other credential pattern.
            if name == "credential-in-url" and (match.group("host").lower() == b"example.invalid" or match.group("host").lower().endswith(b".example.invalid")):
                continue
            found.add(name)
    # Executables and images are still scanned for tokens/home paths, but not IP
    # fragments that happen to occur in binary data.
    if b"\0" not in data[:8192]:
        ip_data = without_svg_geometry(data)
        for match in IPV4.finditer(ip_data):
            value = match.group().decode("ascii")
            try:
                ip = ipaddress.ip_address(value)
            except ValueError:
                continue
            if ip.is_global and value not in DNS_IPS and not any(ip in net for net in DOC_NETWORKS):
                found.add("non-example-public-ipv4")
        for match in IPV6.finditer(ip_data):
            try:
                ip = ipaddress.ip_address(match.group().decode("ascii"))
            except ValueError:
                continue
            if ip.is_global and str(ip) not in DNS_IPV6:
                found.add("non-example-public-ipv6")
    return found


def main() -> int:
    global ROOT
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--repository", type=Path, default=ROOT, help="Git repository or bare mirror to scan")
    parser.add_argument("--history", action="store_true", help="scan reachable blobs, commit metadata and annotated tags")
    parser.add_argument("--history-only", action="store_true", help="only history; supports a bare cleanup mirror")
    parser.add_argument("--artifacts", action="append", default=[], metavar="DIRECTORY", help="scan an extracted public artifact tree")
    parser.add_argument("--artifacts-only", action="store_true")
    options = parser.parse_args()
    ROOT = options.repository.resolve()
    findings: set[tuple[str, str]] = set()
    count = 0

    def inspect(name: str, data: bytes, path_gate: bool = True) -> None:
        nonlocal count
        count += 1
        canonical_path = name.removeprefix("index:").removeprefix("history:").removeprefix("artifact:")
        if path_gate and sensitive_path(canonical_path):
            findings.add((name, "sensitive-file-path"))
        for rule in content_rules(data):
            findings.add((name, rule))

    if not options.artifacts_only and not options.history_only:
        paths = git("ls-files", "-z", "--cached", "--others", "--exclude-standard").split(b"\0")
        for raw in sorted(set(filter(None, paths))):
            name = raw.decode("utf-8", "surrogateescape")
            path = ROOT / name
            if path.is_file():
                inspect(name, path.read_bytes())
        # Also scan the index: a clean worktree must not hide a staged secret.
        index_entries = []
        for entry in git("ls-files", "--stage", "-z").split(b"\0"):
            if not entry:
                continue
            metadata, raw = entry.split(b"\t", 1)
            oid = metadata.split()[1].decode("ascii")
            index_entries.append((oid, raw))
        objects = git_objects([oid for oid, _ in index_entries])
        for oid, raw in index_entries:
            inspect("index:" + raw.decode("utf-8", "surrogateescape"), objects[oid][1])
    if options.history or options.history_only:
        historical_entries = []
        for entry in git("rev-list", "--objects", "--all").splitlines():
            oid, _, raw = entry.partition(b" ")
            historical_entries.append((oid.decode("ascii"), raw))
        objects = git_objects([oid for oid, _ in historical_entries])
        for oid, raw in historical_entries:
            kind, data = objects[oid]
            if kind == "blob" and raw:
                # Historical filenames alone are not a secret. Current Git
                # candidates still enforce the sensitive-path exclusion.
                inspect("history:" + raw.decode("utf-8", "surrogateescape"), data, path_gate=False)
            elif kind in ("commit", "tag"):
                # Messages and annotated tags are also public after a push.
                # Report only their object identity, never their contents.
                inspect(f"history:{kind}:{oid[:12]}", data, path_gate=False)

    for directory in options.artifacts:
        tree = Path(directory).resolve()
        if not tree.is_dir():
            parser.error("artifact directory does not exist")
        for path in sorted(tree.rglob("*")):
            name = "artifact:" + path.relative_to(tree).as_posix()
            if path.is_symlink():
                findings.add((name, "artifact-symlink"))
            elif path.is_file():
                inspect(name, path.read_bytes())

    for name, rule in sorted(findings):
        print(f"FAIL {rule}: {name}")
    print(f"privacy: {count} candidates checked, {len(findings)} finding(s); values redacted")
    return 1 if findings else 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except (OSError, subprocess.CalledProcessError) as error:
        print(f"privacy gate could not complete ({type(error).__name__}); refusing success", file=sys.stderr)
        raise SystemExit(2)
