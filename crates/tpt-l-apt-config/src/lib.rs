//! Parser for APT configuration files (`apt.conf` and `apt.conf.d/`).
//!
//! APT configuration uses a hierarchical key-value format with two syntaxes
//! that are equivalent:
//!
//! ```text
//! // Nested scope syntax
//! APT {
//!   Get {
//!     Assume-Yes "true";
//!   };
//! };
//!
//! // Flat :: syntax
//! APT::Get::Assume-Yes "true";
//! ```
//!
//! This crate stores configuration as a flat `HashMap` keyed by `::` paths.
//! Lists are represented via the `::` index suffix used by APT (`Key::0`,
//! `Key::1`, …) or stored as a [`ConfigValue::List`].

use std::collections::HashMap;
use std::path::Path;
use thiserror::Error;

// ── Error ─────────────────────────────────────────────────────────────────────

/// Errors produced by the APT config parser.
#[derive(Debug, Error)]
pub enum ConfigError {
    /// An I/O error occurred.
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    /// A parse error was encountered.
    #[error("parse error: {0}")]
    Parse(String),
    /// Maximum `#include` depth exceeded (cycle guard).
    #[error("maximum include depth exceeded; possible cycle in {0:?}")]
    IncludeDepth(String),
}

// ── ConfigValue ───────────────────────────────────────────────────────────────

/// A configuration value — either a single string or a list of strings.
#[derive(Debug, Clone)]
pub enum ConfigValue {
    /// A single string value.
    String(String),
    /// An ordered list of string values.
    List(Vec<String>),
}

impl ConfigValue {
    /// Returns the first/only string value, if this is a `String` variant.
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Self::String(s) => Some(s.as_str()),
            Self::List(_) => None,
        }
    }

    /// Returns the list of values, if this is a `List` variant.
    pub fn as_list(&self) -> Option<&[String]> {
        match self {
            Self::List(v) => Some(v.as_slice()),
            Self::String(_) => None,
        }
    }
}

// ── AptConfig ─────────────────────────────────────────────────────────────────

/// Parsed APT configuration.
///
/// Keys use the `::` hierarchy separator, e.g. `APT::Get::Assume-Yes`.
#[derive(Debug, Default, Clone)]
pub struct AptConfig {
    pub(crate) values: HashMap<String, ConfigValue>,
}

impl AptConfig {
    /// Create an empty configuration.
    pub fn new() -> Self {
        Self::default()
    }

    // ── Loading ───────────────────────────────────────────────────────────────

    /// Parse one file at `path` without resolving any `#include` directives.
    pub fn load(path: &Path) -> Result<Self, ConfigError> {
        let content = std::fs::read_to_string(path)?;
        let mut cfg = Self::new();
        parse_into(&content, "", &mut cfg)?;
        Ok(cfg)
    }

    /// Parse one file at `path`, resolving `#include` and `#include-dir`
    /// directives up to `max_depth` levels deep (default 10, capped at 10).
    pub fn load_with_includes(path: &Path) -> Result<Self, ConfigError> {
        let mut cfg = Self::new();
        load_with_includes_inner(path, 0, 10, &mut cfg)?;
        Ok(cfg)
    }

    /// Scan `dir` alphabetically for files and merge all of them in order.
    ///
    /// Files with any name are accepted; APT itself only reads certain
    /// extensions, but this method is not restricted.
    pub fn load_dir(dir: &Path) -> Result<Self, ConfigError> {
        let mut cfg = Self::new();
        let mut paths: Vec<_> = std::fs::read_dir(dir)?
            .filter_map(|e| e.ok())
            .filter(|e| e.path().is_file())
            .map(|e| e.path())
            .collect();
        paths.sort();
        for path in paths {
            let content = std::fs::read_to_string(&path)?;
            parse_into(&content, "", &mut cfg)?;
        }
        Ok(cfg)
    }

    // ── Querying ──────────────────────────────────────────────────────────────

    /// Get a string value by key. Returns `None` if absent or if the value
    /// is a list.
    pub fn get(&self, key: &str) -> Option<&str> {
        self.values.get(key).and_then(|v| v.as_str())
    }

    /// Parse a boolean value at `key`.
    ///
    /// `"true"`, `"yes"`, and `"1"` (case-insensitive) are `true`;
    /// `"false"`, `"no"`, and `"0"` are `false`.  Other values return `None`.
    pub fn get_bool(&self, key: &str) -> Option<bool> {
        match self.get(key)?.to_lowercase().trim() {
            "true" | "yes" | "1" => Some(true),
            "false" | "no" | "0" => Some(false),
            _ => None,
        }
    }

