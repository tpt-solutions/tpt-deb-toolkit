# Changelog

All notable changes to this crate are documented in this file.

This project adheres to [Semantic Versioning](https://semver.org) and the
[Keep a Changelog](https://keepachangelog.com) format.

## [Unreleased]

### Added

- Initial release of `tpt-l-apt-solver` (workspace version 0.1.0).
- CDCL SAT solver (watched literals, 1UIP analysis, non-chronological backtracking, VSIDS).
- `Universe` with virtual-package providers; `from_binary_packages` parallel parse.
- `Resolver` returning an `InstallPlan` (`install`/`remove`/`recommended`/`suggested`).
- Depends/Pre-Depends/Conflicts/Breaks handling; highest-version pinning; upgrade modelling.
- Parallel Rayon portfolio with deterministic `seed == 0` mode.
