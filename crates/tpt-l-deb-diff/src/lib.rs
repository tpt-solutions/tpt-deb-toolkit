//! Structural and content diffing of two Debian `.deb` packages.
//!
//! `tpt-l-deb-diff` compares two `.deb` files on three axes:
//!
//! 1. **Metadata** — control fields from `control.tar`'s `control` file.
//! 2. **File tree** — paths present, added, removed, or modified.
//! 3. **Checksums** — file content digests to flag modified files.
//!
//! The result is a serializable [`DiffReport`] that can be rendered as a
//! human-readable summary or emitted as JSON.
//!
//! # Example
//!
//! ```
//! use tpt_l_deb_diff::DebDiff;
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let a = std::fs::read("fixtures/old.deb").unwrap_or_default();
//! let b = std::fs::read("fixtures/new.deb").unwrap_or_default();
//! let report = DebDiff::compare(&a, &b)?;
//! println!("{}", report);
//! # Ok(())
//! # }
//! ```

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tpt_l_deb_format::{DebError, DebFile, DebMetadata};
use tpt_l_deb_format::DebFile as _;

use thiserror::Error;

/// Errors produced while diffing two `.deb` files.
#[derive(Debug, Error)]
pub enum DebDiffError {
    /// One of the inputs could not be parsed as a `.deb`.
    #[error("deb parse error: {0}")]
    Deb(#[from] DebError),
    /// A SHA-256 computation failed (should not happen in practice).
    #[error("checksum error: {0}")]
    Checksum(String),
}

/// One control-field change between the two packages.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MetaChange {
    /// The control field name (as it appears in the newer package).
    pub field: String,
    /// The value in the old package, or `None` if the field was absent.
    pub old: Option<String>,
    /// The value in the new package, or `None` if the field was removed.
    pub new: Option<String>,
}

/// A file whose content changed between the two packages.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileChange {
    /// Path of the file (relative to the filesystem root).
    pub path: PathBuf,
    /// SHA-256 of the old content, or `None` if the file was added.
    pub old_sha256: Option<String>,
    /// SHA-256 of the new content, or `None` if the file was removed.
    pub new_sha256: Option<String>,
}

/// The complete result of diffing two `.deb` packages.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiffReport {
    /// Control-field changes (only fields whose value differs are listed).
    pub metadata: Vec<MetaChange>,
    /// Files present in the new package but not the old one.
    pub files_added: Vec<PathBuf>,
    /// Files present in the old package but not the new one.
    pub files_removed: Vec<PathBuf>,
    /// Files present in both packages whose content differs.
    pub files_modified: Vec<FileChange>,
}

impl DiffReport {
    /// Returns `true` when the two packages are identical (no differences).
    pub fn is_empty(&self) -> bool {
        self.metadata.is_empty()
            && self.files_added.is_empty()
            && self.files_removed.is_empty()
            && self.files_modified.is_empty()
    }

    /// Total number of changes across all categories.
    pub fn change_count(&self) -> usize {
        self.metadata.len() + self.files_added.len() + self.files_removed.len()
            + self.files_modified.len()
    }
}

fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    let digest = hasher.finalize();
    format!("{:x}", digest)
}

/// Internal parsed view of a single `.deb`.
struct DebContents {
    metadata: DebMetadata,
    files: HashMap<PathBuf, Vec<u8>>,
}

fn read_contents(deb: &[u8]) -> Result<DebContents, DebDiffError> {
    let parsed = DebFile::parse(deb)?;
    let files = parsed.data_contents()?;
    Ok(DebContents {
        metadata: parsed.metadata().clone(),
        files,
    })
}

/// Compares two `.deb` packages.
pub struct DebDiff;

