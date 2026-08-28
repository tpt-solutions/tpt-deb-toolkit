# Changelog

All notable changes to this crate are documented in this file.

This project adheres to [Semantic Versioning](https://semver.org) and the
[Keep a Changelog](https://keepachangelog.com) format.

## [Unreleased]

### Added

- Initial release of `tpt-l-apt-config` (workspace version 0.1.0).
- Parser accepting both nested-scope and flat `::` APT config syntax.
- `#include` / `#include-dir` resolution with a cycle guard.
- Line and block comment support.
- Typed accessors (`get`, `get_bool`, `get_int`, `get_list`, `get_or_default`) and `merge`.
- Convenience shortcuts `sources_list_path` / `status_db_path`.
