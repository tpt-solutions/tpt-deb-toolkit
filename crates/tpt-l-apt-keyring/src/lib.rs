//! OpenPGP keyring management and Release file verification for APT.
//!
//! Uses the pure-Rust [`pgp`] (rPGP) crate — no native library dependencies.
//!
//! # Example
//!
//! ```no_run
//! use tpt_l_apt_keyring::Keyring;
//! use std::path::Path;
//!
//! let keyring = Keyring::load(Path::new(
//!     "/usr/share/keyrings/debian-archive-keyring.gpg"
//! )).unwrap();
//! println!("Loaded {} keys", keyring.key_count());
//! ```

use std::io::Cursor;
use std::path::Path;

use chrono::{DateTime, Utc};
use pgp::composed::cleartext::CleartextSignedMessage;
use pgp::composed::{Deserializable, SignedPublicKey, StandaloneSignature};
use pgp::types::{Fingerprint, PublicKeyTrait};
use thiserror::Error;

// ─── Errors ───────────────────────────────────────────────────────────────────

/// Errors from keyring operations.
#[derive(Debug, Error)]
pub enum KeyringError {
    #[error("failed to load key material: {0}")]
    Load(String),
    #[error("signature verification failed: {0}")]
    Verification(String),
    #[error("no valid signature found")]
    NoSignature,
    #[error("signing key has expired")]
    KeyExpired(String),
    #[error("signing key has been revoked")]
    KeyRevoked(String),
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
}

// ─── Public types ─────────────────────────────────────────────────────────────

/// A hex-encoded OpenPGP key fingerprint.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct KeyId(pub String);

impl std::fmt::Display for KeyId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// Result of a successful signature verification.
#[derive(Debug)]
pub struct VerifyResult {
    /// Fingerprints of keys that produced a valid signature.
    pub signed_by: Vec<KeyId>,
    /// The verified plaintext message.
    pub message: Vec<u8>,
}

/// Policy applied when verifying a signature.
///
/// By default a signature is only accepted if the signing key is neither
/// expired nor revoked *at the reference time* (which defaults to "now").
/// Relaxing `allow_expired`/`allow_revoked` mirrors `apt`'s
/// `--allow-downgrades`/`--force` style escape hatches.
#[derive(Debug, Clone)]
pub struct VerificationPolicy {
    /// Time against which key expiry is checked. `None` means "now".
    pub reference_time: Option<DateTime<Utc>>,
    /// Accept signatures from keys whose validity period has lapsed.
    pub allow_expired: bool,
    /// Accept signatures from keys that carry a revocation signature.
    pub allow_revoked: bool,
}

impl Default for VerificationPolicy {
    fn default() -> Self {
        Self {
            reference_time: None,
            allow_expired: false,
            allow_revoked: false,
        }
    }
}

// ─── Keyring ──────────────────────────────────────────────────────────────────

/// A collection of trusted OpenPGP certificates used to verify APT metadata.
pub struct Keyring {
    keys: Vec<SignedPublicKey>,
}

impl Keyring {
    /// Create an empty keyring.
    pub fn empty() -> Self {
        Self { keys: Vec::new() }
    }

    /// Return the number of certificates in this keyring.
    pub fn key_count(&self) -> usize {
        self.keys.len()
    }

    /// Add all certificates from `other` into this keyring.
    pub fn merge(&mut self, other: Keyring) {
        self.keys.extend(other.keys);
    }

    /// Load certificates from a `.gpg` (binary) or `.asc` (armored) file.
    pub fn load(path: &Path) -> Result<Self, KeyringError> {
        let data = std::fs::read(path)?;
        Self::load_bytes(&data)
    }

    /// Load certificates from raw bytes (binary or ASCII-armored).
    pub fn load_bytes(data: &[u8]) -> Result<Self, KeyringError> {
        if data.is_empty() {
            return Err(KeyringError::Load("empty key material".into()));
        }

        let mut keys = Vec::new();

        if data.starts_with(b"-----") {
            // ASCII-armored keyring
            match SignedPublicKey::from_armor_many(Cursor::new(data)) {
                Ok((iter, _headers)) => {
                    for key_res in iter {
                        match key_res {
                            Ok(key) => keys.push(key),
                            Err(e) => tracing::warn!("skipping invalid armored key: {}", e),
                        }
                    }
                }
                Err(e) => return Err(KeyringError::Load(e.to_string())),
            }
        } else {
            // Binary key material
            for key_res in SignedPublicKey::from_bytes_many(Cursor::new(data)) {
                match key_res {
                    Ok(key) => keys.push(key),
                    Err(e) => tracing::warn!("skipping invalid binary key: {}", e),
                }
            }
        }

        if keys.is_empty() {
            return Err(KeyringError::Load(
                "no valid certificates found in key material".into(),
            ));
        }
        Ok(Self { keys })
    }

    /// Load all `.gpg` and `.asc` files from a directory.
    pub fn load_dir(dir: &Path) -> Result<Self, KeyringError> {
        let mut merged = Self::empty();
        let entries = std::fs::read_dir(dir)?;
        for entry in entries {
            let entry = entry?;
            let path = entry.path();
            let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
            if ext == "gpg" || ext == "asc" {
                match Self::load(&path) {
                    Ok(k) => merged.merge(k),
                    Err(e) => tracing::warn!("skipping {}: {}", path.display(), e),
                }
            }
        }
        Ok(merged)
    }

    /// Verify a clearsigned `InRelease` file using the default policy
    /// ([`VerificationPolicy::default`]). See [`Keyring::verify_clearsigned_with`].
    pub fn verify_clearsigned(&self, data: &[u8]) -> Result<VerifyResult, KeyringError> {
        self.verify_clearsigned_with(data, &VerificationPolicy::default())
    }

