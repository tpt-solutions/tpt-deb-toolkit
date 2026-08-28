# tpt-l-deb-version

Debian package version comparison and constraint evaluation implementing the
algorithm from [Debian Policy §5.6.12][policy].

Part of the [`tpt-deb-toolkit`](https://github.com/tpt-solutions/tpt-deb-toolkit)
workspace — a pure-Rust pipeline for Debian/Ubuntu package management.

A Debian version has the shape `[epoch:]upstream_version[-debian_revision]`. This
crate parses those strings and orders them exactly the way `dpkg` does, including
the notoriously subtle rules around `~` (sorting *before* everything, even the
empty string) and digit-run comparison (leading zeros are ignored).

## Layer

**Layer 1 — Format.** A leaf crate with no internal dependencies; every other
format/APT crate builds on it.

## Features

- Parse the full `[epoch:]upstream-revision` grammar with strict character validation.
- Correct Debian ordering: epoch → upstream (`~` < end < letters < other) → revision.
- `Version` implements `Ord`/`PartialOrd`/`Eq`/`Hash`, so versions sort and dedupe naturally.
- `VersionConstraint` (`<<`, `<=`, `=`, `>=`, `>>`) with a `satisfies()` predicate used by the dependency solver.
- Optional `serde` support behind the `serde` feature for serialization into indices and status files.

## Installation

```toml
[dependencies]
tpt-l-deb-version = "0.1.0"
```

Enable `serde` serialization:

```toml
[dependencies]
tpt-l-deb-version = { version = "0.1.0", features = ["serde"] }
```

## Usage

```rust
use tpt_l_deb_version::{Version, VersionConstraint};

let a = Version::parse("1:1.0-1").unwrap();
let b = Version::parse("2.0~beta").unwrap();

// Epoch wins regardless of the textual upstream version.
assert!(a > b);
// Tilde sorts before the empty string / plain revision.
assert!(Version::parse("1.0~rc1").unwrap() < Version::parse("1.0").unwrap());

// Constraints for dependency relations.
let c = VersionConstraint::parse(">= 2:1.0-1").unwrap();
assert!(c.satisfies(&a));
assert!(!c.satisfies(&b));
```

### Strict character validation

Only ASCII alphanumerics and `. + - ~ :` are permitted in a version component.
Anything else (e.g. `!`) returns `VersionError::InvalidCharacter`.

## API overview

- `Version::parse` / `FromStr` — parse a version string.
- `Version::epoch`, `Version::upstream`, `Version::revision` — components.
- `deb_str_cmp` — the standalone Debian string-comparison routine.
- `VersionConstraint::parse` / `VersionConstraint::satisfies` — dependency operators.
- `VersionError` — all parse failures.

## Design notes

The comparison is implemented exactly as Debian Policy specifies: the version is
split into alternating non-digit/digit runs, non-digit runs are ranked by a fixed
weight table (`~` = −1, end = 0, letters = ASCII, other = 1000+ASCII), and digit
runs are compared as integers so `1.01 == 1.1` but `1.10 > 1.9`.

## License

Dual-licensed under [MIT](../../LICENSE-MIT) or
[Apache-2.0](../../LICENSE-APACHE) at your option.

Copyright © 2026 TPT Solutions.

[policy]: https://www.debian.org/doc/debian-policy/ch-controlfields.html#version
