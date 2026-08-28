# Changelog

All notable changes to this crate are documented in this file.

This project adheres to [Semantic Versioning](https://semver.org) and the
[Keep a Changelog](https://keepachangelog.com) format.

## [Unreleased]

### Added

- Initial release of `tpt-l-apt-keyring` (workspace version 0.1.0).
- `Keyring` management via pure-Rust `pgp` (rPGP) — no native deps.
- Load/merge/import/remove; binary + armored; `load_dir`; `add_to_keyring_file`.
- Clearsigned (`InRelease`) and detached (`Release.gpg`) verification.
- `VerificationPolicy` with time-aware expiry/revocation checks.
- `KeyInfo`/`VerifyResult` reporting of fingerprints and verified plaintext.
