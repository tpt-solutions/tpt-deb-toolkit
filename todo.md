# tpt-deb-toolkit — Task Checklist

> Dual-licensed MIT/Apache-2.0 · Copyright TPT Solutions · crates.io publishing

> Checklist last audited 2026-08-27 against actual repo state. `[~]` = partially done (see note).

---

## Phase 0 — Workspace Bootstrap

- [x] `git init` and initial commit
- [x] Create workspace `Cargo.toml` (members list all 14 crates)
- [~] Create `rust-toolchain.toml` (channel = "stable", edition 2021) — channel set; edition 2021 lives in `[workspace.package]`, not this file
- [x] Create `.gitignore` (target/, *.deb test fixtures, secrets)
- [x] Create `LICENSE-MIT`
- [x] Create `LICENSE-APACHE`
- [x] Create `README.md` (project overview, workspace map, build instructions)
- [x] Create `.github/workflows/ci.yml`
  - [x] `cargo check` on stable
  - [x] `cargo test --workspace` on stable + beta
  - [x] `cargo clippy -- -D warnings`
  - [x] `cargo fmt --check`
  - [x] `cargo doc --no-deps --workspace`
  - [~] Cross-compile check for x86_64-unknown-linux-gnu (job exists but runs on ubuntu-latest, i.e. native not cross-compiled)

---

## Phase 1 — Layer 0: Sandbox

### `tpt-l-linux-sandbox-rs`
- [x] Create `crates/tpt-l-linux-sandbox-rs/Cargo.toml` (crates.io metadata, categories, keywords)
- [x] Define public API: `Sandbox`, `SandboxConfig`, `SandboxBuilder`
- [~] Implement Linux namespace isolation (user, pid, mount, network) — `unshare()` for user/pid/mount/net flags set, but no pivot_root/chroot/mount calls actually applied
- [ ] Implement seccomp profile (allowlist syscalls for maintainer scripts) — not started, no seccomp dep
- [ ] Implement filesystem bind-mount configuration — `extra_bind_mounts` field exists but unused/not applied
- [x] Add `SandboxConfig::maintainer_script_profile()` preset
- [x] Implement `Sandbox::run(cmd, args, env)` → `ExitStatus`
- [x] Handle explicit opt-out (`SandboxConfig::unrestricted()`) with log warning
- [~] Unit tests: namespace isolation verified, seccomp blocks forbidden syscalls — 6 tests check config flags only; no seccomp to test
- [ ] Integration tests: run a real shell script inside sandbox
- [x] Add `#[cfg(target_os = "linux")]` gates (sandbox is Linux-only)
- [x] Documentation + examples

---

## Phase 2 — Layer 1: Format & Parsing

### `tpt-l-deb-version`
- [x] Create `crates/tpt-l-deb-version/Cargo.toml`
- [x] Implement `Version` struct with epoch, upstream, debian-revision fields
- [x] Implement `Version::parse(s: &str) -> Result<Version, VersionError>`
- [x] Implement `Ord`/`PartialOrd` using Debian's comparison algorithm (tilde ordering, non-numeric segments, epoch defaults)
- [x] Implement `VersionConstraint` (<<, <=, =, >=, >>) and `.satisfies(version)` check
- [x] Exhaustive test suite against known-tricky version pairs from real archive (~35 tests)
  - [x] Epoch handling (1:1.0 vs 1.0)
  - [x] Tilde ordering (1.0~beta < 1.0)
  - [x] Non-numeric segments
  - [x] Revision-less packages
- [x] Fuzz target (`cargo-fuzz`) for `Version::parse` — `fuzz/fuzz_targets/parse_version.rs` (no-panic + round-trip invariants; run with `cargo +nightly fuzz run parse_version`)
- [x] Documentation + examples

