"""Negative release/privacy cases. Every secret/IP fixture is synthetic."""
import hashlib
import io
import os
import runpy
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

    def make_assets(self, extra=None, omitted=None, member_transform=None):
        for platform in RELEASE["PLATFORMS"]:
            suffix = ".exe" if platform.startswith("windows-") else ""
            files = {path: b"public placeholder\n" for path in RELEASE["COMMON"]}
            files.update({"bin/" + binary + suffix: b"synthetic binary\0" for binary in RELEASE["BINARIES"]})
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
