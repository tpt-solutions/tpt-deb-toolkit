//! Parser and writer for APT `sources.list` and deb822 `.sources` files.
//!
//! Supports the classic one-line format and the newer deb822 (`.sources`)
//! stanza format. The format is auto-detected by file extension when loading
//! from disk.
//!
//! # One-line format
//!
//! ```text
//! deb [arch=amd64 signed-by=/usr/share/keyrings/ubuntu.gpg] \
//!     http://archive.ubuntu.com/ubuntu focal main restricted
//! deb-src http://archive.ubuntu.com/ubuntu focal main
//! # deb http://archive.ubuntu.com/ubuntu focal-backports main
//! ```
//!
//! # deb822 format (`.sources`)
//!
//! ```text
//! Types: deb deb-src
//! URIs: http://archive.ubuntu.com/ubuntu
//! Suites: focal focal-updates
//! Components: main restricted
//! Enabled: yes
//! Signed-By: /usr/share/keyrings/ubuntu-archive-keyring.gpg
//! ```

use std::collections::HashMap;
use std::path::Path;
use thiserror::Error;

/// Identity shared by entries that should be grouped into one deb822 stanza:
/// `(uri, suite, components, options, enabled)`.
type GroupKey = (String, String, String, HashMap<String, String>, bool);

// ── Error ─────────────────────────────────────────────────────────────────────

/// Errors produced by the sources-list parser.
#[derive(Debug, Error)]
pub enum SourcesError {
    /// An I/O error occurred reading a file.
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    /// A line or stanza could not be parsed.
    #[error("parse error: {0}")]
    Parse(String),
}

// ── SourceType ────────────────────────────────────────────────────────────────

/// Whether a repository entry provides binary or source packages.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceType {
    /// `deb` — binary packages.
    Binary,
    /// `deb-src` — source packages.
    Source,
}

impl SourceType {
    fn from_str(s: &str) -> Option<Self> {
        match s {
            "deb" => Some(Self::Binary),
            "deb-src" => Some(Self::Source),
            _ => None,
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Binary => "deb",
            Self::Source => "deb-src",
        }
    }
}

// ── SourceEntry ───────────────────────────────────────────────────────────────

/// A single APT repository entry.
#[derive(Debug, Clone)]
pub struct SourceEntry {
    /// Whether this is a `deb` (binary) or `deb-src` (source) entry.
    pub source_type: SourceType,
    /// Repository root URI, e.g. `http://archive.ubuntu.com/ubuntu`.
    pub uri: String,
    /// Suite name, e.g. `focal` or `focal-updates`.
    pub suite: String,
    /// Repository components, e.g. `["main", "restricted"]`.
    pub components: Vec<String>,
    /// Options parsed from the `[…]` block, e.g. `arch=amd64`, `signed-by=…`.
    pub options: HashMap<String, String>,
    /// `false` for commented-out one-line entries or `Enabled: no` deb822 stanzas.
    pub enabled: bool,
}

impl SourceEntry {
    /// URL for the `InRelease` file for this suite.
    pub fn release_url(&self) -> String {
        format!(
            "{}/dists/{}/InRelease",
            self.uri.trim_end_matches('/'),
            self.suite
        )
    }

    /// URL for a `Packages` index for one component and architecture.
    pub fn packages_url(&self, component: &str, arch: &str) -> String {
        format!(
            "{}/dists/{}/{}/binary-{}/Packages",
            self.uri.trim_end_matches('/'),
            self.suite,
            component,
            arch,
        )
    }
}

// ── SourcesList ───────────────────────────────────────────────────────────────

/// A parsed collection of APT repository entries.
#[derive(Debug, Default)]
pub struct SourcesList {
    /// All entries in file order (enabled and disabled).
    pub entries: Vec<SourceEntry>,
}

impl SourcesList {
    /// Create an empty sources list.
    pub fn new() -> Self {
        Self::default()
    }

