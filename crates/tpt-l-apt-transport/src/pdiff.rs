//! PDiff (debian delta index) support.
//!
//! APT can update a large `Packages`/`Sources` index without re-downloading the
//! whole file: the archive publishes a `*.diff/Index` listing available
//! *rdiff* deltas between historical revisions, plus gzip-compressed patch
//! files. Given a locally cached index and the target hash from `Release`, a
//! client finds a chain of deltas that transforms the cached file into the new
//! one and applies them.
//!
//! This module implements:
//!
//! * [`PdiffIndex`] parsing of the `*.diff/Index` file,
//! * [`resolve_chain`] — shortest-path resolution from the cached hash to the
//!   target hash over the available diffs,
//! * [`apply_rdiff_delta`] — a pure-Rust applier for the librsync/rdiff2 delta
//!   format (no C `librsync` dependency), and
//! * [`AptTransport::fetch_pdiff`] — async orchestration (download Index +
//!   patches, gunzip, apply, verify).
//!
//! # rdiff2 delta format
//!
//! Deltas are librsync "RDIFF2" files: a big-endian `u32` magic
//! `0x72730236` (`"rs\x026"`), followed by a stream of commands. Each command
//! is a single tag byte; the argument widths are implied by the tag:
//!
//! * `0x00` — end of delta.
//! * `0x41..=0x44` — literal: `tag - 0x40` bytes (1..4) big-endian encode the
//!   literal length, then that many literal bytes.
//! * `0x45..=0x54` — copy from the basis file: the tag selects the widths of
//!   the `where` and `length` arguments (each 1..4 bytes, big-endian).

use std::collections::{HashMap, VecDeque};
use std::io::Read;

use sha2::{Digest, Sha256};
use thiserror::Error;

// ─── Errors ───────────────────────────────────────────────────────────────────

/// Errors specific to PDiff parsing / application.
#[derive(Debug, Error)]
pub enum PdiffError {
    /// The delta did not start with the rdiff2 magic `0x72730236`.
    #[error("invalid rdiff delta magic")]
    BadMagic,
    /// The delta instruction stream was truncated or otherwise malformed.
    #[error("corrupt rdiff delta: {0}")]
    Corrupt(String),
    /// An unknown command byte was encountered in the delta stream.
    #[error("unimplemented rdiff command byte {0:#x}")]
    Unimplemented(u8),
    /// A reconstructed file did not hash to the expected value.
    #[error("sha256 mismatch: expected {expected}, got {actual}")]
    Sha256Mismatch { expected: String, actual: String },
}

// ─── Index ───────────────────────────────────────────────────────────────────

/// A single available delta in a `*.diff/Index` file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PdiffEntry {
    /// SHA-256 of the old (basis) index revision.
    pub old_sha256: String,
    /// SHA-256 of the new index revision produced by this delta.
    pub new_sha256: String,
    /// Size in bytes of the (compressed) patch file.
    pub size: u64,
    /// Patch file path, relative to the index's parent directory
    /// (e.g. `Packages.diff/<old>_<new>.gz`).
    pub patch_file: String,
}

/// A parsed `*.diff/Index` file.
#[derive(Debug, Clone, Default)]
pub struct PdiffIndex {
    /// Available deltas.
    pub entries: Vec<PdiffEntry>,
}

impl PdiffIndex {
    /// Parse a `*.diff/Index` document.
    ///
    /// Each non-empty line is `<old-sha256> <new-sha256> <size> <patch-file>`.
    pub fn parse(text: &str) -> Result<Self, PdiffError> {
        let mut entries = Vec::new();
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() != 4 {
                return Err(PdiffError::Corrupt(format!(
                    "expected 4 fields, got {}: {line:?}",
                    parts.len()
                )));
            }
            let size = parts[2]
                .parse::<u64>()
                .map_err(|e| PdiffError::Corrupt(format!("bad size field: {e}")))?;
            entries.push(PdiffEntry {
                old_sha256: parts[0].to_ascii_lowercase(),
                new_sha256: parts[1].to_ascii_lowercase(),
                size,
                patch_file: parts[3].to_string(),
            });
        }
        Ok(Self { entries })
    }
}

// ─── Chain resolution ─────────────────────────────────────────────────────────

