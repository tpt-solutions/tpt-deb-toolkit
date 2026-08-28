# tpt-l-deb-diff

Structural and content diffing of two Debian `.deb` packages.

Part of the [`tpt-deb-toolkit`](https://github.com/tpt-solutions/tpt-deb-toolkit)
workspace — a pure-Rust pipeline for Debian/Ubuntu package management.

Compare two `.deb` files on three axes — **metadata** (control fields),
**file tree** (added/removed paths), and **content** (SHA-256 of modified files) —
producing a serializable `DiffReport` that renders as human-readable text or JSON.

## Layer

**Layer 1 — Format.** Builds on `tpt-l-deb-format` and `tpt-l-control-file`.

## Features

- Three-way diff: control-field changes, added/removed files, and modified files.
- Content integrity via per-file SHA-256 digests (`FileChange`).
- Serializable `DiffReport` (`serde`) for machine consumption (JSON, etc.).
- `Display` impl for a concise human summary.
- `compare` (byte slices) and `compare_files` (paths on disk) entry points.
- `is_empty` / `change_count` helpers to summarize a result.

## Installation

```toml
[dependencies]
tpt-l-deb-diff = "0.1.0"
```

## Usage

```rust
use tpt_l_deb_diff::DebDiff;

let a = std::fs::read("old.deb").unwrap();
let b = std::fs::read("new.deb").unwrap();
let report = DebDiff::compare(&a, &b).unwrap();

if !report.is_empty() {
    println!("{report}");
    println!("total changes: {}", report.change_count());
}
```

A `DiffReport` contains `metadata` (`MetaChange`), `files_added`,
`files_removed`, and `files_modified` (`FileChange` with old/new SHA-256). It is
fully `serde`-serializable:

```rust
let json = serde_json::to_string(&report).unwrap();
```

## API overview

- `DebDiff::compare` / `DebDiff::compare_files` — diff two packages.
- `DiffReport` — the full result (`is_empty`, `change_count`, `Display`).
- `MetaChange` — a control-field value change (`field`, `old`, `new`).
- `FileChange` — a content change (`path`, `old_sha256`, `new_sha256`).
- `DebDiffError` — parse/checksum failures.

## License

Dual-licensed under [MIT](../../LICENSE-MIT) or
[Apache-2.0](../../LICENSE-APACHE) at your option.

Copyright © 2026 TPT Solutions.