    /// Get a string value, falling back to `default` if absent.
    pub fn get_or_default<'a>(&'a self, key: &str, default: &'a str) -> &'a str {
        self.get(key).unwrap_or(default)
    }

    /// Parse an integer value at `key` (decimal, with surrounding whitespace
    /// ignored). Returns `None` if the key is absent or not a valid integer.
    pub fn get_int(&self, key: &str) -> Option<i64> {
        self.get(key)?.trim().parse::<i64>().ok()
    }

    /// Get a list value at `key`.  Returns an empty slice if absent or if the
    /// value is a scalar string.
    pub fn get_list(&self, key: &str) -> &[String] {
        self.values
            .get(key)
            .and_then(|v| v.as_list())
            .unwrap_or(&[])
    }

    /// Merge `other` into this config.  When the same key appears in both,
    /// `other`'s value wins.
    pub fn merge(&mut self, other: AptConfig) {
        self.values.extend(other.values);
    }

    // ── Convenience shortcuts ─────────────────────────────────────────────────

    /// The configured APT sources.list path.
    pub fn sources_list_path(&self) -> &str {
        self.get_or_default("Dir::Etc::SourceList", "/etc/apt/sources.list")
    }

    /// The configured dpkg status database path.
    pub fn status_db_path(&self) -> &str {
        self.get_or_default("Dir::State::status", "/var/lib/dpkg/status")
    }

    // ── Direct insertion (used by the parser) ─────────────────────────────────

    /// Set a key to a string value.
    pub fn set(&mut self, key: &str, value: impl Into<String>) {
        self.values
            .insert(key.to_string(), ConfigValue::String(value.into()));
    }

    /// Append a value to a list at `key`.
    pub fn push_list(&mut self, key: &str, value: impl Into<String>) {
        match self
            .values
            .entry(key.to_string())
            .or_insert_with(|| ConfigValue::List(Vec::new()))
        {
            ConfigValue::List(v) => v.push(value.into()),
            ConfigValue::String(existing) => {
                let s = std::mem::take(existing);
                *self.values.get_mut(key).unwrap() = ConfigValue::List(vec![s, value.into()]);
            }
        }
    }
}

// ── Parser ────────────────────────────────────────────────────────────────────

/// Recursive include loader (inner).
fn load_with_includes_inner(
    path: &Path,
    depth: usize,
    max_depth: usize,
    cfg: &mut AptConfig,
) -> Result<(), ConfigError> {
    if depth >= max_depth {
        return Err(ConfigError::IncludeDepth(path.display().to_string()));
    }
    let content = std::fs::read_to_string(path)?;
    parse_into_with_includes(
        &content,
        "",
        cfg,
        path.parent().unwrap_or(Path::new(".")),
        depth,
        max_depth,
    )
}

/// Parse the body of an APT config file into `cfg`, with include resolution.
fn parse_into_with_includes(
    content: &str,
    prefix: &str,
    cfg: &mut AptConfig,
    base_dir: &Path,
    depth: usize,
    max_depth: usize,
) -> Result<(), ConfigError> {
    let mut p = Parser::new(content);
    p.parse_block(prefix, cfg)?;

    // Process deferred includes (collected during parse).
    for inc in p.includes {
        let target = if Path::new(&inc.path).is_absolute() {
            std::path::PathBuf::from(&inc.path)
        } else {
            base_dir.join(&inc.path)
        };

        if inc.is_dir {
            // `#include-dir`
            if target.is_dir() {
                let mut paths: Vec<_> = std::fs::read_dir(&target)?
                    .filter_map(|e| e.ok())
                    .filter(|e| e.path().is_file())
                    .map(|e| e.path())
                    .collect();
                paths.sort();
                for p in paths {
                    load_with_includes_inner(&p, depth + 1, max_depth, cfg)?;
                }
            }
        } else {
            // `#include`
            if target.exists() {
                load_with_includes_inner(&target, depth + 1, max_depth, cfg)?;
            }
        }
    }

    Ok(())
}

/// Parse an APT config string into `cfg` (no include resolution).
fn parse_into(content: &str, prefix: &str, cfg: &mut AptConfig) -> Result<(), ConfigError> {
    let mut p = Parser::new(content);
    p.parse_block(prefix, cfg)
}

