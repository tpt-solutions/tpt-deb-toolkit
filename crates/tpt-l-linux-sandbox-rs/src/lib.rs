//! Linux namespace and seccomp sandbox for Debian maintainer scripts.
//!
//! This crate provides isolation primitives for running `preinst`, `postinst`,
//! `prerm`, and `postrm` maintainer scripts in a restricted Linux environment.
//!
//! On non-Linux platforms the types are still available for cross-compilation
//! purposes, but [`Sandbox::run`] will return [`SandboxError::UnsupportedPlatform`].

use std::path::PathBuf;

#[cfg(target_os = "linux")]
use std::process::ExitStatus;

#[cfg(not(target_os = "linux"))]
/// Placeholder ExitStatus for non-Linux builds.
pub struct ExitStatus;

use thiserror::Error;

mod mount;
mod seccomp;

pub use mount::BindMount;
pub use seccomp::{SeccompAction, SeccompProfile, SeccompRule};

/// Errors that can occur when creating or running a sandbox.
#[derive(Debug, Error)]
pub enum SandboxError {
    /// The current platform does not support namespace isolation.
    #[error("sandbox is not supported on this platform")]
    UnsupportedPlatform,

    /// A system call required for namespace setup failed.
    #[error("namespace setup failed: {0}")]
    NamespaceSetup(String),

    /// The child process could not be spawned.
    #[error("failed to spawn child process: {0}")]
    SpawnFailed(String),

    /// An I/O error occurred while communicating with the child.
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    /// A bind mount could not be established.
    #[error("bind mount failed for {src} -> {dst}: {reason}")]
    BindMountFailed {
        src: PathBuf,
        dst: PathBuf,
        reason: String,
    },

    /// A seccomp filter could not be installed.
    #[error("seccomp install failed: {0}")]
    SeccompInstall(String),
}

/// Configuration controlling what resources a sandboxed process can access.
#[derive(Debug, Clone)]
pub struct SandboxConfig {
    /// Whether the sandboxed process may access the network.
    ///
    /// When `false` the process is placed in a new network namespace with no
    /// interfaces other than loopback.
    pub allow_network: bool,

    /// Whether the sandboxed process may access System V IPC facilities.
    pub allow_ipc: bool,

    /// Additional bind mounts to establish inside the sandbox.
    ///
    /// Each entry maps a `source` on the host to a `destination` inside the
    /// sandbox. Mounts are applied to the sandboxed child before the target
    /// command is executed; set [`BindMount::read_only`] to expose a path
    /// without write access.
    pub extra_bind_mounts: Vec<BindMount>,

    /// Seccomp syscall allowlist applied to the sandboxed process.
    ///
    /// Defaults to [`SeccompProfile::maintainer_script_profile`].
    pub seccomp: SeccompProfile,
}

impl Default for SandboxConfig {
    fn default() -> Self {
        Self::maintainer_script_profile()
    }
}

impl SandboxConfig {
    /// Returns a restrictive configuration suitable for Debian maintainer scripts.
    ///
    /// Network access and IPC are both disabled.  This is the recommended
    /// profile for `preinst`, `postinst`, `prerm`, and `postrm` scripts.
    pub fn maintainer_script_profile() -> Self {
        Self {
            allow_network: false,
            allow_ipc: false,
            extra_bind_mounts: Vec::new(),
            seccomp: SeccompProfile::maintainer_script_profile(),
        }
    }

    /// Returns an unrestricted configuration that skips namespace isolation.
    ///
    /// # Warning
    ///
    /// Using this profile gives the child process full access to the host
    /// network and IPC facilities.  A [`tracing::warn!`] message is emitted
    /// whenever a sandbox built with this profile is executed.
    pub fn unrestricted() -> Self {
        Self {
            allow_network: true,
            allow_ipc: true,
            extra_bind_mounts: Vec::new(),
            seccomp: SeccompProfile::disabled(),
        }
    }

    /// Returns `true` if isolation should be skipped entirely.
    #[cfg_attr(not(target_os = "linux"), allow(dead_code))]
    fn is_unrestricted(&self) -> bool {
        self.allow_network
            && self.allow_ipc
            && self.extra_bind_mounts.is_empty()
            && !self.seccomp.enabled
    }
}

