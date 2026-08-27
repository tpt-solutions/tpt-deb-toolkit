//! Debian control file parsing.
//!
//! Handles `Packages` index stanzas, `control` files inside `.deb` archives,
//! and standalone binary/source package metadata.
//!
//! # Example
//!
//! ```
//! use tpt_l_control_file::BinaryPackage;
//!
//! let stanza = "Package: hello\nVersion: 1.0-1\nArchitecture: amd64\n";
//! let pkg = BinaryPackage::parse_stanza(stanza).unwrap();
//! assert_eq!(pkg.name, "hello");
//! assert_eq!(pkg.version_str, "1.0-1");
//! ```

use std::borrow::Cow;
use std::collections::HashMap;
use std::path::Path;

use thiserror::Error;

// ─── Errors ───────────────────────────────────────────────────────────────────

/// Errors that can occur during control file parsing.
#[derive(Debug, Error)]
pub enum ControlError {
    #[error("missing required field: {0}")]
    MissingField(String),
    #[error("invalid field value for {field}: {reason}")]
    InvalidField { field: String, reason: String },
    #[error("duplicate field in stanza: {0}")]
    DuplicateField(String),
    #[error("expected exactly one stanza but found {0}")]
    UnexpectedMultipleStanzas(usize),
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
}

// ─── Generic stanza type ──────────────────────────────────────────────────────

/// A single stanza (paragraph) from a Debian control file.
///
/// Preserves all fields regardless of whether they appear in the typed
/// [`BinaryPackage`] API. Field names are stored in original case; lookups
/// are case-insensitive.
#[derive(Debug, Clone, Default)]
pub struct ControlParagraph {
    fields: HashMap<String, String>,
    order: Vec<String>,
}

impl ControlParagraph {
    /// Create an empty paragraph.
    pub fn new() -> Self {
        Self::default()
    }

    /// Insert or replace `key` (stored as-is) with `value`.
    pub fn set(&mut self, key: &str, value: &str) {
        let key = key.to_string();
        if !self.fields.contains_key(&key) {
            self.order.push(key.clone());
        }
        self.fields.insert(key, value.to_string());
    }

    /// Look up a field value by name (case-insensitive).
    pub fn get(&self, field: &str) -> Option<&str> {
        // Fast path: exact match
        if let Some(v) = self.fields.get(field) {
            return Some(v.as_str());
        }
        // Slow path: case-insensitive
        let lower = field.to_lowercase();
        self.fields
            .iter()
            .find(|(k, _)| k.to_lowercase() == lower)
            .map(|(_, v)| v.as_str())
    }

    /// Iterate over `(key, value)` pairs in insertion order.
    pub fn fields(&self) -> impl Iterator<Item = (&str, &str)> {
        self.order
            .iter()
            .filter_map(move |k| self.fields.get(k).map(|v| (k.as_str(), v.as_str())))
    }

    /// Returns `true` if the paragraph contains no fields.
    pub fn is_empty(&self) -> bool {
        self.fields.is_empty()
    }
}

/// Parse all stanzas from a Debian control file string.
///
/// Stanzas are separated by blank lines. Continuation lines (starting with
/// whitespace) are folded into the previous field value. Duplicate fields are
/// accepted (the last occurrence wins); use [`parse_control_strict`] to reject
/// them.
pub fn parse_control(input: &str) -> Vec<ControlParagraph> {
    split_stanzas(input)
        .into_iter()
        .map(|s| parse_paragraph(s, false).unwrap_or_default())
        .collect()
}

/// Parse all stanzas from a Debian control file string, rejecting any stanza
/// that repeats a field name.
///
/// Debian policy forbids duplicate fields within a single stanza; this variant
/// returns [`ControlError::DuplicateField`] for the offending stanza instead of
/// silently keeping the last value.
pub fn parse_control_strict(input: &str) -> Vec<Result<ControlParagraph, ControlError>> {
    split_stanzas(input)
        .into_iter()
        .map(|s| parse_paragraph(s, true))
        .collect()
}

/// Split a control document into its non-empty stanzas (blank-line separated).
fn split_stanzas(input: &str) -> Vec<&str> {
    input
        .split("\n\n")
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .collect()
}

