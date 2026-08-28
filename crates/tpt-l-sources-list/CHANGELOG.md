# Changelog

All notable changes to this crate are documented in this file.

This project adheres to [Semantic Versioning](https://semver.org) and the
[Keep a Changelog](https://keepachangelog.com) format.

## [Unreleased]

### Added

- Initial release of `tpt-l-sources-list` (workspace version 0.1.0).
- Parser for one-line `sources.list` with option blocks and disabled entries.
- Parser for deb822 `.sources` stanzas, expanded into per-type/URI/suite entries.
- `load_file` (extension auto-detect) and `load_dir` merging `*.list`/`*.sources`.
- Round-trippable `write_one_line` / `write_deb822` serializers.
- URI validation and `release_url` / `packages_url` helpers.
