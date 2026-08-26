//! Debian version comparison implementing the algorithm from Debian Policy §5.6.12.
//!
//! The Debian version format is:
//! ```text
//! [epoch:]upstream_version[-debian_revision]
//! ```
//!
//! - `epoch`: non-negative integer (default 0 when absent)
//! - `upstream_version`: ASCII string, may contain alphanumerics, `.`, `+`, `-`, `~`, `:`
//! - `debian_revision`: the part after the *last* `-`; empty string when absent
//!
//! # Ordering
//!
//! 1. Epochs are compared as unsigned integers.
//! 2. If epochs are equal, `upstream_version` strings are compared using the
//!    Debian string comparison algorithm.
//! 3. If those are also equal, `debian_revision` strings are compared the same way.
//!
//! ## Debian string comparison
//!
//! The string is scanned as alternating runs of *non-digit* and *digit* characters.
//!
//! - Non-digit runs: compared char-by-char with ordering `~` < `""` (end) < letters < other
//! - Digit runs: compared as unsigned integers (leading zeros ignored)

use std::cmp::Ordering;
use std::fmt;
use std::str::FromStr;

use thiserror::Error;

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

// ─── Errors ──────────────────────────────────────────────────────────────────

/// Errors returned when parsing a Debian version string.
#[derive(Debug, Error, PartialEq, Eq, Clone)]
pub enum VersionError {
    /// The version string was empty.
    #[error("version string is empty")]
    EmptyString,

    /// The epoch portion could not be parsed as a non-negative integer.
    #[error("invalid epoch in version string")]
    InvalidEpoch,

    /// The version string contains a character that is not permitted.
    #[error("invalid character '{0}' in version string")]
    InvalidCharacter(char),

    /// The constraint string could not be parsed.
    #[error("invalid version constraint: {0}")]
    InvalidConstraint(String),
}

// ─── Version ─────────────────────────────────────────────────────────────────

/// A parsed Debian package version.
///
/// Versions compare according to the Debian Policy ordering rules.
#[derive(Clone, Debug, Eq, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct Version {
    /// The epoch (default 0 when absent in the original string).
    pub epoch: u32,
    /// The upstream version string.
    pub upstream: String,
    /// The Debian revision string (empty when absent).
    pub revision: String,
}

impl Version {
    /// Parses a Debian version string.
    ///
    /// # Errors
    ///
    /// Returns [`VersionError`] if the string is empty, has an invalid epoch,
    /// or contains characters that are forbidden in version strings.
    pub fn parse(s: &str) -> Result<Self, VersionError> {
        if s.is_empty() {
            return Err(VersionError::EmptyString);
        }

        // Split out epoch
        let (epoch, rest) = if let Some(colon_pos) = s.find(':') {
            let epoch_str = &s[..colon_pos];
            let epoch: u32 = epoch_str.parse().map_err(|_| VersionError::InvalidEpoch)?;
            (epoch, &s[colon_pos + 1..])
        } else {
            (0u32, s)
        };

        if rest.is_empty() {
            return Err(VersionError::EmptyString);
        }

        // Validate characters
        for ch in rest.chars() {
            if !is_valid_version_char(ch) {
                return Err(VersionError::InvalidCharacter(ch));
            }
        }

        // Split on *last* '-' to get revision
        let (upstream, revision) = if let Some(dash_pos) = rest.rfind('-') {
            (&rest[..dash_pos], rest[dash_pos + 1..].to_string())
        } else {
            (rest, String::new())
        };

        Ok(Version {
            epoch,
            upstream: upstream.to_string(),
            revision,
        })
    }
}

/// Returns `true` for characters that are valid in version component strings.
fn is_valid_version_char(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || matches!(ch, '.' | '+' | '-' | '~' | ':')
}

impl fmt::Display for Version {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.epoch != 0 {
            write!(f, "{}:", self.epoch)?;
        }
        write!(f, "{}", self.upstream)?;
        if !self.revision.is_empty() {
            write!(f, "-{}", self.revision)?;
        }
        Ok(())
    }
}