impl DebDiff {
    /// Diff two `.deb` byte blobs and return a structured [`DiffReport`].
    ///
    /// # Errors
    ///
    /// Returns [`DebDiffError::Deb`] if either input is not a valid `.deb`.
    pub fn compare(old: &[u8], new: &[u8]) -> Result<DiffReport, DebDiffError> {
        let old_c = read_contents(old)?;
        let new_c = read_contents(new)?;

        let mut report = DiffReport::default();

        // Metadata diff
        let mut all_fields: Vec<String> = old_c
            .metadata
            .fields
            .keys()
            .chain(new_c.metadata.fields.keys())
            .cloned()
            .collect();
        all_fields.sort();
        all_fields.dedup();
        for field in all_fields {
            let o = old_c.metadata.fields.get(&field).cloned();
            let n = new_c.metadata.fields.get(&field).cloned();
            if o != n {
                report.metadata.push(MetaChange {
                    field,
                    old: o,
                    new: n,
                });
            }
        }

        // File tree diff
        let mut old_paths: Vec<&PathBuf> = old_c.files.keys().cloned().collect();
        old_paths.sort();
        let mut new_paths: Vec<&PathBuf> = new_c.files.keys().cloned().collect();
        new_paths.sort();

        for p in &new_paths {
            if !old_c.files.contains_key(*p) {
                report.files_added.push((*p).clone());
            }
        }
        for p in &old_paths {
            if !new_c.files.contains_key(*p) {
                report.files_removed.push((*p).clone());
            }
        }
        for p in &old_paths {
            if let (Some(o), Some(n)) = (old_c.files.get(*p), new_c.files.get(*p)) {
                if sha256_hex(o) != sha256_hex(n) {
                    report.files_modified.push(FileChange {
                        path: (*p).clone(),
                        old_sha256: Some(sha256_hex(o)),
                        new_sha256: Some(sha256_hex(n)),
                    });
                }
            }
        }

        Ok(report)
    }

    /// Diff two `.deb` files on disk.
    ///
    /// # Errors
    ///
    /// Returns [`DebDiffError::Deb`] if either file cannot be read or parsed.
    pub fn compare_files(old: &Path, new: &Path) -> Result<DiffReport, DebDiffError> {
        let old_bytes = std::fs::read(old).map_err(DebError::Io)?;
        let new_bytes = std::fs::read(new).map_err(DebError::Io)?;
        Self::compare(&old_bytes, &new_bytes)
    }
}

