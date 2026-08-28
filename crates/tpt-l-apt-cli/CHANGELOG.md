# Changelog

All notable changes to this crate are documented in this file.

This project adheres to [Semantic Versioning](https://semver.org) and the
[Keep a Changelog](https://keepachangelog.com) format.

## [Unreleased]

### Added

- Initial release of `tpt-l-apt-cli` (workspace version 0.1.0).
- `update` (PDiff-aware index caching), `install` (SAT-resolve → download → extract → postinst).
- Offline `search` / `show` / `list` against cached indices and the dpkg status DB.
- `completions` for bash/zsh/fish; `--dry-run`, `--verbose`, `--json` flags.
- Library `Apt` API for embedding the CLI logic.