/// Find a chain of [`PdiffEntry`]s transforming `current` into `target`.
///
/// The diffs form a directed graph over revision hashes. A breadth-first
/// search from `current` to `target` yields the fewest-hops chain (APT prefers
/// few patches over a single large merged patch). Returns `None` when no chain
/// exists (e.g. the cached revision is not an ancestor of the target among the
/// published diffs). An empty chain means `current == target` (already current).
pub fn resolve_chain(
    index: &PdiffIndex,
    current: &str,
    target: &str,
) -> Option<Vec<PdiffEntry>> {
    let current = current.to_ascii_lowercase();
    let target = target.to_ascii_lowercase();
    if current == target {
        return Some(Vec::new());
    }

    let mut adj: HashMap<String, Vec<usize>> = HashMap::new();
    for (i, e) in index.entries.iter().enumerate() {
        adj.entry(e.old_sha256.clone()).or_default().push(i);
    }

    let mut seen: HashMap<String, ()> = HashMap::new();
    // node -> (predecessor node, edge index)
    let mut prev: HashMap<String, (String, usize)> = HashMap::new();
    let mut queue: VecDeque<String> = VecDeque::new();
    seen.insert(current.clone(), ());
    queue.push_back(current.clone());

    while let Some(node) = queue.pop_front() {
        if node == target {
            break;
        }
        if let Some(edges) = adj.get(&node) {
            for &ei in edges {
                let nxt = index.entries[ei].new_sha256.clone();
                if !seen.contains_key(&nxt) {
                    seen.insert(nxt.clone(), ());
                    prev.insert(nxt.clone(), (node.clone(), ei));
                    queue.push_back(nxt);
                }
            }
        }
    }

    if !seen.contains_key(&target) {
        return None;
    }

    let mut path = Vec::new();
    let mut node = target;
    while node != current {
        let (p, ei) = prev
            .get(&node)
            .expect("predecessor must exist for reached node")
            .clone();
        path.push(index.entries[ei].clone());
        node = p;
    }
    path.reverse();
    Some(path)
}

// ─── rdiff2 delta apply ───────────────────────────────────────────────────────

const RDIFF_MAGIC: [u8; 4] = [0x72, 0x73, 0x02, 0x36];

/// Read `b` as a big-endian unsigned integer.
fn read_be(b: &[u8]) -> u64 {
    let mut v = 0u64;
    for &x in b {
        v = (v << 8) | u64::from(x);
    }
    v
}

/// Apply a librsync/rdiff2 delta to `basis`, producing the new file.
///
/// Pure Rust: no system `librsync` required.
pub fn apply_rdiff_delta(basis: &[u8], delta: &[u8]) -> Result<Vec<u8>, PdiffError> {
    if delta.len() < 4 || delta[0..4] != RDIFF_MAGIC {
        return Err(PdiffError::BadMagic);
    }
    let mut out = Vec::new();
    let mut i = 4;
    while i < delta.len() {
        let tag = delta[i];
        i += 1;
        match tag {
            // END
            0x00 => break,
            // LITERAL (1..4 byte length)
            0x41..=0x44 => {
                let n = (tag - 0x40) as usize;
                if i + n > delta.len() {
                    return Err(PdiffError::Corrupt("literal length truncated".into()));
                }
                let len = read_be(&delta[i..i + n]) as usize;
                i += n;
                if i + len > delta.len() {
                    return Err(PdiffError::Corrupt("literal data truncated".into()));
                }
                out.extend_from_slice(&delta[i..i + len]);
                i += len;
            }
            // COPY from basis (where_len, len_len each 1..4 bytes)
            0x45..=0x54 => {
                let idx = (tag - 0x45) as usize;
                let where_len = idx / 4 + 1;
                let len_len = idx % 4 + 1;
                if i + where_len + len_len > delta.len() {
                    return Err(PdiffError::Corrupt("copy args truncated".into()));
                }
                let where_ = read_be(&delta[i..i + where_len]) as usize;
                i += where_len;
                let length = read_be(&delta[i..i + len_len]) as usize;
                i += len_len;
                let end = where_
                    .checked_add(length)
                    .ok_or_else(|| PdiffError::Corrupt("copy overflow".into()))?;
                if end > basis.len() {
                    return Err(PdiffError::Corrupt(format!(
                        "copy {where_}..{end} beyond basis length {}",
                        basis.len()
                    )));
                }
                out.extend_from_slice(&basis[where_..end]);
            }
            other => return Err(PdiffError::Unimplemented(other)),
        }
    }
    Ok(out)
}

