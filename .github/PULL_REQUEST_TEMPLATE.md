# Pull request

## Summary

<!-- What does this change and why? Keep it short. -->

## Change type

- [ ] Bug fix
- [ ] New feature
- [ ] Refactor / internal cleanup
- [ ] Documentation
- [ ] CI / release tooling
- [ ] Dependency update

## Verification

<!-- Paste the commands you ran and their result. At minimum: -->

- [ ] `cargo fmt --all -- --check`
- [ ] `cargo clippy --workspace --all-targets -- -D warnings`
- [ ] `cargo test --workspace`
- [ ] `cd admin-web && npm run verify`
- [ ] `bash scripts/e2e/check-docs.sh`
- [ ] `scripts/verify-all` (or the relevant subset, if a full run was impractical)

## Privacy and security

- [ ] No credentials, tokens, real subscription URLs, server IP addresses, runtime data or customer data are included.
- [ ] Example configuration only contains placeholders such as `change-me` / `example.invalid`.
- [ ] Docs and capability tables were updated together with any format/protocol/endpoint/CLI change.
- [ ] The change does not add telemetry, third-party analytics, external fonts or CDN assets.

## Release impact

<!-- Delete the lines that do not apply. -->

- [ ] This changes a published asset layout, install/update behaviour or the mirror contract (release workflow and docs were updated).
- [ ] This requires a version bump / CHANGELOG entry (see CONTRIBUTING.md).
- [ ] No release impact.

## Notes for reviewers

<!-- Optional: risks, follow-ups, screenshots (no real data). -->
