#!/usr/bin/env python3
"""Version-chain preparation and validation; never commits or pushes."""
from __future__ import annotations

import argparse
import json
import re
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]


def current_version() -> str:
    text = (ROOT / "Cargo.toml").read_text(encoding="utf-8")
    match = re.search(r'(?ms)(\[workspace\.package\]\s*.*?^version\s*=\s*")([^"]+)(")', text)
    if not match:
        raise ValueError("workspace.package.version missing")
    return match.group(2)


def check() -> None:
    expected = current_version()
    values = {
        "version.txt": (ROOT / "version.txt").read_text(encoding="utf-8-sig").strip(),
        "admin-web/package.json": json.loads((ROOT / "admin-web/package.json").read_text())["version"],
    }
    lock = json.loads((ROOT / "admin-web/package-lock.json").read_text())
    values["admin-web/package-lock.json"] = lock["version"]
    values["admin-web/package-lock.json root package"] = lock["packages"][""]["version"]
    for file, value in values.items():
        if value != expected:
            raise ValueError(f"version drift in {file}; use scripts/release.ps1 -Version {expected} or scripts/release.sh --version {expected}")
    print(f"version chain: {expected} (all metadata consistent)")


def prepare(version: str) -> None:
    if not re.fullmatch(r"(?:0|[1-9]\d*)\.(?:0|[1-9]\d*)\.(?:0|[1-9]\d*)", version):
        raise ValueError("version must be MAJOR.MINOR.PATCH without a v prefix")
    cargo = ROOT / "Cargo.toml"
    cargo_text = re.sub(r'(?ms)(\[workspace\.package\]\s*.*?^version\s*=\s*")[^"]+(")', lambda m: m.group(1) + version + m.group(2), cargo.read_text(encoding="utf-8"), count=1)
    # Parse all JSON before writing any file, so malformed input cannot leave a
    # partly updated source-of-truth chain.
    package = json.loads((ROOT / "admin-web/package.json").read_text())
    lock = json.loads((ROOT / "admin-web/package-lock.json").read_text())
    package["version"] = lock["version"] = lock["packages"][""]["version"] = version
    changes = {cargo: cargo_text, ROOT / "version.txt": version + "\n", ROOT / "admin-web/package.json": json.dumps(package, indent=2, ensure_ascii=False) + "\n", ROOT / "admin-web/package-lock.json": json.dumps(lock, indent=2, ensure_ascii=False) + "\n"}
    # Cargo.lock stores the local workspace versions too. No dependency update
    # occurs when preparing a release.
    cargo_lock = ROOT / "Cargo.lock"
    local_names = {"vpn233-provider-server", "vpn233-subscribe-server", "vpn233-client", "vpn233-core-lib-rust"}
    changes[cargo_lock] = re.sub(r'(?m)(^name = "([^"]+)"\nversion = ")[^"]+("\n)', lambda m: m.group(1) + version + m.group(3) if m.group(2) in local_names else m.group(0), cargo_lock.read_text(encoding="utf-8"))
    for path, text in changes.items():
        path.write_text(text, encoding="utf-8", newline="\n")
    check()
    print("prepared locally; review and commit the version change before publishing")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--prepare", metavar="VERSION")
    options = parser.parse_args()
    try:
        prepare(options.prepare) if options.prepare else check()
    except (ValueError, OSError, KeyError) as error:
        parser.exit(1, str(error) + "\n")