/// Parse a single stanza. When `strict` is true, a repeated field name is an
/// error.
fn parse_paragraph(stanza: &str, strict: bool) -> Result<ControlParagraph, ControlError> {
    let mut para = ControlParagraph::new();
    let mut current_key: Option<String> = None;

    for line in stanza.lines() {
        if line.is_empty() {
            continue;
        }

        if line.starts_with(' ') || line.starts_with('\t') {
            if let Some(ref key) = current_key {
                if let Some(val) = para.fields.get_mut(key) {
                    val.push('\n');
                    // Debian folds long fields by prefixing the continuation with a
                    // single space (or tab); strip exactly that one folding marker.
                    val.push_str(&line[1..]);
                }
            }
            continue;
        }

        if let Some(colon) = line.find(':') {
            let key = line[..colon].trim().to_string();
            let value = line[colon + 1..].trim().to_string();
            if strict && para.fields.contains_key(&key) {
                return Err(ControlError::DuplicateField(key));
            }
            current_key = Some(key.clone());
            para.set(&key, &value);
        }
    }

    Ok(para)
}

/// Parse a Debian control file from a filesystem path.
pub fn parse_control_file(path: &Path) -> Result<Vec<ControlParagraph>, ControlError> {
    let content = std::fs::read_to_string(path)?;
    Ok(parse_control(&content))
}

/// Build a typed [`BinaryPackage`] from an already-parsed paragraph.
fn build_binary(p: &ControlParagraph) -> Result<BinaryPackage, ControlError> {
    let require = |key: &str| -> Result<String, ControlError> {
        p.get(key)
            .map(|v| v.to_string())
            .ok_or_else(|| ControlError::MissingField(key.to_string()))
    };
    Ok(BinaryPackage {
        name: require("Package")?,
        version_str: require("Version")?,
        architecture: p.get("Architecture").unwrap_or("all").to_string(),
        description: p
            .get("Description")
            .unwrap_or("")
            .lines()
            .next()
            .unwrap_or("")
            .to_string(),
        depends: p.get("Depends").map(str::to_string),
        pre_depends: p.get("Pre-Depends").map(str::to_string),
        conflicts: p.get("Conflicts").map(str::to_string),
        breaks: p.get("Breaks").map(str::to_string),
        provides: p.get("Provides").map(str::to_string),
        recommends: p.get("Recommends").map(str::to_string),
        suggests: p.get("Suggests").map(str::to_string),
        installed_size: p.get("Installed-Size").and_then(|v| v.parse().ok()),
        filename: p.get("Filename").map(str::to_string),
        sha256: p.get("SHA256").map(str::to_string),
        size: p.get("Size").and_then(|v| v.parse().ok()),
    })
}

// ─── Typed binary-package stanza ─────────────────────────────────────────────

/// A parsed binary package stanza from a `Packages` index or `.deb` control file.
#[derive(Debug, Clone)]
pub struct BinaryPackage {
    /// Package name (`Package:` field).
    pub name: String,
    /// Version string (`Version:` field).
    pub version_str: String,
    /// Architecture (`Architecture:` field).
    pub architecture: String,
    /// Short description (`Description:` first line).
    pub description: String,
    /// Raw `Depends:` field value (unparsed).
    pub depends: Option<String>,
    /// Raw `Pre-Depends:` field value (unparsed).
    pub pre_depends: Option<String>,
    /// Raw `Conflicts:` field value (unparsed).
    pub conflicts: Option<String>,
    /// Raw `Breaks:` field value (unparsed).
    pub breaks: Option<String>,
    /// Raw `Provides:` field value (unparsed).
    pub provides: Option<String>,
    /// Raw `Recommends:` field value (unparsed).
    pub recommends: Option<String>,
    /// Raw `Suggests:` field value (unparsed).
    pub suggests: Option<String>,
    /// Installed size in KiB (`Installed-Size:` field).
    pub installed_size: Option<u64>,
    /// Download filename relative to pool root.
    pub filename: Option<String>,
    /// SHA-256 checksum of the `.deb`.
    pub sha256: Option<String>,
    /// File size in bytes.
    pub size: Option<u64>,
}

impl BinaryPackage {
    /// Parse a single control stanza from text.
    ///
    /// Stanzas are separated by blank lines; pass one stanza at a time.
    pub fn parse_stanza(stanza: &str) -> Result<Self, ControlError> {
        let p = parse_paragraph(stanza, false).unwrap_or_default();
        build_binary(&p)
    }

