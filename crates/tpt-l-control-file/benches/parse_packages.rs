//! Micro-benchmark: parse a ~50 MB `Packages` index entirely in memory.
//!
//! Run with `cargo bench -p tpt-l-control-file`. The harness is `false` so no
//! external benchmarking crate is required; we use `std::time` directly.

use std::time::Instant;

use tpt_l_control_file::PackagesIndex;

/// A realistic-looking stanza; repeated enough times to reach ~`target` bytes.
const STANZA: &str = "\
Package: libfoo
Version: 1.2.3-4
Architecture: amd64
Maintainer: TPT Solutions <packaging@tpt.example>
Description: A foo library
 A longer description line that mimics real Packages entries.
Depends: libc6 (>= 2.17), libbar (>= 1.0)
Conflicts: libfoo-old
Provides: libfoo-abi1
Filename: pool/main/libf/libfoo/libfoo_1.2.3-4_amd64.deb
Size: 1048576
SHA256: 0000000000000000000000000000000000000000000000000000000000000000

";

fn make_big_packages(target: usize) -> String {
    let mut s = String::with_capacity(target + STANZA.len());
    while s.len() < target {
        s.push_str(STANZA);
    }
    s
}

fn main() {
    let target = 50 * 1024 * 1024; // 50 MB
    let data = make_big_packages(target);

    let start = Instant::now();
    let count = PackagesIndex::new(&data).len();
    let elapsed = start.elapsed();

    let mb = data.len() as f64 / (1024.0 * 1024.0);
    let mb_per_s = mb / elapsed.as_secs_f64();
    println!(
        "Parsed {} packages from {:.1} MB in {:?} ({:.1} MB/s)",
        count, mb, elapsed, mb_per_s
    );
}
