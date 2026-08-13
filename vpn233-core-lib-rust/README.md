# vpn233-core-lib-rust

The shared Rust library for VPN233's Clash/Mihomo-compatible configuration
model and protocol extension points.

This crate currently provides:

- typed helpers for proxy nodes and outbound definitions;
- deterministic Clash/Mihomo YAML rendering;
- link rendering for provider-generated node manifests;
- a small `CustomProtocol` registry for future Rust protocol adapters.

It is intentionally a library boundary, not a claim that Mihomo's complete
runtime has been reimplemented in Rust. TUN handling, kernel routing, and
protocol transports that still require the upstream Mihomo core remain an
explicit external boundary in `vpn233-client`.

The source directory and public API are organized around the Mihomo
integration boundary.

## Development

```bash
cargo test -p vpn233-core-lib-rust
cargo publish -p vpn233-core-lib-rust --dry-run
```
