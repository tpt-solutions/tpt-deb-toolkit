# tpt-deb-toolkit — Task Checklist

> Dual-licensed MIT/Apache-2.0 · Copyright TPT Solutions · crates.io publishing

---

## Phase 0 — Workspace Bootstrap

- [ ] `git init` and initial commit
- [ ] Create workspace `Cargo.toml` (members list all 14 crates)
- [ ] Create `rust-toolchain.toml` (channel = "stable", edition 2021)
- [ ] Create `.gitignore` (target/, *.deb test fixtures, secrets)
- [ ] Create `LICENSE-MIT`
- [ ] Create `LICENSE-APACHE`
- [ ] Create `README.md` (project overview, workspace map, build instructions)
- [ ] Create `.github/workflows/ci.yml`
  - [ ] `cargo check` on stable
  - [ ] `cargo test --workspace` on stable + beta
  - [ ] `cargo clippy -- -D warnings`
  - [ ] `cargo fmt --check`
  - [ ] `cargo doc --no-deps --workspace`
  - [ ] Cross-compile check for x86_64-unknown-linux-gnu (required target)

---

## Phase 1 — Layer 0: Sandbox

### `tpt-l-linux-sandbox-rs`
- [ ] Create `crates/tpt-l-linux-sandbox-rs/Cargo.toml` (crates.io metadata, categories, keywords)
- [ ] Define public API: `Sandbox`, `SandboxConfig`, `SandboxBuilder`
- [ ] Implement Linux namespace isolation (user, pid, mount, network)
- [ ] Implement seccomp profile (allowlist syscalls for maintainer scripts)
- [ ] Implement filesystem bind-mount configuration
- [ ] Add `SandboxConfig::maintainer_script_profile()` preset
- [ ] Implement `Sandbox::run(cmd, args, env)` → `ExitStatus`
- [ ] Handle explicit opt-out (`SandboxConfig::unrestricted()`) with log warning
- [ ] Unit tests: namespace isolation verified, seccomp blocks forbidden syscalls
- [ ] Integration tests: run a real shell script inside sandbox
- [ ] Add `#[cfg(target_os = "linux")]` gates (sandbox is Linux-only)
- [ ] Documentation + examples

---

## Phase 2 — Layer 1: Format & Parsing

### `tpt-l-deb-version`
- [ ] Create `crates/tpt-l-deb-version/Cargo.toml`
- [ ] Implement `Version` struct with epoch, upstream, debian-revision fields
- [ ] Implement `Version::parse(s: &str) -> Result<Version, VersionError>`
- [ ] Implement `Ord`/`PartialOrd` using Debian's comparison algorithm (tilde ordering, non-numeric segments, epoch defaults)
- [ ] Implement `VersionConstraint` (<<, <=, =, >=, >>) and `.satisfies(version)` check
- [ ] Exhaustive test suite against known-tricky version pairs from real archive
  - [ ] Epoch handling (1:1.0 vs 1.0)
  - [ ] Tilde ordering (1.0~beta < 1.0)
  - [ ] Non-numeric segments
  - [ ] Revision-less packages
- [ ] Fuzz target (`cargo-fuzz`) for `Version::parse`
- [ ] Documentation + examples

### `tpt-l-control-file`
- [ ] Create `crates/tpt-l-control-file/Cargo.toml` (dep: `deb822-lossless`)
- [ ] Evaluate `deb822-lossless` coverage; document which cases are wrapped vs. custom
- [ ] Implement `ControlFile` type wrapping `deb822-lossless` for single-stanza files
- [ ] Implement `PackagesIndex` — streaming multi-stanza parser for large Packages files
  - [ ] Strict multi-stanza validation (no duplicate fields)
  - [ ] Zero-copy field access where possible
- [ ] Implement `SourcesIndex` parser
- [ ] Implement `BinaryPackage`, `SourcePackage` typed structs (derive from stanza fields)
- [ ] Unit tests: parse real-world control files, edge cases (folded fields, multi-line)
- [ ] Benchmark: parse a 50 MB Packages.gz in-memory blob
- [ ] Documentation + examples

