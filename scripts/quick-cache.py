#!/usr/bin/env python3
"""Local, content-addressed successful Quick reports. Never used by Full or CI.

Only hashes and public timing/check counts are persisted. Source, dependency,
tool, configuration and environment values are never printed or saved here.
"""
from __future__ import annotations

import argparse
from concurrent.futures import ThreadPoolExecutor
from datetime import datetime, timezone
import hashlib
import json
import math
import os
from pathlib import Path
import platform
import shutil
import stat
import subprocess
import sys
import uuid

SCHEMA = 1
ROOT = Path(__file__).resolve().parents[1]
ENVIRONMENT_KEYS = {
    "PATH", "PATHEXT", "PSModulePath", "LANG", "LC_ALL", "TZ", "SOURCE_DATE_EPOCH",
    "RUSTFLAGS", "CARGO_ENCODED_RUSTFLAGS", "CARGO_TARGET_DIR", "CARGO_HOME", "RUSTUP_HOME",
    "RUSTUP_TOOLCHAIN", "RUSTC", "RUSTC_WRAPPER", "RUSTC_WORKSPACE_WRAPPER", "RUST_TEST_THREADS",
    "RUST_TEST_NOCAPTURE", "CC", "CXX", "AR", "LIB", "INCLUDE", "CL", "LINK",
    "NODE_ENV", "NODE_OPTIONS", "HTTP_PROXY", "HTTPS_PROXY", "ALL_PROXY", "NO_PROXY",
    "CFLAGS", "CXXFLAGS", "CPPFLAGS", "LDFLAGS", "LIBPATH", "EXTERNAL_INCLUDE", "BASH_ENV",
}
ENVIRONMENT_KEYS = {key.upper() for key in ENVIRONMENT_KEYS}


class CacheUnavailable(Exception):
    pass


def digest(value: bytes) -> str:
    return hashlib.sha256(value).hexdigest()


def canonical(value: object) -> bytes:
    return json.dumps(value, sort_keys=True, separators=(",", ":"), ensure_ascii=True).encode()


def file_digest(path: Path) -> str:
    hashed = hashlib.sha256()
    with path.open("rb") as source:
        for chunk in iter(lambda: source.read(1024 * 1024), b""):
            hashed.update(chunk)
    return hashed.hexdigest()


def tree_digest(root: Path) -> str:
    """Hash every byte, including ignored dependencies; no mtime shortcuts."""
    if not root.is_dir():
        return "missing"
    hashed = hashlib.sha256()
    queue = [root]
    while queue:
        folder = queue.pop()
        with os.scandir(folder) as iterator:
            entries = sorted(iterator, key=lambda item: item.name)
        for entry in entries:
            path = Path(entry.path)
            name = path.relative_to(root).as_posix().encode()
            hashed.update(str(stat.S_IMODE(entry.stat(follow_symlinks=False).st_mode)).encode() + b"\0")
            if entry.is_symlink():
                target = path.resolve(strict=True)
                if not target.is_relative_to(root.resolve()):
                    raise CacheUnavailable("dependency-link-outside-tree")
                hashed.update(b"link\0" + name + b"\0" + os.readlink(path).encode())
            elif entry.is_dir(follow_symlinks=False):
                hashed.update(b"directory\0" + name + b"\0")
                queue.append(path)
            elif entry.is_file(follow_symlinks=False):
                hashed.update(b"file\0" + name + b"\0" + file_digest(path).encode())
            else:
                raise CacheUnavailable("unsupported-dependency-file")
    return hashed.hexdigest()


def run(root: Path, arguments: list[str]) -> bytes:
    return subprocess.run(arguments, cwd=root, stdout=subprocess.PIPE, stderr=subprocess.PIPE, check=True, timeout=20).stdout


def bash_path() -> str:
    if os.name == "nt":
        for directory in (os.environ.get("ProgramFiles", ""), os.environ.get("ProgramFiles(x86)", "")):
            candidate = Path(directory) / "Git/bin/bash.exe"
            if candidate.is_file():
                return str(candidate)
    found = shutil.which("bash")
    if not found:
        raise CacheUnavailable("required-tool-unavailable")
    return found


