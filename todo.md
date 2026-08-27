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
- [x] Implement `PackagesIndex` — streaming multi-stanza parser for large Packages files — `PackagesIndex` type added with lazy `iter()`/`iter_results()` (stanzas parsed on demand)
  - [x] Strict multi-stanza validation (no duplicate fields) — `parse_control_strict` / `BinaryPackage::parse_stanza_strict` / `PackagesIndex::iter_results_strict` reject duplicate fields with `ControlError::DuplicateField`
  - [ ] Zero-copy field access where possible — fields stored as owned `String`s
- [ ] Implement `SourcesIndex` parser — not present (sources handled separately in `tpt-l-sources-list`)
- [x] Implement `BinaryPackage`, `SourcePackage` typed structs (derive from stanza fields) — `SourcePackage` added with `parse_stanza` (unit-tested)
- [x] Unit tests: parse real-world control files, edge cases (folded fields, multi-line) — folded continuation test added (`parse_control` now strips a single folding marker per Debian semantics)
- [x] Benchmark: parse a 50 MB Packages index in-memory blob — `benches/parse_packages.rs` (harness=false) parses ~50 MB in ~0.6s
- [x] Documentation + examples

### `tpt-l-deb-format`
- [x] Create `crates/tpt-l-deb-format/Cargo.toml` (deps: `memmap2`, `tpt-l-control-file`) — `memmap2` now used by `DebFile::open`
- [x] Implement ar archive reader (`ArReader`) — streaming, no full load into RAM
- [x] Implement `DebFile::open(path)` using `memmap2` — memory-maps the file instead of `std::fs::read`
- [x] Identify and expose `control.tar.*` and `data.tar.*` payloads by name
- [x] Handle compression variants: .gz, .xz, .zst, uncompressed
- [x] Implement `DebMetadata` (parsed control fields from control.tar)
- [x] Implement streaming extraction to filesystem path (`DebFile::extract`)
- [x] Implement tar streaming extraction API (`DataEntries`, `ControlEntries` iterators) — lazy, stream-decompressing iterators over the tar payloads
- [x] Unit tests: round-trip with known .deb fixtures — synthesized `.deb` fixtures in `testsupport`; covers open/parse, extract-to-disk, and streaming entries
- [ ] Integration tests: extract a real .deb and verify contents
- [ ] Benchmark: zero-copy metadata read of a 50 MB .deb
- [x] Documentation + examples

---

## Phase 3 — Layer 2: Database & Configuration

### `tpt-l-dpkg-db`
- [x] Create `crates/tpt-l-dpkg-db/Cargo.toml` (deps: `memmap2`, `tpt-l-control-file`) — now depends on both; `memmap2` used by `StatusDb::open`
- [x] Implement `StatusDb::open(path)` with memory-mapped read — now memory-maps via `memmap2` and parses through the mapping
- [x] Implement concurrent read access (multiple readers, single writer) — `ConcurrentStatusDb` wraps `RwLock<StatusDb>`; `read()` returns a shared guard, `apply_changes()` takes the exclusive lock and persists atomically
- [x] Implement `StatusDb::installed_packages()` → iterator over `InstalledPackage`
- [x] Implement `StatusDb::write_atomic(changes)` — `write_atomic` does temp+fsync+rename; `StatusDb::apply_changes(&[StatusChange])` applies a diff (upsert/remove) before persisting
- [x] Implement package state machine: `installed`, `half-installed`, `config-files`, `unpacked`, `half-configured`, `triggers-awaited`, `triggers-pending`
- [x] Unit tests: read a real `/var/lib/dpkg/status` snapshot, write + read back
- [x] Concurrency tests: concurrent readers don't deadlock — 4 readers + a writer over `ConcurrentStatusDb`
- [x] Documentation + examples

### `tpt-l-sources-list`
- [~] Create `crates/tpt-l-sources-list/Cargo.toml` (dep: `tpt-l-control-file`) — does not depend on it; deb822 parsing hand-rolled locally
- [~] Implement `SourcesList::parse_one_line_format(path)` (legacy `sources.list`) — implemented as `parse_one_line` (string) + `load_file` (path), not the exact named method
- [~] Implement `SourcesList::parse_deb822_format(path)` (`.sources` files) — implemented as `parse_deb822` (string) + `load_file` (path)
- [~] Implement `SourcesDir::load(dir)` — scan and parse all entries in `sources.list.d/` — exists as `SourcesList::load_dir(dir)`, no dedicated `SourcesDir` type
- [x] Implement `SourceEntry` struct: `type` (deb/deb-src), `uri`, `suite`, `components`, `options`
- [x] Implement URI validation and option parsing — `SourceEntry::validate_uri`/`SourcesList::validate` reject bad schemes/hosts/whitespace (unit-tested)
- [x] Implement `SourcesList::write(path)` for round-trip writing — `write()` picks format by extension; `write_deb822()` added (unit-tested)
- [x] Unit tests: parse standard Ubuntu sources.list, deb822 format, edge cases (12 tests)
- [x] Documentation + examples