    /// Parse the classic one-line `sources.list` format from a string.
    ///
    /// Lines beginning with `#` that look like a disabled `deb` or `deb-src`
    /// entry are parsed with `enabled = false`; other comment lines are ignored.
    pub fn parse_one_line(input: &str) -> Result<Self, SourcesError> {
        let mut entries = Vec::new();
        for raw in input.lines() {
            let trimmed = raw.trim();
            if trimmed.is_empty() {
                continue;
            }
            let (enabled, line) = if let Some(rest) = trimmed.strip_prefix('#') {
                let rest = rest.trim();
                if rest.starts_with("deb") {
                    (false, rest)
                } else {
                    continue;
                }
            } else {
                // Strip trailing inline comment.
                let eff = trimmed
                    .split_once(" #")
                    .map(|(l, _)| l.trim())
                    .unwrap_or(trimmed);
                (true, eff)
            };
            if let Some(entry) = parse_one_line_entry(line, enabled)? {
                entries.push(entry);
            }
        }
        Ok(Self { entries })
    }

    /// Parse the deb822 `.sources` format from a string.
    ///
    /// Each stanza can produce multiple `SourceEntry` values — one per
    /// (type × URI × suite) combination — all sharing the same component list.
    pub fn parse_deb822(input: &str) -> Result<Self, SourcesError> {
        let mut entries = Vec::new();
        for stanza in split_deb822_stanzas(input) {
            let fields = parse_deb822_stanza(stanza)?;

            let enabled_str = fields.get("enabled").map(String::as_str).unwrap_or("yes");
            let enabled = !matches!(enabled_str.to_lowercase().trim(), "no" | "false" | "0");

            let types_str = fields.get("types").cloned().unwrap_or_default();
            let uris_str = fields.get("uris").cloned().unwrap_or_default();
            let suites_str = fields.get("suites").cloned().unwrap_or_default();
            let comps_str = fields.get("components").cloned().unwrap_or_default();

            let types: Vec<SourceType> = types_str
                .split_whitespace()
                .filter_map(SourceType::from_str)
                .collect();
            let uris: Vec<String> = uris_str.split_whitespace().map(str::to_string).collect();
            let suites: Vec<String> = suites_str.split_whitespace().map(str::to_string).collect();
            let components: Vec<String> =
                comps_str.split_whitespace().map(str::to_string).collect();

            let mut options: HashMap<String, String> = HashMap::new();
            if let Some(signed_by) = fields.get("signed-by") {
                options.insert("signed-by".to_string(), signed_by.clone());
            }
            if let Some(arch) = fields.get("architectures") {
                options.insert("arch".to_string(), arch.clone());
            }

            for &src_type in &types {
                for uri in &uris {
                    for suite in &suites {
                        entries.push(SourceEntry {
                            source_type: src_type,
                            uri: uri.clone(),
                            suite: suite.clone(),
                            components: components.clone(),
                            options: options.clone(),
                            enabled,
                        });
                    }
                }
            }
        }
        Ok(Self { entries })
    }

    /// Load a file from disk, auto-detecting format by extension.
    ///
    /// `.sources` → deb822 format; everything else → one-line format.
    pub fn load_file(path: &Path) -> Result<Self, SourcesError> {
        let content = std::fs::read_to_string(path)?;
        if path.extension().and_then(|e| e.to_str()) == Some("sources") {
            Self::parse_deb822(&content)
        } else {
            Self::parse_one_line(&content)
        }
    }