def tool_evidence(root: Path, flavor: str, engine: str) -> dict:
    shell = engine if flavor == "powershell" else shutil.which("pwsh")
    if not shell:
        raise CacheUnavailable("required-tool-unavailable")
    module_command = "$m = Import-Module PSScriptAnalyzer -PassThru -ErrorAction Stop; [pscustomobject]@{ version=$PSVersionTable.PSVersion.ToString(); module_version=$m.Version.ToString(); module_base=$m.ModuleBase } | ConvertTo-Json -Compress"
    commands = {
        "git": ["git", "--version"], "cargo": ["cargo", "--version"],
        "rustc": ["rustc", "-vV"], "rustfmt": ["rustfmt", "--version"],
        "clippy": ["cargo", "clippy", "--version"], "node": ["node", "--version"],
        "npm": ["npm", "--version"], "bash": [bash_path(), "--version"],
        "shellcheck": ["shellcheck", "--version"], "actionlint": ["actionlint", "-version"],
        "psmodule": [shell, "-NoProfile", "-ExecutionPolicy", "Bypass", "-Command", module_command],
    }
    if flavor == "powershell":
        commands["powershell-fixtures"] = ["powershell", "-NoProfile", "-Command", "$PSVersionTable.PSVersion.ToString()"]
    if sys.platform.startswith("linux"):
        commands["systemd-analyze"] = ["systemd-analyze", "--version"]

    def collect(item: tuple[str, list[str]]) -> tuple[str, dict]:
        name, command = item
        executable = shutil.which(command[0])
        if not executable:
            raise CacheUnavailable("required-tool-unavailable")
        command[0] = executable
        output = run(root, command)
        path = Path(executable)
        files = {str(path): file_digest(path)}
        # PowerShell can select npm.ps1 while Python resolves npm.cmd.
        for suffix in (".exe", ".cmd", ".ps1", ".bat"):
            sibling = path.with_suffix(suffix)
            if sibling.is_file():
                files[str(sibling)] = file_digest(sibling)
        result = {"files": files, "output": digest(output)}
        if name == "psmodule":
            metadata = json.loads(output.decode("utf-8-sig"))
            result["module"] = tree_digest(Path(metadata["module_base"]))
        return name, result

    with ThreadPoolExecutor(max_workers=8) as executor:
        return dict(executor.map(collect, commands.items()))


def dependency_marker_valid(root: Path) -> bool:
    lock = root / "admin-web/package-lock.json"
    marker = root / "admin-web/node_modules/.vpn233-lock.sha256"
    return lock.is_file() and marker.is_file() and marker.read_text().strip().lower() == file_digest(lock)


def source_fingerprint(root: Path, flavor: str, engine: str, *, evidence: dict | None = None, environment: dict | None = None) -> str:
    root = root.resolve()
    environment = os.environ if environment is None else environment
    candidate_output = run(root, ["git", "ls-files", "--cached", "--others", "--exclude-standard", "-z"])
    candidates = sorted(set(filter(None, candidate_output.split(b"\0"))))
    source = hashlib.sha256()
    for raw in candidates:
        name = raw.decode("utf-8", "surrogateescape")
        path = root / name
        source.update(raw + b"\0")
        if path.is_symlink():
            target = path.resolve(strict=True)
            if not target.is_relative_to(root):
                raise CacheUnavailable("source-link-outside-repository")
            source.update(b"link\0" + os.readlink(path).encode())
            if target.is_file():
                source.update(file_digest(target).encode())
        elif path.is_file():
            source.update(str(stat.S_IMODE(path.stat().st_mode)).encode() + b"\0" + file_digest(path).encode())
        elif not path.exists():
            source.update(b"deleted")
        else:
            raise CacheUnavailable("unsupported-source-file")
    source.update(run(root, ["git", "ls-files", "--stage", "-z"]))
    source.update(run(root, ["git", "rev-list", "--all", "--objects"]))
    source.update(run(root, ["git", "show-ref", "--head"]))
    source.update(run(root, ["git", "config", "--list", "--show-origin"]))
    scoped_environment = {
        key: value for key, value in environment.items()
        if key.upper() in ENVIRONMENT_KEYS or key.upper().startswith(("NPM_", "CARGO_", "RUST", "NODE_", "GIT_CONFIG_", "VITE_", "VPN233_", "CC_", "CXX_", "AR_", "CFLAGS_", "CXXFLAGS_", "CPPFLAGS_", "LDFLAGS_", "BASH_", "MSYS"))
    }
    configuration = []
    for path in sorted((root / "admin-web").glob(".env*")):
        if path.is_file():
            configuration.append((str(path), file_digest(path)))
    explicit_files = [(root / "admin-web/.npmrc")]
    explicit_files.extend(Path(value) for key, value in environment.items() if key.upper() in ("NPM_CONFIG_USERCONFIG", "NPM_CONFIG_GLOBALCONFIG") and value)
    for path in explicit_files:
        if path.is_file():
            configuration.append((str(path), file_digest(path)))
    for folder in (root, *root.parents, Path.home(), Path(environment.get("CARGO_HOME", str(Path.home() / ".cargo")))):
        paths = [folder / ".cargo/config", folder / ".cargo/config.toml"]
        if folder == root or folder == Path.home():
            paths.append(folder / ".npmrc")
        if folder == Path(environment.get("CARGO_HOME", str(Path.home() / ".cargo"))):
            paths.extend((folder / "config", folder / "config.toml"))
        for path in paths:
            if path.is_file():
                configuration.append((str(path), file_digest(path)))
    context = {
        "schema": SCHEMA, "flavor": flavor, "engine": engine,
        "system": (sys.version, sys.executable, platform.platform(), platform.machine()),
        "environment": scoped_environment, "configuration": configuration,
        "tools": tool_evidence(root, flavor, engine) if evidence is None else evidence,
    }
    source.update(canonical(context))
    return source.hexdigest()