### `tpt-l-deb-format`
- [ ] Create `crates/tpt-l-deb-format/Cargo.toml` (deps: `memmap2`, `tpt-l-control-file`)
- [ ] Implement ar archive reader (`ArReader`) — streaming, no full load into RAM
- [ ] Implement `DebFile::open(path)` using `memmap2`
- [ ] Identify and expose `control.tar.*` and `data.tar.*` payloads by name
- [ ] Implement tar streaming extraction API (`DataEntries`, `ControlEntries` iterators)
- [ ] Handle compression variants: .gz, .xz, .zst, uncompressed
- [ ] Implement `DebMetadata` (parsed control fields from control.tar)
- [ ] Implement streaming extraction to filesystem path
- [ ] Unit tests: round-trip with known .deb fixtures
- [ ] Integration tests: extract a real .deb and verify contents
- [ ] Benchmark: zero-copy metadata read of a 50 MB .deb
- [ ] Documentation + examples

---

## Phase 3 — Layer 2: Database & Configuration

### `tpt-l-dpkg-db`
- [ ] Create `crates/tpt-l-dpkg-db/Cargo.toml` (deps: `memmap2`, `tpt-l-control-file`)
- [ ] Implement `StatusDb::open(path)` with memory-mapped read
- [ ] Implement concurrent read access (multiple readers, single writer)
- [ ] Implement `StatusDb::installed_packages()` → iterator over `InstalledPackage`
- [ ] Implement `StatusDb::write_atomic(changes)` — write to temp file, fsync, rename
- [ ] Implement package state machine: `installed`, `half-installed`, `config-files`, `unpacked`, `half-configured`, `triggers-awaited`, `triggers-pending`
- [ ] Unit tests: read a real `/var/lib/dpkg/status` snapshot, write + read back
- [ ] Concurrency tests: concurrent readers don't deadlock
- [ ] Documentation + examples

### `tpt-l-sources-list`
- [ ] Create `crates/tpt-l-sources-list/Cargo.toml` (dep: `tpt-l-control-file`)
- [ ] Implement `SourcesList::parse_one_line_format(path)` (legacy `sources.list`)
- [ ] Implement `SourcesList::parse_deb822_format(path)` (`.sources` files)
- [ ] Implement `SourcesDir::load(dir)` — scan and parse all entries in `sources.list.d/`
- [ ] Implement `SourceEntry` struct: `type` (deb/deb-src), `uri`, `suite`, `components`, `options`
- [ ] Implement URI validation and option parsing
- [ ] Implement `SourcesList::write(path)` for round-trip writing
- [ ] Unit tests: parse standard Ubuntu sources.list, deb822 format, edge cases
- [ ] Documentation + examples

### `tpt-l-apt-config`
- [ ] Create `crates/tpt-l-apt-config/Cargo.toml`
- [ ] Implement `AptConfig::load(path)` for `apt.conf`
- [ ] Implement `#include` and `#include-dir` directive resolution
- [ ] Implement type-casting: string, integer, boolean, list values
- [ ] Implement `AptConfig::load_dir(dir)` — scan `apt.conf.d/` in alphabetical order
- [ ] Implement `AptConfig::get(key)`, `get_or_default(key, default)`, `get_list(key)`
- [ ] Unit tests: parse real apt.conf snippets, include resolution, type casting
- [ ] Documentation + examples

---

## Phase 4 — Layer 3: Network & Resolution

### `tpt-l-apt-transport`
- [ ] Create `crates/tpt-l-apt-transport/Cargo.toml` (deps: `tokio`, `reqwest`, `flate2`, `xz2`, `zstd`)
- [ ] Implement `AptTransport` struct with connection pooling
- [ ] Implement async `fetch_index(uri, suite, component)` → `Packages` bytes
- [ ] Implement `InRelease`/`Release` + `Release.gpg` fetch
- [ ] Implement partial download (HTTP Range, `If-Modified-Since`)
- [ ] Implement mirror failover (try next mirror on error)
- [ ] Implement delta index updates (PDiff support)
- [ ] Implement decompression pipeline (detect extension, stream decompress)
- [ ] Implement async `.deb` file download with progress callback
- [ ] Unit tests: mock HTTP server, partial downloads, failover
- [ ] Integration tests: fetch from a real Ubuntu mirror (gated, CI optional)
- [ ] Documentation + examples

