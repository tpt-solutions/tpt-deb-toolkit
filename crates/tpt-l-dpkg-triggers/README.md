# tpt-l-dpkg-triggers

dpkg trigger processing support.

Part of the [`tpt-deb-toolkit`](https://github.com/tpt-solutions/tpt-deb-toolkit)
workspace — a pure-Rust pipeline for Debian/Ubuntu package management.

Debian triggers let one package signal that shared state (a font cache, an
initramfs, a man-page index, …) needs rebuilding. A package declares *interest* in
a named trigger; another package *activates* it; dpkg later runs the interested
package's `postinst triggered` to rebuild that state. This crate models the
trigger database and the deferred-processing loop, and bridges activation to the
`InstallStatus` state machine.

## Layer

**Layer 4 — Scripts.** Depends on `tpt-l-dpkg-db`.

## Features

- `TriggerDb` — track interests and pending triggers per package.
- `interest` / `add_trigger` — register `interest` / `interest-noawait` triggers.
- `activate` / `activate_all` — mark interested packages pending; returns affected packages.
- `process` — dequeue a package's pending triggers (what `postinst triggered` handles).
- Idempotent activation; `is_pending` / `pending_of` / `pending_packages` queries.
- `save_dir` / `load_dir` — persist/restore the database (an abstraction over `/var/lib/dpkg/triggers/`).
- `status` module — `mark_pending` / `mark_awaited` / `clear` transitions on `InstallStatus`.

## Installation

```toml
[dependencies]
tpt-l-dpkg-triggers = "0.1.0"
```

## Usage

```rust
use tpt_l_dpkg_triggers::{TriggerDb, Trigger};

let mut db = TriggerDb::new();
db.interest("man-db", "man-db-rebuild", true);
let affected = db.activate("man-db-rebuild");
assert_eq!(affected, vec!["man-db".to_string()]);
assert!(db.is_pending("man-db"));
let todo = db.process("man-db");
assert_eq!(todo, vec!["man-db-rebuild".to_string()]);
assert!(!db.is_pending("man-db"));
```

## API overview

- `TriggerDb` — the in-memory trigger database (interests + pending state).
- `Trigger` — `interest` / `interest_noawait` constructors; `name`, `awaited`.
- `status` — `mark_pending` / `mark_awaited` / `clear` bridging to `InstallStatus`.
- `TriggerError` — IO/malformed-record failures.

## License

Dual-licensed under [MIT](../../LICENSE-MIT) or
[Apache-2.0](../../LICENSE-APACHE) at your option.

Copyright © 2026 TPT Solutions.