    /// Parse a single control stanza from text, rejecting duplicate fields.
    ///
    /// See [`parse_control_strict`] for the rationale.
    pub fn parse_stanza_strict(stanza: &str) -> Result<Self, ControlError> {
        let p = parse_paragraph(stanza, true)?;
        build_binary(&p)
    }

    /// Parse all stanzas from a `Packages` index file.
    pub fn parse_packages_index(data: &str) -> Vec<Result<Self, ControlError>> {
        data.split("\n\n")
            .filter(|s| !s.trim().is_empty())
            .map(Self::parse_stanza)
            .collect()
    }
}

/// A parsed source package stanza from a `Sources` index or `.dsc`-derived data.
#[derive(Debug, Clone)]
pub struct SourcePackage {
    /// Source package name (`Package`/`Source:` field).
    pub name: String,
    /// Version string (`Version:` field).
    pub version_str: String,
    /// Maintainer (`Maintainer:` field).
    pub maintainer: String,
    /// Architectures the source builds for (`Architecture:` list).
    pub architecture: Vec<String>,
    /// Raw `Build-Depends:` field value (unparsed).
    pub build_depends: Option<String>,
    /// Raw `Build-Depends-Indep:` field value (unparsed).
    pub build_depends_indep: Option<String>,
    /// `Directory:` in the pool where the source lives.
    pub directory: Option<String>,
    /// `Files:` stanza (md5/sha256 + size + filename).
    pub files: Option<String>,
}

impl SourcePackage {
    /// Parse a single source-package stanza from text.
    pub fn parse_stanza(stanza: &str) -> Result<Self, ControlError> {
        let paragraphs = parse_control(stanza);
        let p = paragraphs.into_iter().next().unwrap_or_default();

        let require = |key: &str| -> Result<String, ControlError> {
            p.get(key)
                .map(|v| v.to_string())
                .ok_or_else(|| ControlError::MissingField(key.to_string()))
        };

        Ok(Self {
            name: require("Package").or_else(|_| require("Source"))?,
            version_str: require("Version")?,
            maintainer: p.get("Maintainer").unwrap_or("").to_string(),
            architecture: p
                .get("Architecture")
                .map(|s| s.split_whitespace().map(str::to_string).collect())
                .unwrap_or_default(),
            build_depends: p.get("Build-Depends").map(str::to_string),
            build_depends_indep: p.get("Build-Depends-Indep").map(str::to_string),
            directory: p.get("Directory").map(str::to_string),
            files: p.get("Files").map(str::to_string),
        })
    }
}

/// A lazily-parsed view over a `Packages` index.
///
/// Stanzas are split on blank lines and parsed on demand, so callers can
/// iterate very large indices without materialising every [`BinaryPackage`]
/// up front.
#[derive(Debug, Clone, Copy)]
pub struct PackagesIndex<'a> {
    text: &'a str,
}

impl<'a> PackagesIndex<'a> {
    /// Wrap a `Packages` index document.
    pub fn new(text: &'a str) -> Self {
        Self { text }
    }

    /// Iterate over every stanza, producing a [`BinaryPackage`] or the parse
    /// error for that stanza.
    pub fn iter_results(&self) -> impl Iterator<Item = Result<BinaryPackage, ControlError>> + 'a {
        self.text
            .split("\n\n")
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(BinaryPackage::parse_stanza)
    }

    /// Iterate over only the successfully-parsed packages.
    pub fn iter(&self) -> impl Iterator<Item = BinaryPackage> + 'a {
        self.iter_results().filter_map(Result::ok)
    }

    /// Iterate every stanza as a zero-copy [`BorrowedParagraph`] view, without
    /// materialising a typed [`BinaryPackage`]. Useful for cheap field access
    /// over very large indices.
    pub fn iter_paragraphs(&self) -> impl Iterator<Item = BorrowedParagraph<'a>> + 'a {
        self.text
            .split("\n\n")
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(|s| parse_borrowed_paragraph(s, false).unwrap_or_default())
    }

    /// Iterate every stanza, parsing strictly (duplicate fields rejected) and
    /// producing a [`BinaryPackage`] or the parse error for that stanza.
    pub fn iter_results_strict(
        &self,
    ) -> impl Iterator<Item = Result<BinaryPackage, ControlError>> + 'a {
        self.text
            .split("\n\n")
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(BinaryPackage::parse_stanza_strict)
    }

    /// Count of stanzas that parse successfully as packages.
    pub fn len(&self) -> usize {
        self.iter().count()
    }

    /// `true` when the index contains no packages.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