### `tpt-l-control-file`
- [~] Create `crates/tpt-l-control-file/Cargo.toml` (dep: `deb822-lossless`) — Cargo.toml exists but does NOT depend on `deb822-lossless`; hand-rolled parser used instead
- [ ] Evaluate `deb822-lossless` coverage; document which cases are wrapped vs. custom
- [ ] Implement `ControlFile` type wrapping `deb822-lossless` for single-stanza files — no such type; only `ControlParagraph`
- [ ] Implement `PackagesIndex` — streaming multi-stanza parser for large Packages files — parser loads whole string into memory, no `PackagesIndex` type
  - [ ] Strict multi-stanza validation (no duplicate fields) — duplicate keys silently overwrite
  - [ ] Zero-copy field access where possible — fields stored as owned `String`s
- [ ] Implement `SourcesIndex` parser — not present (sources handled separately in `tpt-l-sources-list`)
- [~] Implement `BinaryPackage`, `SourcePackage` typed structs (derive from stanza fields) — `BinaryPackage` exists; `SourcePackage` missing entirely
- [~] Unit tests: parse real-world control files, edge cases (folded fields, multi-line) — 4 basic tests; no folded/multi-line continuation test
- [ ] Benchmark: parse a 50 MB Packages.gz in-memory blob — no `benches/` dir
- [x] Documentation + examples

### `tpt-l-deb-format`
- [~] Create `crates/tpt-l-deb-format/Cargo.toml` (deps: `memmap2`, `tpt-l-control-file`) — deps declared but `memmap2` never actually used
- [ ] Implement ar archive reader (`ArReader`) — streaming, no full load into RAM — no `ArReader` type; whole archive read into memory
- [~] Implement `DebFile::open(path)` using `memmap2` — method exists but uses `std::fs::read`, not memmap2
- [x] Identify and expose `control.tar.*` and `data.tar.*` payloads by name
- [ ] Implement tar streaming extraction API (`DataEntries`, `ControlEntries` iterators) — `entries()` returns already-materialized slice, not lazy
- [~] Handle compression variants: .gz, .xz, .zst, uncompressed — only .gz and uncompressed handled; .xz/.zst return `UnsupportedCompression` despite deps present
- [x] Implement `DebMetadata` (parsed control fields from control.tar)
- [ ] Implement streaming extraction to filesystem path — no such method
- [ ] Unit tests: round-trip with known .deb fixtures — only 3 trivial tests, none using a real/synthesized .deb
- [ ] Integration tests: extract a real .deb and verify contents
- [ ] Benchmark: zero-copy metadata read of a 50 MB .deb
- [x] Documentation + examples

---

## Phase 3 — Layer 2: Database & Configuration

### `tpt-l-dpkg-db`
- [~] Create `crates/tpt-l-dpkg-db/Cargo.toml` (deps: `memmap2`, `tpt-l-control-file`) — depends on `tpt-l-control-file` but not `memmap2`
- [~] Implement `StatusDb::open(path)` with memory-mapped read — exists but uses `std::fs::read_to_string`, not memory-mapping
- [ ] Implement concurrent read access (multiple readers, single writer) — no locking primitives at all
- [x] Implement `StatusDb::installed_packages()` → iterator over `InstalledPackage`
- [~] Implement `StatusDb::write_atomic(changes)` — write to temp file, fsync, rename — atomic write done, but writes whole DB rather than accepting a `changes` diff
- [x] Implement package state machine: `installed`, `half-installed`, `config-files`, `unpacked`, `half-configured`, `triggers-awaited`, `triggers-pending`
- [x] Unit tests: read a real `/var/lib/dpkg/status` snapshot, write + read back
- [ ] Concurrency tests: concurrent readers don't deadlock — no threading tests
- [x] Documentation + examples

### `tpt-l-sources-list`
- [~] Create `crates/tpt-l-sources-list/Cargo.toml` (dep: `tpt-l-control-file`) — does not depend on it; deb822 parsing hand-rolled locally
- [~] Implement `SourcesList::parse_one_line_format(path)` (legacy `sources.list`) — implemented as `parse_one_line` (string) + `load_file` (path), not the exact named method
- [~] Implement `SourcesList::parse_deb822_format(path)` (`.sources` files) — implemented as `parse_deb822` (string) + `load_file` (path)
- [~] Implement `SourcesDir::load(dir)` — scan and parse all entries in `sources.list.d/` — exists as `SourcesList::load_dir(dir)`, no dedicated `SourcesDir` type
- [x] Implement `SourceEntry` struct: `type` (deb/deb-src), `uri`, `suite`, `components`, `options`
- [~] Implement URI validation and option parsing — option parsing done; no URI validation
- [~] Implement `SourcesList::write(path)` for round-trip writing — `write_one_line()` returns a `String`; no filesystem-writing method, no deb822 writer
- [x] Unit tests: parse standard Ubuntu sources.list, deb822 format, edge cases (12 tests)
- [x] Documentation + examples

