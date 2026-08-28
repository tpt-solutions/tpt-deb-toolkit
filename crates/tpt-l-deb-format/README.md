# tpt-l-deb-format

Reader for Debian `.deb` binary package files — the `ar` archive wrapping
`control.tar.*` and `data.tar.*`.

Part of the [`tpt-deb-toolkit`](https://github.com/tpt-solutions/tpt-deb-toolkit)
workspace — a pure-Rust pipeline for Debian/Ubuntu package management.

A `.deb` is an `ar(1)` archive with three members: `debian-binary`,
`control.tar.*`, and `data.tar.*`. This crate reads them with **zero-copy
streaming**: files are memory-mapped (via `memmap2`) and tar payloads are
decompressed lazily, so even large packages are never copied wholly into RAM up
front.

## Layer

**Layer 1 — Format.** Depends on `tpt-l-control-file` for control metadata.

## Features

- Parse the `ar` container and validate the `2.0` format marker.
- Supported compression: `.gz` (gzip), `.xz`, `.zst` (zstd), and uncompressed.
- `DebFile::open` memory-maps the on-disk file; `DebFile::parse` parses from a byte slice.
- `open_metadata` / `parse_metadata` read *only* the control metadata — cost is
  independent of the payload size, perfect for indexing large repositories.
- Full extraction to disk (`deb.extract`), in-memory payload read, and lazy
  `data_entries()` / `control_entries()` streaming iterators.
- `ArReader` — a low-level streaming `ar` reader yielding one member at a time.

## Installation

```toml
[dependencies]
tpt-l-deb-format = "0.1.0"
```

## Usage

```rust
use tpt_l_deb_format::DebFile;

// Metadata only — no payload decompression.
let meta = DebFile::open_metadata(std::path::Path::new("foo.deb")).unwrap();
println!("{} {} {}", meta.package_name().unwrap(), meta.version().unwrap(), meta.architecture().unwrap());

// Full parse + extract.
let deb = DebFile::open(std::path::Path::new("foo.deb")).unwrap();
deb.extract(std::path::Path::new("./root")).unwrap();
```

### Streaming the payload

```rust
let deb = DebFile::parse(&bytes).unwrap();
let mut entries = deb.data_entries().unwrap();
for entry in entries.entries().unwrap() {
    let entry = entry.unwrap();
    println!("{}", entry.path().unwrap().display());
}
```

## API overview

- `DebFile` — parsed package (`open` / `parse` / `open_metadata` / `parse_metadata`).
- `DebMetadata` — control fields (case-insensitive `get`, `package_name`, `version`, …).
- `DataEntry` — a payload entry (`path`, `size`, `mode`).
- `ArReader` / `ArEntry` — low-level streaming `ar` reader.
- `DataEntries` / `ControlEntries` — lazy decompressing tar iterators.
- `DebError` — all parse/IO/decompression failures.

## Design notes

The control metadata path is deliberately allocation-light: only `control.tar.*`
is decompressed, and the `data.tar.*` member is skipped entirely when all you need
is `Package`/`Version`/`Architecture`. Extraction preserves Unix permission bits
(and symbolic links on Unix) from the tar headers.

## License

Dual-licensed under [MIT](../../LICENSE-MIT) or
[Apache-2.0](../../LICENSE-APACHE) at your option.

Copyright © 2026 TPT Solutions.