### `tpt-l-apt-config`
- [x] Create `crates/tpt-l-apt-config/Cargo.toml`
- [x] Implement `AptConfig::load(path)` for `apt.conf`
- [x] Implement `#include` and `#include-dir` directive resolution
- [x] Implement type-casting: string, integer, boolean, list values — `get_int` added; `Key:: "v";` list-append syntax parsed into `ConfigValue::List` (unit-tested)
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
- [x] Implement async `.deb` file download with progress callback — `download_file` now streams chunks and reports progress after each chunk (no full buffering)
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
- [x] Model: `Depends`, `Pre-Depends`, `Recommends`, `Suggests`, `Conflicts`, `Breaks`, `Provides` (virtual packages) — all six relations modeled; `Recommends`/`Suggests` exposed via `InstallPlan::recommended`/`suggested`
- [x] Implement DPLL-based SAT solver core — full CDCL implemented (verified against brute-force SAT in tests)
  - [x] Unit propagation
  - [x] Conflict analysis and clause learning — 1UIP `analyze()` + learnt-clause addition
  - [x] Non-chronological backtracking — `cancel_until(btlevel)`
  - [x] VSIDS variable ordering — activity scores with `var_inc` bump + `var_decay` decay
- [x] Implement multi-threaded solving with `rayon` — parallel portfolio solver (`Solver::solve_parallel`) runs N seed-diversified CDCL workers via Rayon; first SAT wins
- [x] Handle virtual packages and alternative dependencies (Depends: a | b)
- [x] Implement `InstallPlan` output (install/upgrade/remove + recommended/suggested) — models installed state via `resolve_with_installed`; removes + upgrades computed from the diff
- [~] **Benchmark harness** (`benches/solver_vs_libsolv.rs`) — tpt-side timing scaffold added as `examples/bench_solver.rs` (generates a synthetic universe and times `resolve`); `libsolv` comparison still TODO (needs FFI/subprocess)
  - [ ] Download Ubuntu archive snapshot (scripts in `bench-data/`) — `bench-data/` exists but is empty
  - [x] Run tpt-l-apt-solver and record wall-clock + plan quality — `examples/bench_solver.rs` does this
  - [ ] Run libsolv (via FFI or subprocess) on identical input
  - [ ] Generate reproducible benchmark report
- [x] Unit tests: CDCL vs brute-force SAT (300 random), parallel==serial, Recommends, installed keep/upgrade/conflict removal (16 tests)
- [ ] Integration tests: solve against a real Ubuntu Packages snapshot
- [x] Documentation + examples

---

## Phase 5 — Layer 4: Script & Trigger Execution

### `tpt-l-maintainer-scripts`
> Implemented — `ScriptRunner` runs `preinst`/`postinst`/`prerm`/`postrm` with full `DPKG_MAINTSCRIPT_*`/`DEBIAN_FRONTEND` env, correct ordering contract, exit-code semantics, sandbox-by-default (with `unrestricted()` opt-out + structured warning), async wrapper, and unit tests. Unix-gated execution tests cover exit-code propagation and env injection.
- [x] Create `crates/tpt-l-maintainer-scripts/Cargo.toml` (deps: `tpt-l-linux-sandbox-rs`, `tokio`)
- [x] Implement script runner for `preinst`, `postinst`, `prerm`, `postrm`
- [x] Implement Debian ordering contract (preinst → unpack → postinst; prerm → remove → postrm)
- [x] Implement exit-code semantics (0 = success, nonzero = abort with rollback signal)
- [x] Implement `DEBIAN_FRONTEND` environment variable handling
- [x] Run scripts inside `tpt-l-linux-sandbox-rs` sandbox by default
- [x] Implement explicit opt-out (`ScriptRunner::unrestricted()`) with structured log warning
- [x] Implement environment setup (PATH, DPKG_MAINTSCRIPT_*, etc.)
- [x] Unit tests: mock scripts, exit code handling, environment injection
- [ ] Integration tests: run real package maintainer scripts in sandbox (gated; requires Linux + sh)
- [x] Documentation + examples

### `tpt-l-dpkg-triggers`
> Implemented — `TriggerDb` models `interest`/`interest-noawait`/`activate`/`activate-noawait`, registers interests, activates (idempotent), dequeues pending via `process()`, persists to a triggers dir, and bridges to `tpt_l_dpkg_db::InstallStatus` transitions (`mark_pending`/`mark_awaited`/`clear`). Unit tests cover full lifecycle + state transitions.
- [x] Create `crates/tpt-l-dpkg-triggers/Cargo.toml` (dep: `tpt-l-dpkg-db`)
- [x] Implement trigger types: `interest`, `interest-noawait`, `activate`, `activate-noawait`
- [x] Implement trigger database (read/write pending triggers from dpkg db)
- [x] Implement deferred trigger processing loop
- [x] Implement trigger activation during install/remove
- [x] Implement `triggers-awaited` / `triggers-pending` state transitions in dpkg status
- [x] Unit tests: trigger registration, activation, processing order
- [x] Documentation + examples

