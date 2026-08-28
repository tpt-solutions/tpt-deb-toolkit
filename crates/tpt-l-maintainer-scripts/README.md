# tpt-l-maintainer-scripts

Execution of Debian maintainer scripts (`preinst`, `postinst`, `prerm`,
`postrm`) with the correct environment, ordering contract, and exit-code
semantics.

Part of the [`tpt-deb-toolkit`](https://github.com/tpt-solutions/tpt-deb-toolkit)
workspace — a pure-Rust pipeline for Debian/Ubuntu package management.

## Layer

**Layer 4 — Scripts.** Depends on `tpt-l-linux-sandbox-rs` for sandboxing.

## Features

- One method per script: `run_preinst` / `run_postinst` / `run_prerm` / `run_postrm`.
- Correct `DPKG_MAINTSCRIPT_*` / `DPKG_ROOT` / `DEBIAN_FRONTEND` environment population (`script_env`).
- Pure, testable planning: `plan` builds a `ScriptPlan` (path, action arg, env) without spawning.
- Sandboxed by default on Linux; `ScriptRunner::unrestricted` opts out (emits a warning).
- Async execution via `execute_async` (spawns on a blocking thread).
- `ScriptOutcome` with `success()`; `ScriptError::Signaled` for signal-terminated scripts.

## Installation

```toml
[dependencies]
tpt-l-maintainer-scripts = "0.1.0"
```

## Usage

```rust
use tpt_l_maintainer_scripts::{ScriptRunner, PackageRef};
use std::path::PathBuf;

let runner = ScriptRunner::new(
    PathBuf::from("/var/lib/dpkg/info"),
    PackageRef { name: "curl".into(), version: "8.2.1-1".into(), arch: "amd64".into() },
);
let outcome = runner.run_postinst("configure").unwrap();
assert!(outcome.success());
```

### Inspect a plan without running

```rust
let plan = runner.plan("postinst", "configure").unwrap();
assert_eq!(plan.script_name(), Some("postinst"));
```

## Ordering contract

```
install:  preinst  →  (unpack)  →  postinst
remove:   prerm    →  (remove)  →  postrm
```

This crate does not unpack/remove the payload itself — that belongs to the
package database and extraction layers — but it exposes one method per script so
callers can honour the contract, passing the appropriate *action* (`install`,
`configure`, `remove`, …).

## API overview

- `ScriptRunner` — `new`, `unrestricted`, `with_config`, `plan`, `execute`, `execute_async`, `run_preinst`/`postinst`/`prerm`/`postrm`, `script_env`, `is_sandboxed`.
- `ScriptPlan` — a ready-to-run invocation (`script_path`, `args`, `env`).
- `PackageRef` — package identity for `DPKG_MAINTSCRIPT_*` vars.
- `RunnerConfig` — control dir, `use_sandbox`, `debian_frontend`, `extra_env`, `root`.
- `ScriptOutcome`, `ScriptError` — results and failures.

## License

Dual-licensed under [MIT](../../LICENSE-MIT) or
[Apache-2.0](../../LICENSE-APACHE) at your option.

Copyright © 2026 TPT Solutions.
