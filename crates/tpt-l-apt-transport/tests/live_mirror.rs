//! Integration tests that hit a real APT mirror.
//!
//! These are gated behind the `TPT_LIVE_MIRROR` environment variable (default
//! off) because they require network access and are unsuitable for the
//! hermetic unit/CI runs. Set `TPT_LIVE_MIRROR` to a mirror base URL (e.g.
//! `http://deb.debian.org/debian`) to enable them.
//!
//! ```sh
//! TPT_LIVE_MIRROR=http://deb.debian.org/debian cargo test -p tpt-l-apt-transport --test live_mirror
//! ```

use tpt_l_apt_transport::AptTransport;

fn mirror() -> Option<String> {
    std::env::var("TPT_LIVE_MIRROR")
        .ok()
        .map(|m| m.trim_end_matches('/').to_string())
}

#[tokio::test]
async fn fetch_real_packages_index() {
    let Some(mirror) = mirror() else {
        eprintln!("skipping: set TPT_LIVE_MIRROR to a mirror base URL to enable");
        return;
    };
    let t = AptTransport::with_default_config().unwrap();
    let url = format!("{}/dists/stable/main/binary-amd64/Packages", mirror);
    let bytes = t.fetch_bytes(&url).await.expect("fetch Packages index");
    assert!(!bytes.is_empty(), "mirror returned an empty index");
    let text = String::from_utf8_lossy(&bytes);
    assert!(
        text.contains("\nPackage:") || text.starts_with("Package:"),
        "index did not contain a Package: stanza"
    );
}
