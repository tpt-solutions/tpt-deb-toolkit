//! Seccomp (Linux secure computing) allowlist support.
//!
//! This module builds a classic BPF filter program that restricts the system
//! calls a sandboxed process may make.  The filter *builder* is platform
//! independent (it only produces a list of `BpfInsn`s) so it can be unit
//! tested anywhere; installing the filter via `prctl(2)` is Linux-only.

use std::fmt;

/// Action taken when a syscall is not explicitly allowed by the profile.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SeccompAction {
    /// Allow the syscall (`SECCOMP_RET_ALLOW`).
    Allow,
    /// Deny the syscall and return the given errno to the caller.
    ///
    /// This is the default for the maintainer-script profile: it blocks
    /// forbidden syscalls without terminating the whole script, which is
    /// friendlier to packages that merely probe for optional functionality.
    Errno(u16),
    /// Kill the offending thread (`SECCOMP_RET_KILL_THREAD`).
    KillThread,
    /// Kill the whole process (`SECCOMP_RET_KILL_PROCESS`).
    KillProcess,
}

impl fmt::Display for SeccompAction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SeccompAction::Allow => write!(f, "allow"),
            SeccompAction::Errno(e) => write!(f, "errno({})", e),
            SeccompAction::KillThread => write!(f, "kill-thread"),
            SeccompAction::KillProcess => write!(f, "kill-process"),
        }
    }
}

impl SeccompAction {
    fn to_ret(self) -> u32 {
        const SECCOMP_RET_ALLOW: u32 = 0x7fff_0000;
        const SECCOMP_RET_KILL_THREAD: u32 = 0x0000_0000;
        const SECCOMP_RET_KILL_PROCESS: u32 = 0x8000_0000;
        const SECCOMP_RET_ERRNO: u32 = 0x0005_0000;
        match self {
            SeccompAction::Allow => SECCOMP_RET_ALLOW,
            SeccompAction::Errno(e) => SECCOMP_RET_ERRNO | (u32::from(e) & 0xffff),
            SeccompAction::KillThread => SECCOMP_RET_KILL_THREAD,
            SeccompAction::KillProcess => SECCOMP_RET_KILL_PROCESS,
        }
    }
}

/// A single rule in a [`SeccompProfile`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SeccompRule {
    /// Unconditionally allow the given syscall number.
    AllowSyscall(i64),
    /// Allow the given syscall only when argument `arg` (0..5) equals `value`.
    ///
    /// This is used, for example, to permit `socket(2)` for `AF_UNIX` (and
    /// possibly `AF_NETLINK`) while denying it for `AF_INET`/`AF_INET6`.
    AllowSyscallIfArg(i64, u8, u64),
}

/// A seccomp allowlist profile.
#[derive(Debug, Clone)]
pub struct SeccompProfile {
    /// Whether the profile should be installed at all.
    pub enabled: bool,
    /// Action taken for syscalls that are not explicitly allowed.
    pub action: SeccompAction,
    /// Allowlist rules, evaluated in order.
    pub rules: Vec<SeccompRule>,
}

impl SeccompProfile {
    /// A profile that installs no filter at all.
    pub fn disabled() -> Self {
        Self {
            enabled: false,
            action: SeccompAction::Errno(1),
            rules: Vec::new(),
        }
    }

