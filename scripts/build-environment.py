#!/usr/bin/env python3
"""Configure reproducible Rust path remapping without storing machine paths.

Default emits CARGO_ENCODED_RUSTFLAGS for a calling shell to capture. --github
writes it directly to GITHUB_ENV; neither mode is intended for logging.
"""
import argparse
import os
import shlex
from pathlib import Path


def encoded_flags() -> str:
    separator = chr(31)
    encoded = os.environ.get("CARGO_ENCODED_RUSTFLAGS", "")
    flags = encoded.split(separator) if encoded else shlex.split(os.environ.get("RUSTFLAGS", ""), posix=os.name != "nt")
    mappings = [(Path.home(), "/user"), (Path(__file__).resolve().parents[1], "/vpn233")]
    for key, replacement in (("CARGO_HOME", "/cargo"), ("RUSTUP_HOME", "/rustup")):
        if os.environ.get(key):
            mappings.append((Path(os.environ[key]).resolve(), replacement))
    for path, replacement in mappings:
        for spelling in sorted({str(path), path.as_posix()}):
            flag = f"--remap-path-prefix={spelling}={replacement}"
            if flag not in flags:
                flags.append(flag)
    return separator.join(flags)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--github", action="store_true")
    options = parser.parse_args()
    if options.github:
        with Path(os.environ["GITHUB_ENV"]).open("a", encoding="utf-8") as output:
            output.write("CARGO_ENCODED_RUSTFLAGS=" + encoded_flags() + "\n")
    else:
        print(encoded_flags())