### `tpt-l-apt-config`
- [x] Create `crates/tpt-l-apt-config/Cargo.toml`
- [x] Implement `AptConfig::load(path)` for `apt.conf`
- [x] Implement `#include` and `#include-dir` directive resolution
- [~] Implement type-casting: string, integer, boolean, list values — string/bool done; no dedicated integer getter; list append syntax (`Key:: "v";`) not actually parsed into lists (overwrites instead)
- [x] Implement `AptConfig::load_dir(dir)` — scan `apt.conf.d/` in alphabetical order
- [x] Implement `AptConfig::get(key)`, `get_or_default(key, default)`, `get_list(key)`
- [x] Unit tests: parse real apt.conf snippets, include resolution, type casting (12 tests)
- [x] Documentation + examples

---

## Phase 4 — Layer 3: Network & Resolution

### `tpt-l-apt-transport`
- [x] Create `crates/tpt-l-apt-transport/Cargo.toml` (deps: `tokio`, `reqwest`, `flate2`, `xz2`, `zstd`)
- [~] Implement `AptTransport` struct with connection pooling — struct exists; "pooling" is just default `reqwest::Client` behavior, not explicitly configured
- [~] Implement async `fetch_index(uri, suite, component)` → `Packages` bytes — implemented with an extra `arch` param, otherwise matches
- [~] Implement `InRelease`/`Release` + `Release.gpg` fetch — InRelease→Release fallback done; no separate `Release.gpg` fetch method
- [ ] Implement partial download (HTTP Range, `If-Modified-Since`) — not implemented
- [x] Implement mirror failover (try next mirror on error)
- [ ] Implement delta index updates (PDiff support) — not implemented
- [x] Implement decompression pipeline (detect extension, stream decompress)
- [~] Implement async `.deb` file download with progress callback — buffers whole response via `resp.bytes()` before writing; progress only reported once at the end, not truly streaming
- [~] Unit tests: mock HTTP server, partial downloads, failover — 4 tests cover config/decompression only; no mock HTTP server or partial-download/failover tests
- [ ] Integration tests: fetch from a real Ubuntu mirror (gated, CI optional)
- [x] Documentation + examples

### `tpt-l-apt-keyring`
- [~] Create `crates/tpt-l-apt-keyring/Cargo.toml` (dep: `sequoia-pgp`) — uses `pgp` (rPGP) crate instead of `sequoia-pgp`
- [x] Implement `Keyring::load(path)` for `.gpg` keyring files (also handles `.asc` armored)
- [x] Implement `Keyring::load_dir(dir)` for `/etc/apt/trusted.gpg.d/`
- [~] Implement `Keyring::verify(release_file, signature)` → `VerifyResult` — split into `verify_clearsigned`/`verify_detached`, both return `VerifyResult`
- [x] Implement `InRelease` (clearsigned) verification
- [x] Implement `Release` + `Release.gpg` (detached sig) verification
- [ ] Implement key expiry and revocation checks — not implemented
- [ ] Implement `apt-key` replacement workflow (import, list, delete) — not implemented
- [ ] Unit tests: verify against known Ubuntu signing key + Release fixture — only 4 trivial error-path tests, no real key/signature fixture
- [x] Documentation + examples

