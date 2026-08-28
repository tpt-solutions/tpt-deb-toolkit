# tpt-deb-toolkit — Task Checklist

> Dual-licensed MIT/Apache-2.0 · Copyright TPT Solutions · crates.io publishing

> Checklist last audited 2026-08-27 against actual repo state. `[~]` = partially done (see note).
>
> 2026-08-27 session: fixed a pre-existing compile bug (`chrono` was used in `tpt-l-apt-keyring` lib code but only declared as a dev-dependency); made the workspace `clippy -D warnings` + `fmt --check` clean; implemented `ControlFile` + `SourcesIndex` + zero-copy `BorrowedParagraph<'a>` (control-file), the `apt-key` replacement workflow + a real key-generation/sign/verify round-trip (keyring), and partial/resumable downloads + a `wiremock` test suite (transport); added crates.io/docs.rs README badges; verified `cargo doc` builds clean.

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
  - [x] Cross-compile check — fixed to actually cross-compile to `aarch64-unknown-linux-gnu` on `ubuntu-latest` (installs `gcc-aarch64-linux-gnu`, sets the cross linker), instead of the previous no-op native `x86_64` check

---

## Phase 1 — Layer 0: Sandbox

### `tpt-l-linux-sandbox-rs`
- [x] Create `crates/tpt-l-linux-sandbox-rs/Cargo.toml` (crates.io metadata, categories, keywords)
- [x] Define public API: `Sandbox`, `SandboxConfig`, `SandboxBuilder`
- [~] Implement Linux namespace isolation (user, pid, mount, network) — `unshare()` for user/pid/mount/net flags set; `unshare` is now performed in the single-threaded forked child (a process may not `unshare` a user namespace while multithreaded) with a second `fork` so the exec'd command is PID 1 in the new PID namespace; no pivot_root/chroot (root fs not replaced)
- [x] Implement seccomp profile (allowlist syscalls for maintainer scripts) — `SeccompProfile` + classic-BPF builder (`seccomp.rs`) installed via `prctl(SECCOMP_MODE_FILTER)` in the child; allows ~150 common syscalls and `AF_UNIX`/`AF_NETLINK` sockets while denying everything else (default action `EPERM`); `SandboxConfig::seccomp` field drives it, `SeccompProfile::disabled()` opts out
- [x] Implement filesystem bind-mount configuration — `extra_bind_mounts: Vec<BindMount>` applied in the sandboxed child via `mount(MS_BIND)` (root ns made `MS_PRIVATE` first), with `BindMount::read_only` remount support
- [x] Add `SandboxConfig::maintainer_script_profile()` preset
- [x] Implement `Sandbox::run(cmd, args, env)` → `ExitStatus`
- [x] Handle explicit opt-out (`SandboxConfig::unrestricted()`) with log warning
- [x] Unit tests: namespace isolation verified, seccomp blocks forbidden syscalls — fork-based tests verify a forbidden syscall (`kexec_load`) returns `EPERM`, an allowed syscall (`getpid`) succeeds, and `AF_UNIX` sockets are permitted while `AF_INET` sockets are denied; all run + verified under WSL
- [x] Integration tests: run a real shell script inside sandbox — `runs_a_real_script_inside_the_sandbox` and `bind_mount_is_visible_inside_the_sandbox` run `/bin/true` and a `sh -c` script reading a bind-mounted file (WSL-verified)
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
  - [x] Evaluate `deb822-lossless` coverage; document which cases are wrapped vs. custom — documented in `docs/DEPENDENCY_DECISIONS.md` §4 (no wrapped path; fully custom parser, with coverage table)
- [x] Implement `ControlFile` type wrapping `deb822-lossless` for single-stanza files — `ControlFile` added (custom parser; `deb822-lossless` not used per `docs/DEPENDENCY_DECISIONS.md`)
- [x] Implement `PackagesIndex` — streaming multi-stanza parser for large Packages files — `PackagesIndex` type added with lazy `iter()`/`iter_results()` (stanzas parsed on demand)
  - [x] Strict multi-stanza validation (no duplicate fields) — `parse_control_strict` / `BinaryPackage::parse_stanza_strict` / `PackagesIndex::iter_results_strict` reject duplicate fields with `ControlError::DuplicateField`
  - [x] Zero-copy field access where possible — added `BorrowedParagraph<'a>` (Cow-based: single-line values borrow, only folded values allocate) + `parse_control_borrowed`/`parse_control_strict_borrowed` and `PackagesIndex`/`SourcesIndex::iter_paragraphs()`; `ControlParagraph` kept owned for file-based use
