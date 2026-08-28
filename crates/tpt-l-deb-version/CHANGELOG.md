# Changelog

All notable changes to this crate are documented in this file.

This project adheres to [Semantic Versioning](https://semver.org) and the
[Keep a Changelog](https://keepachangelog.com) format.

## [Unreleased]

### Added

- Initial release of `tpt-l-deb-version` (workspace version 0.1.0).
- `Version` type implementing the Debian Policy §5.6.12 ordering rules
  (epoch, upstream, revision), including correct `~` and digit-run handling.
- `VersionConstraint` (`<<`, `<=`, `=`, `>=`, `>>`) with `satisfies()`.
- `deb_str_cmp` standalone Debian string comparison.
- Optional `serde` feature for serializing `Version`.
