//! Debian `.deb` package format — ar archive reading and metadata extraction.
//!
//! A `.deb` file is an `ar(1)` archive with three members:
//! 1. `debian-binary` — version string `"2.0\n"`
//! 2. `control.tar.*` — compressed tar containing the `control` file
//! 3. `data.tar.*` — compressed tar containing the package payload
//!
//! Supported compression: `.gz` (gzip), `.xz`, `.zst`, and uncompressed.
//!
//! # Streaming
//!
//! [`ArReader`] reads the archive lazily — no member is buffered in full
//! unless you ask for it. [`DebFile::open`] memory-maps the on-disk file
//! (via `memmap2`) so even large `.deb`s are not copied into a Rust `Vec`
//! up front. The [`DataEntries`]/[`ControlEntries`] types expose the
//! tar payloads as lazy, stream-decompressing iterators suitable for
//! incremental extraction.

use std::collections::HashMap;
use std::io::{self, Cursor, Read};
use std::path::{Path, PathBuf};
use std::sync::Arc;

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

// ─── ArReader (streaming ar archive) ─────────────────────────────────────────

enum ReadOutcome {
    Ok,
    Eof,
    Err(io::Error),
}

fn read_exact_eof<R: Read + ?Sized>(r: &mut R, buf: &mut [u8]) -> ReadOutcome {
    match r.read_exact(buf) {
        Ok(()) => ReadOutcome::Ok,
        Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => ReadOutcome::Eof,
        Err(e) => ReadOutcome::Err(e),
    }
}

/// A streaming reader over a Unix `ar(1)` archive.
///
/// Unlike slurping the whole file into memory, [`ArReader`] reads one member
/// header at a time and hands back a [`Read`] handle for each member body.
/// Buffering the body is entirely up to the caller; seeking is never required.
///
/// ```
/// use std::io::Read;
/// use tpt_l_deb_format::ArReader;
///
/// # let bytes = tpt_l_deb_format::testsupport::synthetic_deb();
/// let mut reader = ArReader::new(std::io::Cursor::new(&bytes[..])).unwrap();
/// while let Some(mut entry) = reader.next_entry().unwrap() {
///     let mut body = Vec::new();
///     entry.read_to_end(&mut body).unwrap();
///     println!("member {} ({} bytes)", entry.name(), body.len());
/// }
/// ```
pub struct ArReader<R: Read> {
    reader: R,
}

impl<R: Read> ArReader<R> {
    /// Verify the ar magic and prepare to read members from `reader`.
    pub fn new(mut reader: R) -> Result<Self, DebError> {
        let mut magic = [0u8; 8];
        match read_exact_eof(&mut reader, &mut magic) {
            ReadOutcome::Ok => {}
            ReadOutcome::Eof => {
                return Err(DebError::InvalidFormat(
                    "empty archive (no ar magic)".into(),
                ))
            }
            ReadOutcome::Err(e) => return Err(DebError::Io(e)),
        }
        if &magic != b"!<arch>\n" {
            return Err(DebError::InvalidFormat("not an ar archive".into()));
        }
        Ok(Self { reader })
    }

    /// Return the next archive member, or `Ok(None)` at end of archive.
    ///
    /// The returned [`ArEntry`] borrows this reader for its lifetime. Any
    /// unread portion of the member body (and the required padding byte) is
    /// skipped automatically when the entry is dropped, so the next call to
    /// [`next_entry`](ArReader::next_entry) begins cleanly on the next header.
    pub fn next_entry(&mut self) -> Result<Option<ArEntry<'_, R>>, DebError> {
        let mut hdr = [0u8; 60];
        match read_exact_eof(&mut self.reader, &mut hdr) {
            ReadOutcome::Ok => {}
            ReadOutcome::Eof => return Ok(None),
            ReadOutcome::Err(e) => return Err(DebError::Io(e)),
        }

        if &hdr[58..60] != b"`\n" {
            return Err(DebError::InvalidFormat("bad ar header terminator".into()));
        }

        let raw_name = std::str::from_utf8(&hdr[0..16])
            .map_err(|_| DebError::InvalidFormat("non-UTF-8 ar member name".into()))?;
        let name = raw_name.trim().trim_end_matches('/').to_string();

        let size_str = std::str::from_utf8(&hdr[48..58])
            .map_err(|_| DebError::InvalidFormat("non-UTF-8 ar size".into()))?
            .trim();
        let size: u64 = size_str
            .parse()
            .map_err(|_| DebError::InvalidFormat(format!("invalid ar member size {size_str:?}")))?;

        let inner = (&mut self.reader).take(size);
        Ok(Some(ArEntry { name, size, inner }))
    }
}