// ── Internal parser ───────────────────────────────────────────────────────────

struct IncludeDirective {
    path: String,
    is_dir: bool,
}

struct Parser {
    chars: Vec<char>,
    pos: usize,
    line: usize,
    pub includes: Vec<IncludeDirective>,
}

impl Parser {
    fn new(s: &str) -> Self {
        Self {
            chars: s.chars().collect(),
            pos: 0,
            line: 1,
            includes: Vec::new(),
        }
    }

    fn eof(&self) -> bool {
        self.pos >= self.chars.len()
    }

    fn current(&self) -> char {
        if self.eof() {
            '\0'
        } else {
            self.chars[self.pos]
        }
    }

    fn peek_ahead(&self, n: usize) -> char {
        let i = self.pos + n;
        if i >= self.chars.len() {
            '\0'
        } else {
            self.chars[i]
        }
    }

    fn advance(&mut self) -> char {
        let c = self.chars[self.pos];
        self.pos += 1;
        if c == '\n' {
            self.line += 1;
        }
        c
    }

    fn skip_whitespace(&mut self) {
        while !self.eof() && self.current().is_ascii_whitespace() {
            self.advance();
        }
    }

    fn skip_line_comment(&mut self) {
        while !self.eof() && self.current() != '\n' {
            self.advance();
        }
    }

    fn skip_block_comment(&mut self) -> Result<(), ConfigError> {
        // We've just consumed `/*`; consume until `*/`.
        loop {
            if self.eof() {
                return Err(ConfigError::Parse(format!(
                    "unterminated block comment at line {}",
                    self.line
                )));
            }
            if self.current() == '*' && self.peek_ahead(1) == '/' {
                self.advance(); // *
                self.advance(); // /
                return Ok(());
            }
            self.advance();
        }
    }

    fn skip_whitespace_and_comments(&mut self) -> Result<(), ConfigError> {
        loop {
            self.skip_whitespace();
            if self.eof() {
                break;
            }
            if self.current() == '/' && self.peek_ahead(1) == '/' {
                self.skip_line_comment();
            } else if self.current() == '/' && self.peek_ahead(1) == '*' {
                self.advance(); // /
                self.advance(); // *
                self.skip_block_comment()?;
            } else {
                break;
            }
        }
        Ok(())
    }

    /// Read a key segment: letters, digits, `-`, `_`.
    fn read_key_segment(&mut self) -> String {
        let mut s = String::new();
        while !self.eof() {
            let c = self.current();
            if c.is_alphanumeric() || c == '-' || c == '_' || c == '.' {
                s.push(c);
                self.advance();
            } else {
                break;
            }
        }
        s
    }

    /// Read a quoted string `"…"` with basic escape handling.
    fn read_string(&mut self) -> Result<String, ConfigError> {
        if self.current() != '"' {
            return Err(ConfigError::Parse(format!(
                "expected '\"' at line {}",
                self.line
            )));
        }
        self.advance(); // consume opening "
        let mut s = String::new();
        loop {
            if self.eof() {
                return Err(ConfigError::Parse(format!(
                    "unterminated string at line {}",
                    self.line
                )));
            }
            match self.current() {
                '"' => {
                    self.advance();
                    return Ok(s);
                }
                '\\' => {
                    self.advance();
                    if !self.eof() {
                        s.push(self.advance());
                    }
                }
                c => {
                    s.push(c);
                    self.advance();
                }
            }
        }
    }

    fn expect_char(&mut self, expected: char) -> Result<(), ConfigError> {
        if self.current() == expected {
            self.advance();
            Ok(())
        } else {
            Err(ConfigError::Parse(format!(
                "expected {:?} but got {:?} at line {}",
                expected,
                self.current(),
                self.line
            )))
        }
    }

    /// Read a `#include "path"` or `#include-dir "path"` directive.
    fn read_directive(&mut self) -> Result<(), ConfigError> {
        // We're positioned at `#`.
        self.advance(); // consume `#`
        let word = self.read_key_segment();
        let is_dir = word.eq_ignore_ascii_case("include-dir");
        if !word.eq_ignore_ascii_case("include") && !is_dir {
            // Not a directive we recognise — treat rest of line as comment.
            self.skip_line_comment();
            return Ok(());
        }
        self.skip_whitespace();
        let path = self.read_string()?;
        self.skip_whitespace();
        // Optional trailing `;`
        if self.current() == ';' {
            self.advance();
        }
        self.includes.push(IncludeDirective { path, is_dir });
        Ok(())
    }

