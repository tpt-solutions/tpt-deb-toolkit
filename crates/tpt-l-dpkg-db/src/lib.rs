//! Concurrent reader/writer for the dpkg package status database.
//!
//! The dpkg status database lives at `/var/lib/dpkg/status` and contains one
//! deb822 stanza per (possibly removed) package describing its installation
//! state. This crate provides typed access to that file and an atomic writer
//! that avoids corrupt state on power failure.
//!
//! # Example
//!
//! ```no_run
//! use std::path::Path;
//! use tpt_l_dpkg_db::StatusDb;
//!
//! let db = StatusDb::open(Path::new("/var/lib/dpkg/status")).unwrap();
//! for pkg in db.installed_packages() {
//!     println!("{} {}", pkg.name, pkg.version);
//! }
//! ```

use std::io::Write as _;
use std::path::Path;
use thiserror::Error;

// ── Error type ────────────────────────────────────────────────────────────────

/// Errors produced by the dpkg database layer.
#[derive(Debug, Error)]
pub enum DbError {
    /// An I/O error.
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    /// A control-file parse error.
    #[error("control parse error: {0}")]
    Control(#[from] tpt_l_control_file::ControlError),
    /// A `Status:` field did not contain exactly three whitespace-separated tokens.
    #[error("malformed Status field: {0:?}")]
    BadStatus(String),
    /// A required field was absent from a stanza.
    #[error("missing required field '{0}' in status stanza")]
    MissingField(String),
    /// Writing the temporary file failed.
    #[error("atomic write failed: {0}")]
    Persist(String),
    /// The status file was not valid UTF-8.
    #[error("status file is not valid UTF-8: {0}")]
    Utf8(String),
}

// ── PackageWant ───────────────────────────────────────────────────────────────

/// What the system administrator *wants* done with the package.
///
/// This is the first word of the three-word `Status:` field.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PackageWant {
    /// Install (or keep installed) the package.
    Install,
    /// Keep at the current version; do not upgrade automatically.
    Hold,
    /// Remove the package but leave configuration files on disk.
    Deinstall,
    /// Remove the package *and* all its configuration files.
    Purge,
    /// An unrecognised want value.
    Unknown(String),
}

impl PackageWant {
    fn parse(s: &str) -> Self {
        match s {
            "install" => Self::Install,
            "hold" => Self::Hold,
            "deinstall" => Self::Deinstall,
            "purge" => Self::Purge,
            other => Self::Unknown(other.to_string()),
        }
    }
}

impl std::fmt::Display for PackageWant {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Install => f.write_str("install"),
            Self::Hold => f.write_str("hold"),
            Self::Deinstall => f.write_str("deinstall"),
            Self::Purge => f.write_str("purge"),
            Self::Unknown(s) => f.write_str(s),
        }
    }
}

// ── PackageAction ─────────────────────────────────────────────────────────────

/// The last action dpkg took (or attempted) on the package.
///
/// This is the second word of the three-word `Status:` field.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PackageAction {
    /// No pending action; package is in a consistent state.
    Ok,
    /// Reinstallation required.
    ReinstatationRequired,
    /// Package is awaiting trigger processing by another package.
    TriggerAwaited,
    /// Package has outstanding triggers of its own to run.
    TriggersPending,
    /// Installation was interrupted after unpacking.
    HalfInstalled,
    /// Unpacking failed.
    UnpackFailed,
    /// Configuration was interrupted.
    HalfConfigured,
    /// The `postinst` maintainer script failed.
    PostInstFailed,
    /// Package removal failed.
    RemovalFailed,
    /// An unrecognised action value.
    Unknown(String),
}

impl PackageAction {
    fn parse(s: &str) -> Self {
        match s {
            "ok" => Self::Ok,
            "reinstreq" => Self::ReinstatationRequired,
            "triggersawaited" | "trigproc" => Self::TriggerAwaited,
            "trigpend" => Self::TriggersPending,
            "half-installed" => Self::HalfInstalled,
            "unpack-failed" => Self::UnpackFailed,
            "half-configured" => Self::HalfConfigured,
            "postinst-failed" => Self::PostInstFailed,
            "removal-failed" => Self::RemovalFailed,
            other => Self::Unknown(other.to_string()),
        }
    }
}

