# Project operating rules

## Privacy and secrets

- Never upload, commit, publish, or paste private data, credentials, SSH material, API keys, subscription URLs, subscription passwords, provider heartbeat tokens, runtime data, logs, or customer identifiers.
- Treat all files under service data directories, generated release bundles, local environment files, and exported subscription files as sensitive. Keep them out of Git and out of external tools.
- SSH credentials supplied for a deployment are session-only. Do not write them to the repository, shell scripts, build artifacts, browser storage, or documentation.
- Do not send source files, screenshots, logs, or telemetry to third-party services unless the user explicitly authorizes that exact transfer.
- Before publishing or deploying, scan the staged files and generated artifacts for secrets and private data.

## Repository and release

- `main` is the only allowed branch. Do not create, push, or retain branches whose names contain `agent` or `codex`.
- Keep production configuration separate from tracked example configuration. Use environment variables or untracked deployment files for secrets.
- Verify builds, tests, and the generated admin UI before release. Do not publish an artifact that includes runtime data or credentials.

## Deployment

- Deploy only the built application and required public assets to the user-authorized server.
- Preserve existing production data and configuration unless the user explicitly asks for a migration or reset.
- After deployment, verify the admin health endpoint, provider inventory, and a non-secret subscription response without recording the response body.