impl FromStr for Version {
    type Err = VersionError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Version::parse(s)
    }
}

impl Ord for Version {
    fn cmp(&self, other: &Self) -> Ordering {
        match self.epoch.cmp(&other.epoch) {
            Ordering::Equal => {}
            ord => return ord,
        }
        match deb_str_cmp(&self.upstream, &other.upstream) {
            Ordering::Equal => {}
            ord => return ord,
        }
        deb_str_cmp(&self.revision, &other.revision)
    }
}

impl PartialOrd for Version {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

// ─── Debian string comparison ─────────────────────────────────────────────────

/// Character weight for the non-digit segment comparison.
/// `~` → -1, end-of-segment → 0, letters → ASCII value (65+), other → 1000+ASCII
fn char_order_or_end(opt: Option<char>) -> i32 {
    match opt {
        None => 0,
        Some('~') => -1,
        Some(ch) if ch.is_ascii_alphabetic() => ch as i32,
        Some(ch) => 1000 + ch as i32,
    }
}

/// Compares two Debian version component strings using the Debian algorithm.
pub fn deb_str_cmp(a: &str, b: &str) -> Ordering {
    let mut a_chars = a.chars().peekable();
    let mut b_chars = b.chars().peekable();

    loop {
        // ── Non-digit segment ──────────────────────────────────────────────
        loop {
            let ac = a_chars.peek().copied();
            let bc = b_chars.peek().copied();

            let a_end = ac.is_none_or(|c| c.is_ascii_digit());
            let b_end = bc.is_none_or(|c| c.is_ascii_digit());

            if a_end && b_end {
                break;
            }

            let aw = char_order_or_end(if a_end { None } else { ac });
            let bw = char_order_or_end(if b_end { None } else { bc });

            match aw.cmp(&bw) {
                Ordering::Equal => {
                    if !a_end {
                        a_chars.next();
                    }
                    if !b_end {
                        b_chars.next();
                    }
                }
                ord => return ord,
            }
        }

        // ── Digit segment ──────────────────────────────────────────────────
        let a_num = collect_digits(&mut a_chars);
        let b_num = collect_digits(&mut b_chars);

        match a_num.cmp(&b_num) {
            Ordering::Equal => {}
            ord => return ord,
        }

        if a_chars.peek().is_none() && b_chars.peek().is_none() {
            return Ordering::Equal;
        }
    }
}

fn collect_digits(chars: &mut std::iter::Peekable<std::str::Chars<'_>>) -> u64 {
    let mut n: u64 = 0;
    while let Some(&c) = chars.peek() {
        if c.is_ascii_digit() {
            n = n * 10 + (c as u64 - b'0' as u64);
            chars.next();
        } else {
            break;
        }
    }
    n
}

// ─── Constraint operator ─────────────────────────────────────────────────────

/// Version relationship operator as used in `Depends:`, `Conflicts:`, etc.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConstraintOp {
    /// `<<` — strictly earlier.
    StrictlyEarlier,
    /// `<=` — earlier or equal.
    EarlierOrEqual,
    /// `=` — exactly equal.
    Equal,
    /// `>=` — later or equal.
    LaterOrEqual,
    /// `>>` — strictly later.
    StrictlyLater,
}

impl ConstraintOp {
    /// Parses a constraint operator string (`<<`, `<=`, `=`, `>=`, `>>`).
    pub fn parse(s: &str) -> Result<Self, VersionError> {
        match s.trim() {
            "<<" => Ok(ConstraintOp::StrictlyEarlier),
            "<=" => Ok(ConstraintOp::EarlierOrEqual),
            "=" => Ok(ConstraintOp::Equal),
            ">=" => Ok(ConstraintOp::LaterOrEqual),
            ">>" => Ok(ConstraintOp::StrictlyLater),
            other => Err(VersionError::InvalidConstraint(format!(
                "unknown operator '{}'",
                other
            ))),
        }
    }
}

impl fmt::Display for ConstraintOp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            ConstraintOp::StrictlyEarlier => "<<",
            ConstraintOp::EarlierOrEqual => "<=",
            ConstraintOp::Equal => "=",
            ConstraintOp::LaterOrEqual => ">=",
            ConstraintOp::StrictlyLater => ">>",
        })
    }
}

