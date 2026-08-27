//! Micro-benchmark: "zero-copy" metadata read of a ~50 MB `.deb`.
//!
//! Run with `cargo bench -p tpt-l-deb-format`. The harness is `false` so no
//! external benchmarking crate is required; we use `std::time` directly.
//!
//! It builds a synthetic ~50 MB `.deb` on disk, then times two paths:
//!   1. `DebFile::open` + `metadata()` — the full parse (decompresses the data
//!      payload too).
//!   2. `DebFile::open_metadata` — the zero-copy path (mmap + control.tar only).
//!
//! The second number should be roughly independent of payload size, which is
//! the property the "zero-copy metadata read" claim relies on.

use std::io::{Cursor, Read, Write};
use std::time::Instant;

use flate2::{write::GzEncoder, Compression};
use tar::Builder as TarBuilder;
use tempfile::NamedTempFile;

use tpt_l_deb_format::{DebError, DebFile};

const TARGET_BYTES: usize = 50 * 1024 * 1024; // 50 MB payload

/// Build a synthetic `.deb` whose `data.tar.gz` holds one ~`TARGET_BYTES` file
/// filled with incompressible-looking bytes, plus a tiny `control.tar.gz`.
fn make_large_deb() -> Result<Vec<u8>, DebError> {
    let control_content =
        b"Package: bigpkg\nVersion: 9.9-9\nArchitecture: amd64\nMaintainer: TPT <t@e.com>\nDescription: large fixture\n";
    let control_tar = make_tar_gz(&[("control", control_content, 0o644)]);

    // One large member so the resulting .deb is ~50 MB compressed.
    let mut payload = Vec::with_capacity(TARGET_BYTES);
    let mut seed: u64 = 0x9E37_79B9_7F4A_7C15;
    while payload.len() < TARGET_BYTES {
        seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        payload.extend_from_slice(&seed.to_le_bytes());
    }
    let data_tar = make_tar_gz(&[("usr/lib/bigblob.bin", &payload, 0o644)]);

    let mut out = Vec::new();
    out.extend_from_slice(b"!<arch>\n");
    out.extend_from_slice(&ar_member("debian-binary", b"2.0\n"));
    out.extend_from_slice(&ar_member("control.tar.gz", &control_tar));
    out.extend_from_slice(&ar_member("data.tar.gz", &data_tar));
    Ok(out)
}

fn make_tar_gz(files: &[(&str, &[u8], u32)]) -> Vec<u8> {
    let mut buf = Vec::new();
    {
        let enc = GzEncoder::new(&mut buf, Compression::default());
        let mut builder = TarBuilder::new(enc);
        for (path, content, mode) in files {
            let mut header = tar::Header::new_gnu();
            header.set_size(content.len() as u64);
            header.set_mode(*mode);
            header.set_cksum();
            builder
                .append_data(&mut header, path, *content)
                .expect("append tar entry");
        }
        builder.finish().expect("finish tar");
    }
    buf
}

fn ar_member(name: &str, body: &[u8]) -> Vec<u8> {
    let mut m = Vec::new();
    m.extend_from_slice(&ar_header(name, body.len() as u64));
    m.extend_from_slice(body);
    if !body.len().is_multiple_of(2) {
        m.push(b'\n');
    }
    m
}

fn ar_header(name: &str, size: u64) -> [u8; 60] {
    let mut h = [0u8; 60];
    let mut nf = String::new();
    nf.push_str(name);
    nf.push('/');
    while nf.len() < 16 {
        nf.push(' ');
    }
    h[0..16].copy_from_slice(nf.as_bytes());
    let s = format!("{:<10}", size);
    h[48..58].copy_from_slice(s.as_bytes());
    h[58..60].copy_from_slice(b"`\n");
    h
}

fn main() {
    let bytes = make_large_deb().expect("build synthetic .deb");

    // Persist to a real file so `open` / `open_metadata` exercise the mmap path.
    let mut f = NamedTempFile::new().expect("temp file");
    f.write_all(&bytes).expect("write fixture");
    f.flush().expect("flush");
    let path = f.path().to_path_buf();

    let mb = bytes.len() as f64 / (1024.0 * 1024.0);

    // --- Full parse (decompresses the data payload too) ---
    let start = Instant::now();
    let deb = DebFile::open(&path).expect("open");
    let _ = deb.metadata().package_name();
    let full_elapsed = start.elapsed();

    // --- Zero-copy metadata-only read ---
    let start = Instant::now();
    let meta = DebFile::open_metadata(&path).expect("open_metadata");
    let _ = meta.package_name();
    let meta_elapsed = start.elapsed();

    println!(
        "Synthetic .deb size: {:.1} MB (payload ~{} MB)",
        mb,
        TARGET_BYTES / (1024 * 1024)
    );
    println!(
        "  full parse (open + metadata + payload): {:?} ({:.1} MB/s)",
        full_elapsed,
        mb / full_elapsed.as_secs_f64()
    );
    println!(
        "  metadata-only (open_metadata):          {:?} ({:.1} MB/s)",
        meta_elapsed,
        mb / meta_elapsed.as_secs_f64()
    );
    println!(
        "  speed-up of metadata-only path:         {:.1}x",
        full_elapsed.as_secs_f64() / meta_elapsed.as_secs_f64().max(f64::MIN_POSITIVE)
    );
    println!("  package name: {}", meta.package_name().unwrap_or("<none>"));
}