/// A single member of an [`ArReader`] archive.
///
/// Implements [`Read`]; reading from it consumes at most `size()` bytes of
/// the underlying archive body.
pub struct ArEntry<'a, R: Read> {
    name: String,
    size: u64,
    inner: io::Take<&'a mut R>,
}

impl<'a, R: Read> ArEntry<'a, R> {
    /// Member name (trailing `/` and surrounding whitespace stripped).
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Declared body size in bytes.
    pub fn size(&self) -> u64 {
        self.size
    }

    /// Access the body reader directly (equivalent to `&mut self`).
    pub fn data(&mut self) -> &mut io::Take<&'a mut R> {
        &mut self.inner
    }
}

impl<'a, R: Read> Read for ArEntry<'a, R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.inner.read(buf)
    }
}

impl<'a, R: Read> Drop for ArEntry<'a, R> {
    fn drop(&mut self) {
        // Skip any unread body bytes plus the single padding byte that ar
        // inserts after every odd-length member, so the archive stays aligned.
        let remaining = self.inner.limit();
        let to_skip = remaining + (self.size % 2);
        if to_skip > 0 {
            let underlying = self.inner.get_mut();
            let mut skip = (&mut **underlying).take(to_skip);
            let _ = io::copy(&mut skip, &mut io::sink());
        }
    }
}

// ─── DebFile ─────────────────────────────────────────────────────────────────

/// An opened `.deb` file with parsed metadata and data-entry list.
#[derive(Debug)]
pub struct DebFile {
    metadata: DebMetadata,
    entries: Vec<DataEntry>,
    /// Raw (still-compressed) bytes of the `control.tar.*` member, retained so
    /// the control payload can be streamed out on demand.
    control_member: Option<(String, Arc<Vec<u8>>)>,
    /// Raw (still-compressed) bytes of the `data.tar.*` member, retained so
    /// that the payload can be extracted to disk or read back into memory.
    data_member: Option<(String, Arc<Vec<u8>>)>,
}

impl DebFile {
    /// Open and parse a `.deb` file from the filesystem.
    ///
    /// The file is memory-mapped (via `memmap2`) rather than copied into a
    /// `Vec`, so opening a large package does not require a full read into RAM.
    pub fn open(path: &Path) -> Result<Self, DebError> {
        let file = std::fs::File::open(path)?;
        // SAFETY: we only read the mapping; the underlying file is not mutated
        // by this process, so mmap visibility is stable for the mapping's life.
        let mmap = unsafe { memmap2::Mmap::map(&file)? };
        Self::parse(&mmap)
    }

    /// Parse a `.deb` from an in-memory byte slice.
    pub fn parse(data: &[u8]) -> Result<Self, DebError> {
        let mut reader = ArReader::new(Cursor::new(data))?;

        let mut metadata: Option<DebMetadata> = None;
        let mut control_member: Option<(String, Arc<Vec<u8>>)> = None;
        let mut data_member: Option<(String, Arc<Vec<u8>>)> = None;

        while let Some(mut entry) = reader.next_entry()? {
            let name = entry.name().to_string();
            let size = entry.size();
            let mut body = Vec::with_capacity(size as usize);
            entry.read_to_end(&mut body)?;
            // `entry` is dropped here, which skips the ar padding byte.

            if name == "debian-binary" {
                let txt = std::str::from_utf8(&body).unwrap_or("").trim_end();
                if !txt.starts_with("2.0") {
                    return Err(DebError::InvalidFormat(format!(
                        "unsupported deb format version: {txt:?}"
                    )));
                }
            } else if let Some(ext) = name.strip_prefix("control.tar") {
                let ext = ext.to_string();
                let meta = parse_control_tar(&body, &ext)?;
                metadata = Some(meta);
                control_member = Some((ext, Arc::new(body)));
            } else if let Some(ext) = name.strip_prefix("data.tar") {
                data_member = Some((ext.to_string(), Arc::new(body)));
            }
        }

        let metadata =
            metadata.ok_or_else(|| DebError::InvalidFormat("missing control.tar member".into()))?;
        finish(metadata, control_member, data_member)
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
        read_data_tar_contents(&bytes[..], ext)
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
        extract_data_tar(&bytes[..], ext, dest)
    }

    /// Stream the `data.tar.*` payload as a lazy, decompressing iterator.
    ///
    /// Each yielded [`tar::Entry`] is produced on demand; the tar is never
    /// fully materialised in memory. Use this for incremental extraction of
    /// large packages.
    pub fn data_entries(&self) -> Result<DataEntries, DebError> {
        let (ext, bytes) = self
            .data_member
            .as_ref()
            .ok_or_else(|| DebError::InvalidFormat("missing data.tar member".into()))?;
        Ok(DataEntries {
            archive: build_tar_archive(&bytes[..], ext)?,
        })
    }