    /// Scan `dir` for `*.list` and `*.sources` files and merge their entries.
    ///
    /// Files are processed in lexicographic order (matching `apt`'s behaviour
    /// for `/etc/apt/sources.list.d/`).
    pub fn load_dir(dir: &Path) -> Result<Self, SourcesError> {
        let mut all = Self::new();
        let mut paths: Vec<_> = std::fs::read_dir(dir)?
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| {
                p.is_file()
                    && matches!(
                        p.extension().and_then(|e| e.to_str()),
                        Some("list") | Some("sources")
                    )
            })
            .collect();
        paths.sort();
        for path in paths {
            all.entries.extend(Self::load_file(&path)?.entries);
        }
        Ok(all)
    }

    // ── Iterators ─────────────────────────────────────────────────────────────

    /// Iterate over all entries in file order.
    pub fn entries(&self) -> impl Iterator<Item = &SourceEntry> {
        self.entries.iter()
    }

    /// Iterate over only enabled entries.
    pub fn active_entries(&self) -> impl Iterator<Item = &SourceEntry> {
        self.entries.iter().filter(|e| e.enabled)
    }

    /// Iterate over enabled binary (`deb`) entries.
    pub fn binary_entries(&self) -> impl Iterator<Item = &SourceEntry> {
        self.entries
            .iter()
            .filter(|e| e.enabled && e.source_type == SourceType::Binary)
    }

    /// Iterate over enabled source (`deb-src`) entries.
    pub fn source_entries(&self) -> impl Iterator<Item = &SourceEntry> {
        self.entries
            .iter()
            .filter(|e| e.enabled && e.source_type == SourceType::Source)
    }

    // ── Serialisation ─────────────────────────────────────────────────────────

    /// Serialise all entries to one-line `sources.list` format.
    ///
    /// Disabled entries are written as commented-out lines beginning with `# `.
    pub fn write_one_line(&self) -> String {
        let mut out = String::new();
        for entry in &self.entries {
            if !entry.enabled {
                out.push_str("# ");
            }
            out.push_str(entry.source_type.as_str());
            if !entry.options.is_empty() {
                out.push_str(" [");
                let mut opts: Vec<String> = entry
                    .options
                    .iter()
                    .map(|(k, v)| format!("{}={}", k, v))
                    .collect();
                opts.sort();
                out.push_str(&opts.join(" "));
                out.push(']');
            }
            out.push(' ');
            out.push_str(&entry.uri);
            out.push(' ');
            out.push_str(&entry.suite);
            for comp in &entry.components {
                out.push(' ');
                out.push_str(comp);
            }
            out.push('\n');
        }
        out
    }

    /// Serialise all entries to deb822 `.sources` format.
    ///
    /// Entries are grouped by `(uri, suite, components, options, enabled)` so
    /// that the `Types:` field can list every source type sharing that
    /// identity (e.g. `deb deb-src`).
    pub fn write_deb822(&self) -> String {
        // Group key: identity shared across the Types axis.
        let mut groups: Vec<GroupKey> = Vec::new();
        let mut types_by_group: HashMap<usize, Vec<SourceType>> = HashMap::new();

        for entry in &self.entries {
            let comp_key = entry.components.join(" ");
            let key = (
                entry.uri.clone(),
                entry.suite.clone(),
                comp_key,
                entry.options.clone(),
                entry.enabled,
            );
            let idx = if let Some(pos) = groups.iter().position(|g| {
                g.0 == key.0 && g.1 == key.1 && g.2 == key.2 && g.3 == key.3 && g.4 == key.4
            }) {
                pos
            } else {
                groups.push(key);
                groups.len() - 1
            };
            types_by_group
                .entry(idx)
                .or_default()
                .push(entry.source_type);
        }

        let mut out = String::new();
        for (i, (uri, suite, comps, options, enabled)) in groups.iter().enumerate() {
            let types = types_by_group.get(&i).cloned().unwrap_or_default();
            let types_str: Vec<&str> = types.iter().map(|t| t.as_str()).collect();
            out.push_str(&format!("Types: {}\n", types_str.join(" ")));
            out.push_str(&format!("URIs: {}\n", uri));
            out.push_str(&format!("Suites: {}\n", suite));
            out.push_str(&format!("Components: {}\n", comps));
            out.push_str(&format!(
                "Enabled: {}\n",
                if *enabled { "yes" } else { "no" }
            ));
            if let Some(sb) = options.get("signed-by") {
                out.push_str(&format!("Signed-By: {}\n", sb));
            }
            if let Some(arch) = options.get("arch") {
                out.push_str(&format!("Architectures: {}\n", arch));
            }
            out.push('\n');
        }
        out
    }

    /// Write the sources list to `path`, choosing the format by extension:
    /// `.sources` → deb822, otherwise one-line.
    ///
    /// # Errors
    ///
    /// Returns [`SourcesError::Io`] if the file cannot be written.
    pub fn write(&self, path: &Path) -> Result<(), SourcesError> {
        let body = if path.extension().and_then(|e| e.to_str()) == Some("sources") {
            self.write_deb822()
        } else {
            self.write_one_line()
        };
        std::fs::write(path, body)?;
        Ok(())
    }

    /// Validate every entry's URI.
    ///
    /// A valid URI has the form `<scheme>://<rest>` where `<scheme>` is a
    /// recognised APT scheme (`http`, `https`, `ftp`, `file`, `mirror+http`,
    /// `mirror+https`, `cdrom`) and `<rest>` is non-empty with no whitespace.
    pub fn validate(&self) -> Result<(), SourcesError> {
        for entry in &self.entries {
            entry.validate_uri()?;
        }
        Ok(())
    }
}

