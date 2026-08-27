//! Execution of Debian maintainer scripts (`preinst`, `postinst`, `prerm`,
//! `postrm`) with correct environment, ordering, and exit-code semantics.
//!
//! # Ordering contract
//!
//! dpkg runs the maintainer scripts in a strict order relative to the payload
//! being unpacked/removed:
//!
//! ```text
//! install:  preinst  →  (unpack)  →  postinst
//! remove:   prerm    →  (remove)  →  postrm
//! ```
//!
//! This crate does not perform the unpack/remove itself — that belongs to the
//! package database and extraction layers — but it exposes one method per
//! script so callers can honour the contract, passing the appropriate
//! *action* argument (e.g. `install`, `configure`, `remove`).
//!
//! # Sandboxing
//!
//! By default scripts run inside [`tpt_l_linux_sandbox_rs`] on Linux. Use
//! [`ScriptRunner::unrestricted`] to opt out; doing so emits a structured
//! warning because it grants the script full access to the host.
//!
//! # Example
//!
//! ```no_run
//! use tpt_l_maintainer_scripts::{ScriptRunner, PackageRef};
//! use std::path::PathBuf;
//!
//! let runner = ScriptRunner::new(
//!     PathBuf::from("/var/lib/dpkg/info"),
//!     PackageRef { name: "curl".into(), version: "8.2.1-1".into(), arch: "amd64".into() },
//! );
//! let outcome = runner.run_postinst("configure").unwrap();
//! assert!(outcome.success());
//! ```

use std::collections::HashMap;
use std::path::PathBuf;
use std::process::ExitStatus;

use thiserror::Error;

/// Errors that can occur while planning or running a maintainer script.
#[derive(Debug, Error)]
pub enum ScriptError {
    /// The requested script does not exist in the control directory.
    #[error("maintainer script '{0}' not found in {1}")]
    ScriptNotFound(String, PathBuf),

    /// The script could not be spawned.
    #[error("failed to spawn maintainer script: {0}")]
    SpawnFailed(String),

    /// The script was killed by a signal (no exit code available).
    #[error("maintainer script terminated by signal")]
    Signaled,

    /// An I/O error occurred.
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    /// The sandbox layer rejected the request.
    #[error("sandbox error: {0}")]
    Sandbox(String),
}

/// A package identity used to populate `DPKG_MAINTSCRIPT_*` environment vars.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackageRef {
    /// Package name (`Package:`).
    pub name: String,
    /// Full version string (`Version:`).
    pub version: String,
    /// Architecture (`Architecture:`).
    pub arch: String,
}

/// The outcome of running a maintainer script.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScriptOutcome {
    /// Raw process exit code (0 = success, non-zero = abort/rollback signal).
    pub exit_code: i32,
}

impl ScriptOutcome {
    /// `true` when the script exited `0`.
    pub fn success(&self) -> bool {
        self.exit_code == 0
    }
}

/// Configuration for a [`ScriptRunner`].
#[derive(Debug, Clone)]
pub struct RunnerConfig {
    /// Directory holding the maintainer scripts (e.g. `DEBIAN/` or
    /// `/var/lib/dpkg/info`).
    pub control_dir: PathBuf,
    /// Whether to run scripts inside the Linux sandbox.
    pub use_sandbox: bool,
    /// Value for `DEBIAN_FRONTEND`; defaults to `noninteractive` when `None`.
    pub debian_frontend: Option<String>,
    /// Extra environment variables merged into the script environment.
    pub extra_env: Vec<(String, String)>,
    /// Installation root (`DPKG_ROOT`); defaults to `/`.
    pub root: PathBuf,
}

impl Default for RunnerConfig {
    fn default() -> Self {
        Self {
            control_dir: PathBuf::from("."),
            use_sandbox: true,
            debian_frontend: None,
            extra_env: Vec::new(),
            root: PathBuf::from("/"),
        }
    }
}

/// A ready-to-execute description of a maintainer script invocation.
///
/// Building a plan is pure (no I/O beyond checking the script exists), so it
/// can be inspected and unit-tested on any platform.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScriptPlan {
    /// Absolute/relative path to the script to execute.
    pub script_path: PathBuf,
    /// Arguments passed to the script (the dpkg action, e.g. `configure`).
    pub args: Vec<String>,
    /// The full environment the script will run with.
    pub env: Vec<(String, String)>,
}

impl ScriptPlan {
    /// The script file name (e.g. `postinst`).
    pub fn script_name(&self) -> Option<&str> {
        self.script_path.file_name().and_then(|n| n.to_str())
    }
}

/// Runs Debian maintainer scripts with correct environment and sandboxing.
#[derive(Debug, Clone)]
pub struct ScriptRunner {
    config: RunnerConfig,
    package: PackageRef,
}

impl ScriptRunner {
    /// Create a new runner using the default (sandboxed) configuration.
    pub fn new(control_dir: PathBuf, package: PackageRef) -> Self {
        Self {
            config: RunnerConfig {
                control_dir,
                ..Default::default()
            },
            package,
        }
    }