    /// A restrictive, package-maintainer-friendly allowlist.
    ///
    /// Network-facing sockets (`AF_INET`/`AF_INET6`) are denied while
    /// `AF_UNIX` is still permitted so that scripts can talk to local
    /// services such as `systemd`-notify.  Denied syscalls fail with `EPERM`
    /// rather than killing the process.
    #[cfg(target_os = "linux")]
    #[allow(clippy::unnecessary_cast)]
    pub fn maintainer_script_profile() -> Self {
        use libc::*;
        let rules = vec![
            SeccompRule::AllowSyscall(SYS_read as i64),
            SeccompRule::AllowSyscall(SYS_write as i64),
            SeccompRule::AllowSyscall(SYS_readv as i64),
            SeccompRule::AllowSyscall(SYS_writev as i64),
            SeccompRule::AllowSyscall(SYS_pread64 as i64),
            SeccompRule::AllowSyscall(SYS_pwrite64 as i64),
            SeccompRule::AllowSyscall(SYS_open as i64),
            SeccompRule::AllowSyscall(SYS_openat as i64),
            SeccompRule::AllowSyscall(SYS_close as i64),
            SeccompRule::AllowSyscall(SYS_close_range as i64),
            SeccompRule::AllowSyscall(SYS_lseek as i64),
            SeccompRule::AllowSyscall(SYS_ftruncate as i64),
            SeccompRule::AllowSyscall(SYS_truncate as i64),
            SeccompRule::AllowSyscall(SYS_fstat as i64),
            SeccompRule::AllowSyscall(SYS_newfstatat as i64),
            SeccompRule::AllowSyscall(SYS_lstat as i64),
            SeccompRule::AllowSyscall(SYS_stat as i64),
            SeccompRule::AllowSyscall(SYS_statx as i64),
            SeccompRule::AllowSyscall(SYS_access as i64),
            SeccompRule::AllowSyscall(SYS_faccessat as i64),
            SeccompRule::AllowSyscall(SYS_faccessat2 as i64),
            SeccompRule::AllowSyscall(SYS_dup as i64),
            SeccompRule::AllowSyscall(SYS_dup2 as i64),
            SeccompRule::AllowSyscall(SYS_dup3 as i64),
            SeccompRule::AllowSyscall(SYS_fcntl as i64),
            SeccompRule::AllowSyscall(SYS_ioctl as i64),
            SeccompRule::AllowSyscall(SYS_pipe as i64),
            SeccompRule::AllowSyscall(SYS_pipe2 as i64),
            SeccompRule::AllowSyscall(SYS_poll as i64),
            SeccompRule::AllowSyscall(SYS_ppoll as i64),
            SeccompRule::AllowSyscall(SYS_select as i64),
            SeccompRule::AllowSyscall(SYS_pselect6 as i64),
            SeccompRule::AllowSyscall(SYS_getpid as i64),
            SeccompRule::AllowSyscall(SYS_getppid as i64),
            SeccompRule::AllowSyscall(SYS_gettid as i64),
            SeccompRule::AllowSyscall(SYS_getuid as i64),
            SeccompRule::AllowSyscall(SYS_geteuid as i64),
            SeccompRule::AllowSyscall(SYS_getgid as i64),
            SeccompRule::AllowSyscall(SYS_getegid as i64),
            SeccompRule::AllowSyscall(SYS_getgroups as i64),
            SeccompRule::AllowSyscall(SYS_getcwd as i64),
            SeccompRule::AllowSyscall(SYS_chdir as i64),
            SeccompRule::AllowSyscall(SYS_fchdir as i64),
            SeccompRule::AllowSyscall(SYS_mkdir as i64),
            SeccompRule::AllowSyscall(SYS_mkdirat as i64),
            SeccompRule::AllowSyscall(SYS_rmdir as i64),
            SeccompRule::AllowSyscall(SYS_unlink as i64),
            SeccompRule::AllowSyscall(SYS_unlinkat as i64),
            SeccompRule::AllowSyscall(SYS_rename as i64),
            SeccompRule::AllowSyscall(SYS_renameat as i64),
            SeccompRule::AllowSyscall(SYS_renameat2 as i64),
            SeccompRule::AllowSyscall(SYS_symlink as i64),
            SeccompRule::AllowSyscall(SYS_symlinkat as i64),
            SeccompRule::AllowSyscall(SYS_link as i64),
            SeccompRule::AllowSyscall(SYS_linkat as i64),
            SeccompRule::AllowSyscall(SYS_readlink as i64),
            SeccompRule::AllowSyscall(SYS_readlinkat as i64),
            SeccompRule::AllowSyscall(SYS_chmod as i64),
            SeccompRule::AllowSyscall(SYS_fchmod as i64),
            SeccompRule::AllowSyscall(SYS_fchmodat as i64),
            SeccompRule::AllowSyscall(SYS_chown as i64),
            SeccompRule::AllowSyscall(SYS_fchown as i64),
            SeccompRule::AllowSyscall(SYS_fchownat as i64),
            SeccompRule::AllowSyscall(SYS_lchown as i64),
            SeccompRule::AllowSyscall(SYS_umask as i64),
            SeccompRule::AllowSyscall(SYS_utime as i64),
            SeccompRule::AllowSyscall(SYS_utimes as i64),
            SeccompRule::AllowSyscall(SYS_utimensat as i64),
            SeccompRule::AllowSyscall(SYS_mknod as i64),
            SeccompRule::AllowSyscall(SYS_mknodat as i64),
            SeccompRule::AllowSyscall(SYS_mount as i64),
            SeccompRule::AllowSyscall(SYS_umount2 as i64),
            SeccompRule::AllowSyscall(SYS_fork as i64),
            SeccompRule::AllowSyscall(SYS_vfork as i64),
            SeccompRule::AllowSyscall(SYS_clone as i64),
            SeccompRule::AllowSyscall(SYS_clone3 as i64),
            SeccompRule::AllowSyscall(SYS_execve as i64),
            SeccompRule::AllowSyscall(SYS_execveat as i64),
            SeccompRule::AllowSyscall(SYS_wait4 as i64),
            SeccompRule::AllowSyscall(SYS_waitid as i64),
            SeccompRule::AllowSyscall(SYS_exit as i64),
            SeccompRule::AllowSyscall(SYS_exit_group as i64),
            SeccompRule::AllowSyscall(SYS_kill as i64),
            SeccompRule::AllowSyscall(SYS_tgkill as i64),
            SeccompRule::AllowSyscall(SYS_tkill as i64),
            SeccompRule::AllowSyscall(SYS_rt_sigaction as i64),
            SeccompRule::AllowSyscall(SYS_rt_sigprocmask as i64),
            SeccompRule::AllowSyscall(SYS_rt_sigreturn as i64),
            SeccompRule::AllowSyscall(SYS_sigaltstack as i64),
            SeccompRule::AllowSyscall(SYS_pause as i64),
            SeccompRule::AllowSyscall(SYS_nanosleep as i64),
            SeccompRule::AllowSyscall(SYS_clock_nanosleep as i64),
            SeccompRule::AllowSyscall(SYS_clock_gettime as i64),
            SeccompRule::AllowSyscall(SYS_clock_getres as i64),
            SeccompRule::AllowSyscall(SYS_gettimeofday as i64),
            SeccompRule::AllowSyscall(SYS_time as i64),
            SeccompRule::AllowSyscall(SYS_getrlimit as i64),
            SeccompRule::AllowSyscall(SYS_setrlimit as i64),
            SeccompRule::AllowSyscall(SYS_prlimit64 as i64),
            SeccompRule::AllowSyscall(SYS_prctl as i64),
            SeccompRule::AllowSyscall(SYS_personality as i64),
            SeccompRule::AllowSyscall(SYS_arch_prctl as i64),
            SeccompRule::AllowSyscall(SYS_set_tid_address as i64),
            SeccompRule::AllowSyscall(SYS_set_robust_list as i64),
            SeccompRule::AllowSyscall(SYS_get_robust_list as i64),
            SeccompRule::AllowSyscall(SYS_futex as i64),
            SeccompRule::AllowSyscall(SYS_mmap as i64),
            SeccompRule::AllowSyscall(SYS_munmap as i64),
            SeccompRule::AllowSyscall(SYS_mprotect as i64),
            SeccompRule::AllowSyscall(SYS_mremap as i64),
            SeccompRule::AllowSyscall(SYS_madvise as i64),
            SeccompRule::AllowSyscall(SYS_brk as i64),
            SeccompRule::AllowSyscall(SYS_mlock as i64),
            SeccompRule::AllowSyscall(SYS_munlock as i64),
            SeccompRule::AllowSyscall(SYS_getdents as i64),
            SeccompRule::AllowSyscall(SYS_getdents64 as i64),
            SeccompRule::AllowSyscall(SYS_sched_yield as i64),
            SeccompRule::AllowSyscall(SYS_sched_getaffinity as i64),
            SeccompRule::AllowSyscall(SYS_sched_setaffinity as i64),
            SeccompRule::AllowSyscall(SYS_getpriority as i64),
            SeccompRule::AllowSyscall(SYS_setpriority as i64),
            SeccompRule::AllowSyscall(SYS_capget as i64),
            SeccompRule::AllowSyscall(SYS_capset as i64),
            SeccompRule::AllowSyscall(SYS_getrandom as i64),
            SeccompRule::AllowSyscall(SYS_memfd_create as i64),
            SeccompRule::AllowSyscall(SYS_eventfd2 as i64),
            SeccompRule::AllowSyscall(SYS_epoll_create1 as i64),
            SeccompRule::AllowSyscall(SYS_epoll_ctl as i64),
            SeccompRule::AllowSyscall(SYS_epoll_wait as i64),
            SeccompRule::AllowSyscall(SYS_timerfd_create as i64),
            SeccompRule::AllowSyscall(SYS_timerfd_settime as i64),
            SeccompRule::AllowSyscall(SYS_timerfd_gettime as i64),
            SeccompRule::AllowSyscall(SYS_signalfd4 as i64),
            SeccompRule::AllowSyscall(SYS_socketpair as i64),
            SeccompRule::AllowSyscall(SYS_bind as i64),
            SeccompRule::AllowSyscall(SYS_connect as i64),
            SeccompRule::AllowSyscall(SYS_listen as i64),
            SeccompRule::AllowSyscall(SYS_accept as i64),
            SeccompRule::AllowSyscall(SYS_accept4 as i64),
            SeccompRule::AllowSyscall(SYS_getsockname as i64),
            SeccompRule::AllowSyscall(SYS_getpeername as i64),
            SeccompRule::AllowSyscall(SYS_sendto as i64),
            SeccompRule::AllowSyscall(SYS_recvfrom as i64),
            SeccompRule::AllowSyscall(SYS_sendmsg as i64),
            SeccompRule::AllowSyscall(SYS_recvmsg as i64),
            SeccompRule::AllowSyscall(SYS_getsockopt as i64),
            SeccompRule::AllowSyscall(SYS_setsockopt as i64),
            SeccompRule::AllowSyscall(SYS_shutdown as i64),
            SeccompRule::AllowSyscallIfArg(SYS_socket as i64, 0, AF_UNIX as u64),
            SeccompRule::AllowSyscallIfArg(SYS_socket as i64, 0, AF_NETLINK as u64),
        ];
        Self {
            enabled: true,
            action: SeccompAction::Errno(libc::EPERM as u16),
            rules,
        }
    }

