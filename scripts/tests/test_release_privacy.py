"""Negative release/privacy cases. Every secret/IP fixture is synthetic."""
import hashlib
import io
import os
import runpy
import struct
import subprocess
import sys
import tarfile
import tempfile
import unittest
import zipfile
from pathlib import Path

SCRIPTS = Path(__file__).resolve().parents[1]
PRIVACY = runpy.run_path(str(SCRIPTS / "check-privacy.py"))
RELEASE = runpy.run_path(str(SCRIPTS / "verify-release.py"))
ARCHIVES = runpy.run_path(str(SCRIPTS / "create-archives.py"))


def synthetic_pe(dll: str = "KERNEL32.dll", delay: bool = False) -> bytes:
    """Minimal AMD64 executable with one real import descriptor and thunk table."""
    data = bytearray(0x400)
    data[:2] = b"MZ"
    struct.pack_into("<I", data, 0x3C, 0x80)
    data[0x80:0x84] = b"PE\0\0"
    struct.pack_into("<HHIIIHH", data, 0x84, 0x8664, 1, 0, 0, 0, 0xF0, 0x22)
    optional = 0x98
    struct.pack_into("<H", data, optional, 0x20B)
    struct.pack_into("<III", data, optional + 4, 0x200, 0, 0)
    struct.pack_into("<IIQII", data, optional + 16, 0x1000, 0x1000, 0x140000000, 0x1000, 0x200)
    struct.pack_into("<HH", data, optional + 40, 6, 0)
    struct.pack_into("<HH", data, optional + 48, 6, 0)
    struct.pack_into("<II", data, optional + 56, 0x2000, 0x200)
    struct.pack_into("<H", data, optional + 68, 3)
    struct.pack_into("<QQQQ", data, optional + 72, 0x100000, 0x1000, 0x100000, 0x1000)
    struct.pack_into("<I", data, optional + 108, 16)
    struct.pack_into("<II", data, optional + 112 + (13 if delay else 1) * 8, 0x1020, 64 if delay else 40)
    section = optional + 0xF0
    data[section:section + 8] = b".text\0\0\0"
    struct.pack_into("<IIII", data, section + 8, 0x200, 0x1000, 0x200, 0x200)
    struct.pack_into("<I", data, section + 36, 0x60000020)
    data[0x200] = 0xC3  # ret
    if delay:
        struct.pack_into("<IIIIIIII", data, 0x220, 1, 0x1080, 0x10B0, 0x10D0, 0x10C0, 0, 0, 0)
    else:
        struct.pack_into("<IIIII", data, 0x220, 0x10C0, 0, 0, 0x1080, 0x10D0)
    name = dll.encode("ascii") + b"\0"
    data[0x280:0x280 + len(name)] = name
    struct.pack_into("<Q", data, 0x2C0, 0x10E0)
    struct.pack_into("<Q", data, 0x2D0, 0x10E0)
    data[0x2E0:0x2EE] = b"\0\0ExitProcess\0"
    return bytes(data)