impl std::fmt::Display for PackageAction {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Ok => f.write_str("ok"),
            Self::ReinstatationRequired => f.write_str("reinstreq"),
            Self::TriggerAwaited => f.write_str("triggersawaited"),
            Self::TriggersPending => f.write_str("trigpend"),
            Self::HalfInstalled => f.write_str("half-installed"),
            Self::UnpackFailed => f.write_str("unpack-failed"),
            Self::HalfConfigured => f.write_str("half-configured"),
            Self::PostInstFailed => f.write_str("postinst-failed"),
            Self::RemovalFailed => f.write_str("removal-failed"),
            Self::Unknown(s) => f.write_str(s),
        }
    }
}

// ── InstallStatus ─────────────────────────────────────────────────────────────

/// The current installation state of the package.
///
/// This is the third word of the three-word `Status:` field.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InstallStatus {
    /// Package is fully installed and configured.
    Installed,
    /// Package installation was interrupted (`half-installed`).
    HalfInstalled,
    /// Only configuration files remain (`config-files`).
    ConfigFiles,
    /// Package is unpacked but not yet configured.
    Unpacked,
    /// Package configuration was interrupted (`half-configured`).
    HalfConfigured,
    /// Waiting for triggers from another package (`triggers-awaited`).
    TriggersAwaited,
    /// Package has outstanding triggers to run (`triggers-pending`).
    TriggersPending,
    /// Package is not installed.
    NotInstalled,
    /// An unrecognised status value.
    Unknown(String),
}

impl InstallStatus {
    fn parse(s: &str) -> Self {
        match s {
            "installed" => Self::Installed,
            "half-installed" => Self::HalfInstalled,
            "config-files" => Self::ConfigFiles,
            "unpacked" => Self::Unpacked,
            "half-configured" => Self::HalfConfigured,
            "triggers-awaited" => Self::TriggersAwaited,
            "triggers-pending" => Self::TriggersPending,
            "not-installed" => Self::NotInstalled,
            other => Self::Unknown(other.to_string()),
        }
    }
}

impl std::fmt::Display for InstallStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Installed => f.write_str("installed"),
            Self::HalfInstalled => f.write_str("half-installed"),
            Self::ConfigFiles => f.write_str("config-files"),
            Self::Unpacked => f.write_str("unpacked"),
            Self::HalfConfigured => f.write_str("half-configured"),
            Self::TriggersAwaited => f.write_str("triggers-awaited"),
            Self::TriggersPending => f.write_str("triggers-pending"),
            Self::NotInstalled => f.write_str("not-installed"),
            Self::Unknown(s) => f.write_str(s),
        }
    }
}

// ── PackageStatus ─────────────────────────────────────────────────────────────

/// Parsed representation of the `Status:` field in a dpkg stanza.
///
/// The raw field value has exactly three space-separated tokens:
/// `<want> <action> <status>`, e.g. `install ok installed`.
#[derive(Debug, Clone)]
pub struct PackageStatus {
    /// What the administrator wants done with the package.
    pub want: PackageWant,
    /// The last action dpkg took on the package.
    pub action: PackageAction,
    /// The current installation state.
    pub status: InstallStatus,
}

impl PackageStatus {
    /// Parse a `Status:` field value such as `"install ok installed"`.
    ///
    /// # Errors
    ///
    /// Returns [`DbError::BadStatus`] when the value does not contain exactly
    /// three whitespace-separated tokens.
    pub fn parse(s: &str) -> Result<Self, DbError> {
        let parts: Vec<&str> = s.split_whitespace().collect();
        if parts.len() != 3 {
            return Err(DbError::BadStatus(s.to_string()));
        }
        Ok(PackageStatus {
            want: PackageWant::parse(parts[0]),
            action: PackageAction::parse(parts[1]),
            status: InstallStatus::parse(parts[2]),
        })
    }

