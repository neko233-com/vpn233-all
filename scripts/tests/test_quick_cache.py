"""Quick reports must miss when evidence changes and never survive failure."""
import hashlib
import importlib.util
import json
import os
import stat
from pathlib import Path
import subprocess
import tempfile
import unittest

SPEC = importlib.util.spec_from_file_location("quick_cache", Path(__file__).resolve().parents[1] / "quick-cache.py")
cache = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(cache)


class QuickCacheTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix="vpn233-quick-cache-")
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name).resolve()
        self.git("init", "--quiet")
        self.git("config", "user.name", "fixture")
        self.git("config", "user.email", "fixture@example.invalid")
        self.write(".gitignore", ".cache/\nadmin-web/node_modules/\nadmin-web/dist/\nadmin-web/.env*\nruntime/\n")
        self.write("source.rs", "pub fn synthetic() {}\n")
        self.write("admin-web/package-lock.json", "{}")
        self.write("admin-web/node_modules/module/index.js", "module.exports = 1;")
        self.marker()
        self.write("admin-web/dist/index.html", "<main>fixture</main>")
        self.git("add", ".")
        self.git("commit", "--quiet", "-m", "synthetic fixture")
        self.evidence = {"compiler": "synthetic-version-1"}

    def git(self, *arguments):
        return subprocess.run(["git", "-C", str(self.root), *arguments], check=True, capture_output=True).stdout

    def write(self, path, value):
        destination = self.root / path
        destination.parent.mkdir(parents=True, exist_ok=True)
        destination.write_text(value, encoding="utf-8")

    def marker(self):
        self.write("admin-web/node_modules/.vpn233-lock.sha256", hashlib.sha256((self.root / "admin-web/package-lock.json").read_bytes()).hexdigest())

    def probe(self, **overrides):
        return cache.probe(self.root, "powershell", "fixture-engine", evidence=overrides.get("evidence", self.evidence), environment=overrides.get("environment", {}))

    def finish(self, ticket, success=True, **overrides):
        return cache.finish(self.root, "powershell", "fixture-engine", ticket, 11, 12.5, success, evidence=overrides.get("evidence", self.evidence), environment=overrides.get("environment", {}))

    def seed(self):
        first = self.probe()
        self.assertFalse(first["hit"])
        self.assertTrue(self.finish(first["ticket"])["stored"])
        self.assertTrue(self.probe()["hit"])

    def report(self):
        return self.root / ".cache/quick-verify/powershell-success.json"

    def test_success_reused_and_only_digests_are_persisted(self):
        self.seed()
        text = self.report().read_text()
        self.assertNotIn(str(self.root), text)
        self.assertNotIn("synthetic-version", text)
        self.assertNotIn("module.exports", text)

    def test_source_change_same_size_and_mtime_misses(self):
        self.seed()
        path = self.root / "source.rs"
        before = path.stat()
        self.write("source.rs", "pub fn Synthetic() {}\n")
        os.utime(path, ns=(before.st_atime_ns, before.st_mtime_ns))
        self.assertFalse(self.probe()["hit"])

    def test_new_candidate_deleted_candidate_and_index_change_miss(self):
        for change in (lambda: self.write("new.rs", "// fixture"), lambda: (self.root / "source.rs").unlink(), lambda: self.git("rm", "--cached", "source.rs")):
            with self.subTest(change=change):
                self.seed()
                change()
                self.assertFalse(self.probe()["hit"])

    def test_history_ref_change_misses_even_with_identical_worktree(self):
        self.seed()
        self.git("commit", "--allow-empty", "--quiet", "-m", "second synthetic fixture")
        self.assertFalse(self.probe()["hit"])

    def test_tools_environment_and_configuration_changes_miss(self):
        self.seed()
        self.assertFalse(self.probe(evidence={"compiler": "synthetic-version-2"})["hit"])
        self.seed()
        self.assertFalse(self.probe(environment={"RUSTFLAGS": "--cfg synthetic"})["hit"])
        self.seed()
        self.write(".cargo/config.toml", "[build]\njobs=1\n")
        self.assertFalse(self.probe()["hit"])

    def test_vite_environment_and_ignored_env_file_changes_miss(self):
        self.seed()
        self.assertFalse(self.probe(environment={"VITE_API_BASE_URL": "https://example.invalid"})["hit"])
        self.seed()
        self.write("admin-web/.env.production.local", "VITE_PUBLIC_SUBSCRIPTION_ORIGIN=https://example.invalid\n")
        self.assertFalse(self.probe()["hit"])

    def test_case_variant_path_and_native_build_flags_miss(self):
        self.seed()
        for values in ({"Path": "synthetic-search-path"}, {"CFLAGS": "-DSYNTHETIC"}, {"BASH_ENV": "synthetic-startup"}, {"CARGO_PROFILE_DEV_OPT_LEVEL": "1"}, {"RUSTDOCFLAGS": "--cfg synthetic"}, {"MSYS_NO_PATHCONV": "1"}):
            self.assertFalse(self.probe(environment=values)["hit"])

    def test_ignored_project_and_explicit_external_npm_config_contents_miss(self):
        self.seed()
        self.write("admin-web/.npmrc", "fund=false\n")
        self.assertFalse(self.probe()["hit"])
        self.write("runtime/npm-config", "fund=false\n")
        environment = {"NPM_CONFIG_USERCONFIG": str(self.root / "runtime/npm-config")}
        first = self.probe(environment=environment)
        self.assertTrue(self.finish(first["ticket"], environment=environment)["stored"])
        self.write("runtime/npm-config", "fund=true\n")
        self.assertFalse(self.probe(environment=environment)["hit"])

    def test_dependency_permissions_are_hashed(self):
        self.seed()
        path = self.root / "admin-web/node_modules/module/index.js"
        before = path.stat().st_mode
        try:
            path.chmod(stat.S_IMODE(before) & ~stat.S_IWUSR)
            self.assertFalse(self.probe()["hit"])
        finally:
            path.chmod(stat.S_IMODE(before))

    def test_dependencies_and_output_are_content_hashed(self):
        for name in ("admin-web/node_modules/module/index.js", "admin-web/dist/index.html"):
            with self.subTest(path=name):
                self.seed()
                path = self.root / name
                before = path.stat()
                self.write(name, path.read_text().replace("1", "2").replace("fixture", "Fixture"))
                os.utime(path, ns=(before.st_atime_ns, before.st_mtime_ns))
                self.assertFalse(self.probe()["hit"])

    def test_ignored_runtime_changes_do_not_invalidate(self):
        self.seed()
        self.write("runtime/synthetic.txt", "local synthetic runtime")
        self.assertTrue(self.probe()["hit"])

    def test_bad_marker_ci_and_missing_dependencies_never_hit(self):
        self.seed()
        self.assertFalse(self.probe(environment={"CI": "true"})["hit"])
        self.seed()
        self.write("admin-web/node_modules/.vpn233-lock.sha256", "wrong")
        self.assertFalse(self.probe()["hit"])
        (self.root / "admin-web/node_modules/.vpn233-lock.sha256").unlink()
        self.assertFalse(self.probe()["hit"])

    def test_failed_or_aborted_verification_has_no_success_report(self):
        first = self.probe()
        self.assertFalse(self.finish(first["ticket"], success=False)["stored"])
        self.assertFalse(self.report().exists())
        self.seed()
        self.write("source.rs", "// changed")
        missed = self.probe()
        self.assertFalse(self.report().exists())
        self.assertFalse(self.finish(missed["ticket"], success=False)["stored"])
        self.assertFalse(self.probe()["hit"])

    def test_source_or_dependency_change_during_checks_cannot_seed(self):
        for name in ("source.rs", "admin-web/node_modules/module/index.js"):
            first = self.probe()
            self.write(name, "// changed during verification")
            self.assertFalse(self.finish(first["ticket"])["stored"])

    def test_first_install_requires_another_stable_verification(self):
        (self.root / "admin-web/node_modules/.vpn233-lock.sha256").unlink()
        first = self.probe()
        self.write("admin-web/node_modules/module/index.js", "new installed fixture")
        self.marker()
        self.write("admin-web/dist/index.html", "rebuilt public output")
        self.assertFalse(self.finish(first["ticket"])["stored"])
        self.assertTrue(self.finish(self.probe()["ticket"])["stored"])
        self.assertTrue(self.probe()["hit"])

    def test_store_independently_rejects_ci(self):
        first = self.probe()
        self.assertFalse(self.finish(first["ticket"], environment={"CI": "true"})["stored"])
        self.assertFalse(self.report().exists())

    def test_corrupt_partial_failed_and_wrong_schema_reports_never_hit(self):
        self.seed()
        original = json.loads(self.report().read_text())
        for update in ({"schema": 0}, {"mode": "full"}, {"scope": "partial"}, {"success": False}, {"checks": True}, {"elapsed_seconds": float("nan")}, {"fingerprint": "wrong"}, {"completed_at": "invalid control text"}):
            self.report().write_text(json.dumps(original | update))
            self.assertFalse(self.probe()["hit"])
        self.report().write_text("{corrupt")
        self.assertFalse(self.probe()["hit"])

    def test_ticket_is_single_use_and_path_traversal_rejected(self):
        first = self.probe()
        self.assertTrue(self.finish(first["ticket"])["stored"])
        self.assertFalse(self.finish(first["ticket"])["stored"])
        with self.assertRaises(cache.CacheUnavailable):
            self.finish("../invalid-ticket")

    def test_external_dependency_symlink_disables_cache(self):
        try:
            (self.root / "admin-web/node_modules/external").symlink_to(self.root / "source.rs")
        except OSError:
            self.skipTest("Symlink permission unavailable")
        with self.assertRaises(cache.CacheUnavailable):
            self.probe()


if __name__ == "__main__":
    unittest.main()