def fingerprints(root: Path, flavor: str, engine: str, *, evidence: dict | None = None, environment: dict | None = None) -> dict:
    # Independent full dependency reads overlap native tool/version collection.
    with ThreadPoolExecutor(max_workers=2) as executor:
        dependencies = executor.submit(tree_digest, root / "admin-web/node_modules")
        output = executor.submit(tree_digest, root / "admin-web/dist")
        # Collect native tool evidence once; the second source pass only reads
        # candidate/index/history/config/environment inputs after workers finish.
        evidence = tool_evidence(root, flavor, engine) if evidence is None else evidence
        source = source_fingerprint(root, flavor, engine, evidence=evidence, environment=environment)
        result = {"source": source, "dependencies": dependencies.result(), "output": output.result(), "dependencies_were_ready": dependency_marker_valid(root)}
        if source != source_fingerprint(root, flavor, engine, evidence=evidence, environment=environment):
            raise CacheUnavailable("source-changed-during-fingerprint")
    result["fingerprint"] = digest(canonical({key: result[key] for key in ("source", "dependencies", "output")}))
    return result


def cache_disabled(environment: dict) -> bool:
    values = {key.upper(): value for key, value in environment.items()}
    return values.get("CI", "").lower() not in ("", "0", "false") or bool(values.get("GITHUB_ACTIONS")) or any(values.get(key) for key in ("BASH_ENV", "NODE_OPTIONS", "RUSTC", "RUSTC_WRAPPER", "RUSTC_WORKSPACE_WRAPPER"))


def cache_directory(root: Path) -> Path:
    directory = root / ".cache/quick-verify"
    if directory.resolve().is_relative_to(root.resolve()) and not directory.is_symlink():
        directory.mkdir(parents=True, exist_ok=True)
        return directory
    raise CacheUnavailable("unsafe-cache-directory")


def read_record(path: Path) -> dict | None:
    try:
        if path.is_symlink() or path.stat().st_size > 16384:
            return None
        record = json.loads(path.read_text())
        return record if isinstance(record, dict) else None
    except (OSError, ValueError):
        return None


def atomic_record(path: Path, value: dict) -> None:
    temporary = path.with_name(path.name + "." + uuid.uuid4().hex + ".tmp")
    try:
        with temporary.open("x", encoding="utf-8") as output:
            json.dump(value, output, sort_keys=True)
        temporary.replace(path)
    finally:
        temporary.unlink(missing_ok=True)


def report_valid(record: dict | None, fingerprint: str) -> bool:
    if not (record and record.get("schema") == SCHEMA and record.get("mode") == "quick" and record.get("scope") == "all-quick" and record.get("success") is True and record.get("fingerprint") == fingerprint and type(record.get("checks")) is int and record["checks"] > 0 and isinstance(record.get("completed_at"), str) and type(record.get("elapsed_seconds")) in (float, int) and math.isfinite(record["elapsed_seconds"]) and record["elapsed_seconds"] >= 0):
        return False
    try:
        stamp = datetime.fromisoformat(record["completed_at"])
        return stamp.tzinfo is not None and stamp.isoformat(timespec="seconds") == record["completed_at"]
    except ValueError:
        return False