    /// Parse a block of statements into `cfg`, stopping at EOF or `}`.
    fn parse_block(&mut self, prefix: &str, cfg: &mut AptConfig) -> Result<(), ConfigError> {
        loop {
            self.skip_whitespace_and_comments()?;
            if self.eof() || self.current() == '}' {
                break;
            }

            // Directive starting with `#`
            if self.current() == '#' {
                self.read_directive()?;
                continue;
            }

            // Read the key (possibly including `::` segments).
            let seg = self.read_key_segment();
            if seg.is_empty() {
                return Err(ConfigError::Parse(format!(
                    "unexpected character {:?} at line {}",
                    self.current(),
                    self.line
                )));
            }

            // Accumulate `::` chain.
            let mut key_parts = vec![seg];
            let mut trailing_sep = false;
            loop {
                self.skip_whitespace();
                if self.current() == ':' && self.peek_ahead(1) == ':' {
                    self.advance(); // :
                    self.advance(); // :
                    self.skip_whitespace();
                    let seg = self.read_key_segment();
                    if seg.is_empty() {
                        // `::` followed directly by a value (e.g. `Key:: "v";`)
                        // is APT's list-append syntax.  Flag it and let the
                        // value parser below append to the list.
                        trailing_sep = true;
                        break;
                    }
                    key_parts.push(seg);
                } else {
                    break;
                }
            }

            let local_key = key_parts.join("::");
            let full_key = if prefix.is_empty() {
                local_key.clone()
            } else {
                format!("{}::{}", prefix, local_key)
            };

            self.skip_whitespace();

            if trailing_sep {
                // `Key:: "value";` — append to the list at `full_key`.
                if self.current() != '"' {
                    return Err(ConfigError::Parse(format!(
                        "expected '\"' after '::' for key {:?} at line {}",
                        full_key, self.line
                    )));
                }
                let value = self.read_string()?;
                self.skip_whitespace();
                self.expect_char(';')?;
                cfg.push_list(&full_key, value);
                continue;
            }

            match self.current() {
                '{' => {
                    // Nested scope block.
                    self.advance(); // consume `{`
                    self.parse_block(&full_key, cfg)?;
                    self.skip_whitespace_and_comments()?;
                    self.expect_char('}')?;
                    self.skip_whitespace();
                    // Consume optional `;`
                    if self.current() == ';' {
                        self.advance();
                    }
                }
                '"' => {
                    // String value.
                    let value = self.read_string()?;
                    self.skip_whitespace();
                    self.expect_char(';')?;
                    cfg.set(&full_key, value);
                }
                _ => {
                    return Err(ConfigError::Parse(format!(
                        "expected '{{' or '\"' after key {:?} at line {}",
                        full_key, self.line
                    )));
                }
            }
        }
        Ok(())
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_flat_key_syntax() {
        let input = r#"APT::Get::Assume-Yes "true";"#;
        let mut cfg = AptConfig::new();
        parse_into(input, "", &mut cfg).unwrap();
        assert_eq!(cfg.get("APT::Get::Assume-Yes"), Some("true"));
    }

    #[test]
    fn parse_nested_scope_block() {
        let input = r#"
APT {
  Get {
    Assume-Yes "true";
    List-Cleanup "false";
  };
  Cache {
    Dir "/var/cache/apt";
  };
};
"#;
        let mut cfg = AptConfig::new();
        parse_into(input, "", &mut cfg).unwrap();
        assert_eq!(cfg.get("APT::Get::Assume-Yes"), Some("true"));
        assert_eq!(cfg.get("APT::Get::List-Cleanup"), Some("false"));
        assert_eq!(cfg.get("APT::Cache::Dir"), Some("/var/cache/apt"));
    }

    #[test]
    fn parse_mixed_flat_and_nested() {
        let input = r#"
APT::Acquire::Retries "3";
Dir {
  Etc {
    SourceList "/etc/apt/sources.list";
  };
};
"#;
        let mut cfg = AptConfig::new();
        parse_into(input, "", &mut cfg).unwrap();
        assert_eq!(cfg.get("APT::Acquire::Retries"), Some("3"));
        assert_eq!(
            cfg.get("Dir::Etc::SourceList"),
            Some("/etc/apt/sources.list")
        );
    }

    #[test]
    fn parse_line_comment() {
        let input = r#"
// this is a comment
APT::Get::Assume-Yes "true"; // inline comment ignored (not parsed here)
"#;
        let mut cfg = AptConfig::new();
        parse_into(input, "", &mut cfg).unwrap();
        assert_eq!(cfg.get("APT::Get::Assume-Yes"), Some("true"));
    }

    #[test]
    fn get_bool_true_variants() {
        let mut cfg = AptConfig::new();
        cfg.set("A", "true");
        cfg.set("B", "yes");
        cfg.set("C", "1");
        assert_eq!(cfg.get_bool("A"), Some(true));
        assert_eq!(cfg.get_bool("B"), Some(true));
        assert_eq!(cfg.get_bool("C"), Some(true));
    }

    #[test]
    fn get_bool_false_variants() {
        let mut cfg = AptConfig::new();
        cfg.set("A", "false");
        cfg.set("B", "no");
        cfg.set("C", "0");
        assert_eq!(cfg.get_bool("A"), Some(false));
        assert_eq!(cfg.get_bool("B"), Some(false));
        assert_eq!(cfg.get_bool("C"), Some(false));
    }

    #[test]
    fn get_bool_missing_returns_none() {
        let cfg = AptConfig::new();
        assert_eq!(cfg.get_bool("nonexistent"), None);
    }

    #[test]
    fn get_or_default_falls_back() {
        let cfg = AptConfig::new();
        assert_eq!(
            cfg.get_or_default("Dir::Etc::SourceList", "/etc/apt/sources.list"),
            "/etc/apt/sources.list"
        );
    }

    #[test]
    fn get_or_default_returns_value_when_present() {
        let mut cfg = AptConfig::new();
        cfg.set("Key", "custom");
        assert_eq!(cfg.get_or_default("Key", "default"), "custom");
    }

    #[test]
    fn merge_other_wins_on_conflict() {
        let mut base = AptConfig::new();
        base.set("Key", "base_value");

        let mut other = AptConfig::new();
        other.set("Key", "other_value");
        other.set("New", "new_value");

        base.merge(other);
        assert_eq!(base.get("Key"), Some("other_value"));
        assert_eq!(base.get("New"), Some("new_value"));
    }

    #[test]
    fn load_with_includes_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let main_path = dir.path().join("apt.conf");
        std::fs::write(
            &main_path,
            r#"APT::Get::Assume-Yes "true";
"#,
        )
        .unwrap();
        let cfg = AptConfig::load(&main_path).unwrap();
        assert_eq!(cfg.get("APT::Get::Assume-Yes"), Some("true"));
    }

