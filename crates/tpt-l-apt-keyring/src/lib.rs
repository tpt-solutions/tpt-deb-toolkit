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
use pgp::ser::Serialize;
use pgp::types::{Fingerprint, PublicKeyTrait};
use pgp::ArmorOptions;
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
#[derive(Debug, Clone, Default)]
pub struct VerificationPolicy {
    /// Time against which key expiry is checked. `None` means "now".
    pub reference_time: Option<DateTime<Utc>>,
    /// Accept signatures from keys whose validity period has lapsed.
    pub allow_expired: bool,
    /// Accept signatures from keys that carry a revocation signature.
    pub allow_revoked: bool,
}

// ─── Keyring ──────────────────────────────────────────────────────────────────

/// A collection of trusted OpenPGP certificates used to verify APT metadata.
pub struct Keyring {
    keys: Vec<SignedPublicKey>,
}

/// Metadata describing a single certificate in a [`Keyring`].
#[derive(Debug, Clone)]
pub struct KeyInfo {
    /// Fingerprint of the primary key.
    pub fingerprint: KeyId,
    /// User IDs (UIDs) bound to the key by self-signature.
    pub user_ids: Vec<String>,
    /// Expiry timestamp, if the key has a finite validity period.
    pub expires_at: Option<DateTime<Utc>>,
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

    /// Return metadata about every certificate in the keyring.
    pub fn list(&self) -> Vec<KeyInfo> {
        self.keys
            .iter()
            .map(|key| {
                let user_ids = key
                    .details
                    .users
                    .iter()
                    .map(|u| String::from_utf8_lossy(u.id.id().as_ref()).into_owned())
                    .collect();
                KeyInfo {
                    fingerprint: KeyId(fp_hex(&key.fingerprint())),
                    user_ids,
                    expires_at: key.expires_at(),
                }
            })
            .collect()
    }

    /// Parse and merge the certificates found in `data` into this keyring.
    ///
    /// Mirrors `apt-key add`: unknown/garbage bytes are skipped per-key, but an
    /// entirely empty input still errors via [`KeyringError::Load`].
    pub fn import(&mut self, data: &[u8]) -> Result<(), KeyringError> {
        let other = Self::load_bytes(data)?;
        self.keys.extend(other.keys);
        Ok(())
    }

    /// Remove every certificate whose fingerprint (hex, case-insensitive)
    /// equals `fingerprint`. Returns the number of certificates removed.
    pub fn remove(&mut self, fingerprint: &str) -> usize {
        let target = fingerprint.to_lowercase();
        let before = self.keys.len();
        self.keys
            .retain(|k| fp_hex(&k.fingerprint()).to_lowercase() != target);
        before - self.keys.len()
    }

    /// Remove a certificate by its [`KeyId`]. Returns the number removed (0 or 1).
    pub fn remove_by_id(&mut self, id: &KeyId) -> usize {
        self.remove(&id.0)
    }

    /// Serialize every certificate as a binary (GPG) keyring to `path`.
    pub fn save_binary(&self, path: &Path) -> Result<(), KeyringError> {
        let mut buf = Vec::new();
        for key in &self.keys {
            let bytes = key
                .to_bytes()
                .map_err(|e| KeyringError::Load(e.to_string()))?;
            buf.extend_from_slice(&bytes);
        }
        std::fs::write(path, buf)?;
        Ok(())
    }

    /// Serialize every certificate as an ASCII-armored keyring to `path`.
    pub fn save_armored(&self, path: &Path) -> Result<(), KeyringError> {
        let mut buf = Vec::new();
        for key in &self.keys {
            let bytes = key
                .to_armored_bytes(ArmorOptions::default())
                .map_err(|e| KeyringError::Load(e.to_string()))?;
            buf.extend_from_slice(&bytes);
            buf.push(b'\n');
        }
        std::fs::write(path, buf)?;
        Ok(())
    }

