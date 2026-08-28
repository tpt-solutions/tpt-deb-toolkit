# tpt-deb-toolkit

[![CI](https://github.com/tpt-solutions/tpt-deb-toolkit/actions/workflows/ci.yml/badge.svg)](https://github.com/tpt-solutions/tpt-deb-toolkit/actions/workflows/ci.yml)
[![License: MIT OR Apache-2.0](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](LICENSE-MIT)
[![crates.io](https://img.shields.io/crates/v/tpt-l-deb-version.svg)](https://crates.io/crates/tpt-l-deb-version)
[![docs.rs](https://docs.rs/tpt-l-deb-version/badge.svg)](https://docs.rs/tpt-l-deb-version)

A complete, workspace-scale, zero-copy pipeline for Debian/Ubuntu package management — written in pure Rust.

`tpt-deb-toolkit` is a set of small, composable crates that together implement the core of an `apt`/`dpkg`-style package toolchain: from parsing `.deb` archives and control files, through PGP-verified index transport, to a SAT-based dependency solver and a sandboxed maintainer-script runner. At the top sits a working `tpt-apt` CLI.

## Features

- **Zero-copy parsing** — `.deb` archives (`ar`/`tar`) and `dpkg` status databases are memory-mapped (`memmap2`) and parsed lazily, so large Packages indices (50 MB+) parse in well under a second without loading everything into RAM.
- **Correct Debian version semantics** — epoch/upstream/revision comparison with tilde ordering, plus `<<`/`<=`/`=`/`>=`/`>>` constraint satisfaction, validated against ~35 known-tricky real-archive version pairs.
- **PGP-verified transport** — async `reqwest` transport with connection pooling, mirror failover, partial (resumable) HTTP Range downloads, `If-Modified-Since` caching, and PDiff delta index updates (pure-Rust rdiff2, no C `librsync`).
- **Cryptographic release verification** — `InRelease` (clearsigned) and `Release` + `Release.gpg` (detached) verification via `pgp` (rPGP), including key expiry and revocation checks, plus an `apt-key`-style import/list/remove workflow.
- **SAT-based resolution** — a from-scratch CDCL solver (unit propagation, 1UIP conflict analysis, non-chronological backtracking, VSIDS) with a parallel `rayon` portfolio, modeling Depends/Pre-Depends/Recommends/Suggests/Conflicts/Breaks/Provides and virtual packages.
- **Sandboxed script execution** — maintainer scripts run inside a Linux user/pid/mount/network namespace with a seccomp syscall allowlist and bind-mount policy, with an explicit opt-out.
- **Reproducible diffs** — structural `.deb` comparison (control-field, file-tree, and SHA-256 content diffs) with a serializable, JSON-friendly report.

## Workspace Map

| Crate | Layer | Description |
|-------|-------|-------------|
| `tpt-l-linux-sandbox-rs` | 0 — Sandbox | Linux namespace + seccomp isolation for maintainer scripts |
| `tpt-l-deb-version` | 1 — Format | Debian epoch:upstream-revision version comparison |
| `tpt-l-control-file` | 1 — Format | deb822-style control-file parser (custom lazy zero-copy parser; see `docs/DEPENDENCY_DECISIONS.md`) |
| `tpt-l-deb-format` | 1 — Format | ar/tar `.deb` archive reader with zero-copy streaming extraction |
| `tpt-l-deb-diff` | 1 — Format | Structural diff of two `.deb` files |
| `tpt-l-dpkg-db` | 2 — Database | Concurrent reader/writer for `/var/lib/dpkg/status` |
| `tpt-l-sources-list` | 2 — Database | Parser/writer for `sources.list` and deb822 `.sources` files |
| `tpt-l-apt-config` | 2 — Database | Parser for `apt.conf` and `apt.conf.d/` |
| `tpt-l-apt-transport` | 3 — Network | Async HTTP transport with mirror failover, partial (resumable) downloads, and PDiff |
| `tpt-l-apt-keyring` | 3 — Network | OpenPGP `Release` file verification via `pgp` (rPGP) |
| `tpt-l-apt-solver` | 3 — Network | CDCL SAT-based parallel dependency resolver |
| `tpt-l-maintainer-scripts` | 4 — Scripts | preinst/postinst/prerm/postrm runner with sandbox integration |
| `tpt-l-dpkg-triggers` | 4 — Scripts | dpkg trigger activation and deferred processing |
| `tpt-l-apt-cli` | 5 — CLI | `tpt-apt update / install / search / show / list` |

## Usage

The top-level binary is `tpt-apt`:

```bash
# Fetch and cache package indices (InRelease + Packages, PDiff delta updates)
tpt-apt update

# Search cached indices
tpt-apt search vim

# Show package metadata
tpt-apt show vim

# Install packages (solve -> download -> extract -> postinst)
tpt-apt install vim curl

# Dry run (no changes applied)
tpt-apt install vim --dry-run

# List installed packages from the dpkg status database
tpt-apt list --installed

# Emit machine-readable JSON
tpt-apt show vim --json

# Generate shell completions
tpt-apt completions --shell bash --output ~/.local/share/bash-completion/completions/tpt-apt
```

Individual crates are also usable as libraries. For example, comparing two `.deb` files:

```rust
use tpt_l_deb_diff::DebDiff;

let report = DebDiff::compare("old.deb", "new.deb")?;
println!("{}", report); // human-readable diff
```

## Build

```bash
# Requires Rust stable (edition 2021, see rust-toolchain.toml)
cargo build --workspace

# Run all tests
cargo test --workspace

# Lint and format checks (CI gating)
cargo clippy --workspace -- -D warnings
cargo fmt --check

# Build docs
cargo doc --no-deps --workspace

# Cross-compile check for Linux target (aarch64)
cargo check --target aarch64-unknown-linux-gnu

# Run the fuzz target (requires nightly)
cargo +nightly fuzz run parse_version
```

## Library Integration

Each crate is published independently to crates.io under the `tpt-l-*` namespace and depends on its lower-layer siblings via versioned workspace dependencies. The lowest-level crates (`tpt-l-deb-version`, `tpt-l-linux-sandbox-rs`, `tpt-l-sources-list`, `tpt-l-apt-config`, `tpt-l-apt-keyring`, `tpt-l-apt-transport`) have no internal dependencies and can be adopted on their own.

## Design Decisions & Docs

A few implementation choices diverge from the original spec; they are documented under [`docs/`](docs):

- [`docs/DEPENDENCY_DECISIONS.md`](docs/DEPENDENCY_DECISIONS.md) — why `tpt-l-control-file` uses a custom lazy parser instead of `deb822-lossless`, `tpt-l-apt-keyring` uses `pgp` (rPGP) instead of `sequoia-pgp`, and `tpt-l-sources-list` doesn't depend on `tpt-l-control-file`.
- [`docs/SANDBOX_THREAT_MODEL.md`](docs/SANDBOX_THREAT_MODEL.md) — seccomp allowlist and bind-mount policy, plus threat-model limitations.
- [`docs/BENCHMARK_METHODOLOGY.md`](docs/BENCHMARK_METHODOLOGY.md) — benchmark axes, harness conventions, and "outperform" preconditions.

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md).

## License

Dual-licensed under [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE) at your option.

Copyright © 2026 TPT Solutions.