    /// On non-Linux hosts seccomp cannot be installed; return a disabled profile.
    #[cfg(not(target_os = "linux"))]
    pub fn maintainer_script_profile() -> Self {
        Self::disabled()
    }
}

// ---------------------------------------------------------------------------
// BPF program construction (platform independent)
// ---------------------------------------------------------------------------

/// A single classic-BPF instruction, laid out exactly like `struct sock_filter`.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub(crate) struct BpfInsn {
    pub code: u16,
    pub jt: u8,
    pub jf: u8,
    pub k: u32,
}

const BPF_LD: u16 = 0x00;
const BPF_W: u16 = 0x00;
const BPF_ABS: u16 = 0x20;
const BPF_JMP: u16 = 0x05;
const BPF_JEQ: u16 = 0x10;
const BPF_JA: u16 = 0x00;
const BPF_K: u16 = 0x00;
const BPF_RET: u16 = 0x06;

const SECCOMP_ARCH_OFFSET: u32 = 4; // offset of `arch` in `struct seccomp_data`
const SECCOMP_NR_OFFSET: u32 = 0; // offset of `nr` in `struct seccomp_data`
const SECCOMP_ARG_OFFSET: u32 = 16; // offset of `args[0]` in `struct seccomp_data`
const AUDIT_ARCH_X86_64: u32 = 0xC000_003E;

