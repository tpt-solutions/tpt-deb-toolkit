# Changelog

All notable changes to this crate are documented in this file.

This project adheres to [Semantic Versioning](https://semver.org) and the
[Keep a Changelog](https://keepachangelog.com) format.

## [Unreleased]

### Added

- Initial release of `tpt-l-maintainer-scripts` (workspace version 0.1.0).
- Per-script runners: `run_preinst` / `run_postinst` / `run_prerm` / `run_postrm`.
- `DPKG_MAINTSCRIPT_*` / `DPKG_ROOT` / `DEBIAN_FRONTEND` environment population.
- Pure `plan` building a `ScriptPlan`; `execute` / `execute_async` runners.
- Sandboxed by default on Linux via `tpt-l-linux-sandbox-rs`; `unrestricted` opt-out with warning.
- `ScriptOutcome`/`ScriptError` with signal-termination handling.