/// Compute the lowercase hex SHA-256 of `bytes`.
pub fn sha256_hex(bytes: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(bytes);
    hex::encode(h.finalize())
}

// ─── rdiff2 delta encode (used by tests / tooling) ───────────────────────────

/// A single rdiff2 command.
#[derive(Debug, Clone, Copy)]
pub enum RdiffOp<'a> {
    /// Emit `data` literally.
    Literal(&'a [u8]),
    /// Copy `length` bytes from `basis[offset..]` into the output.
    Copy(u64, u64),
}

fn bytes_needed(v: usize) -> usize {
    if v == 0 {
        return 1;
    }
    let bits = 64 - v.leading_zeros() as usize;
    bits.div_ceil(8).clamp(1, 4)
}

fn write_be(out: &mut Vec<u8>, v: u64, n: usize) {
    for k in (0..n).rev() {
        out.push((v >> (k * 8)) as u8);
    }
}

/// Encode a sequence of [`RdiffOp`]s into an rdiff2 delta matching the format
/// `apply_rdiff_delta` decodes. Mostly useful for tests and for tooling that
/// regenerates index diffs.
pub fn encode_rdiff_delta(ops: &[RdiffOp]) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(&RDIFF_MAGIC);
    for op in ops {
        match op {
            RdiffOp::Literal(data) => {
                let n = bytes_needed(data.len());
                out.push(0x40 + n as u8);
                write_be(&mut out, data.len() as u64, n);
                out.extend_from_slice(data);
            }
            RdiffOp::Copy(where_, len) => {
                let wl = bytes_needed(*where_ as usize);
                let ll = bytes_needed(*len as usize);
                let tag = 0x45 + (wl as u8 - 1) * 4 + (ll as u8 - 1);
                out.push(tag);
                write_be(&mut out, *where_, wl);
                write_be(&mut out, *len, ll);
            }
        }
    }
    out.push(0x00);
    out
}

// ─── Transport integration ────────────────────────────────────────────────────

use crate::{TransportError, AptTransport};

/// The locally cached index a delta chain is applied to.
#[derive(Debug, Clone, Copy)]
pub struct PdiffBasis<'a> {
    /// SHA-256 of the cached index (must match the basis bytes).
    pub sha256: &'a str,
    /// The cached index bytes.
    pub bytes: &'a [u8],
}

/// Outcome of [`AptTransport::fetch_pdiff`].
pub enum PdiffUpdate {
    /// The index was reconstructed by applying a delta chain; contains the new
    /// index bytes (verified to hash to `target_sha256`).
    Reconstructed(Vec<u8>),
    /// No usable delta chain existed, so the caller should fall back to a full
    /// index download.
    NoDelta,
}

