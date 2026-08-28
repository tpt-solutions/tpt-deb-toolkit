# tpt-l-control-file

Debian control-file parsing: binary package stanzas (`Packages` indices,
`DEBIAN/control`), source package stanzas (`Sources`/`.dsc`), and standalone
single-stanza files such as `Release`/`InRelease`.

Part of the [`tpt-deb-toolkit`](https://github.com/tpt-solutions/tpt-deb-toolkit)
workspace — a pure-Rust pipeline for Debian/Ubuntu package management.

Unlike many crates this uses a **custom, lazy, zero-copy parser** rather than
`deb822-lossless` (see `docs/DEPENDENCY_DECISIONS.md`). Field *names* always
borrow from the source; field *values* borrow for ordinary single-line fields and
only allocate when a folded/continuation line forces the value to be joined.

## Layer

**Layer 1 — Format.** Depends only on `tpt-l-deb-version`; depended on by nearly
every higher layer.

## Features

- Parse `Packages`, `Sources`, `Release`, and arbitrary deb822 documents.
- Strict and lenient variants: `parse_control` keeps duplicates (last wins);
  `parse_control_strict` rejects duplicate fields with `ControlError::DuplicateField`.
- Typed accessors: `BinaryPackage`, `SourcePackage`, and `ControlFile` (single-stanza,
  e.g. `DEBIAN/control`).
- Lazily-parsed `PackagesIndex` / `SourcesIndex` views that stream entries on demand —
  ideal for huge indices that must not be materialized up front.
- `BorrowedParagraph` — a zero-copy paragraph view for cheap field access over large inputs.
- Case-insensitive field lookup (`package` matches `Package`).

## Installation

```toml
[dependencies]
tpt-l-control-file = "0.1.0"
```

## Usage

```rust
use tpt_l_control_file::BinaryPackage;

let stanza = "Package: hello\nVersion: 1.0-1\nArchitecture: amd64\n";
let pkg = BinaryPackage::parse_stanza(stanza).unwrap();
assert_eq!(pkg.name, "hello");
assert_eq!(pkg.version_str, "1.0-1");

// Stream a large Packages index without building every struct up front.
use tpt_l_control_file::PackagesIndex;
let index = PackagesIndex::new(big_index_text);
for pkg in index.iter() {
    println!("{} {}", pkg.name, pkg.version_str);
}
```

### Strict parsing

```rust
use tpt_l_control_file::ControlFile;
// `ControlFile` is single-stanza and strict: duplicate fields and multiple
// stanzas are errors, matching dpkg policy.
let cf = ControlFile::parse("Package: a\nVersion: 1.0\n").unwrap();
assert_eq!(cf.field("Package"), Some("a"));
```

## API overview

- `parse_control` / `parse_control_strict` — parse all stanzas.
- `parse_control_borrowed` / `parse_control_strict_borrowed` — zero-copy variants.
- `BinaryPackage`, `SourcePackage` — typed package stanzas.
- `ControlFile` — single strict stanza (e.g. `DEBIAN/control`, `Release`).
- `PackagesIndex`, `SourcesIndex` — lazy multi-stanza views.
- `BorrowedParagraph`, `ControlParagraph` — generic paragraph types.
- `ControlError` — all parse failures.

## Design notes

Continuation (folded) lines are joined with a single leading space stripped, and
all lookups are case-insensitive while preserving the original key casing. The
parser distinguishes ordinary fields (borrowed) from folded values (allocated),
keeping bulk parsing of large indices allocation-free in the common case.

## License

Dual-licensed under [MIT](../../LICENSE-MIT) or
[Apache-2.0](../../LICENSE-APACHE) at your option.

Copyright © 2026 TPT Solutions.