    /// Create a runner that bypasses the sandbox.
    ///
    /// A structured warning is emitted because the script will have full
    /// access to the host filesystem and network.
    pub fn unrestricted(control_dir: PathBuf, package: PackageRef) -> Self {
        tracing::warn!(
            package = %package.name,
            "maintainer scripts will run WITHOUT sandbox isolation (unrestricted mode)"
        );
        Self {
            config: RunnerConfig {
                control_dir,
                use_sandbox: false,
                ..Default::default()
            },
            package,
        }
    }

    /// Override the runner configuration.
    pub fn with_config(mut self, config: RunnerConfig) -> Self {
        self.config = config;
        self
    }

    /// Returns `true` if scripts run inside the sandbox.
    pub fn is_sandboxed(&self) -> bool {
        self.config.use_sandbox
    }

    /// Build the environment a maintainer script would run with.
    ///
    /// Pure (no I/O) and therefore testable on any platform.
    pub fn script_env(&self) -> Vec<(String, String)> {
        self.env_map(&[]).into_iter().collect()
    }

    fn env_map(&self, extra: &[(String, String)]) -> HashMap<String, String> {
        let mut env: HashMap<String, String> = HashMap::new();
        env.insert("PATH".into(), "/usr/sbin:/usr/bin:/sbin:/bin".into());
        env.insert(
            "DEBIAN_FRONTEND".into(),
            self.config
                .debian_frontend
                .clone()
                .unwrap_or_else(|| "noninteractive".into()),
        );
        env.insert("DPKG_MAINTSCRIPT_PACKAGE".into(), self.package.name.clone());
        env.insert(
            "DPKG_MAINTSCRIPT_VERSION".into(),
            self.package.version.clone(),
        );
        env.insert("DPKG_MAINTSCRIPT_ARCH".into(), self.package.arch.clone());
        env.insert(
            "DPKG_ROOT".into(),
            self.config.root.to_string_lossy().into(),
        );
        env.insert(
            "DPKG_ADMINDIR".into(),
            self.config
                .root
                .join("var/lib/dpkg")
                .to_string_lossy()
                .into(),
        );
        for (k, v) in &self.config.extra_env {
            env.insert(k.clone(), v.clone());
        }
        for (k, v) in extra {
            env.insert(k.clone(), v.clone());
        }
        env
    }

    /// Build a [`ScriptPlan`] for `script` (e.g. `postinst`) invoked with
    /// `action` (e.g. `configure`).
    ///
    /// # Errors
    ///
    /// Returns [`ScriptError::ScriptNotFound`] when the script file is absent.
    pub fn plan(&self, script: &str, action: &str) -> Result<ScriptPlan, ScriptError> {
        let script_path = self.config.control_dir.join(script);
        if !script_path.exists() {
            return Err(ScriptError::ScriptNotFound(
                script.to_string(),
                self.config.control_dir.clone(),
            ));
        }
        let env = self.env_map(&[]);
        let env: Vec<(String, String)> = env.into_iter().collect();
        Ok(ScriptPlan {
            script_path,
            args: vec![action.to_string()],
            env,
        })
    }

    /// Execute a previously-built [`ScriptPlan`].
    ///
    /// On Linux, sandboxed runners route through
    /// [`tpt_l_linux_sandbox_rs`]; on other platforms (or when unrestricted)
    /// the script is spawned directly.
    ///
    /// # Errors
    ///
    /// Returns [`ScriptError::Signaled`] if the process was terminated by a
    /// signal, or [`ScriptError::SpawnFailed`] if it could not be started.
    pub fn execute(&self, plan: &ScriptPlan) -> Result<ScriptOutcome, ScriptError> {
        let args: Vec<&str> = plan.args.iter().map(String::as_str).collect();
        let env: Vec<(&str, &str)> = plan
            .env
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str()))
            .collect();
        let cmd = plan.script_path.to_string_lossy().into_owned();

        #[cfg(target_os = "linux")]
        if self.config.use_sandbox {
            use tpt_l_linux_sandbox_rs::Sandbox;
            let sandbox = Sandbox::new();
            return match sandbox.run(&cmd, &args, &env) {
                Ok(status) => exit_to_outcome(status),
                Err(e) => Err(ScriptError::Sandbox(e.to_string())),
            };
        }

        let mut command = std::process::Command::new(&cmd);
        command.args(&args);
        command.env_clear();
        for (k, v) in &env {
            command.env(k, v);
        }
        match command.status() {
            Ok(status) => exit_to_outcome(status),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Err(ScriptError::ScriptNotFound(
                cmd,
                self.config.control_dir.clone(),
            )),
            Err(e) => Err(ScriptError::SpawnFailed(e.to_string())),
        }
    }

    /// Run `preinst` with the given action.
    pub fn run_preinst(&self, action: &str) -> Result<ScriptOutcome, ScriptError> {
        self.execute(&self.plan("preinst", action)?)
    }

    /// Run `postinst` with the given action.
    pub fn run_postinst(&self, action: &str) -> Result<ScriptOutcome, ScriptError> {
        self.execute(&self.plan("postinst", action)?)
    }

    /// Run `prerm` with the given action.
    pub fn run_prerm(&self, action: &str) -> Result<ScriptOutcome, ScriptError> {
        self.execute(&self.plan("prerm", action)?)
    }

    /// Run `postrm` with the given action.
    pub fn run_postrm(&self, action: &str) -> Result<ScriptOutcome, ScriptError> {
        self.execute(&self.plan("postrm", action)?)
    }

    /// Run a maintainer script asynchronously (spawns on a blocking thread).
    pub async fn execute_async(&self, plan: ScriptPlan) -> Result<ScriptOutcome, ScriptError> {
        let this = self.clone();
        tokio::task::spawn_blocking(move || this.execute(&plan))
            .await
            .map_err(|e| ScriptError::SpawnFailed(e.to_string()))?
    }
}