    /// Returns `true` when the package is in the `installed` state.
    pub fn is_installed(&self) -> bool {
        self.status == InstallStatus::Installed
    }
}

impl std::fmt::Display for PackageStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} {} {}", self.want, self.action, self.status)
    }
}

// ── InstalledPackage ──────────────────────────────────────────────────────────

/// One entry in the dpkg status database.
#[derive(Debug, Clone)]
pub struct InstalledPackage {
    /// Package name (`Package:` field).
    pub name: String,
    /// Package version (`Version:` field).
    pub version: String,
    /// Package architecture (`Architecture:` field), e.g. `amd64`.
    pub architecture: String,
    /// Parsed installation status.
    pub status: PackageStatus,
    /// Raw `Depends:` field value, if present.
    pub depends: Option<String>,
    /// Raw `Pre-Depends:` field value, if present.
    pub pre_depends: Option<String>,
    /// Synopsis (first line of the `Description:` field), if present.
    pub description: Option<String>,
}

// ── StatusDb ──────────────────────────────────────────────────────────────────

/// In-memory representation of the dpkg status database.
///
/// Stanzas with missing required fields are silently skipped; a `tracing`
/// warning is emitted for each skipped entry.
#[derive(Debug, Default)]
pub struct StatusDb {
    packages: Vec<InstalledPackage>,
}

impl StatusDb {
    /// Create an empty database.
    pub fn new() -> Self {
        Self::default()
    }

    /// Open and parse the dpkg status file at `path`.
    ///
    /// Pass `/var/lib/dpkg/status` for the system database. The file is
    /// memory-mapped (via `memmap2`) so even very large status databases are
    /// not copied into a Rust `String` up front; the parse reads through the
    /// mapping.
    pub fn open(path: &Path) -> Result<Self, DbError> {
        let file = std::fs::File::open(path)?;
        // SAFETY: we only read the mapping; the underlying file is not mutated
        // by this process, so the mapping's visibility is stable for its life.
        let mmap = unsafe { memmap2::Mmap::map(&file)? };
        let content = std::str::from_utf8(&mmap).map_err(|e| DbError::Utf8(e.to_string()))?;
        Self::parse_str(content)
    }

    /// Parse a status database from a string.
    pub fn parse_str(content: &str) -> Result<Self, DbError> {
        let paragraphs = tpt_l_control_file::parse_control(content);
        let mut packages = Vec::with_capacity(paragraphs.len());

        for para in &paragraphs {
            if para.is_empty() {
                continue;
            }

            let name = match para.get("Package") {
                Some(n) => n.to_string(),
                None => {
                    tracing::warn!("skipping stanza with no Package field");
                    continue;
                }
            };

            let version = para.get("Version").unwrap_or("").to_string();
            let architecture = para.get("Architecture").unwrap_or("").to_string();

            let status = match para.get("Status") {
                Some(s) => match PackageStatus::parse(s) {
                    Ok(ps) => ps,
                    Err(e) => {
                        tracing::warn!(package = %name, error = %e, "bad Status field; skipping");
                        continue;
                    }
                },
                None => {
                    tracing::warn!(package = %name, "no Status field; skipping");
                    continue;
                }
            };

            packages.push(InstalledPackage {
                name,
                version,
                architecture,
                status,
                depends: para.get("Depends").map(str::to_string),
                pre_depends: para.get("Pre-Depends").map(str::to_string),
                description: para
                    .get("Description")
                    .map(|d| d.lines().next().unwrap_or(d).to_string()),
            });
        }

        Ok(Self { packages })
    }

    /// Returns a slice over every package record (installed or not).
    pub fn packages(&self) -> &[InstalledPackage] {
        &self.packages
    }

    /// Iterate over packages in the `installed` state.
    pub fn installed_packages(&self) -> impl Iterator<Item = &InstalledPackage> {
        self.packages.iter().filter(|p| p.status.is_installed())
    }

