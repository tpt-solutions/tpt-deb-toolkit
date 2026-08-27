//! Micro-benchmark harness for `tpt-l-apt-solver`.
//!
//! Generates a synthetic dependency universe of `N` packages (each depending
//! on two pseudo-random earlier packages) and times resolution of `pkg0`.
//!
//! Run with:
//!
//! ```text
//! cargo run -p tpt-l-apt-solver --example bench_solver -- 4000
//! ```
//!
//! The `libsolv` comparison leg from the original spec is intentionally left
//! as a TODO: it requires either FFI to the `libsolv` C library or driving it
//! as a subprocess, neither of which is wired up here. This harness establishes
//! the tpt-side timing so the comparison can be added later without touching
//! the solver itself.

use std::time::Instant;

use tpt_l_apt_solver::{DependencyGroup, Package, Resolver, Universe};
use tpt_l_deb_version::Version;

fn main() {
    let n: usize = std::env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(2000);

    let mut u = Universe::new();
    let mut rng = 0x1234_5678_9abc_def0u64;
    let mut next = || {
        rng = rng
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        rng >> 8
    };

    for i in 0..n {
        let name = format!("pkg{i}");
        let version = Version::parse("1.0").unwrap();
        let mut depends = Vec::new();
        if i > 5 {
            for _ in 0..2 {
                let j = (next() as usize) % i;
                depends.extend(DependencyGroup::parse(&format!("pkg{j}")).unwrap());
            }
        }
        u.add_package(Package {
            name,
            version,
            depends,
            pre_depends: vec![],
            conflicts: vec![],
            breaks: vec![],
            provides: vec![],
            recommends: vec![],
            suggests: vec![],
        });
    }

    let requests = vec!["pkg0"];
    let start = Instant::now();
    let plan = Resolver::new(u).resolve(&requests).unwrap();
    let elapsed = start.elapsed();

    println!(
        "resolved universe of {n} packages in {elapsed:?}; install set = {}",
        plan.install.len()
    );
}
