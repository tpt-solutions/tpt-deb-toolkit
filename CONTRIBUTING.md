# Contributing to tpt-deb-toolkit

Thank you for your interest in contributing! This document explains how to set
up a development environment and the expectations for contributions.

## Development Setup

Requirements:

- Rust stable (the pinned channel is in `rust-toolchain.toml`)
- `rustfmt` and `clippy` (included with rustup: `rustup component add rustfmt clippy`)

```bash
git clone https://github.com/tpt-solutions/tpt-deb-toolkit
cd tpt-deb-toolkit
cargo build --workspace
cargo test --workspace
```

## Workspace Layout

The repository is a Cargo workspace of 14 crates organized in layers
(see the table in `README.md`). Lower layers must never depend on higher
layers:

- Layer 0: sandbox primitives
- Layer 1: format & parsing (`deb-version`, `control-file`, `deb-format`, `deb-diff`)
- Layer 2: database & configuration (`dpkg-db`, `sources-list`, `apt-config`)
- Layer 3: network (`apt-transport`, `apt-keyring`, `apt-solver`)
- Layer 4: scripts & triggers (`maintainer-scripts`, `dpkg-triggers`)
- Layer 5: tools (`apt-cli`)

## Before Opening a Pull Request

All of the following must pass locally:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo doc --no-deps --workspace
```

CI enforces exactly these checks on every pull request.

## Coding Conventions

- All public items must have doc comments with examples where practical.
- Errors are modeled with `thiserror`; application code uses `anyhow`.
- Linux-only functionality must be gated with `#[cfg(target_os = "linux")]`
  while keeping types available on other platforms for cross-compilation.
- Every crate must carry complete crates.io metadata (`description`,
  `license = "MIT OR Apache-2.0"`, `repository`, `keywords`, `categories`)
  and inherit shared fields from `[workspace.package]`.
- Tests live inline in `#[cfg(test)] mod tests` plus `tests/` integration
  directories. Bug fixes must come with a regression test.

## Commit Messages

Use imperative mood and reference issues when applicable:

```
Fix tilde ordering in deb-version comparison algorithm (#42)
```

## License

By contributing you agree that your contributions are dual-licensed under
MIT OR Apache-2.0, per the repository licensing.
