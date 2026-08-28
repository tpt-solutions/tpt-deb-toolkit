# tpt-l-apt-keyring

OpenPGP keyring management and `Release`/`InRelease` file verification for APT.

Part of the [`tpt-deb-toolkit`](https://github.com/tpt-solutions/tpt-deb-toolkit)
workspace — a pure-Rust pipeline for Debian/Ubuntu package management.

Uses the pure-Rust [`pgp`](https://crates.io/crates/pgp) (rPGP) crate — **no
native library dependencies** (this is a deliberate choice; see
`docs/DEPENDENCY_DECISIONS.md`, which rules out `sequoia-pgp` here).

## Layer

**Layer 3 — Network.** Used by `tpt-l-apt-transport` callers to verify fetched `Release` files.

## Features

- `Keyring` — load/merge/import/remove certificates (`.gpg` binary or `.asc` armored).
- `add_to_keyring_file` — an `apt-key add` replacement (auto binary vs armored by extension).
- `load_dir` — merge every `.gpg`/`.asc` in a directory.
- `verify_clearsigned` — verify `InRelease` (clearsigned) files.
- `verify_detached` — verify a detached `Release.gpg` against a `Release` body.
- `VerificationPolicy` — time-aware expiry/revocation checks; distinguishes "no key
  matched" (`NoSignature`) from "key matched but is expired/revoked".
- `list` / `KeyInfo` — enumerate fingerprints, UIDs, and expiry.

## Installation

```toml
[dependencies]
tpt-l-apt-keyring = "0.1.0"
```

## Usage

```rust
use tpt_l_apt_keyring::Keyring;
use std::path::Path;

let keyring = Keyring::load(Path::new("/usr/share/keyrings/debian-archive-keyring.gpg")).unwrap();
let result = keyring.verify_clearsigned(&inrelease_bytes).unwrap();
println!("signed by {} key(s)", result.signed_by.len());
```

### Detached `Release.gpg`

```rust
let result = keyring.verify_detached(&release_bytes, &release_gpg_bytes).unwrap();
```

## API overview

- `Keyring` — certificate collection (`load`, `load_bytes`, `load_dir`, `import`, `merge`, `remove`, `save_binary`, `save_armored`, `add_to_keyring_file`, `list`, `verify_clearsigned`, `verify_detached`).
- `KeyId`, `KeyInfo`, `VerifyResult` — key identities and verification results.
- `VerificationPolicy` — reference time, `allow_expired`, `allow_revoked`.
- `KeyringError` — load/verification/expiry/revocation/IO failures.

## Design notes

A key counts as a valid signer only when its signature verifies *and* it passes the
policy (not expired / not revoked at the reference time). When the only matching
key(s) fail the policy, `KeyExpired`/`KeyRevoked` is returned instead of
`NoSignature`, so callers can distinguish "no key matched" from "key matched but is
unusable".

## License

Dual-licensed under [MIT](../../LICENSE-MIT) or
[Apache-2.0](../../LICENSE-APACHE) at your option.

Copyright © 2026 TPT Solutions.