/// Builder for constructing a [`Sandbox`].
///
/// # Example
///
/// ```no_run
/// use tpt_l_linux_sandbox_rs::{SandboxBuilder, SandboxConfig};
///
/// let sandbox = SandboxBuilder::new()
///     .config(SandboxConfig::maintainer_script_profile())
///     .build();
/// ```
#[derive(Debug, Default)]
pub struct SandboxBuilder {
    config: Option<SandboxConfig>,
}

impl SandboxBuilder {
    /// Creates a new builder with default settings.
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets the [`SandboxConfig`] to use.
    pub fn config(mut self, config: SandboxConfig) -> Self {
        self.config = Some(config);
        self
    }

    /// Builds the [`Sandbox`].
    pub fn build(self) -> Sandbox {
        Sandbox {
            config: self.config.unwrap_or_default(),
        }
    }
}

/// A configured sandbox capable of running commands in an isolated environment.
///
/// On Linux, the sandbox uses user, PID, mount, and (optionally) network
/// namespaces to isolate the child process.  On other platforms the child is
/// run without any isolation and [`Sandbox::run`] returns
/// [`SandboxError::UnsupportedPlatform`].
#[derive(Debug)]
pub struct Sandbox {
    config: SandboxConfig,
}

impl Sandbox {
    /// Creates a [`Sandbox`] using the default [`SandboxConfig`]
    /// ([`SandboxConfig::maintainer_script_profile`]).
    pub fn new() -> Self {
        SandboxBuilder::new().build()
    }

    /// Returns a reference to the active [`SandboxConfig`].
    pub fn config(&self) -> &SandboxConfig {
        &self.config
    }

    /// Runs `cmd` with `args` and `env` inside the sandbox.
    ///
    /// # Errors
    ///
    /// Returns [`SandboxError::UnsupportedPlatform`] on non-Linux hosts.
    /// On Linux, returns errors if namespace setup or process spawning fails.
    pub fn run(
        &self,
        cmd: &str,
        args: &[&str],
        env: &[(&str, &str)],
    ) -> Result<ExitStatus, SandboxError> {
        self.run_impl(cmd, args, env)
    }

    #[cfg(target_os = "linux")]
    fn run_impl(
        &self,
        cmd: &str,
        args: &[&str],
        env: &[(&str, &str)],
    ) -> Result<ExitStatus, SandboxError> {
        if self.config.is_unrestricted() {
            tracing::warn!(
                "Running command '{}' without sandbox isolation (unrestricted profile)",
                cmd
            );
            return self.run_direct(cmd, args, env);
        }

        self.run_namespaced(cmd, args, env)
    }

    #[cfg(target_os = "linux")]
    fn run_direct(
        &self,
        cmd: &str,
        args: &[&str],
        env: &[(&str, &str)],
    ) -> Result<ExitStatus, SandboxError> {
        use std::process::Command;
        let mut child = Command::new(cmd);
        child.args(args);
        child.env_clear();
        for (k, v) in env {
            child.env(k, v);
        }
        let status = child.status()?;
        Ok(status)
    }