### `tpt-l-apt-solver`
- [x] Create `crates/tpt-l-apt-solver/Cargo.toml` (deps: `tpt-l-deb-version`, `tpt-l-control-file`, `rayon`)
- [x] Implement `Universe` — in-memory constraint graph from parsed Packages indices
- [~] Model: `Depends`, `Pre-Depends`, `Recommends`, `Suggests`, `Conflicts`, `Breaks`, `Provides` (virtual packages) — `Recommends`/`Suggests` entirely absent; rest modeled
- [~] Implement DPLL-based SAT solver core — basic DPLL present, simplified vs. spec
  - [x] Unit propagation
  - [ ] Conflict analysis and clause learning — not implemented (plain chronological backtracking)
  - [ ] Non-chronological backtracking — not implemented
  - [~] VSIDS-inspired variable ordering — picks by static occurrence count, not real VSIDS with activity/decay
- [~] Implement multi-threaded solving with `rayon` — rayon only parallelizes `Universe::from_binary_packages` parsing; DPLL search itself is single-threaded
- [x] Handle virtual packages and alternative dependencies (Depends: a | b)
- [~] Implement `InstallPlan` output (ordered list of packages to install/upgrade/remove) — `remove` is always empty; solver doesn't model existing installed state
- [ ] **Benchmark harness** (`benches/solver_vs_libsolv.rs`) — no `benches/` dir
  - [ ] Download Ubuntu archive snapshot (scripts in `bench-data/`) — `bench-data/` exists but is empty
  - [ ] Run tpt-l-apt-solver and record wall-clock + plan quality
  - [ ] Run libsolv (via FFI or subprocess) on identical input
  - [ ] Generate reproducible benchmark report
- [x] Unit tests: small hand-crafted constraint graphs, conflict detection, virtual packages (7 tests)
- [ ] Integration tests: solve against a real Ubuntu Packages snapshot
- [x] Documentation + examples

---

## Phase 5 — Layer 4: Script & Trigger Execution

### `tpt-l-maintainer-scripts`
> Not started — `src/lib.rs` is a 2-line stub. Cargo.toml has crates.io metadata but is missing the `tpt-l-linux-sandbox-rs` and `tokio` dependencies.
- [ ] Create `crates/tpt-l-maintainer-scripts/Cargo.toml` (deps: `tpt-l-linux-sandbox-rs`, `tokio`)
- [ ] Implement script runner for `preinst`, `postinst`, `prerm`, `postrm`
- [ ] Implement Debian ordering contract (preinst → unpack → postinst; prerm → remove → postrm)
- [ ] Implement exit-code semantics (0 = success, nonzero = abort with rollback signal)
- [ ] Implement `DEBIAN_FRONTEND` environment variable handling
- [ ] Run scripts inside `tpt-l-linux-sandbox-rs` sandbox by default
- [ ] Implement explicit opt-out (`ScriptRunner::unrestricted()`) with structured log warning
- [ ] Implement environment setup (PATH, DPKG_MAINTSCRIPT_*, etc.)
- [ ] Unit tests: mock scripts, exit code handling, environment injection
- [ ] Integration tests: run real package maintainer scripts in sandbox
- [ ] Documentation + examples

### `tpt-l-dpkg-triggers`
> Not started — `src/lib.rs` is a 2-line stub. Cargo.toml is missing the `tpt-l-dpkg-db` dependency.
- [ ] Create `crates/tpt-l-dpkg-triggers/Cargo.toml` (dep: `tpt-l-dpkg-db`)
- [ ] Implement trigger types: `interest`, `interest-noawait`, `activate`, `activate-noawait`
- [ ] Implement trigger database (read/write pending triggers from dpkg db)
- [ ] Implement deferred trigger processing loop
- [ ] Implement trigger activation during install/remove
- [ ] Implement `triggers-awaited` / `triggers-pending` state transitions in dpkg status
- [ ] Unit tests: trigger registration, activation, processing order
- [ ] Documentation + examples

---

## Phase 6 — Layer 5: Tools

### `tpt-l-deb-diff`
> Not started — `src/lib.rs` is a 2-line stub. Cargo.toml is missing `tpt-l-deb-format`/`tpt-l-control-file` dependencies.
- [ ] Create `crates/tpt-l-deb-diff/Cargo.toml` (deps: `tpt-l-deb-format`, `tpt-l-control-file`)
- [ ] Implement structural diff of two `.deb` files
- [ ] Diff metadata: control field changes
- [ ] Diff file trees: added, removed, modified files (by path)
- [ ] Diff checksums: flag files with changed content
- [ ] Implement `DiffReport` output type (structured, serializable)
- [ ] Implement human-readable diff output formatter
- [ ] Unit tests: diff identical debs (empty diff), diff with known changes
- [ ] Documentation + examples