// ─── Version constraint ───────────────────────────────────────────────────────

/// A version constraint of the form `<op> <version>`, e.g. `>= 2.0`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VersionConstraint {
    /// The comparison operator.
    pub op: ConstraintOp,
    /// The reference version.
    pub version: Version,
}

impl VersionConstraint {
    /// Parses a constraint string such as `">= 2:1.0-1"`.
    ///
    /// The operator and version must be adjacent or separated by whitespace.
    pub fn parse(s: &str) -> Result<Self, VersionError> {
        let s = s.trim();
        let (op_str, ver_str) = if s.starts_with("<<")
            || s.starts_with("<=")
            || s.starts_with(">=")
            || s.starts_with(">>")
        {
            (&s[..2], s[2..].trim_start())
        } else if let Some(ver) = s.strip_prefix('=') {
            ("=", ver.trim_start())
        } else {
            return Err(VersionError::InvalidConstraint(format!(
                "cannot parse constraint '{}'",
                s
            )));
        };

        let op = ConstraintOp::parse(op_str)?;
        let version = Version::parse(ver_str)?;
        Ok(VersionConstraint { op, version })
    }

    /// Returns `true` if the given installed version satisfies this constraint.
    pub fn satisfies(&self, installed: &Version) -> bool {
        let ord = installed.cmp(&self.version);
        match self.op {
            ConstraintOp::StrictlyEarlier => ord == Ordering::Less,
            ConstraintOp::EarlierOrEqual => ord != Ordering::Greater,
            ConstraintOp::Equal => ord == Ordering::Equal,
            ConstraintOp::LaterOrEqual => ord != Ordering::Less,
            ConstraintOp::StrictlyLater => ord == Ordering::Greater,
        }
    }
}

