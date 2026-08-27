# Benchmark methodology

This document defines how performance is measured across `tpt-deb-toolkit`, and
what "outperform" means before any external comparison is claimed (see the
`Open Questions` note "Benchmark methodology" in `todo.md`).

The toolkit does not yet compare against an external solver (libsolv), so every
number below is **internal / relative** unless explicitly stated. Treat them as
regression guards and capacity estimates, not as competitive claims.

## What we measure

Two axes, reflecting the two things a Debian toolchain cares about:

1. **Wall-clock throughput** — how fast can we parse / solve a given input?
   Reported as MB/s, packages/s, or total elapsed time.
2. **Plan quality** — does the solver produce a correct, minimal install plan
   (correct dependency satisfaction, expected add/remove/upgrade set)?

Plan quality is verified with **differential tests**, not a single metric: the
CDCL solver is checked against a brute-force SAT oracle and against known
expected plans (see `tpt-l-apt-solver` tests). Wall-clock is the optimization
metric.

## Harness

No external benchmarking framework is required. Each micro-benchmark is a
`harness = false` binary (`fn main()`) that times work with `std::time::Instant`
and prints a human-readable line. This keeps the workspace dependency-free and
lets the same binary run under `cargo bench` or standalone.

Conventions:

* Target sizes are explicit (e.g. "50 MB `Packages` index", "50 MB `.deb`").
* Each benchmark prints its input size, the resulting count, elapsed time, and a
  derived rate.
* Benchmarks are deterministic for a given input; input is generated in-process
  from a fixed template so runs are comparable.

## Existing benchmarks

| Crate | File | Measures |
| --- | --- | --- |
| `tpt-l-control-file` | `benches/parse_packages.rs` | Parse a ~50 MB `Packages` index into stanzas (lazy). |
| `tpt-l-deb-format` | `benches/metadata_read.rs` | Open + read control metadata of a ~50 MB `.deb` via `memmap2`. |
| `tpt-l-apt-solver` | `examples/bench_solver.rs` | Time `resolve()` on a synthetic universe; prints add/remove/upgrade counts. |

## How to run

```sh
cargo bench -p tpt-l-control-file      # parse_packages
cargo bench -p tpt-l-deb-format        # metadata_read
cargo run -p tpt-l-apt-solver --example bench_solver
```

For stable, comparable numbers, run on a quiet machine and average 3+ runs. Pin
the toolchain (`rust-toolchain.toml`) so cross-run comparisons are valid.

## Defining "outperform" (before any external claim)

A claim that the toolkit "outperforms" an alternative (e.g. `libsolv`) is only
defensible once **all** of the following are true:

* **Equivalent workload.** Both tools solve the *same* input (identical
  `Packages` snapshot, identical constraints). We have a captured Ubuntu archive
  snapshot under `bench-data/` (currently empty placeholder — TODO) to make this
  reproducible.
* **Equivalent metrics.** Report wall-clock *and* plan quality side by side. A
  faster solver that produces a wrong/larger plan is not "better".
* **Reproducible environment.** Same OS, CPU, and toolchain; warm filesystem
  cache controlled (cold vs warm called out explicitly).
* **Published artifact.** A generated report (`bench-data/report.md` or similar)
  capturing input, commands, raw numbers, and the exact git revision.

## Open items

* Populate `bench-data/` with a real (or synthetic-but-representative) Ubuntu
  `Packages` snapshot and document its provenance.
* Add a `libsolv` comparison (via FFI or subprocess) on that identical input and
  produce a reproducible report. (Tracked in `todo.md` Phase 4 solver bench
  section.)
* Add a zero-copy metadata-read number for `tpt-l-deb-format` to the table above
  (now provided by `benches/metadata_read.rs`).