    /// Runs the command inside new Linux namespaces.
    ///
    /// Namespace setup is performed in a single-threaded child (a process may
    /// not `unshare(2)` a user namespace while it still has multiple threads,
    /// which is always the case inside `cargo test`).  A second `fork(2)` after
    /// `unshare(CLONE_NEWPID)` ensures the exec'd command becomes PID 1 in the
    /// new PID namespace.  UID/GID mapping (real uid/gid → 0) is written so the
    /// child appears to be root inside its user namespace.
    #[cfg(target_os = "linux")]
    fn run_namespaced(
        &self,
        cmd: &str,
        args: &[&str],
        env: &[(&str, &str)],
    ) -> Result<ExitStatus, SandboxError> {
        use libc::{CLONE_NEWNS, CLONE_NEWPID, CLONE_NEWUSER};
        use std::ffi::CString;
        use std::os::unix::process::ExitStatusExt;

        // Determine unshare flags
        let mut flags = CLONE_NEWUSER | CLONE_NEWPID | CLONE_NEWNS;
        if !self.config.allow_network {
            flags |= libc::CLONE_NEWNET;
        }
        if !self.config.allow_ipc {
            flags |= libc::CLONE_NEWIPC;
        }

        // Real uid/gid of the caller, used for the user-namespace id map.
        let uid = unsafe { libc::getuid() };
        let gid = unsafe { libc::getgid() };

        // Build argv/envp CStrings in the parent (malloc is unsafe in the
        // post-fork child because the malloc lock may be held by another thread).
        let c_cmd = CString::new(cmd).map_err(|e| SandboxError::SpawnFailed(e.to_string()))?;
        let mut c_args: Vec<CString> = Vec::with_capacity(args.len() + 1);
        c_args.push(c_cmd.clone());
        for a in args {
            c_args.push(CString::new(*a).map_err(|e| SandboxError::SpawnFailed(e.to_string()))?);
        }
        let mut c_env: Vec<CString> = Vec::with_capacity(env.len());
        for (k, v) in env {
            let pair = format!("{}={}", k, v);
            c_env.push(CString::new(pair).map_err(|e| SandboxError::SpawnFailed(e.to_string()))?);
        }
        let mut argv_ptrs: Vec<*const libc::c_char> = c_args.iter().map(|s| s.as_ptr()).collect();
        argv_ptrs.push(std::ptr::null());
        let mut envp_ptrs: Vec<*const libc::c_char> = c_env.iter().map(|s| s.as_ptr()).collect();
        envp_ptrs.push(std::ptr::null());

        let bind_mounts = &self.config.extra_bind_mounts;
        let seccomp_profile = &self.config.seccomp;

        // Fork: the child is now single-threaded and may unshare(CLONE_NEWUSER).
        let child_pid = unsafe { libc::fork() };
        match child_pid {
            -1 => {
                let err = std::io::Error::last_os_error();
                Err(SandboxError::SpawnFailed(format!("fork failed: {}", err)))
            }
            0 => {
                // Middle child: set up namespaces then fork again so the
                // grandchild exec's as PID 1 in the new PID namespace.
                unsafe {
                    if libc::unshare(flags) != 0 {
                        let err = std::io::Error::last_os_error();
                        eprintln!("sandbox unshare(0x{:x}) failed: {}", flags, err);
                        libc::_exit(127);
                    }

                    // Write UID/GID maps for the new user namespace.  setgroups
                    // must be denied before the gid_map can be written.
                    let uid_map = format!("0 {} 1\n", uid);
                    let gid_map = format!("0 {} 1\n", gid);
                    let _ = std::fs::write("/proc/self/uid_map", uid_map.as_bytes());
                    let _ = std::fs::write("/proc/self/setgroups", b"deny");
                    let _ = std::fs::write("/proc/self/gid_map", gid_map.as_bytes());

                    let grandchild = libc::fork();
                    match grandchild {
                        -1 => libc::_exit(127),
                        0 => {
                            // Grandchild: apply restrictions, then exec.
                            if let Err(e) = mount::apply_bind_mounts(bind_mounts) {
                                eprintln!("sandbox bind mount failed: {}", e);
                                libc::_exit(127);
                            }
                            if let Err(e) = seccomp::install_seccomp(seccomp_profile) {
                                eprintln!("sandbox seccomp install failed: {}", e);
                                libc::_exit(127);
                            }
                            libc::execvpe(c_cmd.as_ptr(), argv_ptrs.as_ptr(), envp_ptrs.as_ptr());
                            libc::_exit(127);
                        }
                        _ => {
                            // Middle child: wait for the grandchild and propagate
                            // its exit status (or 128+signal) to the parent.
                            let mut wstatus: libc::c_int = 0;
                            libc::waitpid(grandchild, &mut wstatus, 0);
                            let code = if libc::WIFEXITED(wstatus) {
                                libc::WEXITSTATUS(wstatus)
                            } else if libc::WIFSIGNALED(wstatus) {
                                128 + libc::WTERMSIG(wstatus)
                            } else {
                                127
                            };
                            libc::_exit(code);
                        }
                    }
                }
            }
            _ => {
                // Parent: wait for the middle child.
                let mut wstatus: libc::c_int = 0;
                loop {
                    let ret = unsafe { libc::waitpid(child_pid, &mut wstatus, 0) };
                    if ret == -1 {
                        let err = std::io::Error::last_os_error();
                        if err.raw_os_error() == Some(libc::EINTR) {
                            continue;
                        }
                        return Err(SandboxError::Io(err));
                    }
                    break;
                }
                Ok(ExitStatus::from_raw(wstatus))
            }
        }
    }

