# Changelog

All notable changes to this crate are documented in this file.

This project adheres to [Semantic Versioning](https://semver.org) and the
[Keep a Changelog](https://keepachangelog.com) format.

## [Unreleased]

### Added

- Initial release of `tpt-l-dpkg-db` (workspace version 0.1.0).
- `StatusDb` parser for `/var/lib/dpkg/status` using memory mapping.
- Typed `Status:` state machine (`PackageWant`, `PackageAction`, `InstallStatus`).
- Atomic write path (`write_atomic`, `apply_changes`).
- `ConcurrentStatusDb` with `RwLock` for safe concurrent readers and serialized writers.
- `StatusChange` diff operations for upsert/remove.