---

## Phase 6 — Layer 5: Tools

### `tpt-l-deb-diff`
> Implemented — `DebDiff::compare` produces a serializable `DiffReport` (metadata field changes, added/removed/modified file trees, content SHA-256 checksums) with a human-readable formatter. Synthesizes `.deb` fixtures in-memory for tests covering identical/version/file changes.
- [x] Create `crates/tpt-l-deb-diff/Cargo.toml` (deps: `tpt-l-deb-format`, `tpt-l-control-file`)
- [x] Implement structural diff of two `.deb` files
- [x] Diff metadata: control field changes
- [x] Diff file trees: added, removed, modified files (by path)
- [x] Diff checksums: flag files with changed content
- [x] Implement `DiffReport` output type (structured, serializable)
- [x] Implement human-readable diff output formatter
- [x] Unit tests: diff identical debs (empty diff), diff with known changes
- [x] Documentation + examples

### `tpt-l-apt-cli`
> Implemented — clap-based CLI (`src/main.rs` + library). Subcommands `update` (fetch+co-cache indices via `tpt-l-apt-transport`), `install` (solver + download + extract + `postinst` via `tpt-l-maintainer-scripts`, with `--dry-run`), `search`/`show` (cached `Packages` indices), `list --installed` (dpkg status db). Global `--config`/`--verbose`/`--dry-run`/`--json` flags. Offline unit tests cover search/show/list.
- [x] Create `crates/tpt-l-apt-cli/Cargo.toml` (deps: all layer crates, `clap`, `anyhow`, `tracing-subscriber`)
- [x] Implement `tpt-l-apt update` — fetch and cache indices
- [x] Implement `tpt-l-apt install <pkg>...` — solve + download + extract + scripts
- [x] Implement `tpt-l-apt search <query>` — search cached indices
- [x] Implement `tpt-l-apt show <pkg>` — display package metadata
- [x] Implement `tpt-l-apt list --installed` — list installed packages from dpkg db
- [x] Implement progress bars / structured output (JSON flag) — `indicatif` bars in `update`/`install` (suppressed under `--json`); all subcommands emit `--json`
- [x] Implement global flags: `--dry-run`, `--verbose`, `--config`
- [ ] End-to-end integration test: install a small package in a chroot (requires network + Linux)
- [x] Documentation + examples
- [x] Shell completions (bash, zsh, fish) via clap — `tpt-l-apt completions --shell <bash|zsh|fish> [--output PATH]`, unit-tested

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
- [x] **deb822-lossless dependency** — decision documented in `docs/DEPENDENCY_DECISIONS.md` (control-file uses a custom lazy parser; rationale + trade-offs recorded)
- [ ] **Sandbox threat model** — define minimum viable syscall + filesystem allowlist for `tpt-l-linux-sandbox-rs` that doesn't break common Ubuntu packages (blocked on seccomp implementation, which hasn't started)

---

## Systemic gaps (flagged by 2026-08-27 audit)

- 4 of 14 crates were pure stubs (`tpt-l-apt-cli`, `tpt-l-deb-diff`, `tpt-l-dpkg-triggers`, `tpt-l-maintainer-scripts`) — all now implemented with libraries, binaries, and unit tests (2026-08-27). Remaining gaps are integration tests requiring network/Linux, clap shell completions, and progress bars.
- No `tests/` or `fuzz/` directories exist yet; all testing is inline `#[cfg(test)]` modules. `tpt-l-control-file` now has a `benches/` dir (50 MB `Packages` parse micro-benchmark); integration and fuzz checklist items are otherwise not done.
- Several crates diverge from the spec's chosen dependencies — **now documented** in `docs/DEPENDENCY_DECISIONS.md`: `tpt-l-control-file` doesn't use `deb822-lossless` (custom lazy parser); `tpt-l-apt-keyring` uses `pgp` (rPGP) instead of `sequoia-pgp`; `tpt-l-sources-list` doesn't depend on `tpt-l-control-file` (legacy one-line format isn't deb822).
- `memmap2` is now used by both `tpt-l-deb-format` (`DebFile::open`) and `tpt-l-dpkg-db` (`StatusDb::open`); both memory-map rather than eagerly slurping files.
- `homepage`/`documentation` Cargo.toml fields were missing on every crate — now added to all 14 manifests via `homepage.workspace`/`documentation.workspace` (2026-08-27).
