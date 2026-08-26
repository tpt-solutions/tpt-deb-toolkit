//! Debian `.deb` package format — ar archive reading and metadata extraction.
//!
//! A `.deb` file is an `ar(1)` archive with three members:
//! 1. `debian-binary` — version string `"2.0\n"`
//! 2. `control.tar.*` — compressed tar containing the `control` file
//! 3. `data.tar.*` — compressed tar containing the package payload
//!
//! Supported compression: `.gz` (gzip), `.xz`, `.zst`, and uncompressed.

use std::collections::HashMap;
use std::io::{Cursor, Read};
use std::path::{Path, PathBuf};

use thiserror::Error;

// ─── Error ────────────────────────────────────────────────────────────────────

/// Errors produced by this crate.
#[derive(Debug, Error)]
pub enum DebError {
    #[error("invalid .deb format: {0}")]
    InvalidFormat(String),
    #[error("control file parse error: {0}")]
    ControlParse(String),
    #[error("unsupported compression format: {0}")]
    UnsupportedCompression(String),
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
}

// ─── DebMetadata ─────────────────────────────────────────────────────────────

/// Metadata extracted from the `control` file inside `control.tar.*`.
#[derive(Debug, Clone, Default)]
pub struct DebMetadata {
    /// All control fields keyed by their canonical (case-sensitive) name.
    pub fields: HashMap<String, String>,
}

impl DebMetadata {
    /// Look up a control field (case-insensitive).
    pub fn get(&self, field: &str) -> Option<&str> {
        if let Some(v) = self.fields.get(field) {
            return Some(v.as_str());
        }
        let lower = field.to_lowercase();
        self.fields
            .iter()
            .find(|(k, _)| k.to_lowercase() == lower)
            .map(|(_, v)| v.as_str())
    }

    /// Returns the `Package` field.
    pub fn package_name(&self) -> Option<&str> {
        self.get("Package")
    }

    /// Returns the `Version` field.
    pub fn version(&self) -> Option<&str> {
        self.get("Version")
    }

    /// Returns the `Architecture` field.
    pub fn architecture(&self) -> Option<&str> {
        self.get("Architecture")
    }

    /// Iterate all fields in arbitrary order.
    pub fn iter(&self) -> impl Iterator<Item = (&str, &str)> {
        self.fields.iter().map(|(k, v)| (k.as_str(), v.as_str()))
    }
}

// ─── DataEntry ───────────────────────────────────────────────────────────────

/// A single entry in the `data.tar.*` payload.
#[derive(Debug, Clone)]
pub struct DataEntry {
    /// Path relative to the filesystem root (e.g. `usr/bin/curl`).
    pub path: PathBuf,
    /// Uncompressed size in bytes.
    pub size: u64,
    /// Unix permission bits.
    pub mode: u32,
}

// ─── DebFile ─────────────────────────────────────────────────────────────────

/// An opened `.deb` file with parsed metadata and data-entry list.
#[derive(Debug)]
pub struct DebFile {
    metadata: DebMetadata,
    entries: Vec<DataEntry>,
    /// Raw (still-compressed) bytes of the `data.tar.*` member, retained so
    /// that the payload can be extracted to disk or read back into memory.
    data_member: Option<(String, Vec<u8>)>,
}

impl DebFile {
    /// Open and parse a `.deb` file from the filesystem.
    pub fn open(path: &Path) -> Result<Self, DebError> {
        let raw = std::fs::read(path)?;
        Self::parse(&raw)
    }

