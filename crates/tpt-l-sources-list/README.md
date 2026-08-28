# tpt-l-sources-list

Parser and writer for APT `sources.list` (one-line) and deb822 `.sources` files.

Part of the [`tpt-deb-toolkit`](https://github.com/tpt-solutions/tpt-deb-toolkit)
workspace — a pure-Rust pipeline for Debian/Ubuntu package management.

Supports both the classic one-line format and the newer deb822 stanza format,
auto-detected by file extension when loading from disk.

## Layer

**Layer 2 — Database.** No internal dependencies.

## Features

- Parse one-line `sources.list` (with `[arch=… signed-by=…]` option blocks,
  comment-disabled entries, inline `#` comments).
- Parse deb822 `.sources` stanzas (`Types`, `URIs`, `Suites`, `Components`,
  `Enabled`, `Signed-By`, …), expanding each into per-type/URI/suite entries.
- `load_file` (extension auto-detect) and `load_dir` (merges `*.list` and
  `*.sources` in lexicographic order, like `apt`).
- Iterators: `entries`, `active_entries`, `binary_entries`, `source_entries`.
- Round-trippable serialization: `write_one_line` and `write_deb822`.
- `validate` / `is_valid_uri` with `https`, `http`, `ftp`, `file`, `mirror+http`,
  `mirror+https`, `cdrom` schemes.
- `release_url` / `packages_url` helpers for building fetch URLs.

## Installation

```toml
[dependencies]
tpt-l-sources-list = "0.1.0"
```

## Usage

```rust
use tpt_l_sources_list::SourcesList;

let sl = SourcesList::parse_one_line(
    "deb [arch=amd64 signed-by=/key.gpg] http://archive.ubuntu.com/ubuntu focal main\n",
).unwrap();
for e in sl.active_entries() {
    println!("{} {} {}", e.source_type, e.uri, e.suite);
}
```

### deb822 round-trip

```rust
let sl = SourcesList::parse_deb822("Types: deb deb-src\nURIs: http://example.com\nSuites: stable\nComponents: main\n");
let deb822 = sl.write_deb822(); // re-grouped by (uri, suite, components, options, enabled)
sl.validate().unwrap();
```

## API overview

- `SourcesList` — collection of entries (`parse_one_line`, `parse_deb822`, `load_file`, `load_dir`, `write_one_line`, `write_deb822`, `write`, `validate`).
- `SourceEntry` — a single repository entry (`source_type`, `uri`, `suite`, `components`, `options`, `enabled`).
- `SourceType` — `Binary` (`deb`) or `Source` (`deb-src`).
- `SourcesError` — parse/IO failures.

## License

Dual-licensed under [MIT](../../LICENSE-MIT) or
[Apache-2.0](../../LICENSE-APACHE) at your option.

Copyright © 2026 TPT Solutions.