// ─── Single-stanza control file ───────────────────────────────────────────────

/// A single-stanza Debian control file such as a binary package's
/// `DEBIAN/control`, a `.changes` file, or a `Release`/`InRelease` header block.
///
/// Debian policy forbids duplicate fields *within* a stanza, so the document is
/// parsed strictly: [`ControlFile::parse`] returns
/// [`ControlError::DuplicateField`] when a field repeats, and
/// [`ControlError::UnexpectedMultipleStanzas`] when more than one stanza is
/// present (use [`PackagesIndex`]/[`SourcesIndex`] for multi-stanza indices).
#[derive(Debug, Clone)]
pub struct ControlFile {
    paragraph: ControlParagraph,
}

impl ControlFile {
    /// Parse exactly one stanza from `text`.
    pub fn parse(text: &str) -> Result<Self, ControlError> {
        let paras = parse_control_strict(text);
        let count = paras.len();
        let mut iter = paras.into_iter();
        let first = iter.next();
        if iter.next().is_some() {
            return Err(ControlError::UnexpectedMultipleStanzas(count));
        }
        match first {
            Some(Ok(p)) => Ok(Self { paragraph: p }),
            Some(Err(e)) => Err(e),
            None => Err(ControlError::MissingField("(empty document)".into())),
        }
    }

    /// Parse a single-stanza control file from a filesystem path.
    pub fn load(path: &Path) -> Result<Self, ControlError> {
        let content = std::fs::read_to_string(path)?;
        Self::parse(&content)
    }

    /// Look up a field value by name (case-insensitive).
    pub fn field(&self, name: &str) -> Option<&str> {
        self.paragraph.get(name)
    }

    /// Iterate over `(key, value)` pairs in insertion order.
    pub fn fields(&self) -> impl Iterator<Item = (&str, &str)> {
        self.paragraph.fields()
    }

    /// Consume the file, returning the underlying paragraph.
    pub fn into_paragraph(self) -> ControlParagraph {
        self.paragraph
    }
}

// ─── Source index ─────────────────────────────────────────────────────────────

/// A lazily-parsed view over a `Sources` index.
///
/// Much like [`PackagesIndex`] but for source packages ([`SourcePackage`]).
/// Stanzas are split on blank lines and parsed on demand.
#[derive(Debug, Clone, Copy)]
pub struct SourcesIndex<'a> {
    text: &'a str,
}

impl<'a> SourcesIndex<'a> {
    /// Wrap a `Sources` index document.
    pub fn new(text: &'a str) -> Self {
        Self { text }
    }

    /// Iterate over every stanza, producing a [`SourcePackage`] or the parse
    /// error for that stanza.
    pub fn iter_results(&self) -> impl Iterator<Item = Result<SourcePackage, ControlError>> + 'a {
        self.text
            .split("\n\n")
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(SourcePackage::parse_stanza)
    }

    /// Iterate over only the successfully-parsed source packages.
    pub fn iter(&self) -> impl Iterator<Item = SourcePackage> + 'a {
        self.iter_results().filter_map(Result::ok)
    }

    /// Iterate every stanza as a zero-copy [`BorrowedParagraph`] view, without
    /// materialising a typed [`SourcePackage`].
    pub fn iter_paragraphs(&self) -> impl Iterator<Item = BorrowedParagraph<'a>> + 'a {
        self.text
            .split("\n\n")
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(|s| parse_borrowed_paragraph(s, false).unwrap_or_default())
    }

    /// Count of stanzas that parse successfully as source packages.
    pub fn len(&self) -> usize {
        self.iter().count()
    }

    /// `true` when the index contains no source packages.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

// ─── Zero-copy (borrowing) stanza view ───────────────────────────────────────

/// A zero-copy view of a single control stanza.
///
/// Field *names* always borrow from the source text. Field *values* are borrowed
/// (`Cow::Borrowed`) for ordinary single-line fields, and only allocate
/// (`Cow::Owned`) when a folded/continuation line forces the value to be joined
/// across multiple source lines. This keeps bulk parsing of large indices
/// allocation-free for the common case — "zero-copy where possible".
#[derive(Debug, Clone, Default)]
pub struct BorrowedParagraph<'a> {
    fields: HashMap<&'a str, Cow<'a, str>>,
    order: Vec<&'a str>,
}