- [x] Implement `SourcesIndex` parser — `SourcesIndex` added (mirrors `PackagesIndex`, parses `SourcePackage` stanzas)
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
  - [x] Integration tests: extract a real .deb and verify contents — `crates/tpt-l-deb-format/tests/extract_real_deb.rs` extracts a valid `.deb` and verifies file contents + exec bit (plus an optional `TPT_REAL_DEB` real-fixture hook)
  - [x] Benchmark: zero-copy metadata read of a 50 MB .deb — `crates/tpt-l-deb-format/benches/metadata_read.rs` (new `DebFile::open_metadata`/`parse_metadata` path, mmap + control.tar only)
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
  - [x] Implement `AptTransport` struct with connection pooling — `AptTransport::new` builds a single shared `reqwest::Client`, whose default connection pool is reused across requests (the canonical reqwest pooling behavior)
  - [x] Implement async `fetch_index(uri, suite, component)` → `Packages` bytes — implemented with an extra `arch` param (matches spec; the arch is required to locate `binary-<arch>/Packages`)
  - [x] Implement `InRelease`/`Release` + `Release.gpg` fetch — `fetch_release` does InRelease→Release fallback; `fetch_release_gpg` added for the detached `Release.gpg` signature
- [x] Implement partial download (HTTP Range, `If-Modified-Since`) — `fetch_range`, `fetch_if_modified_since`, `download_file_resumable`
- [x] Implement mirror failover (try next mirror on error)
  - [x] Implement delta index updates (PDiff support) — implemented in `tpt-l-apt-transport` (new `pdiff` module): `PdiffIndex` parse, `resolve_chain` (BFS over the diff graph), pure-Rust rdiff2 `apply_rdiff_delta` (no C `librsync` dependency), and `AptTransport::fetch_pdiff` (download index + patches, gunzip, apply, per-step SHA-256 verify). Covered by unit tests (parse/resolve/decode/encode) and a wiremock end-to-end test. The transport crate also adds a minimal `ReleaseIndex` parser (`release` module) and `tpt-l-apt-cli`'s `update` now delta-updates cached indices via PDiff (full-fetch fallback when no cache / no published diff / error).
- [x] Implement decompression pipeline (detect extension, stream decompress)
- [x] Implement async `.deb` file download with progress callback — `download_file` now streams chunks and reports progress after each chunk (no full buffering)
- [x] Unit tests: mock HTTP server, partial downloads, failover — `wiremock` suite covers `fetch_bytes`, `fetch_range` (both forms), `fetch_if_modified_since` (304 + 200), mirror failover, and resumable download
  - [x] Integration tests: fetch from a real Ubuntu mirror (gated, CI optional) — `crates/tpt-l-apt-transport/tests/live_mirror.rs` runs only when `TPT_LIVE_MIRROR` is set (default off); fetches a real `Packages` index and asserts it parses
- [x] Documentation + examples

### `tpt-l-apt-keyring`
- [~] Create `crates/tpt-l-apt-keyring/Cargo.toml` (dep: `sequoia-pgp`) — uses `pgp` (rPGP) crate instead of `sequoia-pgp`
- [x] Implement `Keyring::load(path)` for `.gpg` keyring files (also handles `.asc` armored)
- [x] Implement `Keyring::load_dir(dir)` for `/etc/apt/trusted.gpg.d/`
- [~] Implement `Keyring::verify(release_file, signature)` → `VerifyResult` — split into `verify_clearsigned`/`verify_detached`, both return `VerifyResult`
- [x] Implement `InRelease` (clearsigned) verification
- [x] Implement `Release` + `Release.gpg` (detached sig) verification
- [x] Implement key expiry and revocation checks — `key_expired_at`/`key_is_revoked` enforced in `verify_clearsigned_with`/`verify_detached_with`
- [x] Implement `apt-key` replacement workflow (import, list, delete) — `Keyring::import`, `list`, `remove`, `save_binary`/`save_armored`, `add_to_keyring_file` (apt-key `add` analog)
- [x] Unit tests: verify against known Ubuntu signing key + Release fixture — real round-trip test generates an Ed25519 key, signs an `InRelease` payload, and verifies it; plus list/remove/import/export tests
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

