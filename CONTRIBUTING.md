# Contributing to VPN233

Thanks for helping improve VPN233. This repository is public and every commit,
issue and release artifact is world-readable, so privacy and secret hygiene are
part of the contribution rules.

## Ground rules

- `main` is the only branch. Do not create long-lived branches whose names
  contain `agent` or `codex`.
- Never commit real credentials, tokens, subscription URLs, server IPs, runtime
  data or customer identifiers. Example configuration may only use placeholders
  such as `change-me`, `example.invalid` and `127.0.0.1`.
- No telemetry, analytics, external fonts or CDN assets may be added to the
  services or the admin web.
- Documentation and the capability tables must describe what the code actually
  does. Update them in the same change that alters a format, protocol, endpoint,
  CLI flag or script argument.
- Every release must keep CI, the local `verify-all`, the E2E suites and the
  published-asset verification green.

## Development environment

| Tool | Version |
| --- | --- |
| Rust | 1.99.1 (`rust-toolchain.toml` pins it; minimum supported version 1.96) |
| Node.js | 24.x (admin web) |
| Rust development tools | The `vpn233-xtask` workspace crate and the same pinned Rust toolchain |
| Git Bash | Required on Windows for the `.sh` test suites |

```bash
# Rust
cargo fmt --all
cargo clippy --locked --workspace --all-targets -- -D warnings
cargo test --locked --workspace --no-fail-fast
cargo build --locked --release --workspace --exclude vpn233-xtask

# Admin web
cd admin-web
npm ci
npm run build
npm run check
```

## Full local verification

```bash
# Linux / macOS / Git Bash on Windows
bash scripts/verify-all.sh

# Windows PowerShell
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/verify-all.ps1
```

Useful subsets:

```bash
bash scripts/verify-all.sh --quick        # incremental development checks
export VPN233_XTASK_BIN="$(bash scripts/xtask.sh --path)"
bash scripts/e2e/e2e.sh                   # real three-service E2E (Linux/macOS/Git Bash)
pwsh -File scripts/e2e/e2e.ps1             # Windows E2E (PowerShell)
bash scripts/e2e/check-docs.sh            # documentation gate
bash scripts/tests/test-mirrors.sh        # mirror helper tests
bash scripts/tests/test-installers.sh     # installer tests (offline)
pwsh -File scripts/tests/test-installers.ps1
```

The E2E suites build no credentials into the repository: they generate random
passwords in a temporary directory and delete it afterwards.

Verification, fixtures, privacy checks, archive creation and release checks are
implemented in Rust. `cargo test --locked --workspace --no-fail-fast` includes their regression
tests. `cargo xtask --help` lists the development commands. The wrappers build
the tool once in `.cache/xtask-bootstrap`, so product flags and test builds cannot
rewrite the executable used by a Quick cache transaction. Set
`$env:VPN233_XTASK_BIN = & ./scripts/xtask.ps1 -PathOnly` before standalone
PowerShell E2E. The development tool is excluded from product archives.

## Code style

- Rust: `cargo fmt`, no clippy warnings, ASCII source, comments in English.
- Shell: `set -Eeuo pipefail`, quote everything, shellcheck clean at `warning`
  level. Keep scripts ASCII-only and LF line endings.
- PowerShell: Windows PowerShell 5.1 compatible (no `??`, no ternary operator,
  no `-Parallel`), ASCII-only, CRLF line endings. Avoid aliases and bare
  `Write-Output`. Use approved verbs for functions.
- Frontend: no new dependencies without a reason, no external asset URLs, keep
  `npm run check` green.
- Markdown: relative links only for in-repo targets, balanced code fences,
  front matter on every page under `docs/`.

## Commits and pull requests

- Small, logical commits with a conventional prefix: `feat:`, `fix:`,
  `refactor:`, `test:`, `docs:`, `chore:`, `ci:`.
- Do not run `git add .`; stage explicit paths. Never commit build output,
  `target*`, `node_modules`, `dist`, bundles, logs or configuration.
- `scripts/commit.ps1` checks the explicitly staged files and runs quick verification
  before committing. `-Push` runs full verification before pushing only `origin/main`.
  The legacy Windows commit/build/release launchers use these same guarded entry points.
- Fill in the pull request template, including which verification commands you
  ran. Security-sensitive changes should mention the threat model.
- CI must pass before merge. The `security` job scans the full history with
  gitleaks and runs RustSec and npm audits.

## Release process

Releases are tag-driven and must be created from `main` only:

1. Bump the version in one chain: root `Cargo.toml`
   (`[workspace.package].version`), `version.txt` and `admin-web/package.json`.
   `bash scripts/release.sh --version X.Y.Z` or `scripts/release.ps1 -Version X.Y.Z`
   synchronizes version metadata. Document the actual changes in CHANGELOG.md.
2. Run `scripts/verify-all.sh` (or the Windows variant) end to end.
3. Commit `chore(release): vX.Y.Z` and push `main`. After CI passes, run the release
   helper with `--publish` / `-Publish` to verify again and push the new tag.
4. The release workflow re-runs the full CI pipeline, builds Linux/Windows/macOS
   bundles, publishes them with `SHA256SUMS` and `version.txt`, and verifies the
   published assets and actual public installation/update on Linux and Windows
   before the release is considered successful.
5. Never publish when the E2E suites, the checksum verification or the generated
   admin UI are red.

GitHub Pages runs after a successful formal release, or through a manual request
that names an already successful release run. Both paths use the same Rust gate:
the latest stable tag, exact commit, all release jobs and public install/update
must be verified. An unchanged documentation tree skips Jekyll, upload and
deployment. Generated HTML must pass privacy, internal links, anchors and browser
external-request checks before publication. A successful deployment records its
actual release commit and documentation tree in public GitHub deployment metadata.

## Security reports

Do not open public issues for vulnerabilities. See [SECURITY.md](SECURITY.md).

## License

By contributing you agree that your contributions are licensed under the MIT
License in [LICENSE](LICENSE).
