# AGENTS.md

Compact guidance for working in `tpt-deb-toolkit`, a Rust cargo workspace (14 `tpt-l-*` crates) implementing a Debian/Ubuntu package toolchain.

## Commands

- Build/test everything: `cargo build --workspace`, `cargo test --workspace`
- CI-equivalent gating (run before pushing):
  - `cargo clippy --workspace -- -D warnings`
  - `cargo fmt --check --all`
  - `cargo doc --no-deps --workspace` (CI sets `RUSTDOCFLAGS=-D warnings`)
  - `cargo check --workspace`
- Single package: `cargo test -p tpt-l-deb-version`
- Single test: `cargo test -p tpt-l-deb-version <test_name>`
- Cross-compile (aarch64, as in CI): install `gcc-aarch64-linux-gnu` and set `CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_LINKER=aarch64-linux-gnu-gcc`, then `cargo check --workspace --target aarch64-unknown-linux-gnu`.

## Toolchain quirks

- Toolchain is pinned to `stable` in `rust-toolchain.toml`; edition `2021` lives in `[workspace.package]`, not the toolchain file.
- CI sets `RUSTFLAGS="-D warnings"`, so warnings fail the build. Run clippy with `-D warnings` locally to match; don't rely on plain `cargo build` to catch them.
- `fuzz/` is a **separate, excluded workspace** (it declares its own `[workspace]`). Run fuzzing from inside it with nightly: `cd fuzz && cargo +nightly fuzz run parse_version`.

## Testing gotchas

- `tpt-l-linux-sandbox-rs` and `tpt-l-maintainer-scripts` execute maintainer scripts via Linux namespaces/seccomp and are `#[cfg(target_os = "linux")]`-gated. On non-Linux (e.g. this Windows/WSL-less host) those tests are compiled out — you cannot run them locally here; verify by inspection or in CI/Linux.
- `tpt-l-deb-format` has an optional real-fixture test gated on `TPT_REAL_DEB=/path/to/file.deb`.
- `tpt-l-apt-transport` has a live-network test gated on `TPT_LIVE_MIRROR=<mirror base url>` (e.g. `http://deb.debian.org/debian`). These need network and are off by default.
- Integration tests requiring network/Linux remain incomplete by design (documented in `todo.md`); don't treat their absence as a bug.

## Architecture notes (not obvious from filenames)

- Layered dependency stack: 0 Sandbox → 1 Format → 2 Database → 3 Network → 4 Scripts → 5 CLI (`tpt-l-apt-cli`). Lower layers have no internal deps and are independently publishable.
- `tpt-l-control-file` intentionally uses a **custom lazy zero-copy parser**, NOT `deb822-lossless` (despite the older `Cargo.toml` comment). `tpt-l-apt-keyring` uses `pgp` (rPGP), NOT `sequoia-pgp`. `tpt-l-sources-list` does NOT depend on `tpt-l-control-file`. Rationale in `docs/DEPENDENCY_DECISIONS.md` — read it before re-litigating these choices.
- Zero-copy is a core principle: `.deb`/status files use `memmap2` and lazy `BorrowedParagraph<'a>`; avoid introducing eager full-file reads.
- Sandbox threat model and limitations: `docs/SANDBOX_THREAT_MODEL.md`. Benchmark conventions: `docs/BENCHMARK_METHODOLOGY.md`.

## Publishing

- All crates dual-licensed `MIT OR Apache-2.0`; workspace deps pin internal crates with `version = "0.1.0"`.
- `cargo publish` must proceed in dependency order (leaves first). Internal deps are not on crates.io yet, so dry-runs for dependent crates fail at the index lookup until leaves are published.

## Docs to read first

`README.md`, `todo.md` (audit of actual state), and the three `docs/*.md` files above. Treat `todo.md` as the source of truth for what is implemented vs. stubbed.
