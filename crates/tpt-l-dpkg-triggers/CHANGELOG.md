# Changelog

All notable changes to this crate are documented in this file.

This project adheres to [Semantic Versioning](https://semver.org) and the
[Keep a Changelog](https://keepachangelog.com) format.

## [Unreleased]

### Added

- Initial release of `tpt-l-dpkg-triggers` (workspace version 0.1.0).
- `TriggerDb` tracking interests and pending triggers per package.
- `interest` / `add_trigger`; `activate` / `activate_all` (idempotent, returns affected packages).
- `process` to dequeue a package's pending triggers; `is_pending`/`pending_of`/`pending_packages`.
- `save_dir` / `load_dir` persistence; `status` module bridging to `InstallStatus` transitions.
