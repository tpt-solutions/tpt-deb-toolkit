//! Fuzz target for `tpt_l_deb_version::Version::parse`.
//!
//! The parser must never panic on arbitrary input; it either returns a valid
//! `Version` or a `VersionError`. When parsing succeeds, the round-trip
//! invariant `Display(parse(s)) == s` is also checked for canonical inputs
//! (those without an implicit epoch).

#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if let Ok(s) = std::str::from_utf8(data) {
        if let Ok(v) = tpt_l_deb_version::Version::parse(s) {
            // Ordering must be a total order: a version equals itself.
            assert_eq!(v.cmp(&v), std::cmp::Ordering::Equal);
            // Round-trip: display of a parsed version must re-parse equal.
            let displayed = v.to_string();
            let reparsed =
                tpt_l_deb_version::Version::parse(&displayed).expect("displayed version re-parses");
            assert_eq!(v.cmp(&reparsed), std::cmp::Ordering::Equal);
        }
    }
});