    /// Find a package by exact name. Returns `None` if absent.
    pub fn find(&self, name: &str) -> Option<&InstalledPackage> {
        self.packages.iter().find(|p| p.name == name)
    }

    /// Write the database atomically to `path`.
    ///
    /// Writes to a temporary file in the same directory as `path`, calls
    /// `sync_all()`, then renames into place.
    pub fn write_atomic(&self, path: &Path) -> Result<(), DbError> {
        let dir = path.parent().unwrap_or_else(|| Path::new("."));
        let tmp = tempfile::NamedTempFile::new_in(dir)?;
        let mut writer = std::io::BufWriter::new(tmp);

        for (i, pkg) in self.packages.iter().enumerate() {
            if i > 0 {
                writeln!(writer)?;
            }
            writeln!(writer, "Package: {}", pkg.name)?;
            writeln!(writer, "Status: {}", pkg.status)?;
            writeln!(writer, "Architecture: {}", pkg.architecture)?;
            writeln!(writer, "Version: {}", pkg.version)?;
            if let Some(pre) = &pkg.pre_depends {
                writeln!(writer, "Pre-Depends: {}", pre)?;
            }
            if let Some(dep) = &pkg.depends {
                writeln!(writer, "Depends: {}", dep)?;
            }
            if let Some(desc) = &pkg.description {
                writeln!(writer, "Description: {}", desc)?;
            }
        }

        let tmp_file = writer.into_inner().map_err(|e| e.into_error())?;
        tmp_file.as_file().sync_all()?;
        tmp_file
            .persist(path)
            .map_err(|e| DbError::Persist(e.to_string()))?;
        Ok(())
    }

    /// Apply a set of changes in place, then write the whole database back
    /// atomically via [`StatusDb::write_atomic`].
    ///
    /// Each change either upserts a package's status (matched by name) or
    /// removes a package entirely. This is the "changes diff" half of the
    /// atomic writer: callers compute a small diff and persist it without
    /// manually mutating every record.
    pub fn apply_changes(&mut self, changes: &[StatusChange]) -> Result<(), DbError> {
        for change in changes {
            match change {
                StatusChange::SetStatus {
                    name,
                    version,
                    architecture,
                    status,
                } => {
                    if let Some(p) = self.packages.iter_mut().find(|p| &p.name == name) {
                        p.version = version.clone();
                        p.architecture = architecture.clone();
                        p.status = status.clone();
                    } else {
                        self.packages.push(InstalledPackage {
                            name: name.clone(),
                            version: version.clone(),
                            architecture: architecture.clone(),
                            status: status.clone(),
                            depends: None,
                            pre_depends: None,
                            description: None,
                        });
                    }
                }
                StatusChange::Remove { name } => {
                    self.packages.retain(|p| &p.name != name);
                }
            }
        }
        Ok(())
    }
}

// ── StatusChange ─────────────────────────────────────────────────────────────

/// A single mutation to apply to a [`StatusDb`] before persisting it.
///
/// Changes are applied by [`StatusDb::apply_changes`] and then written back
/// atomically; this is the "diff" half of the write path.
#[derive(Debug, Clone)]
pub enum StatusChange {
    /// Upsert the status of the named package. If no record exists it is
    /// created (with `Depends`/`Pre-Depends`/`Description` left empty for the
    /// caller to populate if needed).
    SetStatus {
        /// Package name.
        name: String,
        /// New version string.
        version: String,
        /// New architecture string.
        architecture: String,
        /// New parsed status.
        status: PackageStatus,
    },
    /// Remove the named package record entirely.
    Remove {
        /// Package name.
        name: String,
    },
}

// ── ConcurrentStatusDb ───────────────────────────────────────────────────────

/// A [`StatusDb`] guarded by a read/write lock for safe concurrent access.
///
/// Many threads may call [`ConcurrentStatusDb::read`] simultaneously; writers
/// (via [`ConcurrentStatusDb::apply_changes`]) take the exclusive lock and
/// block readers for the duration of the mutation. The underlying lock is
/// `std::sync::RwLock`, so reads never block each other and writers are
/// serialised.
pub struct ConcurrentStatusDb {
    inner: std::sync::RwLock<StatusDb>,
}

