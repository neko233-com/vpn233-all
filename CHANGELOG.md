# Changelog

## Unreleased (2.0.0 workspace)

- Expand subscription imports to URI, Base64/Base64URL, Clash/Mihomo YAML and
  compatible sing-box JSON, preserving supported transport and TLS settings.
- Export Clash/Mihomo, Stash, sing-box, URI/Base64 and client-specific formats;
  expose the actual protocol matrix through `/api/v1/formats`.
- Connect the client authentication, SOCKS5 upstream, timeout and statistics
  modules to the executable; destination recording is explicitly opt-in.
- Provide matching shell and PowerShell installers with GitHub mirror fallback,
  fixed-version downloads, mandatory checksums, private first-boot configuration,
  offline installation and application rollback after failed activation.
- Add quick incremental verification, complete local verification, automatic
  Linux/Windows service E2E and real Chromium admin interaction tests.
- Reuse compiled binaries and UI artifacts in CI and release jobs; build
  reproducible archives, remap machine paths and verify package allowlists,
  privacy and SHA256SUMS before publication.
- Add bilingual entry points and GitHub Pages documentation covering setup,
  deployment, compatibility, API, mirrors, testing and privacy.
- Scan working-tree candidates, the index, historical blobs and public artifacts;
  remove tracked legacy runtime configuration and keep explicit public examples.
- Update compatible Rust dependencies and frontend/build dependencies with locked
  versions. Encrypted proxy transports remain an external core boundary.
- Restrict JWT crypto to the required HS256 adapter and remove unused RSA/EC
  dependencies. Replace deprecated YAML dependencies with the YAML organization's
  maintained [yaml_serde](https://github.com/yaml/yaml-serde) fork.

Formal release availability is recorded on
[GitHub Releases](https://github.com/neko233-com/vpn233-all/releases).
