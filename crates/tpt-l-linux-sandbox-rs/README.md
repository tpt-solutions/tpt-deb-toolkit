# tpt-l-linux-sandbox-rs

Linux namespace and seccomp sandbox for running Debian maintainer scripts.

Part of the [`tpt-deb-toolkit`](https://github.com/tpt-solutions/tpt-deb-toolkit)
workspace — a pure-Rust pipeline for Debian/Ubuntu package management.

Provides isolation primitives for running `preinst`/`postinst`/`prerm`/`postrm`
in a restricted Linux environment. On non-Linux platforms the types are still
available for cross-compilation, but `Sandbox::run` returns
`SandboxError::UnsupportedPlatform`.

## Layer

**Layer 0 — Sandbox.** The foundation crate; nothing in the toolkit depends on anything below it.

## Features

- User + PID + mount (+ optional network/IPC) namespaces via `unshare(2)`.
- Seccomp syscall allowlist (`SeccompProfile` / `SeccompRule` / `SeccompAction`).
- `BindMount` — expose host paths read-only or read-write inside the sandbox.
- `SandboxConfig` — `maintainer_script_profile()` (restrictive, default) and `unrestricted()`.
- `SandboxBuilder` — ergonomic construction.
- Child reports setup failures to the parent over an `O_CLOEXEC` pipe, so errors surface as real `SandboxError`s instead of silent failures.
- Runs the exec'd command as PID 1 in the new PID namespace; writes UID/GID maps so the child appears root in its user namespace.

## Installation

```toml
[dependencies]
tpt-l-linux-sandbox-rs = "0.1.0"
```

## Usage

```rust
use tpt_l_linux_sandbox_rs::{SandboxBuilder, SandboxConfig};

let sandbox = SandboxBuilder::new()
    .config(SandboxConfig::maintainer_script_profile())
    .build();

let status = sandbox.run("/bin/true", &[], &[]);
```

> Requires Linux with user-namespace support. On hosts that restrict namespaces
> (some CI containers via AppArmor/seccomp), `run_namespaced` fails at runtime and
> the integration tests skip rather than fail.

## API overview

- `Sandbox` / `SandboxBuilder` — build and `run` a sandboxed command.
- `SandboxConfig` — `maintainer_script_profile` / `unrestricted`; `allow_network`, `allow_ipc`, `extra_bind_mounts`, `seccomp`.
- `BindMount` — bind-mount source→destination (`read_only`).
- `SeccompProfile` / `SeccompRule` / `SeccompAction` — syscall filtering.
- `SandboxError` — unsupported-platform / namespace / spawn / IO / bind-mount / seccomp failures.

## Design notes

Namespace setup happens in a single-threaded child (a process may not `unshare` a
user namespace while it still has multiple threads — always the case inside
`cargo test`). A second `fork` after `unshare(CLONE_NEWPID)` makes the exec'd
command PID 1 in the new PID namespace.

## License

Dual-licensed under [MIT](../../LICENSE-MIT) or
[Apache-2.0](../../LICENSE-APACHE) at your option.

Copyright © 2026 TPT Solutions.