impl std::fmt::Display for DiffReport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.is_empty() {
            return writeln!(f, "No differences found.");
        }

        if !self.metadata.is_empty() {
            writeln!(f, "Metadata changes:")?;
            for m in &self.metadata {
                match (&m.old, &m.new) {
                    (Some(o), Some(n)) => writeln!(f, "  {}: {!r} -> {!r}", m.field, o, n)?,
                    (Some(o), None) => writeln!(f, "  {}: {!r} -> (removed)", m.field, o)?,
                    (None, Some(n)) => writeln!(f, "  {}: (added) -> {!r}", m.field, n)?,
                    (None, None) => {}
                }
            }
        }

        if !self.files_added.is_empty() {
            writeln!(f, "Added files ({}):", self.files_added.len())?;
            for p in &self.files_added {
                writeln!(f, "  + {}", p.display())?;
            }
        }

        if !self.files_removed.is_empty() {
            writeln!(f, "Removed files ({}):", self.files_removed.len())?;
            for p in &self.files_removed {
                writeln!(f, "  - {}", p.display())?;
            }
        }

        if !self.files_modified.is_empty() {
            writeln!(f, "Modified files ({}):", self.files_modified.len())?;
            for p in &self.files_modified {
                writeln!(f, "  * {}", p.path.display())?;
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write as _;
    use tar::{Builder, Header};

    /// Build a minimal `.deb` in memory from a control string and a set of
    /// data files (path -> contents).
    fn build_deb(control: &str, files: &[(&str, &[u8])]) -> Vec<u8> {
        let mut buf: Vec<u8> = Vec::new();

        // debian-binary
        write_ar_member(&mut buf, "debian-binary", b"2.0\n");

        // control.tar.gz
        let mut ctrl_tar: Vec<u8> = Vec::new();
        {
            let mut builder = Builder::new(&mut ctrl_tar);
            let mut header = Header::new_gnu();
            let data = control.as_bytes();
            header.set_size(data.len() as u64);
            header.set_mode(0o644);
            header.set_cksum();
            builder
                .append_data(&mut header, "control", data)
                .unwrap();
            builder.finish().unwrap();
        }
        write_ar_member(&mut buf, "control.tar.gz", &ctrl_tar);

        // data.tar.gz
        let mut data_tar: Vec<u8> = Vec::new();
        {
            let mut builder = Builder::new(&mut data_tar);
            for (path, content) in files {
                let mut header = Header::new_gnu();
                header.set_size(content.len() as u64);
                header.set_mode(0o644);
                header.set_cksum();
                builder
                    .append_data(&mut header, path, *content)
                    .unwrap();
            }
            builder.finish().unwrap();
        }
        write_ar_member(&mut buf, "data.tar.gz", &data_tar);

        buf
    }

    fn write_ar_member(out: &mut Vec<u8>, name: &str, body: &[u8]) {
        let mut padded_name = name.to_string();
        while padded_name.len() < 16 {
            padded_name.push(' ');
        }
        let size = format!("{:<10}", body.len());
        let header = format!(
            "{}{}0         0     0     100644     {}\n`\n",
            padded_name, size, size
        );
        out.extend_from_slice(header.as_bytes());
        out.extend_from_slice(body);
        if body.len() % 2 != 0 {
            out.push(b'\n');
        }
    }

    const CONTROL_A: &str = "Package: foo\nVersion: 1.0-1\nArchitecture: amd64\nDescription: test\n";
    const CONTROL_B: &str = "Package: foo\nVersion: 1.0-2\nArchitecture: amd64\nDescription: test\n";

    #[test]
    fn identical_debs_produce_empty_diff() {
        let a = build_deb(CONTROL_A, &[("usr/bin/foo", b"hello")]);
        let b = build_deb(CONTROL_A, &[("usr/bin/foo", b"hello")]);
        let report = DebDiff::compare(&a, &b).unwrap();
        assert!(report.is_empty());
        assert_eq!(report.change_count(), 0);
    }

    #[test]
    fn version_change_detected_in_metadata() {
        let a = build_deb(CONTROL_A, &[("usr/bin/foo", b"hello")]);
        let b = build_deb(CONTROL_B, &[("usr/bin/foo", b"hello")]);
        let report = DebDiff::compare(&a, &b).unwrap();
        assert_eq!(report.metadata.len(), 1);
        assert_eq!(report.metadata[0].field, "Version");
        assert_eq!(report.metadata[0].old.as_deref(), Some("1.0-1"));
        assert_eq!(report.metadata[0].new.as_deref(), Some("1.0-2"));
    }

    #[test]
    fn added_removed_and_modified_files() {
        let a = build_deb(
            CONTROL_A,
            &[
                ("usr/bin/foo", b"hello"),
                ("usr/bin/old", b"gone"),
            ],
        );
        let b = build_deb(
            CONTROL_A,
            &[
                ("usr/bin/foo", b"hello world"),
                ("usr/bin/new", b"fresh"),
            ],
        );
        let report = DebDiff::compare(&a, &b).unwrap();
        assert_eq!(report.files_added, vec![PathBuf::from("usr/bin/new")]);
        assert_eq!(report.files_removed, vec![PathBuf::from("usr/bin/old")]);
        assert_eq!(report.files_modified.len(), 1);
        assert_eq!(report.files_modified[0].path, PathBuf::from("usr/bin/foo"));
    }

    #[test]
    fn invalid_input_errors() {
        let bad = b"not a deb";
        let good = build_deb(CONTROL_A, &[]);
        assert!(DebDiff::compare(bad, &good).is_err());
    }

    #[test]
    fn report_is_serializable() {
        let a = build_deb(CONTROL_A, &[("usr/bin/foo", b"a")]);
        let b = build_deb(CONTROL_B, &[("usr/bin/foo", b"b")]);
        let report = DebDiff::compare(&a, &b).unwrap();
        let json = serde_json::to_string(&report).unwrap();
        let _ = json;
    }
}