/// Map a [`std::process::ExitStatus`] to a [`ScriptOutcome`].
fn exit_to_outcome(status: ExitStatus) -> Result<ScriptOutcome, ScriptError> {
    if let Some(code) = status.code() {
        Ok(ScriptOutcome { exit_code: code })
    } else {
        Err(ScriptError::Signaled)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pkg() -> PackageRef {
        PackageRef {
            name: "curl".into(),
            version: "8.2.1-1".into(),
            arch: "amd64".into(),
        }
    }

    #[test]
    fn plan_sets_dpkg_env_vars() {
        let runner = ScriptRunner::new(PathBuf::from("/tmp/ctrl"), pkg());
        let env: HashMap<_, _> = runner.script_env().into_iter().collect();
        assert_eq!(env.get("DPKG_MAINTSCRIPT_PACKAGE").unwrap(), "curl");
        assert_eq!(env.get("DPKG_MAINTSCRIPT_VERSION").unwrap(), "8.2.1-1");
        assert_eq!(env.get("DPKG_MAINTSCRIPT_ARCH").unwrap(), "amd64");
        assert_eq!(env.get("DEBIAN_FRONTEND").unwrap(), "noninteractive");
        assert_eq!(env.get("PATH").unwrap(), "/usr/sbin:/usr/bin:/sbin:/bin");
    }

    #[test]
    fn plan_default_frontend_overridable() {
        let mut config = RunnerConfig {
            control_dir: PathBuf::from("/tmp/ctrl"),
            ..Default::default()
        };
        config.debian_frontend = Some("noninteractive".into());
        let runner = ScriptRunner::new(PathBuf::from("/tmp/ctrl"), pkg()).with_config(config);
        let env: HashMap<_, _> = runner.script_env().into_iter().collect();
        assert_eq!(env.get("DEBIAN_FRONTEND").unwrap(), "noninteractive");
    }

    #[test]
    fn missing_script_is_error() {
        let runner = ScriptRunner::new(PathBuf::from("/nonexistent-dir-xyz"), pkg());
        let err = runner.run_postinst("configure");
        assert!(matches!(err, Err(ScriptError::ScriptNotFound(..))));
    }

    #[test]
    fn unrestricted_disables_sandbox() {
        let runner = ScriptRunner::unrestricted(PathBuf::from("/tmp/ctrl"), pkg());
        assert!(!runner.is_sandboxed());
    }

    #[cfg(unix)]
    #[test]
    fn exit_code_propagation() {
        use std::io::Write;
        let dir = tempfile::tempdir().unwrap();
        let script = dir.path().join("postinst");
        let mut f = std::fs::File::create(&script).unwrap();
        writeln!(f, "#!/bin/sh\nexit 3\n").unwrap();
        drop(f);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = std::fs::metadata(&script).unwrap().permissions();
            perms.set_mode(0o755);
            std::fs::set_permissions(&script, perms).unwrap();
        }
        let runner = ScriptRunner::unrestricted(dir.path().to_path_buf(), pkg());
        let outcome = runner.run_postinst("configure").unwrap();
        assert!(!outcome.success());
        assert_eq!(outcome.exit_code, 3);
    }

    #[cfg(unix)]
    #[test]
    fn environment_injection_visible_to_script() {
        use std::io::Write;
        let dir = tempfile::tempdir().unwrap();
        let script = dir.path().join("postinst");
        let mut f = std::fs::File::create(&script).unwrap();
        writeln!(
            f,
            "#!/bin/sh\n[ \"$DPKG_MAINTSCRIPT_PACKAGE\" = \"curl\" ] || exit 1\nexit 0\n"
        )
        .unwrap();
        drop(f);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = std::fs::metadata(&script).unwrap().permissions();
            perms.set_mode(0o755);
            std::fs::set_permissions(&script, perms).unwrap();
        }
        let runner = ScriptRunner::unrestricted(dir.path().to_path_buf(), pkg());
        let outcome = runner.run_postinst("configure").unwrap();
        assert!(outcome.success());
    }
}