class PrivacyTests(unittest.TestCase):
    def test_commit_and_tag_messages_are_scanned_and_redacted(self):
        for object_kind in ("commit", "tag"):
            with self.subTest(kind=object_kind), tempfile.TemporaryDirectory(prefix="vpn233-metadata-test-") as scratch:
                repo = Path(scratch)
                subprocess.run(["git", "init", "--quiet", str(repo)], check=True)
                subprocess.run(["git", "-C", str(repo), "config", "user.name", "synthetic-test"], check=True)
                subprocess.run(["git", "-C", str(repo), "config", "user.email", "fixture@example.invalid"], check=True)
                (repo / "public.txt").write_text("public fixture")
                subprocess.run(["git", "-C", str(repo), "add", "public.txt"], check=True)
                secret = "gh" + "p_" + "Q" * 35
                message = secret if object_kind == "commit" else "public fixture"
                subprocess.run(["git", "-C", str(repo), "commit", "--quiet", "-m", message], check=True)
                if object_kind == "tag":
                    subprocess.run(["git", "-C", str(repo), "tag", "-a", "v0.0.1", "-m", secret], check=True)
                result = subprocess.run([sys.executable, str(SCRIPTS / "check-privacy.py"), "--repository", str(repo), "--history-only"], capture_output=True)
                self.assertEqual(1, result.returncode)
                self.assertIn(("history:" + object_kind + ":").encode(), result.stdout)
                self.assertNotIn(secret.encode(), result.stdout + result.stderr)

    def test_runtime_paths_rejected(self):
        for path in ("config.yaml", "nested/.env.prod", "data/users.json", "runtime/state.json", "backups/old.txt", "logs/output.log"):
            with self.subTest(path=path):
                self.assertTrue(PRIVACY["sensitive_path"](path))

    def test_public_paths_allowed(self):
        for path in ("provider-server/config.example.yaml", "examples/provider-server.yaml", "scripts/lib/mirrors.sh"):
            self.assertFalse(PRIVACY["sensitive_path"](path))

    def test_credential_patterns(self):
        fixtures = [b"gh" + b"p_" + b"Q" * 35, b"-----BEGIN " + b"PRIVATE KEY-----", b"https://" + b"user:random-password@" + b"host.example"]
        for data in fixtures:
            self.assertTrue(PRIVACY["content_rules"](data))

    def test_reserved_addresses(self):
        self.assertFalse(PRIVACY["content_rules"](b"127.0.0.1 192.0.2.8 198.51.100.9 203.0.113.5 1.1.1.1"))

    def test_public_server_address(self):
        data = ".".join(str(value) for value in (45, 33, 85, 32)).encode()
        self.assertIn("non-example-public-ipv4", PRIVACY["content_rules"](data))

    def test_machine_path(self):
        data = b"C:" + b"/" + b"Users" + b"/" + b"synthetic-person/project"
        self.assertIn("machine-home-path", PRIVACY["content_rules"](data))

    def test_svg_geometric_numbers_are_not_server_addresses(self):
        address = ".".join(str(value) for value in (45, 33, 85, 32)).encode()
        data = b'<svg xmlns="http://www.w3.org/2000/svg"><path d="M0-' + address + b'"/></svg>'
        self.assertNotIn("non-example-public-ipv4", PRIVACY["content_rules"](data))

    def test_svg_text_and_url_addresses_are_still_scanned(self):
        address = ".".join(str(value) for value in (45, 33, 85, 32)).encode()
        for data in (b"<svg><text>" + address + b"</text></svg>", b'<svg><image href="https://' + address + b'/asset.png"/></svg>'):
            self.assertIn("non-example-public-ipv4", PRIVACY["content_rules"](data))

    def test_svg_key_is_never_exempted(self):
        secret = b"-----BEGIN " + b"PRIVATE KEY-----"
        self.assertIn("private-key", PRIVACY["content_rules"](b'<svg><path d="M0 0"/><text>' + secret + b"</text></svg>"))

    def test_invalid_svg_path_is_not_exempted(self):
        address = ".".join(str(value) for value in (45, 33, 85, 32)).encode()
        self.assertIn("non-example-public-ipv4", PRIVACY["content_rules"](b'<svg><path d="https://' + address + b'"/></svg>'))

    def test_index_is_scanned_even_when_worktree_file_is_removed(self):
        with tempfile.TemporaryDirectory(prefix="vpn233-index-test-") as scratch:
            repo = Path(scratch)
            subprocess.run(["git", "init", "--quiet", str(repo)], check=True)
            secret = b"gh" + b"p_" + b"Q" * 35
            fixture = repo / "config.yaml"
            fixture.write_bytes(secret)
            subprocess.run(["git", "-C", str(repo), "add", "config.yaml"], check=True)
            fixture.unlink()
            result = subprocess.run([sys.executable, str(SCRIPTS / "check-privacy.py"), "--repository", str(repo)], capture_output=True)
            self.assertEqual(1, result.returncode)
            self.assertIn(b"index:config.yaml", result.stdout)
            self.assertIn(b"sensitive-file-path", result.stdout)
            self.assertNotIn(secret, result.stdout + result.stderr)

    def test_history_only_scans_a_bare_mirror_and_redacts_values(self):
        with tempfile.TemporaryDirectory(prefix="vpn233-history-test-") as scratch:
            repo, mirror = Path(scratch) / "source", Path(scratch) / "mirror.git"
            subprocess.run(["git", "init", "--quiet", str(repo)], check=True)
            secret = b"gh" + b"p_" + b"Q" * 35
            (repo / "old.txt").write_bytes(secret)
            subprocess.run(["git", "-C", str(repo), "add", "old.txt"], check=True)
            subprocess.run(["git", "-C", str(repo), "-c", "user.name=synthetic-test", "-c", "user.email=fixture@example.invalid", "commit", "--quiet", "-m", "synthetic fixture"], check=True)
            subprocess.run(["git", "clone", "--quiet", "--mirror", str(repo), str(mirror)], check=True)
            result = subprocess.run([sys.executable, str(SCRIPTS / "check-privacy.py"), "--repository", str(mirror), "--history-only"], capture_output=True)
            self.assertEqual(1, result.returncode)
            self.assertIn(b"history:old.txt", result.stdout)
            self.assertNotIn(secret, result.stdout + result.stderr)


