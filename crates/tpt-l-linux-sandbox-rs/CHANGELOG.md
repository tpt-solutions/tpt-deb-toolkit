# Changelog

All notable changes to this crate are documented in this file.

This project adheres to [Semantic Versioning](https://semver.org) and the
[Keep a Changelog](https://keepachangelog.com) format.

## [Unreleased]

### Added

- Initial release of `tpt-l-linux-sandbox-rs` (workspace version 0.1.0).
- Namespace sandbox (user/PID/mount, optional network/IPC) via `unshare(2)`.
- Seccomp allowlist (`SeccompProfile` / `SeccompRule` / `SeccompAction`).
- `BindMount` support (read-only / read-write).
- `SandboxConfig` (`maintainer_script_profile` / `unrestricted`) and `SandboxBuilder`.
- Child→parent error reporting over an `O_CLOEXEC` pipe for real `SandboxError`s.
- Cross-compilation types preserved; `UnsupportedPlatform` on non-Linux.
