//! Minimal `Release` / `InRelease` parser.
//!
//! Enough to read the per-file SHA-256 table so callers can locate a
//! `Packages` index's target hash (for PDiff verification) and its
//! `*.diff/Index` path. Only the `SHA256:` section is consulted; other hash
//! sections (`SHA512:`, `MD5Sum:`) are ignored.
//!
//! The parser is deliberately lenient: lines in a hash section that do not
//! look like `<hash> <size> <path>` (such as the trailing PGP signature block
//! of an `InRelease` clearsigned file) are skipped rather than rejected, so an
//! `InRelease` can be parsed directly without first stripping the signature.

use std::collections::HashMap;

use thiserror::Error;

/// Errors from parsing a `Release` file.
#[derive(Debug, Error)]
pub enum ReleaseError {
    /// A hash line could not be interpreted.
    #[error("malformed Release hash line: {0}")]
    Malformed(String),
    /// A requested file was not present in the index.
    #[error("file not found in Release: {0}")]
    FileNotFound(String),
}

/// A single file entry from a `Release` hash section.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReleaseFile {
    /// Path relative to `dists/<suite>/` (e.g. `main/binary-amd64/Packages`).
    pub path: String,
    /// Uncompressed size in bytes.
    pub size: u64,
    /// Lowercase hex SHA-256.
    pub sha256: String,
}

/// A parsed `Release` / `InRelease` document.
#[derive(Debug, Clone, Default)]
pub struct ReleaseIndex {
    /// Files keyed by their `dists/<suite>`-relative path.
    pub files: HashMap<String, ReleaseFile>,
}

impl ReleaseIndex {
    /// Parse a `Release` document (the inner text of a clearsigned `InRelease`
    /// or a plain `Release` file).
    pub fn parse(text: &str) -> Result<Self, ReleaseError> {
        let mut files = HashMap::new();

        // Skip the leading header block (everything before the first blank
        // line). For a clearsigned InRelease this discards the
        // `-----BEGIN PGP SIGNED MESSAGE-----` / `Hash:` armor.
        let mut lines = text.lines().peekable();
        while lines.peek().map(|l| !l.trim().is_empty()).unwrap_or(false) {
            lines.next();
        }
        // Consume the separating blank line.
        if lines.peek().map(|l| l.trim().is_empty()).unwrap_or(false) {
            lines.next();
        }

        let mut in_sha256 = false;
        for line in lines {
            let line = line.trim_end();
            if line.trim().is_empty() {
                in_sha256 = false;
                continue;
            }
            // A section header like `SHA256:` (and only that).
            if line.ends_with(':') && !line.contains(' ') {
                in_sha256 = line.trim_end_matches(':').eq_ignore_ascii_case("SHA256");
                continue;
            }
            if !in_sha256 {
                continue;
            }
            // `<sha256> <size> <path>` — tolerate trailing whitespace/columns.
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() < 3 {
                continue;
            }
            let size = match parts[1].parse::<u64>() {
                Ok(s) => s,
                Err(_) => continue,
            };
            files.insert(
                parts[2].to_string(),
                ReleaseFile {
                    path: parts[2].to_string(),
                    size,
                    sha256: parts[0].to_ascii_lowercase(),
                },
            );
        }

        Ok(Self { files })
    }

    /// SHA-256 of the uncompressed `Packages` index for `component`/`arch`.
    pub fn packages_sha256(&self, component: &str, arch: &str) -> Option<&str> {
        let key = format!("{component}/binary-{arch}/Packages");
        self.files.get(&key).map(|f| f.sha256.as_str())
    }

    /// Path (relative to `dists/<suite>/`) of the `*.diff/Index` for
    /// `component`/`arch`, if the archive publishes one.
    pub fn packages_diff_index_path(&self, component: &str, arch: &str) -> Option<&str> {
        let key = format!("{component}/binary-{arch}/Packages.diff/Index");
        self.files.get(&key).map(|f| f.path.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RELEASE: &str = "\
Origin: Debian
Label: Debian
Suite: stable
Components: main
Architectures: amd64

SHA256:
 aaaa1111 1234 main/binary-amd64/Packages
 bbbb2222 5678 main/binary-amd64/Packages.diff/Index
 cccc3333 99   main/source/Sources

SHA512:
 dddd4444 9999 main/binary-amd64/Packages
";

    #[test]
    fn parses_sha256_section() {
        let idx = ReleaseIndex::parse(RELEASE).unwrap();
        let f = idx.files.get("main/binary-amd64/Packages").unwrap();
        assert_eq!(f.sha256, "aaaa1111");
        assert_eq!(f.size, 1234);
        // SHA512 section is ignored.
        assert!(!idx.files.contains_key("main/binary-amd64/Packages.gz"));
    }

    #[test]
    fn looks_up_packages_and_diff() {
        let idx = ReleaseIndex::parse(RELEASE).unwrap();
        assert_eq!(idx.packages_sha256("main", "amd64"), Some("aaaa1111"));
        assert_eq!(
            idx.packages_diff_index_path("main", "amd64"),
            Some("main/binary-amd64/Packages.diff/Index")
        );
        assert_eq!(idx.packages_sha256("nope", "amd64"), None);
    }

    #[test]
    fn parses_clearsigned_inrelease() {
        let inrelease = "\
-----BEGIN PGP SIGNED MESSAGE-----
Hash: SHA256

SHA256:
 eeee5555 42 main/binary-amd64/Packages

-----BEGIN PGP SIGNATURE-----
fakebase64
-----END PGP SIGNATURE-----
";
        let idx = ReleaseIndex::parse(inrelease).unwrap();
        assert_eq!(idx.packages_sha256("main", "amd64"), Some("eeee5555"));
    }
}
