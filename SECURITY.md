# Security Policy

## Supported Versions

| Version | Supported |
|---------|-----------|
| 0.1.x   | ✅ |

## Reporting a Vulnerability

**Do not open a public GitHub issue for security vulnerabilities.**

Instead, report privately:

1. Email: **security@tpt-solutions.dev**
2. Or use GitHub's private vulnerability reporting on this repository.

Please include:

- Affected crate(s) and version(s)
- A description of the issue and its impact
- Steps or a proof-of-concept to reproduce

You will receive an acknowledgment within 48 hours. We aim to release a fix
within 90 days of confirmation, sooner for remotely exploitable issues.

## Scope Notes

This toolkit parses untrusted input from package repositories and executes
Debian maintainer scripts. Areas of particular sensitivity:

- `tpt-l-control-file`, `tpt-l-deb-format`, `tpt-l-sources-list`: parsers of
  untrusted archive/index data — memory safety and DoS resistance matter.
- `tpt-l-apt-keyring`: OpenPGP verification logic — signature bypass reports
  are treated as critical.
- `tpt-l-linux-sandbox-rs`: namespace/seccomp isolation — any escape vector
  is treated as critical.
- `tpt-l-maintainer-scripts`: script execution paths — privilege escalation
  concerns are treated as critical.

## Disclosure Policy

We follow coordinated disclosure: after a fix is released we publish an
advisory (GitHub Security Advisory / RUSTSEC) crediting the reporter unless
anonymity is requested.
