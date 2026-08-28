# tpt-l-apt-config

Parser for APT configuration files (`apt.conf` and `apt.conf.d/`).

Part of the [`tpt-deb-toolkit`](https://github.com/tpt-solutions/tpt-deb-toolkit)
workspace — a pure-Rust pipeline for Debian/Ubuntu package management.

APT configuration uses a hierarchical key-value format with two equivalent
syntaxes — a nested `{ }` scope syntax and a flat `::` path syntax. This crate
normalizes both into a flat `HashMap` keyed by `::` paths, including APT's
list-append (`Key:: "value";`) syntax.

## Layer

**Layer 2 — Database.** No internal dependencies.

## Features

- Accepts both nested-scope and flat `::` syntax (fully equivalent).
- `#include` and `#include-dir` resolution with a cycle guard (`max_depth`, default 10).
- Line (`//`) and block (`/* */`) comments.
- Typed accessors: `get`, `get_bool` (`true`/`yes`/`1`), `get_int`, `get_list`,
  `get_or_default`.
- `merge` — last-wins override semantics.
- Convenience shortcuts: `sources_list_path`, `status_db_path`.
- `load_dir` merges files in alphabetical order, mirroring `apt`.

## Installation

```toml
[dependencies]
tpt-l-apt-config = "0.1.0"
```

## Usage

```rust
use tpt_l_apt_config::AptConfig;

let cfg = AptConfig::load_with_includes(std::path::Path::new("/etc/apt/apt.conf")).unwrap();
assert_eq!(cfg.get("APT::Get::Assume-Yes"), Some("true"));
assert_eq!(cfg.get_bool("APT::Get::Assume-Yes"), Some(true));
```

### List-append syntax

```rust
let mut cfg = AptConfig::new();
cfg.set("Acquire::http::Proxy", "http://a:3128");
cfg.push_list("Acquire::http::Proxy", "http://b:3128");
assert_eq!(cfg.get_list("Acquire::http::Proxy").len(), 2);
```

## API overview

- `AptConfig` — parsed configuration (`load`, `load_with_includes`, `load_dir`, `merge`).
- `ConfigValue` — `String` or `List` value.
- `get` / `get_bool` / `get_int` / `get_list` / `get_or_default`, `set`, `push_list`.
- `sources_list_path` / `status_db_path` shortcuts.
- `ConfigError` — IO/parse/include-cycle failures.

## License

Dual-licensed under [MIT](../../LICENSE-MIT) or
[Apache-2.0](../../LICENSE-APACHE) at your option.

Copyright © 2026 TPT Solutions.
