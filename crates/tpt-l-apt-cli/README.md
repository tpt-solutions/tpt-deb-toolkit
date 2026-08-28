# tpt-l-apt-cli

Command-line interface for the `tpt-deb-toolkit` APT layer.

Part of the [`tpt-deb-toolkit`](https://github.com/tpt-solutions/tpt-deb-toolkit)
workspace — a pure-Rust pipeline for Debian/Ubuntu package management.

Provides `update`, `install`, `search`, `show`, `list`, and `completions`
subcommands built on the layer crates. Offline commands (`search`, `show`,
`list --installed`) work against locally cached indices and the dpkg status
database; `update` and `install` perform network I/O.

## Layer

**Layer 5 — CLI.** The top of the stack; depends on every layer below.

## Subcommands

| Command | Description |
|---------|-------------|
| `update` | Fetch and cache `Packages` indices; delta-updates via PDiff when a cached revision exists. |
| `install <pkgs…>` | Resolve dependencies (SAT solver), download, extract, and run `postinst`. `--dry-run` prints the plan only. |
| `search <query>` | Search cached indices by name/description (case-insensitive). |
| `show <pkg>` | Show metadata for a single package. |
| `list [--installed]` | List packages from the dpkg status database. |
| `completions --shell <bash\|zsh\|fish>` | Emit a shell completion script. |

Global flags: `--config <path>` (sources list / config / cache dir), `--dry-run`,
`--verbose`, `--json` (machine-readable output; suppresses the progress bar).

## Installation

```toml
[dependencies]
tpt-l-apt-cli = "0.1.0"
```

Or build the binary from the workspace:

```bash
cargo build -p tpt-l-apt-cli
```

## Usage

```bash
# Cache indices (network).
tpt-l-apt update

# Resolve + install (network); print the plan without touching the system.
tpt-l-apt --dry-run install curl

# Offline queries against cached data + dpkg status.
tpt-l-apt search "transfer library"
tpt-l-apt show curl
tpt-l-apt list --installed
```

### Library use

```rust
use tpt_l_apt_cli::Apt;

let apt = Apt::new(false);
let hits = apt.search("curl").unwrap();
for h in hits {
    println!("{} {} {}", h.name, h.version, h.description);
}
```

## API overview

- `Apt` — high-level app state: `search`, `show`, `list`, `update`, `install`.
- `Cli` / `Command` — `clap` argument definitions; `run` is the shared entry point.
- `SearchHit` — a search result row (`name`, `version`, `description`).

## License

Dual-licensed under [MIT](../../LICENSE-MIT) or
[Apache-2.0](../../LICENSE-APACHE) at your option.

Copyright © 2026 TPT Solutions.