    /// Parse a `.deb` from an in-memory byte slice.
    pub fn parse(data: &[u8]) -> Result<Self, DebError> {
        let mut cur = Cursor::new(data);

        // Verify ar magic
        let mut magic = [0u8; 8];
        cur.read_exact(&mut magic)?;
        if &magic != b"!<arch>\n" {
            return Err(DebError::InvalidFormat("not an ar archive".into()));
        }

        let mut control_member: Option<(String, Vec<u8>)> = None;
        let mut data_member: Option<(String, Vec<u8>)> = None;

        loop {
            let mut hdr = [0u8; 60];
            match cur.read_exact(&mut hdr) {
                Ok(_) => {}
                Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
                Err(e) => return Err(DebError::Io(e)),
            }

            let raw_name = std::str::from_utf8(&hdr[0..16])
                .map_err(|_| DebError::InvalidFormat("non-UTF-8 ar member name".into()))?;
            let name = raw_name.trim().trim_end_matches('/').to_string();

            let size_str = std::str::from_utf8(&hdr[48..58])
                .map_err(|_| DebError::InvalidFormat("non-UTF-8 ar size".into()))?
                .trim();
            let size: u64 = size_str.parse().map_err(|_| {
                DebError::InvalidFormat(format!("invalid ar member size {size_str:?}"))
            })?;

            if &hdr[58..60] != b"`\n" {
                return Err(DebError::InvalidFormat("bad ar header terminator".into()));
            }

            let mut body = vec![0u8; size as usize];
            cur.read_exact(&mut body)?;
            if !size.is_multiple_of(2) {
                let _ = cur.read_exact(&mut [0u8; 1]);
            }

            if name == "debian-binary" {
                let txt = std::str::from_utf8(&body).unwrap_or("").trim_end();
                if !txt.starts_with("2.0") {
                    return Err(DebError::InvalidFormat(format!(
                        "unsupported deb format version: {txt:?}"
                    )));
                }
            } else if let Some(ext) = name.strip_prefix("control.tar") {
                control_member = Some((ext.to_string(), body));
            } else if let Some(ext) = name.strip_prefix("data.tar") {
                data_member = Some((ext.to_string(), body));
            }
        }

        let (ctrl_ext, ctrl_data) = control_member
            .ok_or_else(|| DebError::InvalidFormat("missing control.tar member".into()))?;
        let metadata = parse_control_tar(&ctrl_data, &ctrl_ext)?;

        let entries = match &data_member {
            Some((ext, d)) => parse_data_tar(d, ext)?,
            None => vec![],
        };

        Ok(Self {
            metadata,
            entries,
            data_member,
        })
    }

    /// Returns the parsed control metadata.
    pub fn metadata(&self) -> &DebMetadata {
        &self.metadata
    }

    /// Returns the list of payload entries from `data.tar.*`.
    pub fn entries(&self) -> &[DataEntry] {
        &self.entries
    }

    /// Read the full contents of the `data.tar.*` payload into memory.
    ///
    /// Returns a map from each file path (relative to the filesystem root) to
    /// its uncompressed content. Directory entries are omitted.
    pub fn data_contents(&self) -> Result<HashMap<PathBuf, Vec<u8>>, DebError> {
        let (ext, bytes) = self
            .data_member
            .as_ref()
            .ok_or_else(|| DebError::InvalidFormat("missing data.tar member".into()))?;
        read_data_tar_contents(bytes, ext)
    }

    /// Extract the `data.tar.*` payload into `dest` on the filesystem.
    ///
    /// Entries are written preserving their relative path and Unix permission
    /// bits. Directory entries are created as needed.
    pub fn extract(&self, dest: &Path) -> Result<(), DebError> {
        let (ext, bytes) = self
            .data_member
            .as_ref()
            .ok_or_else(|| DebError::InvalidFormat("missing data.tar member".into()))?;
        extract_data_tar(bytes, ext, dest)
    }
}

/// Read the `data.tar.*` payload from raw bytes into a path→content map.
fn read_data_tar_contents(
    data: &[u8],
    ext: &str,
) -> Result<HashMap<PathBuf, Vec<u8>>, DebError> {
    let tar_bytes = decompress_member(data, ext)?;
    let mut archive = tar::Archive::new(Cursor::new(tar_bytes));
    let mut out = HashMap::new();
    for entry in archive.entries()? {
        let mut entry = entry?;
        let path = entry.path()?.into_owned();
        if entry.header().entry_type() == tar::EntryType::Regular {
            let mut content = Vec::new();
            entry.read_to_end(&mut content)?;
            out.insert(path, content);
        }
    }
    Ok(out)
}