    /// Stream the `control.tar.*` payload as a lazy, decompressing iterator.
    pub fn control_entries(&self) -> Result<ControlEntries, DebError> {
        let (ext, bytes) = self
            .control_member
            .as_ref()
            .ok_or_else(|| DebError::InvalidFormat("missing control.tar member".into()))?;
        Ok(ControlEntries {
            archive: build_tar_archive(&bytes[..], ext)?,
        })
    }
}

/// Finish constructing a [`DebFile`] from the parsed control metadata and the
/// retained compressed members.
fn finish(
    metadata: DebMetadata,
    control_member: Option<(String, Arc<Vec<u8>>)>,
    data_member: Option<(String, Arc<Vec<u8>>)>,
) -> Result<DebFile, DebError> {
    let entries = match &data_member {
        Some((ext, d)) => parse_data_tar(&d[..], ext)?,
        None => vec![],
    };
    Ok(DebFile {
        metadata,
        entries,
        control_member,
        data_member,
    })
}

/// A lazy, stream-decompressing view over a `.deb`'s `data.tar.*` payload.
pub struct DataEntries {
    archive: tar::Archive<Box<dyn Read>>,
}

impl DataEntries {
    /// Iterate the tar entries (header + body) lazily.
    pub fn entries(&mut self) -> io::Result<tar::Entries<'_, Box<dyn Read>>> {
        self.archive.entries()
    }
}

/// A lazy, stream-decompressing view over a `.deb`'s `control.tar.*` payload.
pub struct ControlEntries {
    archive: tar::Archive<Box<dyn Read>>,
}

impl ControlEntries {
    /// Iterate the tar entries (header + body) lazily.
    pub fn entries(&mut self) -> io::Result<tar::Entries<'_, Box<dyn Read>>> {
        self.archive.entries()
    }
}

/// Build a [`tar::Archive`] that decodes (and decompresses) `bytes` on the fly.
fn build_tar_archive(bytes: &[u8], ext: &str) -> Result<tar::Archive<Box<dyn Read>>, DebError> {
    let decoder: Box<dyn Read> = match ext {
        ".gz" => Box::new(flate2::read::GzDecoder::new(Cursor::new(bytes.to_vec()))),
        ".xz" => Box::new(xz2::read::XzDecoder::new(Cursor::new(bytes.to_vec()))),
        ".zst" => Box::new(zstd::stream::read::Decoder::new(Cursor::new(
            bytes.to_vec(),
        ))?),
        "" => Box::new(Cursor::new(bytes.to_vec())),
        other => return Err(DebError::UnsupportedCompression(other.to_string())),
    };
    Ok(tar::Archive::new(decoder))
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

/// Read the `data.tar.*` payload from raw bytes into a path→content map.
fn read_data_tar_contents(data: &[u8], ext: &str) -> Result<HashMap<PathBuf, Vec<u8>>, DebError> {
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
            #[cfg(unix)]
            {
                if let Some(link) = entry.link_name()? {
                    let _ = std::fs::remove_file(&target);
                    std::os::unix::fs::symlink(link.into_owned(), &target)?;
                }
            }
        }
    }
    Ok(())
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

    #[test]
    fn round_trip_open_and_parse() {
        let bytes = testsupport::synthetic_deb();
        let deb = DebFile::parse(&bytes).unwrap();
        assert_eq!(deb.metadata().package_name(), Some("foo"));
        assert_eq!(deb.metadata().version(), Some("1.0-1"));
        assert_eq!(deb.metadata().architecture(), Some("amd64"));
        let names: Vec<String> = deb
            .entries()
            .iter()
            .map(|e| e.path.to_string_lossy().into_owned())
            .collect();
        assert!(names.contains(&"usr/bin/foo".to_string()));
        assert!(names.contains(&"usr/share/doc/foo/README".to_string()));
    }

    #[test]
    fn round_trip_extract_to_disk() {
        let bytes = testsupport::synthetic_deb();
        let deb = DebFile::parse(&bytes).unwrap();
        let dir = tempfile::tempdir().unwrap();
        deb.extract(dir.path()).unwrap();
        let readme = dir.path().join("usr/share/doc/foo/README");
        assert!(readme.exists());
        let content = std::fs::read_to_string(readme).unwrap();
        assert_eq!(content, "hello\n");
        let bin = dir.path().join("usr/bin/foo");
        assert!(bin.exists());
    }

    #[test]
    fn round_trip_streaming_data_entries() {
        let bytes = testsupport::synthetic_deb();
        let deb = DebFile::parse(&bytes).unwrap();
        let mut ents = deb.data_entries().unwrap();
        let names: Vec<PathBuf> = ents
            .entries()
            .unwrap()
            .map(|e| e.unwrap().path().unwrap().into_owned())
            .collect();
        assert!(names.iter().any(|p| p.ends_with("usr/bin/foo")));
    }

    #[test]
    fn round_trip_streaming_control_entries() {
        let bytes = testsupport::synthetic_deb();
        let deb = DebFile::parse(&bytes).unwrap();
        let mut ents = deb.control_entries().unwrap();
        let names: Vec<PathBuf> = ents
            .entries()
            .unwrap()
            .map(|e| e.unwrap().path().unwrap().into_owned())
            .collect();
        assert!(names.iter().any(|p| p.ends_with("control")));
    }

    #[test]
    fn ar_reader_streams_members() {
        let bytes = testsupport::synthetic_deb();
        let mut reader = ArReader::new(Cursor::new(&bytes[..])).unwrap();
        let mut seen = Vec::new();
        while let Some(mut entry) = reader.next_entry().unwrap() {
            let name = entry.name().to_string();
            let size = entry.size();
            let mut body = Vec::new();
            entry.read_to_end(&mut body).unwrap();
            assert_eq!(body.len() as u64, size);
            seen.push(name);
        }
        assert_eq!(
            seen,
            vec![
                "debian-binary".to_string(),
                "control.tar.gz".to_string(),
                "data.tar.gz".to_string()
            ]
        );
    }

    #[test]
    fn ar_reader_partial_read_then_skip() {
        let bytes = testsupport::synthetic_deb();
        let mut reader = ArReader::new(Cursor::new(&bytes[..])).unwrap();
        // Read only one byte of the first member, then drop it; the reader
        // must still align to the next member header.
        let mut first = reader.next_entry().unwrap().unwrap();
        let mut one = [0u8; 1];
        assert_eq!(first.read(&mut one).unwrap(), 1);
        drop(first);
        let second = reader.next_entry().unwrap().unwrap();
        assert_eq!(second.name(), "control.tar.gz");
    }
}