#[allow(dead_code)]
fn ld_abs(off: u32) -> BpfInsn {
    BpfInsn {
        code: BPF_LD | BPF_W | BPF_ABS,
        jt: 0,
        jf: 0,
        k: off,
    }
}

#[allow(dead_code)]
fn jeq(k: u32, jt: u8, jf: u8) -> BpfInsn {
    BpfInsn {
        code: BPF_JMP | BPF_JEQ | BPF_K,
        jt,
        jf,
        k,
    }
}

#[allow(dead_code)]
fn ja(off: u32) -> BpfInsn {
    BpfInsn {
        code: BPF_JMP | BPF_JA,
        jt: 0,
        jf: 0,
        k: off,
    }
}

#[allow(dead_code)]
fn ret(k: u32) -> BpfInsn {
    BpfInsn {
        code: BPF_RET | BPF_K,
        jt: 0,
        jf: 0,
        k,
    }
}

/// Build the classic-BPF program for `profile`.
///
/// The program fails closed: if the CPU architecture is not the expected
/// x86-64, or if a syscall is not on the allowlist, the configured default
/// `action` is taken.
#[allow(dead_code)]
pub(crate) fn build_program(profile: &SeccompProfile) -> Vec<BpfInsn> {
    const SECCOMP_RET_ALLOW: u32 = 0x7fff_0000;
    let mut p: Vec<BpfInsn> = vec![
        // 0: load arch
        ld_abs(SECCOMP_ARCH_OFFSET),
        // 1: if arch == x86-64 -> next (load nr); else -> wrong-arch kill
        jeq(AUDIT_ARCH_X86_64, 0, 1),
        // 2: load syscall nr
        ld_abs(SECCOMP_NR_OFFSET),
        // 3: skip the wrong-arch kill that follows
        ja(1),
        // 4: wrong-arch kill (default action)
        ret(profile.action.to_ret()),
    ];

    for rule in &profile.rules {
        match rule {
            SeccompRule::AllowSyscall(nr) => {
                // if nr matches -> ALLOW; else fall through to next rule
                p.push(jeq(*nr as u32, 0, 1));
                p.push(ret(SECCOMP_RET_ALLOW));
            }
            SeccompRule::AllowSyscallIfArg(nr, arg, val) => {
                // if nr matches -> load arg; else skip past the arg check + ALLOW
                p.push(jeq(*nr as u32, 0, 3));
                p.push(ld_abs(SECCOMP_ARG_OFFSET + u32::from(*arg) * 8));
                // if arg matches -> ALLOW; else fall through to next rule
                p.push(jeq(*val as u32, 0, 1));
                p.push(ret(SECCOMP_RET_ALLOW));
            }
        }
    }

    // default deny
    p.push(ret(profile.action.to_ret()));
    p
}

