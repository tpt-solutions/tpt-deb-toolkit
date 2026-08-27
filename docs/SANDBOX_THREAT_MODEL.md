# Sandbox threat model

This document describes the security assumptions, guarantees, and known
limitations of `tpt-l-linux-sandbox-rs`, the isolation primitive used to run
Debian maintainer scripts (`preinst`, `postinst`, `prerm`, `postrm`). It
companion to the `Open Questions` note in `todo.md` ("Sandbox threat model"):
the allowlist and bind-mount policy now exist in code; this file records the
rationale and the residual risk.

## Scope and goal

The sandbox is a **defense-in-depth, best-effort** containment layer for
untrusted-to-semi-trusted package maintainer scripts. It is **not** a
general-purpose hardened container. Its goals, in priority order:

1. **Prevent network egress** from a maintainer script (no outbound
   connections to attacker-controlled infrastructure during install).
2. **Constrain the kernel attack surface** by denying syscalls a maintainer
   script has no legitimate reason to use (e.g. `kexec_load`, `reboot`,
   `mount` of arbitrary filesystems, `bpf`, `ptrace`, `swapon`, `init_module`).
3. **Limit filesystem blast radius** to explicitly bind-mounted paths.
4. **Fail closed and loud**: if isolation cannot be established, the script is
   not silently run unrestricted.

## Mechanisms

### User + PID + mount + network + IPC namespaces

`Sandbox::run` (on Linux) forks a single-threaded child and calls
`unshare(CLONE_NEWUSER | CLONE_NEWPID | CLONE_NEWNS | CLONE_NEWNET? | CLONE_NEWIPC?)`
(see `lib.rs` `run_namespaced`). The child then `fork`s again so the exec'd
command is **PID 1** in its fresh PID namespace.

* **User namespace** with a `0 <real-uid> 1` UID/GID map: the script runs as
  root *inside* the namespace, but the host sees it as the invoking unprivileged
  user. This is what allows sandboxing without `setuid`/root on the host.
* **PID namespace**: the script cannot see or signal host processes.
* **Mount namespace**: the host mount tree is not propagated; `apply_bind_mounts`
  first marks the tree `MS_REC | MS_PRIVATE` so the sandbox's mounts cannot
  leak back to the host (`mount.rs`).
* **Network namespace** (default on): the script gets only a loopback
  interface, so it cannot open sockets to the internet. Opt-in via
  `SandboxConfig::allow_network`.
* **IPC namespace** (default on): `CLONE_NEWIPC` isolates SysV IPC / POSIX
  message queues.

The root filesystem is **not** replaced (no `pivot_root`/`chroot`). The script
sees the host's filesystem tree; containment of what it can *write* relies on
the bind-mount policy and seccomp, not on a separate root.

### seccomp allowlist

`SeccompProfile::maintainer_script_profile()` installs a classic-BPF
`SECCOMP_MODE_FILTER` via `prctl` (`seccomp.rs`). The policy is **default-deny**:
any syscall not on the allowlist fails with `EPERM` (not a kill, so scripts
that merely *probe* for optional functionality keep working).

* ~150 common syscalls are allowed (file I/O, process lifecycle, memory,
  signals, basic sockets for local IPC).
* `socket(2)` is allowed **conditionally**: `AF_UNIX` and `AF_NETLINK` are
  permitted (so scripts can talk to `systemd`-notify / local agents), while
  `AF_INET`/`AF_INET6` are denied even though the network namespace already has
  no external routes. This is defense in depth.
* The BPF program validates the CPU architecture (`AUDIT_ARCH_X86_64`) first and
  fails closed on mismatch, blocking syscall-number-reuse attacks from a
  different arch.

### Bind-mount policy

`BindMount` entries map a host path into the sandbox. `BindMount::read_only`
remounts the bind with `MS_REMOUNT | MS_BIND | MS_RDONLY`. The default
`maintainer_script_profile()` adds no mounts, so by default the script can read
the host tree but only modify paths the caller explicitly exposes.

## What it protects against

* A compromised or malicious maintainer script trying to phone home.
* A script exploiting a kernel bug via an exotic, rarely-used syscall.
* Accidental writes to the host, beyond what the integrator explicitly mounts.
* Cross-process interference on the host (separate PID/IPC namespaces).

## Known limitations / residual risk

These are **accepted limitations**, not bugs to be silently fixed:

* **No rootfs replacement.** Because the root is shared (just namespaced), a
  script that can write to a bind-mounted-writable host path (or to a host path
  it can reach through the mount namespace) can still affect the host. The
  integrator is responsible for mounting *only* what the script needs, and
  preferring `read_only`.
* **No filesystem *write* filtering beyond mounts.** seccomp constrains
  *syscalls*, not file paths; `open(O_RDWR)` to an exposed writable mount is
  allowed by design.
* **No resource limits.** CPU, memory, fd, and wall-clock limits (cgroups /
  `setrlimit`-style policy) are **not** enforced by this crate. A fork-bomb or
  memory-hog script is only bounded by the PID namespace (limited PID space) and
  the host's own limits.
* **No `pivot_root`/chroot.** See above; this is intentional to keep the crate
  dependency- and privilege-free, but it widens the visible filesystem.
* **allow_network / allow_ipc opt-out.** `SandboxConfig::unrestricted()` and
  the per-field toggles remove isolation and emit a `tracing::warn!`. Opting in
  is the integrator's explicit choice and is logged.
* **Architecture-specific BPF.** The filter is hardcoded to `x86-64`. On other
  Linux architectures the arch check fails closed (denies everything), which is
  safe but means the sandbox is effectively unusable there without extension.
* **Kernel version assumptions.** User-namespace sandboxing requires a kernel
  that permits unprivileged user namespaces. Some hardened distros disable
  `unprivileged_userns_clone`; on those hosts `unshare` fails and the
  `Sandbox::run` returns an error rather than degrading silently.

## Hardening recommendations for integrators

* Always prefer `SandboxConfig::maintainer_script_profile()`; only relax
  `allow_network`/`allow_ipc` when a specific package demonstrably needs it, and
  log the exception.
* Expose host paths with `BindMount::read_only` unless the script must write.
* Run the installer itself under a cgroup/ulimit envelope to bound resources.
* Treat `unrestricted()` usage as a security-relevant event in your audit logs.

## Testing

The crate ships fork-based tests (`lib.rs`, `seccomp.rs`) that verify, under
Linux:

* a forbidden syscall (`kexec_load`) returns `EPERM`;
* an allowed syscall (`getpid`) succeeds;
* `AF_UNIX` sockets are permitted while `AF_INET` sockets are denied;
* a real `/bin/true` and a `sh -c` reading a bind-mounted file run successfully.

These are the regression guards for the threat model above.