    #[test]
    fn load_dir_merges_files_alphabetically() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("00-base"), r#"Base "value";"#).unwrap();
        std::fs::write(dir.path().join("10-override"), r#"Base "override";"#).unwrap();
        let cfg = AptConfig::load_dir(dir.path()).unwrap();
        assert_eq!(cfg.get("Base"), Some("override"));
    }

    #[test]
    fn get_int_parses_integer() {
        let mut cfg = AptConfig::new();
        cfg.set("APT::Acquire::Retries", "3");
        assert_eq!(cfg.get_int("APT::Acquire::Retries"), Some(3));
        assert_eq!(cfg.get_int("missing"), None);
        cfg.set("Bad", "not-a-number");
        assert_eq!(cfg.get_int("Bad"), None);
    }

    #[test]
    fn list_append_syntax_builds_list() {
        let input = r#"
Acquire::http::Proxy:: "http://proxy1:3128";
Acquire::http::Proxy:: "http://proxy2:3128";
"#;
        let mut cfg = AptConfig::new();
        parse_into(input, "", &mut cfg).unwrap();
        let list = cfg.get_list("Acquire::http::Proxy");
        assert_eq!(list.len(), 2);
        assert_eq!(list[0], "http://proxy1:3128");
        assert_eq!(list[1], "http://proxy2:3128");
    }

    #[test]
    fn scalar_and_list_keys_are_distinct() {
        let mut cfg = AptConfig::new();
        parse_into(r#"Key "scalar"; Key:: "a"; Key:: "b";"#, "", &mut cfg).unwrap();
        // A `Key "x"` scalar followed by `Key::` appends becomes a list.
        assert_eq!(cfg.get("Key"), None);
        let list = cfg.get_list("Key");
        assert_eq!(list.len(), 3);
        assert_eq!(list[0], "scalar");
        assert_eq!(list[1], "a");
        assert_eq!(list[2], "b");
    }
}
