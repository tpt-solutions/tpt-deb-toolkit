//! Filesystem bind-mount configuration for the sandbox.
//!
//! These helpers are applied inside the sandboxed child process (Linux only)
//! to expose selected host paths inside the sandbox without copying them.

#[cfg(target_os = "linux")]
use crate::SandboxError;

#[cfg(target_os = "linux")]
use std::path::Path;

/// A bind mount applied to the sandbox filesystem.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BindMount {
    /// Path on the host to mount from.
    pub source: std::path::PathBuf,
    /// Path inside the sandbox to mount at.
    pub destination: std::path::PathBuf,
    /// When `true` the mount is remounted read-only.
    pub read_only: bool,
}

impl BindMount {
    /// Create a writable bind mount from `source` to `destination`.
    pub fn new(
        source: impl Into<std::path::PathBuf>,
        destination: impl Into<std::path::PathBuf>,
    ) -> Self {
        Self {
            source: source.into(),
            destination: destination.into(),
            read_only: false,
        }
    }

    /// Create a read-only bind mount from `source` to `destination`.
    pub fn read_only(
        source: impl Into<std::path::PathBuf>,
        destination: impl Into<std::path::PathBuf>,
    ) -> Self {
        Self {
            source: source.into(),
            destination: destination.into(),
            read_only: true,
        }
    }
}

#[cfg(target_os = "linux")]
fn ensure_destination(source: &Path, destination: &Path) -> Result<(), SandboxError> {
    use std::io;
    if destination.exists() {
        return Ok(());
    }
    let is_dir = std::fs::metadata(source)
        .map(|m| m.is_dir())
        .unwrap_or(false);
    if is_dir {
        std::fs::create_dir_all(destination)
    } else {
        if let Some(parent) = destination.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::File::create(destination).map(|_| ())
    }
    .map_err(|e: io::Error| SandboxError::BindMountFailed {
        src: source.to_path_buf(),
        dst: destination.to_path_buf(),
        reason: format!("prepare destination: {}", e),
    })
}

#[cfg(target_os = "linux")]
pub(crate) fn apply_bind_mounts(mounts: &[BindMount]) -> Result<(), SandboxError> {
    use std::io;
    use std::os::unix::ffi::OsStrExt;

    // Nothing to do when no bind mounts were requested.  Skipping the
    // `mount(MS_REC | MS_PRIVATE, "/")` call here matters because some hosts
    // (notably CI runners with restricted user namespaces) deny the
    // propagation-change mount even though they otherwise permit the sandbox to
    // run.  A maintainer-script profile with no extra mounts must not pay for a
    // mount syscall it does not need.
    if mounts.is_empty() {
        return Ok(());
    }

    unsafe {
        // Mark the whole tree private so our mounts do not leak to the host.
        let root = std::ffi::CString::new("/").unwrap();
        if libc::mount(
            std::ptr::null(),
            root.as_ptr(),
            std::ptr::null(),
            libc::MS_REC | libc::MS_PRIVATE,
            std::ptr::null(),
        ) != 0
        {
            return Err(SandboxError::NamespaceSetup(format!(
                "make mount namespace private: {}",
                io::Error::last_os_error()
            )));
        }
    }

    for m in mounts {
        ensure_destination(&m.source, &m.destination)?;
        unsafe {
            let src = std::ffi::CString::new(m.source.as_os_str().as_bytes()).map_err(|e| {
                SandboxError::BindMountFailed {
                    src: m.source.clone(),
                    dst: m.destination.clone(),
                    reason: e.to_string(),
                }
            })?;
            let dst =
                std::ffi::CString::new(m.destination.as_os_str().as_bytes()).map_err(|e| {
                    SandboxError::BindMountFailed {
                        src: m.source.clone(),
                        dst: m.destination.clone(),
                        reason: e.to_string(),
                    }
                })?;
            if libc::mount(
                src.as_ptr(),
                dst.as_ptr(),
                std::ptr::null(),
                libc::MS_BIND,
                std::ptr::null(),
            ) != 0
            {
                return Err(SandboxError::BindMountFailed {
                    src: m.source.clone(),
                    dst: m.destination.clone(),
                    reason: io::Error::last_os_error().to_string(),
                });
            }
            if m.read_only
                && libc::mount(
                    std::ptr::null(),
                    dst.as_ptr(),
                    std::ptr::null(),
                    libc::MS_REMOUNT | libc::MS_BIND | libc::MS_RDONLY,
                    std::ptr::null(),
                ) != 0
            {
                return Err(SandboxError::BindMountFailed {
                    src: m.source.clone(),
                    dst: m.destination.clone(),
                    reason: format!("remount read-only: {}", io::Error::last_os_error()),
                });
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bind_mount_new_is_writable() {
        let m = BindMount::new("/host", "/sandbox");
        assert!(!m.read_only);
        assert_eq!(m.source, std::path::Path::new("/host"));
        assert_eq!(m.destination, std::path::Path::new("/sandbox"));
    }

    #[test]
    fn bind_mount_read_only_marks_flag() {
        let m = BindMount::read_only("/host", "/sandbox");
        assert!(m.read_only);
    }
}