    /// Verify a clearsigned `InRelease` file, applying [`VerificationPolicy`].
    ///
    /// A key only counts as a valid signer when its signature verifies *and* it
    /// passes the policy (not expired / not revoked at the reference time).
    /// When the only matching key(s) fail the policy, [`KeyringError::KeyExpired`]
    /// or [`KeyringError::KeyRevoked`] is returned instead of [`KeyringError::NoSignature`]
    /// so callers can distinguish "no key matched" from "key matched but is unusable".
    pub fn verify_clearsigned_with(
        &self,
        data: &[u8],
        policy: &VerificationPolicy,
    ) -> Result<VerifyResult, KeyringError> {
        let (msg, _) = CleartextSignedMessage::from_armor(Cursor::new(data))
            .map_err(|e| KeyringError::Verification(e.to_string()))?;

        let when = policy.reference_time.unwrap_or_else(Utc::now);
        let mut signed_by = Vec::new();
        let (mut expired_hit, mut revoked_hit) = (false, false);
        for key in &self.keys {
            if msg.verify(key).is_err() {
                continue;
            }
            if !policy.allow_expired && key_expired_at(key, when) {
                expired_hit = true;
                continue;
            }
            if !policy.allow_revoked && key_is_revoked(key) {
                revoked_hit = true;
                continue;
            }
            signed_by.push(KeyId(fp_hex(&key.fingerprint())));
        }

        if signed_by.is_empty() {
            if expired_hit {
                return Err(KeyringError::KeyExpired(
                    "a matching signature was produced by an expired key".into(),
                ));
            }
            if revoked_hit {
                return Err(KeyringError::KeyRevoked(
                    "a matching signature was produced by a revoked key".into(),
                ));
            }
            return Err(KeyringError::NoSignature);
        }

        let plaintext = msg.signed_text().as_bytes().to_vec();
        Ok(VerifyResult {
            signed_by,
            message: plaintext,
        })
    }

    /// Verify a detached `Release.gpg` signature against a `Release` body using
    /// the default policy. See [`Keyring::verify_detached_with`].
    pub fn verify_detached(
        &self,
        data: &[u8],
        signature: &[u8],
    ) -> Result<VerifyResult, KeyringError> {
        self.verify_detached_with(data, signature, &VerificationPolicy::default())
    }

    /// Verify a detached `Release.gpg` signature against a `Release` body,
    /// applying [`VerificationPolicy`]. Mirrors [`Keyring::verify_clearsigned_with`]
    /// for detached signatures.
    pub fn verify_detached_with(
        &self,
        data: &[u8],
        signature: &[u8],
        policy: &VerificationPolicy,
    ) -> Result<VerifyResult, KeyringError> {
        let (sig, _) = StandaloneSignature::from_armor_single(Cursor::new(signature))
            .map_err(|e| KeyringError::Verification(e.to_string()))?;

        let when = policy.reference_time.unwrap_or_else(Utc::now);
        let mut signed_by = Vec::new();
        let (mut expired_hit, mut revoked_hit) = (false, false);
        for key in &self.keys {
            if sig.verify(key, data).is_err() {
                continue;
            }
            if !policy.allow_expired && key_expired_at(key, when) {
                expired_hit = true;
                continue;
            }
            if !policy.allow_revoked && key_is_revoked(key) {
                revoked_hit = true;
                continue;
            }
            signed_by.push(KeyId(fp_hex(&key.fingerprint())));
        }

        if signed_by.is_empty() {
            if expired_hit {
                return Err(KeyringError::KeyExpired(
                    "a matching signature was produced by an expired key".into(),
                ));
            }
            if revoked_hit {
                return Err(KeyringError::KeyRevoked(
                    "a matching signature was produced by a revoked key".into(),
                ));
            }
            return Err(KeyringError::NoSignature);
        }

        Ok(VerifyResult {
            signed_by,
            message: data.to_vec(),
        })
    }
}

/// Returns `true` when `key`'s validity period has ended at `when`.
fn key_expired_at(key: &SignedPublicKey, when: DateTime<Utc>) -> bool {
    match key.expires_at() {
        Some(expiry) => when > expiry,
        None => false,
    }
}

/// Returns `true` when `key` carries a revocation signature.
///
/// This is a presence check: `SignedKeyDetails::revocation_signatures` is
/// populated by the parser for both self- and authority-revoked keys. Full
/// cryptographic validation of the revocation is performed by
/// `SignedKeyDetails::verify()`, which is invoked by callers validating the
/// whole key.
fn key_is_revoked(key: &SignedPublicKey) -> bool {
    !key.details.revocation_signatures.is_empty()
}

fn fp_hex(fp: &Fingerprint) -> String {
    match fp {
        Fingerprint::V2(b) => hex::encode(b),
        Fingerprint::V3(b) => hex::encode(b),
        Fingerprint::V4(b) => hex::encode(b),
        Fingerprint::V5(b) => hex::encode(b),
        Fingerprint::V6(b) => hex::encode(b),
        Fingerprint::Unknown(b) => hex::encode(b.as_ref()),
    }
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_keyring_has_zero_keys() {
        let kr = Keyring::empty();
        assert_eq!(kr.key_count(), 0);
    }

    #[test]
    fn merge_two_empty_keyrings() {
        let mut a = Keyring::empty();
        let b = Keyring::empty();
        a.merge(b);
        assert_eq!(a.key_count(), 0);
    }

    #[test]
    fn load_bytes_errors_on_empty_input() {
        let result = Keyring::load_bytes(b"");
        assert!(result.is_err());
    }

    #[test]
    fn load_bytes_errors_on_garbage() {
        let result = Keyring::load_bytes(b"not pgp data at all!!");
        assert!(result.is_err());
    }
}