### `tpt-l-apt-cli`
> Not started — `src/lib.rs` is a 2-line stub. Cargo.toml is missing `clap`, `anyhow`, `tracing-subscriber`, and all layer-crate dependencies.
- [ ] Create `crates/tpt-l-apt-cli/Cargo.toml` (deps: all layer crates, `clap`, `anyhow`, `tracing-subscriber`)
- [ ] Implement `tpt-l-apt update` — fetch and cache indices
- [ ] Implement `tpt-l-apt install <pkg>...` — solve + download + extract + scripts
- [ ] Implement `tpt-l-apt search <query>` — search cached indices
- [ ] Implement `tpt-l-apt show <pkg>` — display package metadata
- [ ] Implement `tpt-l-apt list --installed` — list installed packages from dpkg db
- [ ] Implement progress bars / structured output (JSON flag)
- [ ] Implement global flags: `--dry-run`, `--verbose`, `--config`
- [ ] End-to-end integration test: install a small package in a chroot
- [ ] Documentation + examples
- [ ] Shell completions (bash, zsh, fish) via clap

---

## Phase 7 — Polish & Publishing

- [~] Audit all `Cargo.toml` files for crates.io required fields (description, license, repository, homepage, documentation, keywords, categories) — description/license/repository/keywords/categories set on all 14; `homepage`/`documentation` missing everywhere despite being defined in `[workspace.package]`
- [x] Set `license = "MIT OR Apache-2.0"` in all crates (via `license.workspace = true`)
- [x] Set `authors = ["TPT Solutions"]` in all crates (via `authors.workspace = true`)
- [~] Add README shields (CI status, crates.io version, docs.rs) — CI + license badges present; no crates.io/docs.rs badges yet (not published)
- [ ] Run `cargo publish --dry-run` for each crate in dependency order
- [ ] Verify `cargo doc --no-deps --workspace` produces clean docs
- [x] Write `CONTRIBUTING.md`
- [x] Write `SECURITY.md`
- [ ] Tag `v0.1.0` release and publish to crates.io in dependency order
- [ ] Post-publish: verify all docs.rs pages build correctly

---

## Open Questions (from spec §5)

- [ ] **Benchmark methodology** — decide what "outperform" means (wall-clock, plan quality, or both) before any external claims
- [ ] **deb822-lossless dependency** — audit coverage; document wrap-vs-custom decision in `tpt-l-control-file` (currently unused — a custom parser was written instead, undocumented)
- [ ] **Sandbox threat model** — define minimum viable syscall + filesystem allowlist for `tpt-l-linux-sandbox-rs` that doesn't break common Ubuntu packages (blocked on seccomp implementation, which hasn't started)

---

## Systemic gaps (flagged by 2026-08-27 audit)

- 4 of 14 crates are pure stubs (`tpt-l-apt-cli`, `tpt-l-deb-diff`, `tpt-l-dpkg-triggers`, `tpt-l-maintainer-scripts`) — all of Phase 5 and Phase 6 is unimplemented.
- No `tests/`, `benches/`, or `fuzz/` directories exist anywhere — all testing is inline `#[cfg(test)]` modules; every integration/benchmark/fuzz checklist item is not done.
- Several crates diverge from the spec's chosen dependencies, undocumented: `tpt-l-control-file` doesn't use `deb822-lossless`; `tpt-l-apt-keyring` uses `pgp` (rPGP) instead of `sequoia-pgp`; `tpt-l-sources-list` doesn't depend on `tpt-l-control-file`.
- `memmap2` is declared but unused in `tpt-l-deb-format`, and absent (though required) in `tpt-l-dpkg-db` — neither crate does memory-mapped I/O yet.
- `homepage`/`documentation` Cargo.toml fields are missing on every crate.