impl<'a> BorrowedParagraph<'a> {
    /// Create an empty zero-copy paragraph.
    pub fn new() -> Self {
        Self::default()
    }

    /// Insert or replace `key` (borrowed as-is) with a borrowed `value`.
    pub fn set(&mut self, key: &'a str, value: &'a str) {
        if !self.fields.contains_key(key) {
            self.order.push(key);
        }
        self.fields.insert(key, Cow::Borrowed(value));
    }

    /// Append a continuation line to an existing field (folding). Only this
    /// path may allocate.
    fn append(&mut self, key: &'a str, continuation: &'a str) {
        if let Some(slot) = self.fields.get_mut(key) {
            match slot {
                Cow::Borrowed(b) => {
                    let mut owned = String::from(*b);
                    owned.push('\n');
                    owned.push_str(continuation);
                    *slot = Cow::Owned(owned);
                }
                Cow::Owned(o) => {
                    o.push('\n');
                    o.push_str(continuation);
                }
            }
        }
    }

    /// Look up a field value by name (case-insensitive). Returns a slice that
    /// borrows from the source text for ordinary fields.
    pub fn get(&self, field: &str) -> Option<&str> {
        if let Some(v) = self.fields.get(field) {
            return Some(v.as_ref());
        }
        let lower = field.to_lowercase();
        self.fields
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(lower.as_str()))
            .map(|(_, v)| v.as_ref())
    }

    /// Iterate over `(key, value)` pairs in insertion order.
    pub fn fields(&self) -> impl Iterator<Item = (&'a str, &str)> + '_ {
        self.order
            .iter()
            .filter_map(move |k| self.fields.get(k).map(|v| (*k, v.as_ref())))
    }

    /// Returns `true` if the paragraph contains no fields.
    pub fn is_empty(&self) -> bool {
        self.fields.is_empty()
    }
}

/// Parse a single zero-copy stanza.
fn parse_borrowed_paragraph<'a>(
    stanza: &'a str,
    strict: bool,
) -> Result<BorrowedParagraph<'a>, ControlError> {
    let mut para = BorrowedParagraph::new();
    let mut current_key: Option<&'a str> = None;
    for line in stanza.lines() {
        if line.is_empty() {
            continue;
        }
        if line.starts_with(' ') || line.starts_with('\t') {
            if let Some(key) = current_key {
                para.append(key, &line[1..]);
            }
            continue;
        }
        if let Some(colon) = line.find(':') {
            let key = &line[..colon];
            let value = line[colon + 1..].trim();
            if strict && para.fields.contains_key(key) {
                return Err(ControlError::DuplicateField(key.to_string()));
            }
            current_key = Some(key);
            para.set(key, value);
        }
    }
    Ok(para)
}

/// Parse every stanza in `input` as a zero-copy [`BorrowedParagraph`] view.
pub fn parse_control_borrowed<'a>(input: &'a str) -> Vec<BorrowedParagraph<'a>> {
    split_stanzas(input)
        .into_iter()
        .map(|s| parse_borrowed_paragraph(s, false).unwrap_or_default())
        .collect()
}