impl SourceEntry {
    /// Whether this entry's [`uri`](SourceEntry::uri) is syntactically valid.
    pub fn is_valid_uri(&self) -> bool {
        self.validate_uri().is_ok()
    }

    /// Validate this entry's URI, returning an error describing the problem.
    pub fn validate_uri(&self) -> Result<(), SourcesError> {
        let uri = self.uri.trim();
        if uri.is_empty() {
            return Err(SourcesError::Parse("empty URI".to_string()));
        }
        if uri.split_whitespace().count() != 1 {
            return Err(SourcesError::Parse(format!(
                "URI contains whitespace: {:?}",
                uri
            )));
        }
        let Some((scheme, rest)) = uri.split_once("://") else {
            return Err(SourcesError::Parse(format!(
                "URI missing scheme separator '://': {:?}",
                uri
            )));
        };
        if scheme.is_empty() {
            return Err(SourcesError::Parse(format!(
                "empty URI scheme in {:?}",
                uri
            )));
        }
        let valid_scheme = matches!(
            scheme,
            "http" | "https" | "ftp" | "file" | "mirror+http" | "mirror+https" | "cdrom"
        ) && scheme
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '+' || c == '-');
        if !valid_scheme {
            return Err(SourcesError::Parse(format!(
                "unsupported URI scheme: {:?}",
                scheme
            )));
        }
        if rest.is_empty() {
            return Err(SourcesError::Parse(format!(
                "empty URI authority in {:?}",
                uri
            )));
        }
        Ok(())
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn parse_one_line_entry(line: &str, enabled: bool) -> Result<Option<SourceEntry>, SourcesError> {
    let mut words = line.split_ascii_whitespace();
    let type_str = match words.next() {
        Some(t) => t,
        None => return Ok(None),
    };
    let source_type = match SourceType::from_str(type_str) {
        Some(t) => t,
        None => return Ok(None),
    };

    let rest: Vec<&str> = words.collect();
    if rest.is_empty() {
        return Err(SourcesError::Parse(format!("missing URI in: {:?}", line)));
    }

    let mut options: HashMap<String, String> = HashMap::new();
    let (uri_idx, after_idx): (usize, usize) = if rest[0].starts_with('[') {
        // Find the closing `]`.
        let mut end = 0;
        let mut found = false;
        for (i, tok) in rest.iter().enumerate() {
            if tok.contains(']') {
                end = i;
                found = true;
                break;
            }
        }
        if !found {
            return Err(SourcesError::Parse(
                "unclosed options block '['".to_string(),
            ));
        }
        let block = rest[..=end].join(" ");
        let inner = block
            .trim_start_matches('[')
            .split(']')
            .next()
            .unwrap_or("")
            .trim();
        for part in inner.split_whitespace() {
            if let Some(eq) = part.find('=') {
                options.insert(part[..eq].to_lowercase(), part[eq + 1..].to_string());
            }
        }
        if end + 1 >= rest.len() {
            return Err(SourcesError::Parse(format!(
                "missing URI after options in: {:?}",
                line
            )));
        }
        (end + 1, end + 2)
    } else {
        (0, 1)
    };

    let uri = rest[uri_idx].to_string();
    let suite = match rest.get(after_idx) {
        Some(s) => s.to_string(),
        None => return Err(SourcesError::Parse(format!("missing suite in: {:?}", line))),
    };
    let components: Vec<String> = rest[after_idx + 1..]
        .iter()
        .map(|s| s.to_string())
        .collect();

    Ok(Some(SourceEntry {
        source_type,
        uri,
        suite,
        components,
        options,
        enabled,
    }))
}

fn split_deb822_stanzas(input: &str) -> impl Iterator<Item = &str> {
    input.split("\n\n").map(str::trim).filter(|s| !s.is_empty())
}

fn parse_deb822_stanza(stanza: &str) -> Result<HashMap<String, String>, SourcesError> {
    let mut fields: HashMap<String, String> = HashMap::new();
    let mut current_key: Option<String> = None;
    let mut current_val = String::new();

    for line in stanza.lines() {
        if line.starts_with(' ') || line.starts_with('\t') {
            if current_key.is_some() {
                let stripped = line.trim_start();
                current_val.push('\n');
                if stripped != "." {
                    current_val.push_str(stripped);
                }
            }
        } else if let Some(colon) = line.find(':') {
            if let Some(k) = current_key.take() {
                fields.insert(k, current_val.trim().to_string());
                current_val.clear();
            }
            current_key = Some(line[..colon].trim().to_lowercase());
            current_val = line[colon + 1..].trim().to_string();
        } else if !line.trim().is_empty() {
            return Err(SourcesError::Parse(format!(
                "malformed deb822 line: {:?}",
                line
            )));
        }
    }
    if let Some(k) = current_key {
        fields.insert(k, current_val.trim().to_string());
    }
    Ok(fields)
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    const ONE_LINE_SAMPLE: &str = "\
# Ubuntu main archive
deb http://archive.ubuntu.com/ubuntu focal main restricted universe
deb http://archive.ubuntu.com/ubuntu focal-updates main restricted
deb-src http://archive.ubuntu.com/ubuntu focal main
# deb http://archive.ubuntu.com/ubuntu focal-backports main
";

    const DEB822_SAMPLE: &str = "\
Types: deb deb-src
URIs: http://archive.ubuntu.com/ubuntu
Suites: focal
Components: main restricted
Enabled: yes
Signed-By: /usr/share/keyrings/ubuntu-archive-keyring.gpg

Types: deb
URIs: http://archive.ubuntu.com/ubuntu
Suites: focal-updates
Components: main restricted
Enabled: no
";

    #[test]
    fn parse_standard_ubuntu_sources_list() {
        let sl = SourcesList::parse_one_line(ONE_LINE_SAMPLE).unwrap();
        // 3 enabled + 1 disabled (commented-out backports)
        assert_eq!(sl.entries.len(), 4);
    }

    #[test]
    fn active_entries_excludes_disabled() {
        let sl = SourcesList::parse_one_line(ONE_LINE_SAMPLE).unwrap();
        let active: Vec<_> = sl.active_entries().collect();
        assert_eq!(active.len(), 3);
        assert!(active.iter().all(|e| e.enabled));
    }

    #[test]
    fn disabled_entry_parsed() {
        let sl = SourcesList::parse_one_line(ONE_LINE_SAMPLE).unwrap();
        let disabled: Vec<_> = sl.entries().filter(|e| !e.enabled).collect();
        assert_eq!(disabled.len(), 1);
        assert!(disabled[0].suite.contains("backports"));
    }

    #[test]
    fn source_type_variants_present() {
        let sl = SourcesList::parse_one_line(ONE_LINE_SAMPLE).unwrap();
        assert!(sl.entries().any(|e| e.source_type == SourceType::Binary));
        assert!(sl.entries().any(|e| e.source_type == SourceType::Source));
    }

    #[test]
    fn options_block_parsed() {
        let input = "deb [arch=amd64 signed-by=/path/to/key.gpg] http://example.com focal main\n";
        let sl = SourcesList::parse_one_line(input).unwrap();
        let e = &sl.entries[0];
        assert_eq!(e.options.get("arch"), Some(&"amd64".to_string()));
        assert_eq!(
            e.options.get("signed-by"),
            Some(&"/path/to/key.gpg".to_string())
        );
    }

    #[test]
    fn write_one_line_round_trip() {
        let input = "deb http://archive.ubuntu.com/ubuntu focal main restricted\n";
        let sl = SourcesList::parse_one_line(input).unwrap();
        let out = sl.write_one_line();
        let sl2 = SourcesList::parse_one_line(&out).unwrap();
        assert_eq!(sl2.entries.len(), sl.entries.len());
        assert_eq!(sl2.entries[0].uri, sl.entries[0].uri);
        assert_eq!(sl2.entries[0].suite, sl.entries[0].suite);
    }

    #[test]
    fn write_one_line_disabled_entries_commented() {
        let input = "# deb http://archive.ubuntu.com/ubuntu focal-backports main\n";
        let sl = SourcesList::parse_one_line(input).unwrap();
        let out = sl.write_one_line();
        assert!(
            out.starts_with("# deb"),
            "disabled entry should start with '# deb'; got: {:?}",
            out
        );
    }

    #[test]
    fn parse_deb822_stanza_test() {
        let sl = SourcesList::parse_deb822(DEB822_SAMPLE).unwrap();
        // Stanza 1: 2 types × 1 URI × 1 suite = 2 entries
        // Stanza 2: 1 type × 1 URI × 1 suite  = 1 entry (disabled)
        assert_eq!(sl.entries.len(), 3);
    }

    #[test]
    fn deb822_disabled_excluded_from_active() {
        let sl = SourcesList::parse_deb822(DEB822_SAMPLE).unwrap();
        assert_eq!(sl.active_entries().count(), 2);
    }

    #[test]
    fn deb822_signed_by_in_options() {
        let sl = SourcesList::parse_deb822(DEB822_SAMPLE).unwrap();
        assert_eq!(
            sl.entries[0].options.get("signed-by"),
            Some(&"/usr/share/keyrings/ubuntu-archive-keyring.gpg".to_string())
        );
    }

    #[test]
    fn load_dir_merges_multiple_list_files() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("a.list"),
            "deb http://archive.ubuntu.com/ubuntu focal main\n",
        )
        .unwrap();
        std::fs::write(
            dir.path().join("b.list"),
            "deb http://example.com/repo stable main\n",
        )
        .unwrap();
        let sl = SourcesList::load_dir(dir.path()).unwrap();
        assert_eq!(sl.entries.len(), 2);
    }

    #[test]
    fn release_url_format() {
        let e = SourceEntry {
            source_type: SourceType::Binary,
            uri: "http://archive.ubuntu.com/ubuntu".to_string(),
            suite: "focal".to_string(),
            components: vec!["main".to_string()],
            options: HashMap::new(),
            enabled: true,
        };
        assert_eq!(
            e.release_url(),
            "http://archive.ubuntu.com/ubuntu/dists/focal/InRelease"
        );
    }

    #[test]
    fn uri_validation_accepts_valid_schemes() {
        let e = SourceEntry {
            source_type: SourceType::Binary,
            uri: "https://archive.ubuntu.com/ubuntu".to_string(),
            suite: "focal".to_string(),
            components: vec!["main".to_string()],
            options: HashMap::new(),
            enabled: true,
        };
        assert!(e.is_valid_uri());
        assert!(
            SourcesList::parse_one_line("deb https://example.com focal main")
                .unwrap()
                .validate()
                .is_ok()
        );
    }

    #[test]
    fn uri_validation_rejects_bad_uris() {
        for bad in ["not a uri", "ftp://", "://nohost", "weird://x", ""] {
            let sl = SourcesList::parse_one_line(&format!("deb {} focal main", bad));
            // Parse may succeed but validation must fail (empty/whitespace URIs
            // error at parse time; the rest fail validation).
            let result = match &sl {
                Ok(s) => s.validate(),
                Err(_) => continue,
            };
            assert!(result.is_err(), "expected {:?} to be invalid", bad);
        }
    }

    #[test]
    fn write_deb822_round_trip() {
        let input = "deb http://archive.ubuntu.com/ubuntu focal main restricted\n\
                     deb-src http://archive.ubuntu.com/ubuntu focal main\n";
        let sl = SourcesList::parse_one_line(input).unwrap();
        let deb822 = sl.write_deb822();
        let sl2 = SourcesList::parse_deb822(&deb822).unwrap();
        assert_eq!(sl2.entries.len(), 2);
        assert!(sl2
            .entries
            .iter()
            .any(|e| e.source_type == SourceType::Binary));
        assert!(sl2
            .entries
            .iter()
            .any(|e| e.source_type == SourceType::Source));
    }

    #[test]
    fn write_to_file_round_trips_one_line() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sources.list");
        let input = "deb http://archive.ubuntu.com/ubuntu focal main\n";
        let sl = SourcesList::parse_one_line(input).unwrap();
        sl.write(&path).unwrap();
        let reloaded = SourcesList::load_file(&path).unwrap();
        assert_eq!(reloaded.entries.len(), 1);
        assert_eq!(reloaded.entries[0].suite, "focal");
    }

    #[test]
    fn write_to_file_round_trips_deb822() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("ubuntu.sources");
        let sl = SourcesList::parse_one_line("deb http://archive.ubuntu.com/ubuntu focal main\n")
            .unwrap();
        sl.write(&path).unwrap();
        let reloaded = SourcesList::load_file(&path).unwrap();
        assert_eq!(reloaded.entries.len(), 1);
        assert_eq!(reloaded.entries[0].uri, "http://archive.ubuntu.com/ubuntu");
    }
}