/// Extract the `data.tar.*` payload from raw bytes into `dest`.
fn extract_data_tar(data: &[u8], ext: &str, dest: &Path) -> Result<(), DebError> {
    let tar_bytes = decompress_member(data, ext)?;
    let mut archive = tar::Archive::new(Cursor::new(tar_bytes));
    for entry in archive.entries()? {
        let mut entry = entry?;
        let path = entry.path()?.into_owned();
        let mode = entry.header().mode().unwrap_or(0o644);
        let target = dest.join(&path);
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent)?;
        }
        if entry.header().entry_type() == tar::EntryType::Regular {
            let mut f = std::fs::File::create(&target)?;
            std::io::copy(&mut entry, &mut f)?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(&target, std::fs::Permissions::from_mode(mode))?;
            }
            #[cfg(not(unix))]
            let _ = mode;
        } else if entry.header().entry_type() == tar::EntryType::Symlink {
            if let Some(link) = entry.link_name()? {
                let _ = std::fs::remove_file(&target);
                std::os::unix::fs::symlink(link.into_owned(), &target)?;
            }
        }
    }
    Ok(())
}

// ─── Internal helpers ────────────────────────────────────────────────────────

fn decompress_member(data: &[u8], ext: &str) -> Result<Vec<u8>, DebError> {
    match ext {
        ".gz" => {
            let mut dec = flate2::read::GzDecoder::new(data);
            let mut out = Vec::new();
            dec.read_to_end(&mut out)?;
            Ok(out)
        }
        ".xz" => {
            let mut dec = xz2::read::XzDecoder::new(data);
            let mut out = Vec::new();
            dec.read_to_end(&mut out)?;
            Ok(out)
        }
        ".zst" => {
            let mut dec = zstd::stream::read::Decoder::new(data)?;
            let mut out = Vec::new();
            dec.read_to_end(&mut out)?;
            Ok(out)
        }
        "" => Ok(data.to_vec()),
        other => Err(DebError::UnsupportedCompression(other.to_string())),
    }
}

fn parse_control_tar(data: &[u8], ext: &str) -> Result<DebMetadata, DebError> {
    let tar_bytes = decompress_member(data, ext)?;
    let mut archive = tar::Archive::new(Cursor::new(tar_bytes));

    for entry in archive.entries()? {
        let mut entry = entry?;
        let path = entry.path()?.into_owned();
        let fname = path.file_name().and_then(|n| n.to_str()).unwrap_or("");

        if fname == "control" {
            let mut content = String::new();
            entry.read_to_string(&mut content)?;
            let paras = tpt_l_control_file::parse_control(&content);
            if let Some(para) = paras.into_iter().next() {
                let fields: HashMap<String, String> = para
                    .fields()
                    .map(|(k, v)| (k.to_string(), v.to_string()))
                    .collect();
                return Ok(DebMetadata { fields });
            }
        }
    }

    Err(DebError::InvalidFormat(
        "no 'control' file in control.tar".into(),
    ))
}

fn parse_data_tar(data: &[u8], ext: &str) -> Result<Vec<DataEntry>, DebError> {
    let tar_bytes = decompress_member(data, ext)?;
    let mut archive = tar::Archive::new(Cursor::new(tar_bytes));
    let mut entries = Vec::new();

    for entry in archive.entries()? {
        let entry = entry?;
        let path = entry.path()?.into_owned();
        let size = entry.size();
        let mode = entry.header().mode()?;
        entries.push(DataEntry { path, size, mode });
    }
    Ok(entries)
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn metadata_case_insensitive_get() {
        let mut fields = HashMap::new();
        fields.insert("Package".to_string(), "curl".to_string());
        let meta = DebMetadata { fields };
        assert_eq!(meta.get("package"), Some("curl"));
        assert_eq!(meta.package_name(), Some("curl"));
    }

    #[test]
    fn parse_bad_magic_errors() {
        let result = DebFile::parse(b"NOT_AN_AR");
        assert!(matches!(result, Err(DebError::InvalidFormat(_))));
    }

    #[test]
    fn data_entry_construction() {
        let e = DataEntry {
            path: PathBuf::from("usr/bin/curl"),
            size: 9999,
            mode: 0o755,
        };
        assert_eq!(e.size, 9999);
    }
}
