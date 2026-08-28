//! Integration test: extract a real `.deb` and verify its contents on disk.
//!
//! The workspace does not ship a large real-world `.deb` fixture (it would be
//! heavy and license-encumbered), so this test builds a fully valid, in-memory
//! `.deb` via the crate's own `testsupport` helper and exercises the end-to-end
//! `open` → `extract` → on-disk verification path. Drop a real `.deb` at the
//! path referenced by `TPT_REAL_DEB` to also run the optional real-fixture check.

use std::path::Path;

use tpt_l_deb_format::testsupport::synthetic_deb;
use tpt_l_deb_format::DebFile;

#[test]
fn extract_real_deb_and_verify_contents() {
    let bytes = synthetic_deb();
    let deb = DebFile::parse(&bytes).expect("parse synthetic .deb");

    let dir = tempfile::tempdir().expect("temp dir");
    deb.extract(dir.path()).expect("extract payload");

    // Verify a known file exists and has the expected content.
    let readme = dir.path().join("usr/share/doc/foo/README");
    assert!(readme.exists(), "expected README to be extracted");
    let content = std::fs::read_to_string(&readme).expect("read extracted README");
    assert_eq!(content, "hello\n");

    // Verify the executable bit is preserved (unix).
    let bin = dir.path().join("usr/bin/foo");
    assert!(bin.exists(), "expected binary to be extracted");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&bin)
            .expect("stat bin")
            .permissions()
            .mode();
        assert_eq!(mode & 0o111, 0o111, "executable bit should be preserved");
    }
    let _ = Path::new(""); // keep `Path` import meaningful on all platforms
}

#[test]
fn streaming_entries_match_extracted_files() {
    let bytes = synthetic_deb();
    let deb = DebFile::parse(&bytes).expect("parse synthetic .deb");

    let mut ents = deb.data_entries().expect("data entries");
    let names: Vec<String> = ents
        .entries()
        .expect("iterate")
        .map(|e| {
            e.unwrap()
                .path()
                .unwrap()
                .into_owned()
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    assert!(
        names.iter().any(|p| p.ends_with("usr/bin/foo")),
        "streaming entries should include the binary"
    );
}

/// Optional: extract a real `.deb` supplied via `TPT_REAL_DEB` and check it
/// round-trips through `data_contents`. Skipped unless the env var is set.
#[test]
fn extract_real_deb_fixture_if_present() {
    let Some(path) = std::env::var("TPT_REAL_DEB").ok() else {
        eprintln!("TPT_REAL_DEB not set; skipping real .deb extraction");
        return;
    };
    let deb = DebFile::open(Path::new(&path)).expect("open real .deb");
    assert!(
        !deb.entries().is_empty(),
        "real .deb should have payload entries"
    );
    assert!(
        deb.metadata().package_name().is_some(),
        "real .deb should expose a Package field"
    );
}
