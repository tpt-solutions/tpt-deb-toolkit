//! dpkg trigger processing.
//!
//! Debian triggers let one package signal that some shared state (a font
//! cache, an initramfs, a manual page index, …) needs to be recomputed. A
//! package declares *interest* in a named trigger via its control metadata;
//! another package *activates* the trigger, and dpkg later runs the interested
//! package's `postinst` with the `triggered` action to rebuild that state.
//!
//! This crate models the trigger database and the deferred processing loop,
//! and bridges activation/processing to the
//! [`tpt_l_dpkg_db::InstallStatus`] state machine.
//!
//! # Example
//!
//! ```
//! use tpt_l_dpkg_triggers::{TriggerDb, Trigger};
//!
//! let mut db = TriggerDb::new();
//! db.interest("man-db", "man-db-rebuild", true);
//! let affected = db.activate("man-db-rebuild");
//! assert_eq!(affected, vec!["man-db".to_string()]);
//! assert!(db.is_pending("man-db"));
//! let todo = db.process("man-db");
//! assert_eq!(todo, vec!["man-db-rebuild".to_string()]);
//! assert!(!db.is_pending("man-db"));
//! ```

use std::collections::HashMap;
use std::path::Path;

use thiserror::Error;

/// Errors produced by the trigger layer.
#[derive(Debug, Error)]
pub enum TriggerError {
    /// An I/O error while reading or writing the trigger database.
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    /// The trigger database directory did not contain the expected layout.
    #[error("malformed trigger record: {0}")]
    Malformed(String),
}

/// A trigger a package is interested in, or that is being activated.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Trigger {
    /// The trigger name (e.g. `man-db-rebuild`, `ldconfig`).
    pub name: String,
    /// `true` for `interest`/`activate` (the activator awaits completion);
    /// `false` for `interest-noawait`/`activate-noawait`.
    pub awaited: bool,
}

impl Trigger {
    /// An `interest` trigger (awaited).
    pub fn interest(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            awaited: true,
        }
    }

    /// An `interest-noawait` trigger.
    pub fn interest_noawait(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            awaited: false,
        }
    }
}

/// The in-memory trigger database.
///
/// Tracks, for every package, which triggers it is *interested* in, and which
/// triggers are currently *pending* (activated but not yet processed).
#[derive(Debug, Clone, Default)]
pub struct TriggerDb {
    /// `package ->` triggers it is interested in.
    interests: HashMap<String, Vec<Trigger>>,
    /// `package ->` trigger names pending processing.
    pending: HashMap<String, Vec<String>>,
}

impl TriggerDb {
    /// Create an empty database.
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a package's interest in a trigger.
    ///
    /// Duplicate interests are ignored.
    pub fn interest(&mut self, package: &str, name: &str, awaited: bool) {
        let list = self.interests.entry(package.to_string()).or_default();
        if !list.iter().any(|t| t.name == name) {
            list.push(Trigger {
                name: name.to_string(),
                awaited,
            });
        }
    }

    /// Register a package's interest in a [`Trigger`].
    pub fn add_trigger(&mut self, package: &str, trigger: Trigger) {
        self.interest(package, &trigger.name, trigger.awaited);
    }

    /// Return the triggers `package` is interested in.
    pub fn interests_of(&self, package: &str) -> &[Trigger] {
        self.interests
            .get(package)
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }

    /// Activate a named trigger.
    ///
    /// Every package interested in `name` (that does not already have it
    /// pending) is marked pending. Returns the list of package names that
    /// became newly pending.
    pub fn activate(&mut self, name: &str) -> Vec<String> {
        let mut newly = Vec::new();
        for (pkg, triggers) in &self.interests {
            if triggers.iter().any(|t| t.name == name) {
                let entry = self.pending.entry(pkg.clone()).or_default();
                if !entry.contains(&name.to_string()) {
                    entry.push(name.to_string());
                    newly.push(pkg.clone());
                }
            }
        }
        newly
    }

    /// Activate every trigger in `names`, returning the union of affected
    /// packages.
    pub fn activate_all<'a>(&mut self, names: impl IntoIterator<Item = &'a str>) -> Vec<String> {
        let mut all = Vec::new();
        for n in names {
            all.extend(self.activate(n));
        }
        all.sort();
        all.dedup();
        all
    }

    /// Whether `package` currently has pending triggers.
    pub fn is_pending(&self, package: &str) -> bool {
        self.pending
            .get(package)
            .map(|v| !v.is_empty())
            .unwrap_or(false)
    }

    /// Return the trigger names pending for `package` (without clearing them).
    pub fn pending_of(&self, package: &str) -> &[String] {
        self.pending.get(package).map(Vec::as_slice).unwrap_or(&[])
    }

    /// Dequeue and return the pending triggers for `package`, clearing its
    /// pending state. The returned list is what the package's `postinst
    /// triggered` invocation should handle.
    pub fn process(&mut self, package: &str) -> Vec<String> {
        self.pending.remove(package).unwrap_or_default()
    }

    /// All package names that currently have pending triggers.
    pub fn pending_packages(&self) -> Vec<&str> {
        self.pending
            .iter()
            .filter(|(_, v)| !v.is_empty())
            .map(|(p, _)| p.as_str())
            .collect()
    }

    /// Persist the database to `dir` (one file per package holding its pending
    /// triggers, plus an `Interests` file mapping interests).
    ///
    /// The layout is an abstraction over dpkg's `/var/lib/dpkg/triggers/`
    /// directory and is sufficient for round-tripping this crate's state.
    pub fn save_dir(&self, dir: &Path) -> Result<(), TriggerError> {
        std::fs::create_dir_all(dir)?;

        // Interests file: "<package> <name> <awaited>\n"
        let mut interest_lines = String::new();
        let mut packages: Vec<&String> = self.interests.keys().collect();
        packages.sort();
        for pkg in packages {
            for t in &self.interests[pkg] {
                interest_lines.push_str(&format!("{} {} {}\n", pkg, t.name, t.awaited));
            }
        }
        std::fs::write(dir.join("Interests"), interest_lines)?;

        // One pending file per package.
        for pkg in self.pending.keys() {
            let body = self.pending[pkg].join("\n") + "\n";
            std::fs::write(dir.join(pkg), body)?;
        }
        Ok(())
    }

    /// Load a database previously written by [`TriggerDb::save_dir`].
    pub fn load_dir(dir: &Path) -> Result<Self, TriggerError> {
        let mut db = TriggerDb::new();

        let interests_path = dir.join("Interests");
        if interests_path.exists() {
            let content = std::fs::read_to_string(&interests_path)?;
            for line in content.lines() {
                if line.trim().is_empty() {
                    continue;
                }
                let parts: Vec<&str> = line.split_whitespace().collect();
                if parts.len() != 3 {
                    return Err(TriggerError::Malformed(line.to_string()));
                }
                let awaited = parts[2] == "true";
                db.interest(parts[0], parts[1], awaited);
            }
        }

        let entries = std::fs::read_dir(dir)?;
        for entry in entries {
            let entry = entry?;
            let fname = entry.file_name();
            let name = fname.to_string_lossy();
            if name == "Interests" {
                continue;
            }
            let content = std::fs::read_to_string(entry.path())?;
            let triggers: Vec<String> = content
                .lines()
                .map(str::trim)
                .filter(|l| !l.is_empty())
                .map(str::to_string)
                .collect();
            if !triggers.is_empty() {
                db.pending.insert(name.into_owned(), triggers);
            }
        }

        Ok(db)
    }
}