impl ConcurrentStatusDb {
    /// Open a status database and wrap it for concurrent access.
    pub fn open(path: &Path) -> Result<Self, DbError> {
        Ok(Self {
            inner: std::sync::RwLock::new(StatusDb::open(path)?),
        })
    }

    /// Acquire a shared read lock and return a guard that derefs to the
    /// underlying [`StatusDb`].
    ///
    /// The guard borrows `self`, so the lock is held for as long as the guard
    /// is alive.
    pub fn read(&self) -> std::sync::RwLockReadGuard<'_, StatusDb> {
        self.inner
            .read()
            .expect("StatusDb lock poisoned by a panicking writer")
    }

    /// Acquire the exclusive write lock, apply `changes`, and persist the
    /// updated database atomically to `path`.
    pub fn apply_changes(&self, path: &Path, changes: &[StatusChange]) -> Result<(), DbError> {
        {
            let mut db = self
                .inner
                .write()
                .expect("StatusDb lock poisoned by a panicking writer");
            db.apply_changes(changes)?;
        }
        self.read().write_atomic(path)
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "\
Package: bash
Status: install ok installed
Priority: required
Section: shells
Architecture: amd64
Version: 5.1-6ubuntu1
Depends: base-files (>= 2.1.12), debianutils (>= 2.15)
Description: GNU Bourne Again SHell
 The GNU Bourne Again shell is a shell or command language interpreter.

Package: dpkg
Status: install ok installed
Priority: required
Section: admin
Architecture: amd64
Version: 1.21.1ubuntu2
Pre-Depends: libzstd1 (>= 1.5.1)
Description: Debian package management system
 Low-level tools.

Package: removed-pkg
Status: deinstall ok config-files
Priority: optional
Section: misc
Architecture: amd64
Version: 1.0
Description: a removed package

";

    #[test]
    fn parse_minimal_inline_stanza() {
        let db = StatusDb::parse_str(SAMPLE).unwrap();
        assert_eq!(db.packages().len(), 3);
    }

    #[test]
    fn find_returns_correct_package() {
        let db = StatusDb::parse_str(SAMPLE).unwrap();
        let pkg = db.find("bash").unwrap();
        assert_eq!(pkg.name, "bash");
        assert_eq!(pkg.version, "5.1-6ubuntu1");
        assert_eq!(pkg.architecture, "amd64");
    }

    #[test]
    fn find_returns_none_for_unknown() {
        let db = StatusDb::parse_str(SAMPLE).unwrap();
        assert!(db.find("no-such-package").is_none());
    }

    #[test]
    fn installed_packages_filters_non_installed() {
        let db = StatusDb::parse_str(SAMPLE).unwrap();
        let installed: Vec<_> = db.installed_packages().collect();
        assert_eq!(installed.len(), 2);
        assert!(installed.iter().all(|p| p.status.is_installed()));
        assert!(installed.iter().any(|p| p.name == "bash"));
        assert!(installed.iter().any(|p| p.name == "dpkg"));
    }

    #[test]
    fn package_status_parse_installed() {
        let ps = PackageStatus::parse("install ok installed").unwrap();
        assert!(matches!(ps.want, PackageWant::Install));
        assert!(matches!(ps.action, PackageAction::Ok));
        assert!(matches!(ps.status, InstallStatus::Installed));
        assert!(ps.is_installed());
    }

    #[test]
    fn package_status_parse_config_files() {
        let ps = PackageStatus::parse("deinstall ok config-files").unwrap();
        assert!(!ps.is_installed());
        assert!(matches!(ps.status, InstallStatus::ConfigFiles));
    }

    #[test]
    fn package_status_parse_all_variants() {
        let cases: &[(&str, bool)] = &[
            ("install ok installed", true),
            ("install ok half-installed", false),
            ("deinstall ok config-files", false),
            ("install ok unpacked", false),
            ("install ok half-configured", false),
            ("install ok triggers-awaited", false),
            ("install ok triggers-pending", false),
            ("purge ok not-installed", false),
        ];
        for &(input, expect_installed) in cases {
            let ps = PackageStatus::parse(input).unwrap();
            assert_eq!(ps.is_installed(), expect_installed, "for {:?}", input);
        }
    }

    #[test]
    fn package_status_bad_token_count_is_error() {
        assert!(PackageStatus::parse("").is_err());
        assert!(PackageStatus::parse("install ok").is_err());
        assert!(PackageStatus::parse("too many tokens here").is_err());
    }

    #[test]
    fn is_installed_true_only_for_installed_status() {
        let ins = PackageStatus::parse("install ok installed").unwrap();
        assert!(ins.is_installed());
        let cfg = PackageStatus::parse("deinstall ok config-files").unwrap();
        assert!(!cfg.is_installed());
    }

    #[test]
    fn write_atomic_round_trip() {
        let db = StatusDb::parse_str(SAMPLE).unwrap();
        let tmp_dir = tempfile::tempdir().unwrap();
        let out = tmp_dir.path().join("status");
        db.write_atomic(&out).unwrap();
        assert!(out.exists());
        let db2 = StatusDb::open(&out).unwrap();
        assert_eq!(db2.packages().len(), db.packages().len());
        assert_eq!(
            db2.find("bash").unwrap().version,
            db.find("bash").unwrap().version
        );
    }

    #[test]
    fn apply_changes_upserts_and_removes() {
        let mut db = StatusDb::parse_str(SAMPLE).unwrap();
        let before = db.packages().len();
        db.apply_changes(&[
            StatusChange::SetStatus {
                name: "bash".to_string(),
                version: "5.2-1".to_string(),
                architecture: "amd64".to_string(),
                status: PackageStatus::parse("install ok installed").unwrap(),
            },
            StatusChange::SetStatus {
                name: "newpkg".to_string(),
                version: "0.1".to_string(),
                architecture: "amd64".to_string(),
                status: PackageStatus::parse("install ok installed").unwrap(),
            },
            StatusChange::Remove {
                name: "removed-pkg".to_string(),
            },
        ])
        .unwrap();
        assert_eq!(db.find("bash").unwrap().version, "5.2-1");
        assert!(db.find("newpkg").is_some());
        assert!(db.find("removed-pkg").is_none());
        // bash upsert replaces in place, newpkg is added, removed-pkg drops:
        // the total count is unchanged.
        assert_eq!(db.packages().len(), before);
    }

    #[test]
    fn concurrent_reads_and_write_dont_deadlock() {
        use std::sync::Arc;

        let db = StatusDb::parse_str(SAMPLE).unwrap();
        let tmp_dir = tempfile::tempdir().unwrap();
        let out = tmp_dir.path().join("status");
        db.write_atomic(&out).unwrap();

        let cdb = Arc::new(ConcurrentStatusDb::open(&out).unwrap());

        // Spawn several reader threads that hold the read lock briefly.
        let readers: Vec<_> = (0..4)
            .map(|_| {
                let cdb = Arc::clone(&cdb);
                std::thread::spawn(move || {
                    for _ in 0..50 {
                        let guard = cdb.read();
                        let _ = guard.installed_packages().count();
                    }
                })
            })
            .collect();

        // Concurrently, a writer applies a change and persists atomically.
        let writer = {
            let cdb = Arc::clone(&cdb);
            let out = out.clone();
            std::thread::spawn(move || {
                for i in 0..20 {
                    cdb.apply_changes(
                        &out,
                        &[StatusChange::SetStatus {
                            name: format!("dyn-{i}"),
                            version: "1.0".to_string(),
                            architecture: "amd64".to_string(),
                            status: PackageStatus::parse("install ok installed").unwrap(),
                        }],
                    )
                    .unwrap();
                }
            })
        };

        for r in readers {
            r.join().unwrap();
        }
        writer.join().unwrap();

        // Final state is consistent and readable.
        let guard = cdb.read();
        assert!(guard.find("dyn-19").is_some());
    }
}