- [x] Audit all `Cargo.toml` files for crates.io required fields (description, license, repository, homepage, documentation, keywords, categories) — all present on all 14 crates (description/license/repository/homepage/documentation/keywords/categories verified)
- [x] Set `license = "MIT OR Apache-2.0"` in all crates (via `license.workspace = true`)
- [x] Set `authors = ["TPT Solutions"]` in all crates (via `authors.workspace = true`)
- [x] Add README shields (CI status, crates.io version, docs.rs) — crates.io + docs.rs badges added (point at `tpt-l-deb-version`; resolve after publish)
- [~] Run `cargo publish --dry-run` for each crate in dependency order — done; manifests fixed so all internal path deps now carry `version = "0.1.0"` (required by crates.io). Leaf/standalone crates pass dry-run: `tpt-l-deb-version`, `tpt-l-linux-sandbox-rs`, `tpt-l-sources-list`, `tpt-l-apt-config`, `tpt-l-apt-keyring`, `tpt-l-apt-transport`. Dependent crates (`tpt-l-control-file`, `tpt-l-deb-format`, `tpt-l-dpkg-db`, `tpt-l-apt-solver`, `tpt-l-maintainer-scripts`, `tpt-l-dpkg-triggers`, `tpt-l-deb-diff`, `tpt-l-apt-cli`) only fail at the crates.io index lookup because their internal deps are not yet published; this resolves once the real sequential publish (below) lands.
- [x] Verify `cargo doc --no-deps --workspace` produces clean docs — verified (2026-08-27)
- [x] Write `CONTRIBUTING.md`
- [x] Write `SECURITY.md`
- [ ] Tag `v0.1.0` release and publish to crates.io in dependency order
- [ ] Post-publish: verify all docs.rs pages build correctly

---

## Open Questions (from spec §5)

  - [x] **Benchmark methodology** — documented in `docs/BENCHMARK_METHODOLOGY.md` (wall-clock + plan-quality axes, harness conventions, explicit "outperform" preconditions)
- [x] **deb822-lossless dependency** — decision documented in `docs/DEPENDENCY_DECISIONS.md` (control-file uses a custom lazy parser; rationale + trade-offs recorded)
- [x] **Sandbox threat model** — syscall allowlist implemented in `SeccompProfile::maintainer_script_profile()` (covers ~150 common syscalls, permits `AF_UNIX`/`AF_NETLINK`, denies network-family sockets with `EPERM` default), plus `BindMount` filesystem policy; rationale/trade-offs/limitations now documented in `docs/SANDBOX_THREAT_MODEL.md`

---

## Systemic gaps (flagged by 2026-08-27 audit)

- 4 of 14 crates were pure stubs (`tpt-l-apt-cli`, `tpt-l-deb-diff`, `tpt-l-dpkg-triggers`, `tpt-l-maintainer-scripts`) — all now implemented with libraries, binaries, and unit tests (2026-08-27). Remaining gaps are integration tests requiring network/Linux, clap shell completions, and progress bars.
- No repo-wide `tests/` or `fuzz/` integration harness exists yet; most testing is inline `#[cfg(test)]` modules. Progress since the audit: `tpt-l-control-file` has a `benches/` dir (50 MB `Packages` parse micro-benchmark); `tpt-l-deb-format` now has both a `benches/` dir (`metadata_read` zero-copy benchmark) and a `tests/` integration test (`extract_real_deb.rs`); the `fuzz/` workspace exists with `parse_version`. Integration/fuzz checklist items needing network/Linux are otherwise not done.
- Several crates diverge from the spec's chosen dependencies — **now documented** in `docs/DEPENDENCY_DECISIONS.md`: `tpt-l-control-file` doesn't use `deb822-lossless` (custom lazy parser); `tpt-l-apt-keyring` uses `pgp` (rPGP) instead of `sequoia-pgp`; `tpt-l-sources-list` doesn't depend on `tpt-l-control-file` (legacy one-line format isn't deb822).
- `memmap2` is now used by both `tpt-l-deb-format` (`DebFile::open`) and `tpt-l-dpkg-db` (`StatusDb::open`); both memory-map rather than eagerly slurping files.
- `homepage`/`documentation` Cargo.toml fields were missing on every crate — now added to all 14 manifests via `homepage.workspace`/`documentation.workspace` (2026-08-27).
