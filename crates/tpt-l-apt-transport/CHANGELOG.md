# Changelog

All notable changes to this crate are documented in this file.

This project adheres to [Semantic Versioning](https://semver.org) and the
[Keep a Changelog](https://keepachangelog.com) format.

## [Unreleased]

### Added

- Initial release of `tpt-l-apt-transport` (workspace version 0.1.0).
- Async `AptTransport` on `tokio` + `reqwest` (rustls).
- Index fetching with `.xz`/`.gz`/raw fallback; `InRelease`/`Release` fetching.
- Mirror failover via `fetch_with_mirrors`.
- Resumable downloads (`download_file_resumable`) and partial `fetch_range` fetches.
- `If-Modified-Since` (`304`) caching; streaming `download_file` with progress.
- `pdiff` module (rdiff delta apply/encode) and `release` `ReleaseIndex` parsing.