impl AptTransport {
    /// Fetch and apply a PDiff update for an index.
    ///
    /// `diff_index_url` is the `.diff/Index` URL (e.g.
    /// `.../dists/<suite>/<component>/binary-<arch>/Packages.diff/Index`).
    /// `current` is the locally cached revision; when `None` (no cache) the
    /// function returns [`PdiffUpdate::NoDelta`] so the caller can fetch the
    /// full index. `target_sha256` is the desired revision's hash (from
    /// `Release`).
    ///
    /// On success the reconstructed bytes are verified to hash to
    /// `target_sha256` at every step.
    pub async fn fetch_pdiff(
        &self,
        diff_index_url: &str,
        current: Option<&PdiffBasis<'_>>,
        target_sha256: &str,
    ) -> Result<PdiffUpdate, TransportError> {
        let (current_hash, mut basis) = match current {
            Some(b) => (b.sha256.to_ascii_lowercase(), b.bytes.to_vec()),
            None => return Ok(PdiffUpdate::NoDelta),
        };

        if current_hash == target_sha256.to_ascii_lowercase() {
            return Ok(PdiffUpdate::Reconstructed(basis));
        }

        let index_bytes = self.fetch_bytes(diff_index_url).await?;
        let index = PdiffIndex::parse(&String::from_utf8_lossy(&index_bytes))
            .map_err(|e| TransportError::Pdiff(e.to_string()))?;

        let chain = resolve_chain(&index, &current_hash, target_sha256).ok_or_else(|| {
            TransportError::Pdiff("no delta chain from current to target".into())
        })?;

        // Patch paths are relative to the directory containing the `.diff/`
        // folder, i.e. the parent of `/Packages.diff/`.
        let dir = if let Some(p) = diff_index_url.rfind("/Packages.diff/") {
            format!("{}/", &diff_index_url[..p])
        } else {
            let cut = diff_index_url.rfind('/').unwrap_or(0);
            format!("{}/", &diff_index_url[..=cut])
        };

        for entry in &chain {
            let patch_url = format!("{}{}", dir, entry.patch_file);
            let gz = self.fetch_bytes(&patch_url).await?;
            let delta = gunzip(&gz)?;
            basis = apply_rdiff_delta(&basis, &delta)
                .map_err(|e| TransportError::Pdiff(e.to_string()))?;
            let got = sha256_hex(&basis);
            if got != entry.new_sha256 {
                return Err(TransportError::Pdiff(format!(
                    "patch produced hash {got}, expected {}",
                    entry.new_sha256
                )));
            }
        }

        let got = sha256_hex(&basis);
        if got != target_sha256.to_ascii_lowercase() {
            return Err(TransportError::Pdiff(format!(
                "final hash {got} != target {target_sha256}"
            )));
        }
        Ok(PdiffUpdate::Reconstructed(basis))
    }
}

/// Build the standard `*.diff/Index` URL for a binary `Packages` index.
pub fn packages_diff_index_url(
    base_url: &str,
    suite: &str,
    component: &str,
    arch: &str,
) -> String {
    format!(
        "{}/dists/{}/{}/binary-{}/Packages.diff/Index",
        base_url.trim_end_matches('/'),
        suite,
        component,
        arch
    )
}

/// Build the standard `*.diff/Index` URL for a `Sources` index.
pub fn sources_diff_index_url(base_url: &str, suite: &str, component: &str) -> String {
    format!(
        "{}/dists/{}/{}/source/Sources.diff/Index",
        base_url.trim_end_matches('/'),
        suite,
        component
    )
}

