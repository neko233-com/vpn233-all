# Changelog

## [2.1.1] - 2026-10-22

- Resolve the default installation directory in the script body so native
  Windows PowerShell 5.1 can start, stop and uninstall without arguments.
  Exercise the actual `-File` entry point in regression coverage.
- Keep successful background launchers under explicit native process ownership,
  with bounded private captures, deadlines and cleanup of owned descendants.
- Install the already verified native archive offline and check real startup,
  all health endpoints, UI, provider discovery, subscription rendering and
  owned shutdown in Full verification and CI before publishing a tag.
- Publish complete checksum-verified candidates as prereleases; promote to
  stable/latest only after both public installation and online update gates
  pass. Pages additionally requires the successful promotion job.
- Report fixed diagnostic stages without private paths or raw errors. Use an
  explicit one-second heartbeat in synthetic customer tests; production keeps
  its documented 30-second default.

## [2.1.0] - 2026-10-22 (prerelease; Windows startup acceptance failed)

- Move repository verification, privacy, version preparation, deterministic
  archives, release checks, content-addressed Quick reports and network fixtures
  into the Rust workspace. Remove the Python runtime and its source/test tools.
- Preserve the previous negative cases in native Rust tests, use one loopback
  HTTP fixture across operating systems, and validate official Mihomo output
  with the same checksum-pinned parser matrix.
- Keep the verification executable outside product target-cache pruning and
  reuse tested, privacy-remapped native tools in CI. Optimize the compression
  and hashing dependencies while collecting all workspace test-target failures.
  Product archives still contain only the three application binaries.
- Require a successful stable release, published assets, and real public
  installation/online update on Linux and Windows before Pages can publish.
  Ordinary commits do not build Pages; unchanged documentation skips publishing.
- Verify generated pages locally before upload and record the actual deployed
  release commit and documentation tree with stale-release protection.
- Strengthen archive metadata privacy, bounded extraction-free validation,
  Windows path handling, architecture checks and paired tar/ZIP content checks.
- Protect and verify Windows customer-test directories through native SID-only
  ACL operations without shell startup, module discovery or account lookup.

## [2.0.2] - 2026-10-21 (unpublished)

- Recover interrupted release downloads with bounded same-source byte-range
  retries and verify the complete archive before publishing it locally.
- Check the expected SHA-256 separately for each mirror. Truncated or incorrect
  responses cannot become successful cache entries or block later sources.
- Keep shell and Windows PowerShell installation, update preservation and
  explicit unsigned-download behavior consistent, with real local HTTP tests.

## [2.0.1] - 2026-10-21

- Start the loopback HTTP test fixture without reverse DNS, avoiding the known
  macOS runner hostname-lookup stall. Read the kernel-assigned listening port
  from a private readiness file instead of releasing a port before binding it.
- Add startup and shutdown regression coverage while retaining both native
  macOS architecture checks and the complete real-service E2E release gate.

## [2.0.0] - 2026-10-21

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
- Check generated inbounds and subscription/import/transport fixtures with the
  checksum-verified official Mihomo parser. Fix its VLESS/VMess UUID, Trojan
  user-list and Hysteria2/TUIC/AnyTLS user-map schemas.
- Run bundled self-update scripts with Bash on Unix and the explicit local
  PowerShell execution policy on Windows; test actual executable dispatch,
  paths containing spaces and failure propagation.
- Reuse compiled binaries and UI artifacts in CI and release jobs; build
  reproducible archives, remap machine paths and verify package allowlists,
  privacy and SHA256SUMS before publication.
- Add bilingual entry points and GitHub Pages documentation covering setup,
  deployment, compatibility, API, mirrors, testing and privacy.
- Scan working-tree candidates, the index, historical blobs and public artifacts;
  remove tracked legacy runtime configuration and keep explicit public examples.
- Update compatible Rust dependencies and frontend/build dependencies with locked
  versions; pin the Rust 1.99.1 toolchain. Encrypted proxy transports remain an
  external core boundary.
- Restrict JWT crypto to the required HS256 adapter and remove unused RSA/EC
  dependencies. Replace deprecated YAML dependencies with the YAML organization's
  maintained [yaml_serde](https://github.com/yaml/yaml-serde) fork.

Formal release availability is recorded on
[GitHub Releases](https://github.com/neko233-com/vpn233-all/releases).
