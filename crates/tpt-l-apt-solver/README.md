# tpt-l-apt-solver

Parallel DPLL/CDCL SAT-based dependency resolver for Debian packages.

Part of the [`tpt-deb-toolkit`](https://github.com/tpt-solutions/tpt-deb-toolkit)
workspace — a pure-Rust pipeline for Debian/Ubuntu package management.

The dependency problem (Depends/Pre-Depends/Conflicts/Breaks, virtual packages,
Recommends/Suggests) is encoded as propositional CNF clauses and solved with a
conflict-driven clause learner (CDCL): unit propagation with watched literals,
1UIP conflict analysis, non-chronological backtracking, and a VSIDS variable
ordering.

## Layer

**Layer 3 — Network.** Depends on `tpt-l-deb-version` and `tpt-l-control-file`.

## Features

- `Universe` — all available packages for an architecture, including virtual-package providers.
- `Resolver` — produces an `InstallPlan` with `install`, `remove`, `recommended`, `suggested`.
- Handles Depends/Pre-Depends (AND of OR-groups), Conflicts/Breaks, and virtual packages.
- Upgrades by pinning the highest available version of requested packages.
- Models an existing installed set (`resolve_with_installed`) for upgrade/remove planning.
- Greedy minimization of removals and auto-pull of `Recommends`.
- Parallel portfolio via Rayon: multiple workers explore different decision orders; `seed == 0` stays deterministic.

## Installation

```toml
[dependencies]
tpt-l-apt-solver = "0.1.0"
```

## Usage

```rust
use tpt_l_apt_solver::{Package, Universe, Resolver};
use tpt_l_deb_version::Version;

let mut u = Universe::new();
u.add_package(Package {
    name: "hello".to_string(),
    version: Version::parse("1.0").unwrap(),
    depends: vec![],
    pre_depends: vec![],
    conflicts: vec![],
    breaks: vec![],
    provides: vec![],
    recommends: vec![],
    suggests: vec![],
});

let plan = Resolver::new(u).resolve(&["hello"]).unwrap();
assert_eq!(plan.install.len(), 1);
```

### From a `Packages` index

```rust
use tpt_l_apt_solver::Universe;
let pkgs: Vec<tpt_l_control_file::BinaryPackage> = /* parsed from an index */ vec![];
let universe = Universe::from_binary_packages(&pkgs).unwrap(); // parsed in parallel
```

## API overview

- `Universe` — package set (`new`, `add_package`, `packages_named`, `providers_of`, `from_binary_packages`).
- `Resolver` — `resolve` / `resolve_with_installed`.
- `InstallPlan` — `install`, `remove`, `recommended`, `suggested`.
- `Package`, `DependencyGroup`, `DependencySpec` — the dependency data model.
- `SolverError` — no-solution / parse / unknown-package failures.

## Design notes

The CDCL solver is verified against brute-force SAT on randomized formulae in the
test suite, and the parallel portfolio is asserted to agree with the single-threaded
solver on both satisfiability and model validity. Version pinning and the
keep-installed minimization together approximate `apt`'s behaviour (install newest,
avoid spurious removals).

## License

Dual-licensed under [MIT](../../LICENSE-MIT) or
[Apache-2.0](../../LICENSE-APACHE) at your option.

Copyright © 2026 TPT Solutions.