/// Test helper for building a small, valid `.deb` entirely in memory so the
/// round-trip tests need no external fixtures.
#[doc(hidden)]
pub mod testsupport {
    /// Build a minimal `.deb`: `debian-binary`, `control.tar.gz`, `data.tar.gz`.
    pub fn synthetic_deb() -> Vec<u8> {
        let control_content = b"Package: foo\nVersion: 1.0-1\nArchitecture: amd64\nMaintainer: TPT <t@e.com>\nDescription: test package\n";
        let control_tar = make_tar_gz(&[("control", control_content, 0o644)]);
        let data_tar = make_tar_gz(&[
            ("usr/bin/foo", b"#!/bin/sh\necho hi\n", 0o755),
            ("usr/share/doc/foo/README", b"hello\n", 0o644),
        ]);

        let mut out = Vec::new();
        out.extend_from_slice(b"!<arch>\n");
        out.extend_from_slice(&ar_member("debian-binary", b"2.0\n"));
        out.extend_from_slice(&ar_member("control.tar.gz", &control_tar));
        out.extend_from_slice(&ar_member("data.tar.gz", &data_tar));
        out
    }

    fn make_tar_gz(files: &[(&str, &[u8], u32)]) -> Vec<u8> {
        let mut buf = Vec::new();
        {
            let enc = flate2::write::GzEncoder::new(&mut buf, flate2::Compression::default());
            let mut builder = tar::Builder::new(enc);
            for (path, content, mode) in files {
                let mut header = tar::Header::new_gnu();
                header.set_size(content.len() as u64);
                header.set_mode(*mode);
                header.set_cksum();
                builder.append_data(&mut header, path, *content).unwrap();
            }
            builder.finish().unwrap();
        }
        buf
    }

    fn ar_member(name: &str, body: &[u8]) -> Vec<u8> {
        let mut m = Vec::new();
        m.extend_from_slice(&ar_header(name, body.len() as u64));
        m.extend_from_slice(body);
        if !body.len().is_multiple_of(2) {
            m.push(b'\n');
        }
        m
    }

    fn ar_header(name: &str, size: u64) -> [u8; 60] {
        let mut h = [0u8; 60];
        let mut nf = String::new();
        nf.push_str(name);
        nf.push('/');
        while nf.len() < 16 {
            nf.push(' ');
        }
        h[0..16].copy_from_slice(nf.as_bytes());
        write_field(&mut h[16..28], b"0"); // mtime
        write_field(&mut h[28..34], b"0"); // uid
        write_field(&mut h[34..40], b"0"); // gid
        h[40..48].copy_from_slice(b"00000644"); // mode (octal, 8 wide)
        let s = format!("{:<10}", size);
        h[48..58].copy_from_slice(s.as_bytes());
        h[58..60].copy_from_slice(b"`\n");
        h
    }

    fn write_field(dst: &mut [u8], val: &[u8]) {
        let mut v = vec![b' '; dst.len()];
        let start = dst.len() - val.len();
        v[start..].copy_from_slice(val);
        dst.copy_from_slice(&v);
    }
}