/// Like [`parse_control_borrowed`] but rejects duplicate fields per stanza.
pub fn parse_control_strict_borrowed<'a>(
    input: &'a str,
) -> Vec<Result<BorrowedParagraph<'a>, ControlError>> {
    split_stanzas(input)
        .into_iter()
        .map(|s| parse_borrowed_paragraph(s, true))
        .collect()
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    const STANZA: &str = "\
Package: libfoo
Version: 1.2.3-4
Architecture: amd64
Description: A foo library
Depends: libc6 (>= 2.17), libbar
Conflicts: libfoo-old
Provides: libfoo-abi1
";

    #[test]
    fn parse_basic_stanza() {
        let pkg = BinaryPackage::parse_stanza(STANZA).unwrap();
        assert_eq!(pkg.name, "libfoo");
        assert_eq!(pkg.version_str, "1.2.3-4");
        assert_eq!(pkg.architecture, "amd64");
        assert!(pkg.depends.is_some());
        assert!(pkg.conflicts.is_some());
        assert!(pkg.provides.is_some());
    }

    #[test]
    fn missing_package_field_errors() {
        let result = BinaryPackage::parse_stanza("Version: 1.0\n");
        assert!(matches!(result, Err(ControlError::MissingField(_))));
    }

    #[test]
    fn parse_control_multi_stanza() {
        let input = "Package: a\nVersion: 1.0\n\nPackage: b\nVersion: 2.0\n";
        let stanzas = parse_control(input);
        assert_eq!(stanzas.len(), 2);
        assert_eq!(stanzas[0].get("Package"), Some("a"));
        assert_eq!(stanzas[1].get("Package"), Some("b"));
    }

    #[test]
    fn control_paragraph_case_insensitive_get() {
        let mut p = ControlParagraph::new();
        p.set("Package", "hello");
        assert_eq!(p.get("package"), Some("hello"));
        assert_eq!(p.get("PACKAGE"), Some("hello"));
    }

    #[test]
    fn folded_description_joined_with_newlines() {
        let stanza = "\
Package: foo
Version: 1.0
Description: short synopsis
 this is a continuation line
  and another with leading spaces preserved
";
        let pkg = BinaryPackage::parse_stanza(stanza).unwrap();
        assert_eq!(pkg.description, "short synopsis");
        let desc = parse_control(stanza)
            .into_iter()
            .next()
            .unwrap()
            .get("Description")
            .unwrap()
            .to_string();
        assert!(desc.starts_with("short synopsis"));
        assert!(desc.contains("this is a continuation line"));
        assert!(desc.contains(" and another with leading spaces preserved"));
    }

    #[test]
    fn source_package_parse() {
        let stanza = "\
Package: glibc
Version: 2.38-1
Maintainer: Nobody <nobody@example.com>
Architecture: all amd64
Build-Depends: libc6-dev (>= 2.0), gcc
Directory: pool/main/g/glibc
";
        let src = SourcePackage::parse_stanza(stanza).unwrap();
        assert_eq!(src.name, "glibc");
        assert_eq!(src.version_str, "2.38-1");
        assert_eq!(
            src.architecture,
            vec!["all".to_string(), "amd64".to_string()]
        );
        assert!(src.build_depends.is_some());
        assert_eq!(src.directory.as_deref(), Some("pool/main/g/glibc"));
    }

    #[test]
    fn packages_index_streams_entries() {
        let index = "\
Package: a
Version: 1.0

Package: b
Version: 2.0

Package: c
Version: 3.0
";
        let idx = PackagesIndex::new(index);
        assert_eq!(idx.len(), 3);
        let names: Vec<_> = idx.iter().map(|p| p.name).collect();
        assert_eq!(
            names,
            vec!["a".to_string(), "b".to_string(), "c".to_string()]
        );
    }

    #[test]
    fn strict_parse_rejects_duplicate_fields() {
        let input = "Package: a\nVersion: 1.0\nPackage: b\n";
        let paras = parse_control_strict(input);
        assert_eq!(paras.len(), 1);
        assert!(matches!(paras[0], Err(ControlError::DuplicateField(_))));
    }

    #[test]
    fn strict_parse_accepts_unique_fields() {
        let input = "Package: a\nVersion: 1.0\n\nPackage: b\nVersion: 2.0\n";
        let paras = parse_control_strict(input);
        assert!(paras.iter().all(|p| p.is_ok()));
    }

    #[test]
    fn binary_stanza_strict_rejects_dupes() {
        let stanza = "Package: a\nVersion: 1.0\nPackage: b\n";
        assert!(matches!(
            BinaryPackage::parse_stanza_strict(stanza),
            Err(ControlError::DuplicateField(_))
        ));
    }

    #[test]
    fn packages_index_strict_iter() {
        let index = "Package: a\nVersion: 1.0\nPackage: a2\nVersion: 1.0\n";
        let idx = PackagesIndex::new(index);
        let results: Vec<_> = idx.iter_results_strict().collect();
        assert_eq!(results.len(), 1);
        assert!(results[0].is_err());
    }

    #[test]
    fn control_file_parses_single_stanza() {
        let doc = "Package: hello\nVersion: 1.0-1\nArchitecture: amd64\n";
        let cf = ControlFile::parse(doc).unwrap();
        assert_eq!(cf.field("Package"), Some("hello"));
        assert_eq!(cf.field("version"), Some("1.0-1"));
        assert_eq!(cf.field("Architecture"), Some("amd64"));
    }

    #[test]
    fn control_file_rejects_duplicate_fields() {
        let doc = "Package: a\nVersion: 1.0\nPackage: b\n";
        assert!(matches!(
            ControlFile::parse(doc),
            Err(ControlError::DuplicateField(_))
        ));
    }

    #[test]
    fn control_file_rejects_multiple_stanzas() {
        let doc = "Package: a\nVersion: 1.0\n\nPackage: b\nVersion: 2.0\n";
        assert!(matches!(
            ControlFile::parse(doc),
            Err(ControlError::UnexpectedMultipleStanzas(_))
        ));
    }

    #[test]
    fn control_file_empty_document_errors() {
        assert!(ControlFile::parse("   \n  ").is_err());
    }

    #[test]
    fn sources_index_streams_entries() {
        let index = "\
Package: glibc
Version: 2.38-1
Directory: pool/main/g/glibc

Package: bash
Version: 5.2-1
Directory: pool/main/b/bash
";
        let idx = SourcesIndex::new(index);
        assert_eq!(idx.len(), 2);
        let names: Vec<_> = idx.iter().map(|p| p.name).collect();
        assert_eq!(names, vec!["glibc".to_string(), "bash".to_string()]);
    }

    #[test]
    fn sources_index_reports_parse_errors() {
        let index = "Package: glibc\nVersion: 2.38-1\n\nVersion: 5.2-1\n";
        let idx = SourcesIndex::new(index);
        let results: Vec<_> = idx.iter_results().collect();
        assert_eq!(results.len(), 2);
        assert!(results[0].is_ok());
        assert!(results[1].is_err());
    }

    #[test]
    fn borrowed_paragraph_zero_copy_get() {
        let input = "Package: hello\nVersion: 1.0-1\nArchitecture: amd64\n";
        let paras = parse_control_borrowed(input);
        assert_eq!(paras.len(), 1);
        // Case-insensitive lookup, borrowed (no allocation) for simple fields.
        assert_eq!(paras[0].get("package"), Some("hello"));
        assert_eq!(paras[0].get("Version"), Some("1.0-1"));
        assert_eq!(paras[0].get("ARCHITECTURE"), Some("amd64"));
        assert!(paras[0].get("Missing").is_none());
    }

    #[test]
    fn borrowed_paragraph_folding_allocates_only_when_needed() {
        let stanza = "\
Package: foo
Version: 1.0
Description: short synopsis
 this is a continuation line
";
        let para = parse_borrowed_paragraph(stanza, false).unwrap();
        // The folded value is the only one that required an owned allocation.
        let desc = para.get("Description").unwrap();
        assert!(desc.starts_with("short synopsis"));
        assert!(desc.contains("this is a continuation line"));
        // Non-folded fields remain borrowed from the source.
        assert_eq!(para.get("Package"), Some("foo"));
    }

    #[test]
    fn borrowed_paragraph_strict_rejects_dupes() {
        let input = "Package: a\nVersion: 1.0\nPackage: b\n";
        let paras = parse_control_strict_borrowed(input);
        assert_eq!(paras.len(), 1);
        assert!(matches!(paras[0], Err(ControlError::DuplicateField(_))));
    }

    #[test]
    fn packages_index_iter_paragraphs_is_zero_copy() {
        let index = "\
Package: a
Version: 1.0

Package: b
Version: 2.0
";
        let idx = PackagesIndex::new(index);
        let names: Vec<_> = idx
            .iter_paragraphs()
            .map(|p| p.get("Package").unwrap().to_string())
            .collect();
        assert_eq!(names, vec!["a".to_string(), "b".to_string()]);
    }

    #[test]
    fn sources_index_iter_paragraphs_is_zero_copy() {
        let index = "\
Package: glibc
Version: 2.38-1

Package: bash
Version: 5.2-1
";
        let idx = SourcesIndex::new(index);
        let names: Vec<_> = idx
            .iter_paragraphs()
            .map(|p| p.get("Package").unwrap().to_string())
            .collect();
        assert_eq!(names, vec!["glibc".to_string(), "bash".to_string()]);
    }
}
