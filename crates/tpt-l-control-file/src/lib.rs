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
/// whitespace) are folded into the previous field value.
pub fn parse_control(input: &str) -> Vec<ControlParagraph> {
    let mut paragraphs: Vec<ControlParagraph> = Vec::new();
    let mut current = ControlParagraph::new();
    let mut current_key: Option<String> = None;

    for line in input.lines() {
        if line.is_empty() {
            if !current.is_empty() {
                paragraphs.push(std::mem::take(&mut current));
                current_key = None;
            }
            continue;
        }

        if line.starts_with(' ') || line.starts_with('\t') {
            if let Some(ref key) = current_key {
                let val = current.fields.get_mut(key).unwrap();
                val.push('\n');
                val.push_str(line.trim_start());
            }
            continue;
        }

        if let Some(colon) = line.find(':') {
            let key = line[..colon].trim().to_string();
            let value = line[colon + 1..].trim().to_string();
            current_key = Some(key.clone());
            current.set(&key, &value);
        }
    }

    if !current.is_empty() {
        paragraphs.push(current);
    }

    paragraphs
}

/// Parse a Debian control file from a filesystem path.
pub fn parse_control_file(path: &Path) -> Result<Vec<ControlParagraph>, ControlError> {
    let content = std::fs::read_to_string(path)?;
    Ok(parse_control(&content))
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
        let paragraphs = parse_control(stanza);
        let p = paragraphs.into_iter().next().unwrap_or_default();

        let require = |key: &str| -> Result<String, ControlError> {
            p.get(key)
                .map(|v| v.to_string())
                .ok_or_else(|| ControlError::MissingField(key.to_string()))
        };

        Ok(Self {
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
            installed_size: p.get("Installed-Size").and_then(|v| v.parse().ok()),
            filename: p.get("Filename").map(str::to_string),
            sha256: p.get("SHA256").map(str::to_string),
            size: p.get("Size").and_then(|v| v.parse().ok()),
        })
    }

    /// Parse all stanzas from a `Packages` index file.
    pub fn parse_packages_index(data: &str) -> Vec<Result<Self, ControlError>> {
        data.split("\n\n")
            .filter(|s| !s.trim().is_empty())
            .map(Self::parse_stanza)
            .collect()
    }
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
}