    #[cfg(not(target_os = "linux"))]
    fn run_impl(
        &self,
        _cmd: &str,
        _args: &[&str],
        _env: &[(&str, &str)],
    ) -> Result<ExitStatus, SandboxError> {
        Err(SandboxError::UnsupportedPlatform)
    }
}

impl Default for Sandbox {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maintainer_script_profile_disallows_network() {
        let cfg = SandboxConfig::maintainer_script_profile();
        assert!(
            !cfg.allow_network,
            "network should be disallowed in maintainer script profile"
        );
    }

    #[test]
    fn maintainer_script_profile_disallows_ipc() {
        let cfg = SandboxConfig::maintainer_script_profile();
        assert!(
            !cfg.allow_ipc,
            "IPC should be disallowed in maintainer script profile"
        );
    }

    #[test]
    fn unrestricted_allows_network() {
        let cfg = SandboxConfig::unrestricted();
        assert!(
            cfg.allow_network,
            "network should be allowed in unrestricted profile"
        );
    }

    #[test]
    fn unrestricted_allows_ipc() {
        let cfg = SandboxConfig::unrestricted();
        assert!(
            cfg.allow_ipc,
            "IPC should be allowed in unrestricted profile"
        );
    }

    #[test]
    fn builder_uses_provided_config() {
        let cfg = SandboxConfig::maintainer_script_profile();
        let sandbox = SandboxBuilder::new().config(cfg.clone()).build();
        assert!(!sandbox.config().allow_network);
        assert!(!sandbox.config().allow_ipc);
    }

    #[test]
    fn default_sandbox_uses_restrictive_config() {
        let sandbox = Sandbox::new();
        assert!(!sandbox.config().allow_network);
        assert!(!sandbox.config().allow_ipc);
    }

    #[cfg(not(target_os = "linux"))]
    #[test]
    fn non_linux_returns_unsupported() {
        let sandbox = Sandbox::new();
        let result = sandbox.run("echo", &["hello"], &[]);
        assert!(matches!(result, Err(SandboxError::UnsupportedPlatform)));
    }

    #[cfg(target_os = "linux")]
    fn namespaces_available() -> bool {
        // Detect whether the host permits unprivileged user namespaces, which
        // the sandbox depends on.  When they are disabled (common inside some
        // CI containers) `unshare(CLONE_NEWUSER)` fails with EPERM and the
        // sandbox cannot isolate anything; such tests are skipped instead.
        unsafe {
            let pid = libc::fork();
            if pid == 0 {
                let ok = libc::unshare(libc::CLONE_NEWUSER) == 0;
                libc::_exit(if ok { 0 } else { 1 });
            }
            let mut status: libc::c_int = 0;
            libc::waitpid(pid, &mut status, 0);
            libc::WIFEXITED(status) && libc::WEXITSTATUS(status) == 0
        }
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn runs_a_real_script_inside_the_sandbox() {
        if !namespaces_available() {
            eprintln!("skipping: user namespaces are unavailable on this host");
            return;
        }
        let sandbox = Sandbox::new();
        let status = sandbox
            .run("/bin/true", &[], &[])
            .expect("sandbox should run /bin/true");
        assert!(status.success(), "sandboxed /bin/true must exit 0");
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn bind_mount_is_visible_inside_the_sandbox() {
        if !namespaces_available() {
            eprintln!("skipping: user namespaces are unavailable on this host");
            return;
        }
        use std::io::Write;
        let host = tempfile::NamedTempFile::new().expect("temp file");
        host.as_file()
            .write_all(b"hello-sandbox")
            .expect("write marker");
        let dest = tempfile::tempdir().expect("temp dir").keep().join("flag");
        let mut cfg = SandboxConfig::maintainer_script_profile();
        cfg.extra_bind_mounts
            .push(BindMount::read_only(host.path(), &dest));
        let sandbox = SandboxBuilder::new().config(cfg).build();

        let script = format!("test \"$(cat {})\" = hello-sandbox", dest.display());
        let status = sandbox
            .run("/bin/sh", &["-c", &script], &[])
            .expect("sandbox should run script");
        assert!(
            status.success(),
            "sandboxed script must read the bind-mounted file"
        );
    }
}