/// Gunzip `raw` (used for the `.gz`-compressed patch files).
fn gunzip(raw: &[u8]) -> Result<Vec<u8>, TransportError> {
    let mut decoder = flate2::read::GzDecoder::new(raw);
    let mut out = Vec::new();
    decoder
        .read_to_end(&mut out)
        .map_err(|e| TransportError::DecompressError(e.to_string()))?;
    Ok(out)
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_index_basic() {
        let text = "\
deadbeef  cafe0000  1234  Packages.diff/deadbeef_cafe0000.gz
cafe0000   feedface  99    Packages.diff/cafe0000_feedface.gz
";
        let idx = PdiffIndex::parse(text).unwrap();
        assert_eq!(idx.entries.len(), 2);
        assert_eq!(idx.entries[0].old_sha256, "deadbeef");
        assert_eq!(idx.entries[0].size, 1234);
        assert_eq!(idx.entries[0].patch_file, "Packages.diff/deadbeef_cafe0000.gz");
    }

    #[test]
    fn parse_index_rejects_bad_line() {
        assert!(PdiffIndex::parse("only three fields here\n").is_err());
    }

    #[test]
    fn resolve_chain_single_hop() {
        let idx = PdiffIndex::parse(
            "aaaa bbbb 1 p/a_b.gz\nbbbb cccc 1 p/b_c.gz\n",
        )
        .unwrap();
        let chain = resolve_chain(&idx, "aaaa", "cccc").unwrap();
        assert_eq!(chain.len(), 2);
        assert_eq!(chain[0].old_sha256, "aaaa");
        assert_eq!(chain[1].new_sha256, "cccc");
    }

    #[test]
    fn resolve_chain_nonexistent() {
        let idx = PdiffIndex::parse("aaaa bbbb 1 p/a_b.gz\n").unwrap();
        assert!(resolve_chain(&idx, "aaaa", "zzzz").is_none());
    }

    #[test]
    fn resolve_chain_already_current() {
        let idx = PdiffIndex::parse("aaaa bbbb 1 p/a_b.gz\n").unwrap();
        assert_eq!(resolve_chain(&idx, "aaaa", "aaaa").unwrap().len(), 0);
    }

    #[test]
    fn apply_literal_only() {
        let delta = encode_rdiff_delta(&[RdiffOp::Literal(b"hello world")]);
        let out = apply_rdiff_delta(b"ignored basis", &delta).unwrap();
        assert_eq!(out, b"hello world");
    }

    #[test]
    fn apply_copy_only() {
        let basis = b"HelloWorld";
        // copy bytes 0..5 ("Hello")
        let delta = encode_rdiff_delta(&[RdiffOp::Copy(0, 5)]);
        let out = apply_rdiff_delta(basis, &delta).unwrap();
        assert_eq!(out, b"Hello");
    }

    #[test]
    fn apply_copy_and_literal() {
        let basis = b"The quick brown fox jumps over the lazy dog";
        // new = "The quick RED fox jumps over the lazy dog"
        let delta = encode_rdiff_delta(&[
            RdiffOp::Copy(0, 10),     // "The quick "
            RdiffOp::Literal(b"RED "), // "RED "
            RdiffOp::Copy(16, 27),    // "fox jumps over the lazy dog"
        ]);
        let out = apply_rdiff_delta(basis, &delta).unwrap();
        assert_eq!(out, b"The quick RED fox jumps over the lazy dog");
    }

    #[test]
    fn apply_rejects_bad_magic() {
        assert!(matches!(
            apply_rdiff_delta(b"x", b"not a delta"),
            Err(PdiffError::BadMagic)
        ));
    }

    #[test]
    fn encode_decode_round_trip() {
        let basis = b"abcdefghijklmnop";
        let delta = encode_rdiff_delta(&[RdiffOp::Copy(3, 5), RdiffOp::Literal(b"ZZ")]);
        // basis[3..8] = "defgh" + "ZZ"
        let out = apply_rdiff_delta(basis, &delta).unwrap();
        assert_eq!(out, b"defghZZ");
    }

    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[tokio::test]
    async fn fetch_pdiff_end_to_end() {
        let basis = b"the quick brown fox jumps over the lazy dog";
        let new = b"the quick RED fox jumps over the lazy dog";
        let current = sha256_hex(basis);
        let target = sha256_hex(new);

        let delta = encode_rdiff_delta(&[
            RdiffOp::Copy(0, 10),
            RdiffOp::Literal(b"RED "),
            RdiffOp::Copy(16, 27),
        ]);
        let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        std::io::Write::write_all(&mut gz, &delta).unwrap();
        let patch = gz.finish().unwrap();

        let index_text = format!(
            "{} {} {} Packages.diff/{}_{}.gz\n",
            current, target, patch.len(), current, target
        );

        let mock = MockServer::start().await;
        let base = mock.uri();
        Mock::given(method("GET"))
            .and(path("/dists/stable/main/binary-amd64/Packages.diff/Index"))
            .respond_with(ResponseTemplate::new(200).set_body_string(index_text))
            .mount(&mock)
            .await;
        Mock::given(method("GET"))
            .and(path(format!(
                "/dists/stable/main/binary-amd64/Packages.diff/{}_{}.gz",
                current, target
            )))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(patch))
            .mount(&mock)
            .await;

        let t = AptTransport::with_default_config().unwrap();
        let url = packages_diff_index_url(&base, "stable", "main", "amd64");
        let result = t
            .fetch_pdiff(
                &url,
                Some(&PdiffBasis {
                    sha256: &current,
                    bytes: basis,
                }),
                &target,
            )
            .await
            .unwrap();

        match result {
            PdiffUpdate::Reconstructed(bytes) => assert_eq!(bytes, new),
            PdiffUpdate::NoDelta => panic!("expected a reconstructed delta"),
        }
    }

    #[tokio::test]
    async fn fetch_pdiff_no_cache_returns_no_delta() {
        let mock = MockServer::start().await;
        // No mocks needed: with no current basis we short-circuit to NoDelta.
        let t = AptTransport::with_default_config().unwrap();
        let url = packages_diff_index_url(&mock.uri(), "stable", "main", "amd64");
        let result = t
            .fetch_pdiff(&url, None, "deadbeef")
            .await
            .unwrap();
        assert!(matches!(result, PdiffUpdate::NoDelta));
    }
}
