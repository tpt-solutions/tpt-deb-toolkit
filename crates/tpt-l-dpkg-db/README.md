# tpt-l-dpkg-db

Concurrent reader/writer for the dpkg package status database
(`/var/lib/dpkg/status`).

Part of the [`tpt-deb-toolkit`](https://github.com/tpt-solutions/tpt-deb-toolkit)
workspace — a pure-Rust pipeline for Debian/Ubuntu package management.

The status database is one deb822 stanza per package describing its installation
state. This crate provides typed access plus an **atomic writer** that avoids
corrupting the database on power failure (write to a temp file, `sync_all`,
rename into place).

## Layer

**Layer 2 — Database.** Depends on `tpt-l-control-file`.

## Features

- Parse `/var/lib/dpkg/status` via `memmap2` (no full copy into a `String`).
- Typed state machine for the three-word `Status:` field: `PackageWant`,
  `PackageAction`, `InstallStatus`.
- `StatusDb` with `installed_packages()`, `find()`, and `packages()`.
- `apply_changes` + `write_atomic` for safe in-place mutation.
- `ConcurrentStatusDb` — an `RwLock`-guarded database: many concurrent readers,
  serialized writers, atomic persistence after each mutation.
- `StatusChange` diff operations (`SetStatus`, `Remove`).

## Installation

```toml
[dependencies]
tpt-l-dpkg-db = "0.1.0"
```

## Usage

```rust
use std::path::Path;
use tpt_l_dpkg_db::StatusDb;

let db = StatusDb::open(Path::new("/var/lib/dpkg/status")).unwrap();
for pkg in db.installed_packages() {
    println!("{} {}", pkg.name, pkg.version);
}
```

### Concurrent, atomic writes

```rust
use tpt_l_dpkg_db::{ConcurrentStatusDb, StatusChange, PackageStatus};

let db = ConcurrentStatusDb::open(Path::new("./status")).unwrap();
let changes = [StatusChange::SetStatus {
    name: "newpkg".into(),
    version: "1.0".into(),
    architecture: "amd64".into(),
    status: PackageStatus::parse("install ok installed").unwrap(),
}];
db.apply_changes(Path::new("./status"), &changes).unwrap();
```

## API overview

- `StatusDb` — in-memory status database (`open`, `parse_str`, `installed_packages`, `find`, `write_atomic`, `apply_changes`).
- `ConcurrentStatusDb` — `RwLock`-guarded variant (`read`, `apply_changes`).
- `InstalledPackage` — a parsed status stanza.
- `PackageStatus` / `PackageWant` / `PackageAction` / `InstallStatus` — the status state machine.
- `StatusChange` — a single mutation to persist.
- `DbError` — parse/IO/atomic-write failures.

## Design notes

`ConcurrentStatusDb` uses `std::sync::RwLock`, so reads never block each other and
writers are serialized. Stanzas with missing `Package`/`Status` fields are skipped
with a `tracing` warning rather than aborting the whole parse. The atomic writer
writes to a `NamedTempFile` in the target directory, syncs it, then renames — the
rename is the only operation a reader can observe, so the on-disk file is always
consistent.

## License

Dual-licensed under [MIT](../../LICENSE-MIT) or
[Apache-2.0](../../LICENSE-APACHE) at your option.

Copyright © 2026 TPT Solutions.