class ReleaseTests(unittest.TestCase):
    def setUp(self):
        self.scratch = tempfile.TemporaryDirectory(prefix="vpn233-release-test-")
        self.addCleanup(self.scratch.cleanup)
        self.directory = Path(self.scratch.name)
        self.version = "2.0.0"

    def make_assets(self, extra=None, omitted=None, member_transform=None, windows_binary=None):
        for platform in RELEASE["PLATFORMS"]:
            suffix = ".exe" if platform.startswith("windows-") else ""
            files = {path: b"public placeholder\n" for path in RELEASE["COMMON"]}
            payload = (synthetic_pe() if windows_binary is None else windows_binary) if suffix else b"synthetic binary\0"
            files.update({"bin/" + binary + suffix: payload for binary in RELEASE["BINARIES"]})
            files["version.txt"] = self.version.encode()
            if extra:
                files.update(extra)
            if omitted:
                files.pop(omitted, None)
            prefix = "vpn233-rust-" + platform
            for extension in ("tar.gz", "zip"):
                archive = self.directory / (prefix + "." + extension)
                if extension == "zip":
                    with zipfile.ZipFile(archive, "w") as out:
                        for path, data in files.items():
                            name = prefix + "/" + path
                            out.writestr(member_transform(name) if member_transform else name, data)
                else:
                    with tarfile.open(archive, "w:gz") as out:
                        for path, data in files.items():
                            name = prefix + "/" + path
                            item = tarfile.TarInfo(member_transform(name) if member_transform else name)
                            item.size = len(data)
                            out.addfile(item, io.BytesIO(data))
        self.rehash()

    def rehash(self):
        archives = sorted(list(self.directory.glob("*.zip")) + list(self.directory.glob("*.tar.gz")))
        (self.directory / "SHA256SUMS").write_text("".join(hashlib.sha256(path.read_bytes()).hexdigest() + "  " + path.name + "\n" for path in archives))
        (self.directory / "version.txt").write_text(self.version)

    def verify(self):
        return RELEASE["verify"](self.directory, self.version)

    def test_valid_matrix(self):
        self.make_assets()
        self.assertEqual([], self.verify())

    def test_windows_system_runtime_imports_are_allowed(self):
        for dll in ("KERNEL32.dll", "ucrtbase.dll", "api-ms-win-crt-runtime-l1-1-0.dll", "msvcrt.dll"):
            for delay in (False, True):
                with self.subTest(dll=dll, delay=delay):
                    self.assertEqual({dll.lower()}, RELEASE["pe_imports"](synthetic_pe(dll, delay)))
                    self.assertEqual(set(), RELEASE["windows_binary_rules"](synthetic_pe(dll, delay)))

    def test_windows_extra_vc_runtime_imports_are_rejected(self):
        for dll in ("VCRUNTIME140.dll", "vcruntime140_1.dll", "MSVCP140.dll", "msvcr120.dll", "MSVCRTD.dll", "concrt140.dll", "vcomp140.dll", "vccorlib140.dll", "mfc140u.dll", "mfcm140u.dll", "atl100.dll"):
            for delay in (False, True):
                with self.subTest(dll=dll, delay=delay):
                    self.assertEqual({"windows-extra-vc-runtime"}, RELEASE["windows_binary_rules"](synthetic_pe(dll, delay)))

    def test_windows_runtime_gate_applies_to_each_binary_in_both_archives(self):
        self.make_assets(windows_binary=synthetic_pe("VCRUNTIME140.dll"))
        findings = self.verify()
        self.assertEqual(6, len(findings))
        self.assertTrue(all("windows-extra-vc-runtime" in item for item in findings))
        self.assertTrue(all("VCRUNTIME140.dll" not in item for item in findings))

    def test_windows_import_gate_uses_descriptors_not_unreferenced_strings(self):
        self.assertEqual(set(), RELEASE["windows_binary_rules"](synthetic_pe() + b"VCRUNTIME140.dll\0"))

    def test_windows_non_pe_archive_members_are_rejected(self):
        self.make_assets(windows_binary=b"synthetic binary\0")
        self.assertEqual(6, sum("windows-invalid-amd64-pe" in item for item in self.verify()))

    def test_windows_truncated_pe_is_rejected(self):
        valid = synthetic_pe()
        for length in (0, 2, 63, 0x83, 0x98, 0x187, 0x1FF, 0x2FF, len(valid) - 1):
            with self.subTest(length=length):
                self.assertEqual({"windows-invalid-amd64-pe"}, RELEASE["windows_binary_rules"](valid[:length]))

    def test_windows_malformed_headers_and_imports_are_rejected(self):
        cases = {
            "wrong-machine": (0x84, "<H", 0x14C),
            "wrong-optional-magic": (0x98, "<H", 0x10B),
            "missing-section": (0x86, "<H", 0),
            "dll-instead-of-exe": (0x96, "<H", 0x2022),
            "short-optional-header": (0x94, "<H", 112),
            "oversized-directory-count": (0x98 + 108, "<I", 17),
            "unmapped-import-directory": (0x98 + 120, "<I", 0x9000),
            "zero-import-rva": (0x98 + 120, "<I", 0),
            "zero-import-size": (0x98 + 124, "<I", 0),
            "short-import-directory": (0x98 + 124, "<I", 19),
            "unterminated-import-directory": (0x98 + 124, "<I", 20),
            "unmapped-dll-name": (0x220 + 12, "<I", 0x9000),
            "zero-dll-name": (0x220 + 12, "<I", 0),
            "dll-name-in-zero-fill-tail": (0x220 + 12, "<I", 0x1200),
            "raw-section-overlaps-headers": (0x188 + 20, "<I", 0x100),
            "virtual-section-overlaps-headers": (0x188 + 12, "<I", 0x100),
            "raw-section-past-eof": (0x188 + 16, "<I", 0x300),
            "non-executable-entry": (0x98 + 16, "<I", 0),
        }
        for label, (offset, layout, value) in cases.items():
            with self.subTest(case=label):
                data = bytearray(synthetic_pe())
                struct.pack_into(layout, data, offset, value)
                self.assertEqual({"windows-invalid-amd64-pe"}, RELEASE["windows_binary_rules"](data))
        for dll in ("../VCRUNTIME140.dll", "C:\\VCRUNTIME140.dll", "VCRUNTIME140", "VCRUNTIME140.dll.", "VCRUNTIME140.dll "):
            with self.subTest(case="invalid-name", dll=dll):
                self.assertEqual({"windows-invalid-amd64-pe"}, RELEASE["windows_binary_rules"](synthetic_pe(dll)))

    def test_windows_malformed_delay_imports_are_rejected(self):
        for attributes in (0, 3):
            with self.subTest(attributes=attributes):
                data = bytearray(synthetic_pe(delay=True))
                struct.pack_into("<I", data, 0x220, attributes)
                self.assertEqual({"windows-invalid-amd64-pe"}, RELEASE["windows_binary_rules"](data))

    def test_windows_dll_name_cannot_read_beyond_its_section(self):
        data = bytearray(synthetic_pe())
        data[0x280:] = b"A" * (len(data) - 0x280)
        self.assertEqual({"windows-invalid-amd64-pe"}, RELEASE["windows_binary_rules"](data + b".dll\0"))

    def test_checksum_tampering(self):
        self.make_assets()
        next(self.directory.glob("*.zip")).write_bytes(b"tampered")
        self.assertTrue(any("SHA256 mismatch" in finding for finding in self.verify()))

    def test_missing_checksum(self):
        self.make_assets()
        (self.directory / "SHA256SUMS").unlink()
        self.assertTrue(self.verify())

    def test_missing_member(self):
        self.make_assets(omitted="scripts/lib/mirrors.sh")
        self.assertTrue(any("missing scripts/lib/mirrors.sh" in item for item in self.verify()))

    def test_private_file(self):
        self.make_assets(extra={"config/config.yaml": b"private"})
        self.assertTrue(any("sensitive-file-path" in item for item in self.verify()))

    def test_unexpected_member(self):
        self.make_assets(extra={"admin-web/dist/assets/source.map": b"{}"})
        self.assertTrue(any("non-allowlisted" in item for item in self.verify()))

    def test_path_traversal(self):
        self.make_assets(member_transform=lambda name: "../" + name)
        self.assertTrue(any("unsafe archive member" in item for item in self.verify()))

    def test_secret_inside_archive(self):
        self.make_assets(extra={"admin-web/dist/assets/app.js": b"gh" + b"p_" + b"Q" * 35})
        self.assertTrue(any("github-token" in item for item in self.verify()))

    def test_extra_release_asset(self):
        self.make_assets()
        (self.directory / "debug.txt").write_text("local")
        self.assertTrue(any("exact allowlist" in item for item in self.verify()))

    def test_wrong_version(self):
        self.make_assets()
        (self.directory / "version.txt").write_text("0.0.0")
        self.assertTrue(self.verify())

    def test_broken_relative_documentation_link(self):
        self.make_assets(extra={"README.md": b"[missing documentation](docs/missing.md)"})
        self.assertTrue(any("broken bundle link" in item for item in self.verify()))

    def test_archives_are_reproducible_after_mtime_change(self):
        bundle = self.directory / "bundle" / "vpn233-rust-linux-x86_64"
        bundle.mkdir(parents=True)
        asset = bundle / "version.txt"
        asset.write_text(self.version)
        first, second = self.directory / "first", self.directory / "second"
        ARCHIVES["create"](bundle, first, 1600000000)
        os.utime(asset, (1700000000, 1700000000))
        ARCHIVES["create"](bundle, second, 1600000000)
        for extension in ("tar.gz", "zip"):
            name = bundle.name + "." + extension
            self.assertEqual((first / name).read_bytes(), (second / name).read_bytes())


if __name__ == "__main__":
    unittest.main()
