# Changelog

All notable changes to this crate are documented in this file.

This project adheres to [Semantic Versioning](https://semver.org) and the
[Keep a Changelog](https://keepachangelog.com) format.

## [Unreleased]

### Added

- Initial release of `tpt-l-deb-format` (workspace version 0.1.0).
- Streaming `ar` reader (`ArReader` / `ArEntry`) for `.deb` containers.
- `DebFile` with memory-mapped `open` and in-memory `parse`.
- Zero-copy metadata read (`open_metadata` / `parse_metadata`) independent of payload size.
- Full extraction, in-memory payload read, and lazy `data_entries` / `control_entries` streaming.
- Compression support: `.gz`, `.xz`, `.zst`, and uncompressed `data.tar.*` / `control.tar.*`.
