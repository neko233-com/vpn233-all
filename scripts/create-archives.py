#!/usr/bin/env python3
"""Create deterministic tar.gz and zip bundles using fast level-1 compression."""
import argparse
import datetime
import gzip
import os
import tarfile
import zipfile
from pathlib import Path


def create(bundle: Path, output: Path, epoch: int) -> None:
    output.mkdir(parents=True, exist_ok=True)
    paths = [bundle, *sorted(bundle.rglob("*"))]
    if any(path.is_symlink() for path in paths):
        raise ValueError("archive input must not contain symlinks")
    with (output / (bundle.name + ".tar.gz")).open("wb") as stream:
        with gzip.GzipFile(filename="", mode="wb", fileobj=stream, compresslevel=1, mtime=epoch) as compressed:
            with tarfile.open(fileobj=compressed, mode="w", format=tarfile.PAX_FORMAT) as archive:
                for path in paths:
                    relative = path.relative_to(bundle.parent).as_posix()
                    item = archive.gettarinfo(str(path), arcname=relative)
                    item.uid = item.gid = 0
                    item.uname = item.gname = ""
                    item.mtime = epoch
                    item.mode = 0o755 if path.is_dir() or path.suffix in (".sh", ".exe") or path.parent.name == "bin" else 0o644
                    if path.is_file():
                        with path.open("rb") as source:
                            archive.addfile(item, source)
                    else:
                        archive.addfile(item)
    date = datetime.datetime.fromtimestamp(max(epoch, 315532800), datetime.timezone.utc).timetuple()[:6]
    with zipfile.ZipFile(output / (bundle.name + ".zip"), "w", compression=zipfile.ZIP_DEFLATED, compresslevel=1) as archive:
        for path in paths:
            name = path.relative_to(bundle.parent).as_posix() + ("/" if path.is_dir() else "")
            member = zipfile.ZipInfo(name, date_time=date)
            member.create_system = 3
            mode = 0o755 if path.is_dir() or path.suffix in (".sh", ".exe") or path.parent.name == "bin" else 0o644
            member.external_attr = ((0o040000 if path.is_dir() else 0o100000) | mode) << 16
            member.compress_type = zipfile.ZIP_DEFLATED
            archive.writestr(member, path.read_bytes() if path.is_file() else b"", compresslevel=1)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("bundle", type=Path)
    parser.add_argument("output", type=Path)
    options = parser.parse_args()
    create(options.bundle.resolve(), options.output.resolve(), int(os.environ.get("SOURCE_DATE_EPOCH", "315532800")))