impl fmt::Display for VersionConstraint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} {}", self.op, self.version)
    }
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn v(s: &str) -> Version {
        Version::parse(s).unwrap_or_else(|e| panic!("parse({:?}): {}", s, e))
    }

    // ── Parsing ──────────────────────────────────────────────────────────────

    #[test]
    fn parse_simple() {
        let ver = v("1.0");
        assert_eq!(ver.epoch, 0);
        assert_eq!(ver.upstream, "1.0");
        assert_eq!(ver.revision, "");
    }

    #[test]
    fn parse_with_revision() {
        let ver = v("1.0-1");
        assert_eq!(ver.epoch, 0);
        assert_eq!(ver.upstream, "1.0");
        assert_eq!(ver.revision, "1");
    }

    #[test]
    fn parse_with_epoch() {
        let ver = v("2:3.4-5");
        assert_eq!(ver.epoch, 2);
        assert_eq!(ver.upstream, "3.4");
        assert_eq!(ver.revision, "5");
    }

    #[test]
    fn parse_epoch_only_no_revision() {
        let ver = v("1:1.0");
        assert_eq!(ver.epoch, 1);
        assert_eq!(ver.upstream, "1.0");
        assert_eq!(ver.revision, "");
    }

    #[test]
    fn parse_empty_returns_error() {
        assert!(matches!(Version::parse(""), Err(VersionError::EmptyString)));
    }

    #[test]
    fn parse_invalid_epoch() {
        assert!(matches!(
            Version::parse("abc:1.0"),
            Err(VersionError::InvalidEpoch)
        ));
    }

    #[test]
    fn parse_invalid_character() {
        assert!(matches!(
            Version::parse("1.0!foo"),
            Err(VersionError::InvalidCharacter('!'))
        ));
    }

    // ── Display ──────────────────────────────────────────────────────────────

    #[test]
    fn display_no_epoch_no_revision() {
        assert_eq!(v("1.2.3").to_string(), "1.2.3");
    }

    #[test]
    fn display_with_epoch_and_revision() {
        assert_eq!(v("2:1.0-3").to_string(), "2:1.0-3");
    }

    // ── Epoch ordering ────────────────────────────────────────────────────────

    #[test]
    fn epoch_1_beats_epoch_0() {
        assert!(v("1:1.0") > v("1.0"));
    }

    #[test]
    fn higher_epoch_wins_regardless_of_upstream() {
        assert!(v("2:0.1") > v("1:99.99"));
    }

    // ── Tilde ordering ────────────────────────────────────────────────────────

    #[test]
    fn tilde_sorts_before_plain() {
        assert!(v("1.0~beta1") < v("1.0"));
    }

    #[test]
    fn tilde_less_than_empty_revision() {
        assert!(v("1.0~beta") < v("1.0"));
        assert!(v("1.0") < v("1.0-1"));
    }

    #[test]
    fn double_tilde_sorts_before_single() {
        assert!(v("1.0~~") < v("1.0~"));
    }

    // ── Non-numeric segment ordering ─────────────────────────────────────────

    #[test]
    fn letters_sort_before_other_chars() {
        // 'a' (alpha, weight 97) < '+' (other, weight 1000+43)
        assert!(v("1.0a") < v("1.0+"));
    }

    #[test]
    fn longer_non_digit_segment_after_shorter() {
        // "1.0ab" has 'b' vs end: 'b'(98) > END(0)
        assert!(v("1.0a") < v("1.0ab"));
    }

    // ── Digit segment ordering ────────────────────────────────────────────────

    #[test]
    fn leading_zeros_ignored_in_digit_segment() {
        // Debian compares digit runs as integers: 01 == 1.
        // The structs are not structurally equal, so compare via Ord.
        assert_eq!(v("1.01").cmp(&v("1.1")), Ordering::Equal);
    }

    #[test]
    fn larger_digit_segment_wins() {
        assert!(v("1.10") > v("1.9"));
    }

    // ── Revision ordering ─────────────────────────────────────────────────────

    #[test]
    fn revision_comparison() {
        assert!(v("1.0-2") > v("1.0-1"));
        assert!(v("1.0-1") < v("1.0-2"));
    }

    // ── Constraint parsing ────────────────────────────────────────────────────

    #[test]
    fn constraint_strictly_earlier() {
        let c = VersionConstraint::parse("<< 2.0").unwrap();
        assert!(c.satisfies(&v("1.9")));
        assert!(!c.satisfies(&v("2.0")));
        assert!(!c.satisfies(&v("2.1")));
    }

    #[test]
    fn constraint_earlier_or_equal() {
        let c = VersionConstraint::parse("<= 2.0").unwrap();
        assert!(c.satisfies(&v("1.9")));
        assert!(c.satisfies(&v("2.0")));
        assert!(!c.satisfies(&v("2.1")));
    }

    #[test]
    fn constraint_equal() {
        let c = VersionConstraint::parse("= 2.0").unwrap();
        assert!(!c.satisfies(&v("1.9")));
        assert!(c.satisfies(&v("2.0")));
        assert!(!c.satisfies(&v("2.1")));
    }

    #[test]
    fn constraint_later_or_equal() {
        let c = VersionConstraint::parse(">= 2.0").unwrap();
        assert!(!c.satisfies(&v("1.9")));
        assert!(c.satisfies(&v("2.0")));
        assert!(c.satisfies(&v("2.1")));
    }

    #[test]
    fn constraint_strictly_later() {
        let c = VersionConstraint::parse(">> 2.0").unwrap();
        assert!(!c.satisfies(&v("1.9")));
        assert!(!c.satisfies(&v("2.0")));
        assert!(c.satisfies(&v("2.1")));
    }

    #[test]
    fn constraint_with_epoch() {
        let c = VersionConstraint::parse(">= 1:2.0-1").unwrap();
        assert!(c.satisfies(&v("1:2.0-1")));
        assert!(!c.satisfies(&v("2.0-1")));
    }

    #[test]
    fn constraint_display() {
        let c = VersionConstraint::parse(">= 1.0").unwrap();
        assert_eq!(c.to_string(), ">= 1.0");
    }

    #[test]
    fn constraint_invalid_op() {
        assert!(VersionConstraint::parse("~~ 1.0").is_err());
    }
}