    /// `apt-key add` replacement: load the existing keyring at `path` (or start
    /// a new one if the file does not exist), import `data`, and write it back
    /// in the format implied by the file extension (`.asc` → armored, else
    /// binary).
    pub fn add_to_keyring_file(path: &Path, data: &[u8]) -> Result<(), KeyringError> {
        let mut kr = if path.exists() {
            Self::load(path)?
        } else {
            Self::empty()
        };
        kr.import(data)?;
        let armored = path
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.eq_ignore_ascii_case("asc"))
            .unwrap_or(false);
        if armored {
            kr.save_armored(path)
        } else {
            kr.save_binary(path)
        }
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

    use pgp::composed::key::{KeyType, SecretKeyParamsBuilder};
    use pgp::composed::SignedSecretKey;
    use pgp::ArmorOptions;
    use rand::thread_rng;

    /// Generate a fresh self-signed Ed25519 key (signing + certify capable).
    fn generate_test_key() -> (SignedSecretKey, SignedPublicKey) {
        let mut rng = thread_rng();
        let params = SecretKeyParamsBuilder::default()
            .key_type(KeyType::Ed25519)
            .can_sign(true)
            .can_certify(true)
            .primary_user_id("tpt test key <test@tpt.example>".to_string())
            .build()
            .expect("build key params");
        let secret = params.generate(&mut rng).expect("generate key");
        let signed = secret.sign(&mut rng, String::new).expect("self-sign key");
        let public: SignedPublicKey = signed.clone().into();
        (signed, public)
    }

    #[test]
    fn workflow_list_and_remove() {
        let (_, public) = generate_test_key();
        let mut kr = Keyring { keys: vec![public] };
        assert_eq!(kr.key_count(), 1);

        let info = &kr.list()[0];
        assert_eq!(info.user_ids, vec!["tpt test key <test@tpt.example>"]);
        assert!(!info.fingerprint.0.is_empty());

        let removed = kr.remove(&info.fingerprint.0);
        assert_eq!(removed, 1);
        assert_eq!(kr.key_count(), 0);
    }

    #[test]
    fn workflow_import_and_export_armored_roundtrip() {
        let (_, public) = generate_test_key();
        let kr = Keyring {
            keys: vec![public.clone()],
        };
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("keyring.asc");

        kr.save_armored(&path).unwrap();
        let reloaded = Keyring::load(&path).unwrap();
        assert_eq!(reloaded.key_count(), 1);
        assert_eq!(
            reloaded.list()[0].fingerprint.0,
            fp_hex(&public.fingerprint())
        );
    }

    #[test]
    fn workflow_add_to_keyring_file_appends() {
        let (_, public_a) = generate_test_key();
        let (_, public_b) = generate_test_key();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("trusted.gpg");

        Keyring {
            keys: vec![public_a],
        }
        .save_binary(&path)
        .unwrap();

        let mut buf = Vec::new();
        buf.extend_from_slice(&public_b.to_armored_bytes(ArmorOptions::default()).unwrap());
        Keyring::add_to_keyring_file(&path, &buf).unwrap();

        let reloaded = Keyring::load(&path).unwrap();
        assert_eq!(reloaded.key_count(), 2);
    }

    #[test]
    fn verify_real_generated_key_succeeds() {
        let (signed, public) = generate_test_key();
        let mut rng = thread_rng();
        let text = "Origin: Debian\nLabel: Debian\nSuite: stable\n";
        let msg = CleartextSignedMessage::sign(&mut rng, text, &signed, String::new).unwrap();
        let armored = msg.to_armored_bytes(ArmorOptions::default()).unwrap();

        let kr = Keyring { keys: vec![public] };
        let result = kr.verify_clearsigned(&armored).expect("valid signature");
        assert_eq!(result.signed_by.len(), 1);
        assert!(String::from_utf8_lossy(&result.message).contains("Suite: stable"));
    }

    #[test]
    fn verify_fails_with_wrong_key() {
        let (signed, _) = generate_test_key();
        let (_, other_public) = generate_test_key();
        let mut rng = thread_rng();
        let msg = CleartextSignedMessage::sign(&mut rng, "data", &signed, String::new).unwrap();
        let armored = msg.to_armored_bytes(ArmorOptions::default()).unwrap();

        let kr = Keyring {
            keys: vec![other_public],
        };
        assert!(kr.verify_clearsigned(&armored).is_err());
    }
}