/// State-machine helpers bridging trigger activity to dpkg's
/// `InstallStatus`.
pub mod status {
    use tpt_l_dpkg_db::InstallStatus;

    /// Mark a package as having pending triggers to process.
    ///
    /// Transitions `installed` → `triggers-pending`. Other states are left
    /// unchanged (the package is already in a transitional state).
    pub fn mark_pending(status: InstallStatus) -> InstallStatus {
        if status == InstallStatus::Installed {
            InstallStatus::TriggersPending
        } else {
            status
        }
    }

    /// Mark a package as awaiting triggers from another package.
    ///
    /// Transitions `installed` → `triggers-awaited`.
    pub fn mark_awaited(status: InstallStatus) -> InstallStatus {
        if status == InstallStatus::Installed {
            InstallStatus::TriggersAwaited
        } else {
            status
        }
    }

    /// Clear trigger state, returning the package to `installed`.
    pub fn clear(status: InstallStatus) -> InstallStatus {
        match status {
            InstallStatus::TriggersPending | InstallStatus::TriggersAwaited => {
                InstallStatus::Installed
            }
            other => other,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interest_then_activate_marks_pending() {
        let mut db = TriggerDb::new();
        db.interest("man-db", "man-db-rebuild", true);
        let affected = db.activate("man-db-rebuild");
        assert_eq!(affected, vec!["man-db".to_string()]);
        assert!(db.is_pending("man-db"));
        assert_eq!(db.pending_of("man-db"), &["man-db-rebuild".to_string()]);
    }

    #[test]
    fn activate_is_idempotent() {
        let mut db = TriggerDb::new();
        db.interest("a", "t", true);
        db.activate("t");
        let again = db.activate("t");
        assert!(again.is_empty());
        assert_eq!(db.pending_of("a"), &["t".to_string()]);
    }

    #[test]
    fn process_returns_and_clears_pending() {
        let mut db = TriggerDb::new();
        db.interest("a", "t1", true);
        db.interest("a", "t2", true);
        db.activate_all(["t1", "t2"]);
        let mut todo = db.process("a");
        todo.sort();
        assert_eq!(todo, vec!["t1".to_string(), "t2".to_string()]);
        assert!(!db.is_pending("a"));
    }

    #[test]
    fn only_interested_packages_affected() {
        let mut db = TriggerDb::new();
        db.interest("a", "shared", true);
        db.interest("b", "other", true);
        let affected = db.activate("shared");
        assert_eq!(affected, vec!["a".to_string()]);
    }

    #[test]
    fn save_and_load_round_trip() {
        let mut db = TriggerDb::new();
        db.interest("man-db", "man-db-rebuild", true);
        db.interest("libc6", "ldconfig", false);
        db.activate_all(["man-db-rebuild", "ldconfig"]);
        let dir = tempfile::tempdir().unwrap();
        db.save_dir(dir.path()).unwrap();
        let db2 = TriggerDb::load_dir(dir.path()).unwrap();
        assert!(db2.is_pending("man-db"));
        assert!(db2.is_pending("libc6"));
        assert!(db2.interests_of("man-db")[0].awaited);
        assert!(!db2.interests_of("libc6")[0].awaited);
    }

    #[test]
    fn status_transitions() {
        use status::*;
        use tpt_l_dpkg_db::InstallStatus;
        assert_eq!(
            mark_pending(InstallStatus::Installed),
            InstallStatus::TriggersPending
        );
        assert_eq!(
            mark_awaited(InstallStatus::Installed),
            InstallStatus::TriggersAwaited
        );
        assert_eq!(
            clear(InstallStatus::TriggersPending),
            InstallStatus::Installed
        );
        assert_eq!(
            clear(InstallStatus::TriggersAwaited),
            InstallStatus::Installed
        );
        // Non-terminal states are untouched.
        assert_eq!(
            mark_pending(InstallStatus::HalfInstalled),
            InstallStatus::HalfInstalled
        );
    }
}