def probe(root: Path, flavor: str, engine: str, *, evidence: dict | None = None, environment: dict | None = None) -> dict:
    environment = os.environ if environment is None else environment
    directory = cache_directory(root)
    report = directory / (flavor + "-success.json")
    if cache_disabled(environment):
        report.unlink(missing_ok=True)
        return {"hit": False, "ticket": "", "message": "Quick cache disabled in CI or custom execution environment; real checks required"}
    try:
        current = fingerprints(root, flavor, engine, evidence=evidence, environment=environment)
    except Exception:
        report.unlink(missing_ok=True)
        raise
    previous = read_record(report)
    if current["dependencies_were_ready"] and report_valid(previous, current["fingerprint"]):
        return {"hit": True, "ticket": "", "message": f'QUICK CACHED (not release validation): reused {previous["checks"]} successful checks from {previous["completed_at"]}; current privacy/version/diff gates ran live'}
    # A miss invalidates old success before any new check can fail or abort.
    report.unlink(missing_ok=True)
    ticket = uuid.uuid4().hex
    atomic_record(directory / (ticket + ".pending.json"), {"schema": SCHEMA, "flavor": flavor, "source": current["source"], "dependencies": current["dependencies"], "dependencies_were_ready": current["dependencies_were_ready"]})
    return {"hit": False, "ticket": ticket, "message": "Quick cache miss: real checks required"}


def finish(root: Path, flavor: str, engine: str, ticket: str, checks: int, elapsed: float, success: bool, *, evidence: dict | None = None, environment: dict | None = None) -> dict:
    directory = cache_directory(root)
    report = directory / (flavor + "-success.json")
    if not ticket or len(ticket) != 32 or any(character not in "0123456789abcdef" for character in ticket):
        raise CacheUnavailable("invalid-ticket")
    pending = directory / (ticket + ".pending.json")
    record = read_record(pending)
    pending.unlink(missing_ok=True)
    report.unlink(missing_ok=True)
    if not success or cache_disabled(os.environ if environment is None else environment):
        return {"stored": False, "message": "Quick cache invalidated after failure or interruption"}
    current = fingerprints(root, flavor, engine, evidence=evidence, environment=environment)
    if not record or record.get("schema") != SCHEMA or record.get("flavor") != flavor or record.get("source") != current["source"] or checks < 1 or not math.isfinite(elapsed) or elapsed < 0 or not current["dependencies_were_ready"] or not record.get("dependencies_were_ready") or record.get("dependencies") != current["dependencies"]:
        return {"stored": False, "message": "Quick inputs changed during verification; no reusable report was saved"}
    report_data = {"schema": SCHEMA, "mode": "quick", "scope": "all-quick", "success": True, "fingerprint": current["fingerprint"], "checks": checks, "elapsed_seconds": elapsed, "completed_at": datetime.now(timezone.utc).isoformat(timespec="seconds")}
    atomic_record(report, report_data)
    return {"stored": True, "message": "Quick successful report saved locally; Full/CI never reuse it"}


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("action", choices=("probe", "store", "invalidate", "clear"))
    parser.add_argument("--flavor", choices=("powershell", "bash"), required=True)
    parser.add_argument("--engine", required=True)
    parser.add_argument("--ticket", default="")
    parser.add_argument("--checks", type=int, default=0)
    parser.add_argument("--elapsed", type=float, default=0)
    args = parser.parse_args()
    try:
        if args.action == "clear":
            (cache_directory(ROOT) / (args.flavor + "-success.json")).unlink(missing_ok=True)
            result = {"stored": False, "message": "Quick cache cleared"}
        elif args.action == "probe":
            result = probe(ROOT, args.flavor, args.engine)
        else:
            result = finish(ROOT, args.flavor, args.engine, args.ticket, args.checks, args.elapsed, args.action == "store")
        print(json.dumps(result))
        return 0
    except (OSError, ValueError, TypeError, KeyError, CacheUnavailable, subprocess.SubprocessError):
        # Never include exception text: native tool output/config paths may be private.
        print(json.dumps({"hit": False, "ticket": "", "stored": False, "message": "Quick cache unavailable; real checks required"}))
        return 1


if __name__ == "__main__":
    sys.exit(main())
