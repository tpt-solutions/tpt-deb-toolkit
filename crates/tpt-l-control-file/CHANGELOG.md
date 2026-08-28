# Changelog

All notable changes to this crate are documented in this file.

This project adheres to [Semantic Versioning](https://semver.org) and the
[Keep a Changelog](https://keepachangelog.com) format.

## [Unreleased]

### Added

- Initial release of `tpt-l-control-file` (workspace version 0.1.0).
- Custom lazy/zero-copy deb822-style parser (no `deb822-lossless` dependency).
- `BinaryPackage` and `SourcePackage` typed stanzas.
- `ControlFile` strict single-stanza parser (duplicate fields / multiple stanzas are errors).
- `PackagesIndex` / `SourcesIndex` lazy streaming views over large indices.
- `BorrowedParagraph` zero-copy paragraph view; case-insensitive field lookup.
- `parse_control` / `parse_control_strict` and zero-copy `parse_control_borrowed` variants.
