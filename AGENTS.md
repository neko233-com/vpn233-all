# Project operating rules

These rules apply to every human and automated contributor. The repository is
published publicly at <https://github.com/neko233-com/vpn233-all>: anything that
lands in Git or in a release artifact is world-readable forever.

## Privacy and secrets

- Never commit, publish, paste, or upload private data: credentials, SSH
  material, API keys, subscription URLs, subscription passwords, provider feed
  or heartbeat tokens, runtime data, logs, customer identifiers, or real
  server IP addresses.
- `**/config.yaml`, `*.env`, service data directories, exported subscriptions,
  generated installers, and release bundles are sensitive. They must stay out
  of Git (see `.gitignore`) and out of external tools.
- Tracked example configuration (`config.example.yaml`) must only contain
  placeholders such as `change-me`, `example.invalid`, or empty strings.
- SSH credentials supplied for a deployment are session-only. Never write them
  to the repository, scripts, build artifacts, browser storage, or docs.
- No telemetry: the services and the admin web must not call third-party
  analytics, fonts, CDNs, or heartbeat endpoints. Only the modules a user
  explicitly configures (provider registry, upstream subscription sources,
  client stats relay) may make outbound connections.
- Do not send source files, screenshots, logs, or telemetry to third-party
  services unless the user explicitly authorises that exact transfer.
- Before every push, release, or deployment: scan the staged diff, the working
  tree, and the generated artifacts for secrets and private data. The
  `gitleaks` workflow and `scripts/e2e/check-docs.sh` enforce this in CI.

## Repository and release

- `main` is the only allowed branch. Do not create, push, or retain feature
  branches whose names contain `agent` or `codex`.
- Keep production configuration separate from tracked example configuration.
  Use environment variables or untracked deployment files for secrets.
- Version numbers are owned by a single source of truth chain: root
  `Cargo.toml` (`[workspace.package].version`), `version.txt`, and
  `admin-web/package.json`. Bump them together through
  `scripts/release.sh` / `scripts/release.ps1`; never hand-edit one of them.
- Releases are tag-driven (`vMAJOR.MINOR.PATCH`). A release must not be created
  unless all of the following are green: CI workflows, the local `verify-all`
  script, the E2E suites, the checksum verification job, and the generated
  admin UI.
- Every release asset is published together with `SHA256SUMS`. Installers fail
  closed when a checksum is missing or mismatched; unsigned installation is
  only possible through an explicit opt-in flag.
- The GitHub mirror prefixes used by the installers are a contract shared by
  `scripts/lib/mirrors.sh`, `scripts/lib/mirrors.ps1`, the node installers that
  `provider-server` generates, and the docs. Change them in one place at a time
  and keep all copies consistent (CI checks the docs copy).
- Do not publish an artifact that contains runtime data, credentials, local
  paths that identify a machine, or files from `data/`, `config/`, `runtime/`,
  `logs/`, or `backups/`.

## Deployment

- Deploy only the built application and required public assets to the
  user-authorised server.
- Preserve existing production data and configuration unless the user
  explicitly asks for a migration or reset.
- After deployment, verify the admin health endpoint, the provider inventory,
  and a non-secret subscription response without recording the response body.
- The default `root` / `root` credentials are first-boot only. Any deployment
  reachable from a public network must set `VPN233_ADMIN_PASSWORD` or edit the
  untracked configuration before the service is exposed.

## Documentation

- `README.md` (English) and `README.zh-CN.md` (Chinese) are entry points; the
  long-form documentation lives in `docs/` and is published on GitHub Pages.
- Documentation must describe what the code actually does. When a format,
  protocol, endpoint, CLI flag, or script argument changes, update the docs and
  the capability tables in the same change.

## Verification and documentation publication

- Repository verification, privacy, versioning, packaging and test fixtures
  belong in the Rust `vpn233-xtask` workspace crate. Do not add Python tooling
  or interpreter fallbacks. Drive the real Bash/PowerShell installers and the
  existing browser UI tests from the verified toolchain.
- Audit the complete affected execution chain before a coordinated change.
  Preserve existing negative cases and run the combined verification gates.
- GitHub Pages must not build or deploy on ordinary commits. Require a fully
  successful stable release, including published-asset and public online
  install/update checks; manual requests use the same release gates.
- Reuse the verified release tool, check the exact release commit and skip
  Jekyll/upload/deploy when the documentation tree has not changed. Verify
  generated pages before publication and record the actual deployed release
  commit and documentation tree, rather than the default-branch event SHA.