### `tpt-l-apt-keyring`
- [ ] Create `crates/tpt-l-apt-keyring/Cargo.toml` (dep: `sequoia-pgp`)
- [ ] Implement `Keyring::load(path)` for `.gpg` keyring files
- [ ] Implement `Keyring::load_dir(dir)` for `/etc/apt/trusted.gpg.d/`
- [ ] Implement `Keyring::verify(release_file, signature)` → `VerifyResult`
- [ ] Implement `InRelease` (clearsigned) verification
- [ ] Implement `Release` + `Release.gpg` (detached sig) verification
- [ ] Implement key expiry and revocation checks
- [ ] Implement `apt-key` replacement workflow (import, list, delete)
- [ ] Unit tests: verify against known Ubuntu signing key + Release fixture
- [ ] Documentation + examples

### `tpt-l-apt-solver`
- [ ] Create `crates/tpt-l-apt-solver/Cargo.toml` (deps: `tpt-l-deb-version`, `tpt-l-control-file`, `rayon`)
- [ ] Implement `Universe` — in-memory constraint graph from parsed Packages indices
- [ ] Model: `Depends`, `Pre-Depends`, `Recommends`, `Suggests`, `Conflicts`, `Breaks`, `Provides` (virtual packages)
- [ ] Implement DPLL-based SAT solver core
  - [ ] Unit propagation
  - [ ] Conflict analysis and clause learning
  - [ ] Non-chronological backtracking
  - [ ] VSIDS-inspired variable ordering
- [ ] Implement multi-threaded solving with `rayon`
- [ ] Handle virtual packages and alternative dependencies (Depends: a | b)
- [ ] Implement `InstallPlan` output (ordered list of packages to install/upgrade/remove)
- [ ] **Benchmark harness** (`benches/solver_vs_libsolv.rs`):
  - [ ] Download Ubuntu archive snapshot (scripts in `bench-data/`)
  - [ ] Run tpt-l-apt-solver and record wall-clock + plan quality
  - [ ] Run libsolv (via FFI or subprocess) on identical input
  - [ ] Generate reproducible benchmark report
- [ ] Unit tests: small hand-crafted constraint graphs, conflict detection, virtual packages
- [ ] Integration tests: solve against a real Ubuntu Packages snapshot
- [ ] Documentation + examples

---

## Phase 5 — Layer 4: Script & Trigger Execution

### `tpt-l-maintainer-scripts`
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

- [ ] Audit all `Cargo.toml` files for crates.io required fields (description, license, repository, homepage, documentation, keywords, categories)
- [ ] Set `license = "MIT OR Apache-2.0"` in all crates
- [ ] Set `authors = ["TPT Solutions"]` in all crates
- [ ] Add README shields (CI status, crates.io version, docs.rs)
- [ ] Run `cargo publish --dry-run` for each crate in dependency order
- [ ] Verify `cargo doc --no-deps --workspace` produces clean docs
- [ ] Write `CONTRIBUTING.md`
- [ ] Write `SECURITY.md`
- [ ] Tag `v0.1.0` release and publish to crates.io in dependency order
- [ ] Post-publish: verify all docs.rs pages build correctly

---

## Open Questions (from spec §5)

- [ ] **Benchmark methodology** — decide what "outperform" means (wall-clock, plan quality, or both) before any external claims
- [ ] **deb822-lossless dependency** — audit coverage; document wrap-vs-custom decision in `tpt-l-control-file`
- [ ] **Sandbox threat model** — define minimum viable syscall + filesystem allowlist for `tpt-l-linux-sandbox-rs` that doesn't break common Ubuntu packages
