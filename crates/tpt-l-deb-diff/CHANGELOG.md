# Changelog

All notable changes to this crate are documented in this file.

This project adheres to [Semantic Versioning](https://semver.org) and the
[Keep a Changelog](https://keepachangelog.com) format.

## [Unreleased]

### Added

- Initial release of `tpt-l-deb-diff` (workspace version 0.1.0).
- `DebDiff::compare` / `compare_files` for diffing two `.deb` packages.
- Three-way diff report: control metadata, added/removed files, modified files.
- `FileChange` with per-file SHA-256 digests for content integrity.
- Serializable (`serde`) `DiffReport` with a human-readable `Display` impl.
