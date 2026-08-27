# tpt-deb-toolkit

[![CI](https://github.com/tpt-solutions/tpt-deb-toolkit/actions/workflows/ci.yml/badge.svg)](https://github.com/tpt-solutions/tpt-deb-toolkit/actions/workflows/ci.yml)
[![License: MIT OR Apache-2.0](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](LICENSE-MIT)
[![crates.io](https://img.shields.io/crates/v/tpt-l-deb-version.svg)](https://crates.io/crates/tpt-l-deb-version)
[![docs.rs](https://docs.rs/tpt-l-deb-version/badge.svg)](https://docs.rs/tpt-l-deb-version)

A complete, workspace-scale, zero-copy pipeline for Debian/Ubuntu package management — written in pure Rust.

## Workspace Map

| Crate | Layer | Description |
|-------|-------|-------------|
| `tpt-l-linux-sandbox-rs` | 0 — Sandbox | Linux namespace + seccomp isolation for maintainer scripts |
| `tpt-l-deb-version` | 1 — Format | Debian epoch:upstream-revision version comparison |
| `tpt-l-control-file` | 1 — Format | deb822-style control-file parser (custom lazy parser; see `docs/DEPENDENCY_DECISIONS.md`) |
| `tpt-l-deb-format` | 1 — Format | ar/tar `.deb` archive reader with zero-copy streaming |
| `tpt-l-deb-diff` | 1 — Format | Structural diff of two `.deb` files |
| `tpt-l-dpkg-db` | 2 — Database | Concurrent reader/writer for `/var/lib/dpkg/status` |
| `tpt-l-sources-list` | 2 — Database | Parser/writer for `sources.list` and deb822 `.sources` files |
| `tpt-l-apt-config` | 2 — Database | Parser for `apt.conf` and `apt.conf.d/` |
| `tpt-l-apt-transport` | 3 — Network | Async HTTP transport with mirror failover, partial (resumable) downloads, and PDiff |
| `tpt-l-apt-keyring` | 3 — Network | OpenPGP `Release` file verification via `pgp` (rPGP) |
| `tpt-l-apt-solver` | 3 — Network | DPLL SAT-based parallel dependency resolver |
| `tpt-l-maintainer-scripts` | 4 — Scripts | preinst/postinst/prerm/postrm runner with sandbox integration |
| `tpt-l-dpkg-triggers` | 4 — Scripts | dpkg trigger activation and deferred processing |
| `tpt-l-apt-cli` | 5 — CLI | `tpt-apt update / install / search / show / list` |

## Build

```bash
# Requires Rust stable (see rust-toolchain.toml)
cargo build --workspace

# Run all tests
cargo test --workspace

# Cross-compile check for Linux target
cargo check --target x86_64-unknown-linux-gnu
```

## License

Dual-licensed under [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE) at your option.

Copyright © 2026 TPT Solutions.