// ---------------------------------------------------------------------------
// Installation (Linux only)
// ---------------------------------------------------------------------------

#[cfg(target_os = "linux")]
#[repr(C)]
pub(crate) struct SockFprog {
    pub len: u16,
    pub filter: *const BpfInsn,
}

#[cfg(target_os = "linux")]
pub(crate) fn install_seccomp(profile: &SeccompProfile) -> Result<(), crate::SandboxError> {
    use crate::SandboxError;
    if !profile.enabled {
        return Ok(());
    }
    let prog = build_program(profile);
    let fprog = SockFprog {
        len: prog.len() as u16,
        filter: prog.as_ptr(),
    };
    unsafe {
        if libc::prctl(libc::PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) != 0 {
            return Err(SandboxError::SeccompInstall(format!(
                "prctl(PR_SET_NO_NEW_PRIVS): {}",
                std::io::Error::last_os_error()
            )));
        }
        if libc::prctl(
            libc::PR_SET_SECCOMP,
            libc::SECCOMP_MODE_FILTER as libc::c_ulong,
            &fprog as *const SockFprog as libc::c_ulong,
            0,
            0,
        ) != 0
        {
            return Err(SandboxError::SeccompInstall(format!(
                "prctl(PR_SET_SECCOMP, SECCOMP_MODE_FILTER): {}",
                std::io::Error::last_os_error()
            )));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn program_starts_with_arch_validation() {
        let prog = build_program(&SeccompProfile::maintainer_script_profile());
        // arch load, arch compare, nr load, skip, wrong-arch kill
        assert!(prog.len() >= 5);
        assert_eq!(prog[0].code, BPF_LD | BPF_W | BPF_ABS);
        assert_eq!(prog[0].k, SECCOMP_ARCH_OFFSET);
        assert_eq!(prog[1].code, BPF_JMP | BPF_JEQ | BPF_K);
        assert_eq!(prog[1].k, AUDIT_ARCH_X86_64);
        // final instruction is the default deny
        assert_eq!(prog.last().unwrap().k, SeccompAction::Errno(1).to_ret());
    }

    #[test]
    fn allowed_syscall_emits_an_allow_ret() {
        let profile = SeccompProfile {
            enabled: true,
            action: SeccompAction::Errno(1),
            rules: vec![SeccompRule::AllowSyscall(42)],
        };
        let prog = build_program(&profile);
        assert!(prog.iter().any(|i| i.k == 0x7fff_0000));
    }

    #[test]
    fn denied_syscall_has_no_allow_ret_for_it() {
        let profile = SeccompProfile {
            enabled: true,
            action: SeccompAction::Errno(1),
            rules: vec![SeccompRule::AllowSyscall(42)],
        };
        let prog = build_program(&profile);
        // 9999 is not in the allowlist; there must be no unconditional ALLOW
        // instruction guarding it, so the default deny remains the only ALLOW
        // paths and they are reachable only for nr==42.
        let allow_count = prog.iter().filter(|i| i.k == 0x7fff_0000).count();
        assert_eq!(allow_count, 1);
    }

    #[cfg(target_os = "linux")]
    fn fork_install_seccomp(profile: &SeccompProfile, body: impl FnOnce() -> i32) -> i32 {
        unsafe {
            let pid = libc::fork();
            assert!(pid >= 0, "fork failed");
            if pid == 0 {
                match install_seccomp(profile) {
                    Ok(()) => libc::_exit(body()),
                    Err(e) => {
                        let msg = format!("seccomp install failed: {}\n", e);
                        libc::write(2, msg.as_ptr() as *const libc::c_void, msg.len());
                        libc::_exit(2);
                    }
                }
            }
            let mut st: libc::c_int = 0;
            libc::waitpid(pid, &mut st, 0);
            if libc::WIFEXITED(st) {
                libc::WEXITSTATUS(st)
            } else {
                127
            }
        }
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn seccomp_denies_forbidden_syscall_with_eperm() {
        let profile = SeccompProfile::maintainer_script_profile();
        let code = fork_install_seccomp(&profile, || {
            // kexec_load is deliberately not on the allowlist.
            let r = unsafe { libc::syscall(libc::SYS_kexec_load, 0, 0, 0, 0) };
            if r == -1 && unsafe { *libc::__errno_location() } == libc::EPERM {
                0
            } else {
                1
            }
        });
        assert_eq!(code, 0, "forbidden syscall should be denied with EPERM");
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn seccomp_still_allows_common_syscalls() {
        let profile = SeccompProfile::maintainer_script_profile();
        let code =
            fork_install_seccomp(
                &profile,
                || {
                    if unsafe { libc::getpid() } > 0 {
                        0
                    } else {
                        1
                    }
                },
            );
        assert_eq!(code, 0, "allowed syscall should succeed");
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn seccomp_allows_unix_but_denies_inet_socket() {
        let profile = SeccompProfile::maintainer_script_profile();
        let code = fork_install_seccomp(&profile, || {
            let inet = unsafe { libc::socket(libc::AF_INET, libc::SOCK_STREAM, 0) };
            if inet != -1 {
                return 1;
            }
            if unsafe { *libc::__errno_location() } != libc::EPERM {
                return 1;
            }
            let unix = unsafe { libc::socket(libc::AF_UNIX, libc::SOCK_STREAM, 0) };
            if unix < 0 {
                return 1;
            }
            unsafe { libc::close(unix) };
            0
        });
        assert_eq!(
            code, 0,
            "AF_UNIX socket allowed, AF_INET socket denied with EPERM"
        );
    }
}
