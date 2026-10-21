# vpn233-core-lib-rust

The shared Rust library for VPN233's Clash/Mihomo-compatible configuration
model and protocol extension points.

This crate currently provides:

- typed helpers for proxy nodes and outbound definitions;
- deterministic Clash/Mihomo YAML rendering;
- URI/Base64, Mihomo YAML and sing-box JSON imports;
- 15 structured Mihomo protocols and 10 share-link protocols;
- protocol and transport aware exports for nine subscription formats;
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
cargo run -p vpn233-core-lib-rust --example mihomo_fixtures --locked -- target/mihomo-fixtures
cargo publish -p vpn233-core-lib-rust --dry-run
```

The example exports 57 synthetic, self-contained configurations into a fresh
directory. Use the repository's `scripts/e2e/check_mihomo_parser.py` with
`--core-fixtures-dir target/mihomo-fixtures` and a verified official Mihomo
binary to check them with `-t`. Parsing does not verify real proxy connectivity.
