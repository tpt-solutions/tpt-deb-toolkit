# Dependency decisions and divergences from the spec

This document records where the implemented crates diverge from the
dependencies named in the original design spec, and why. It mirrors the
"Open Questions" / "Systemic gaps" notes in `todo.md` (audit 2026-08-27).

## 1. `tpt-l-control-file` does not use `deb822-lossless`

The spec listed `deb822-lossless` as the deb822 parser for control files.
The crate instead ships a hand-rolled stanza parser (`parse_paragraph`,
`ControlParagraph`, `BinaryPackage::parse_stanza`, etc.).

### Why

* **Streaming / zero-copy control.** `PackagesIndex` parses stanzas lazily
  over a `&str` so callers can iterate a 50 MB `Packages` file without
  materialising every package up front. The `deb822-lossless` API centres on
  an owned `Deb822` document and typed `Paragraph` accessors; while it can be
  wrapped, the lazy iterator we wanted was simpler to express directly.
* **Strict-mode option.** The spec requires rejecting duplicate fields in a
  stanza (`ControlError::DuplicateField`). The hand-rolled parser offers both
  a lenient path (`parse_control`) and a strict path (`parse_control_strict` /
  `BinaryPackage::parse_stanza_strict`), which was easier to expose behind a
  single parser than to bolt onto the external type.
* **Fewer transitive deps.** Avoiding `deb822-lossless` (and its `rtoolbox` /
  `memchr` chain) keeps the lowest layer lean.

### Trade-offs / follow-ups

* A pure-deb822 conformance fuzz target should be added to confirm the
  hand-rolled parser accepts exactly what `dpkg` accepts; the
  `fuzz/fuzz_targets/parse_version.rs` pattern can be copied.
* If we later want first-class deb822 editing (preserving exact formatting,
  comments, ordering), wrapping `deb822-lossless` for the *write* path is the
  natural extension. The read path can remain custom.

## 2. `tpt-l-apt-keyring` uses `pgp` (rPGP) instead of `sequoia-pgp`

The spec named `sequoia-pgp`. The crate depends on `pgp` (rPGP) and uses
`pgp::SignedPublicKey` / `pgp::verify` for clearsigned (`InRelease`) and
detached (`Release.gpg`) signature verification.

### Why

* **Pure-Rust, no C toolchain.** rPGP is implemented in pure Rust and builds
  without a C compiler or bundled Nettle/OpenSSL, which keeps the workspace
  easy to cross-compile (e.g. `x86_64-unknown-linux-gnu`).
* **Smaller surface for the operations we need.** We only verify
  clearsigned/detached signatures over release files; rPGP covers that.
* **Async-friendly.** The transport layer is Tokio-based; rPGP's sync API is
  called from blocking helpers without pulling in Sequoia's crypto backend.

### Trade-offs / follow-ups

* **Key expiry / revocation is not yet implemented.** Both backends require
  walking the key's signatures and self-signatures; this is still TODO and is
  independent of the backend choice.
* If distribution-grade policy (e.g. `sq` keystore, gpgv-style trustdb) is
  ever required, Sequoia is the more complete option and can replace `pgp`
  behind the existing `Keyring`/`VerifyResult` API without touching callers.

## 3. `tpt-l-sources-list` does not depend on `tpt-l-control-file`

The spec implied `sources.list`/`.sources` parsing would reuse the control
parser. `tpt-l-sources-list` instead parses both formats locally.

### Why

* **Different grammar.** `.sources` files are deb822, but the legacy
  one-line `sources.list` format is *not* deb822 — it is a whitespace-token
  grammar (`deb [options] uri suite component...`). A single deb822 parser
  cannot handle the legacy format, so a dedicated tokenizer was written.
* **Avoiding a cycle / keeping layers ordered.** `tpt-l-control-file` is a
  lower layer than `tpt-l-sources-list`; depending on it for the legacy
  format only would have coupled the layers for marginal benefit.

### Trade-offs / follow-ups

* The `.sources` (deb822) branch *could* reuse `tpt-l-control-file`, but the
  bespoke parser already covers the required fields (`Types`, `URIs`,
  `Suites`, `Components`, `Signed-By`, `Trusted`). Consolidation is optional.
