# tpt-l-apt-transport

Async HTTP/HTTPS transport for fetching APT indices and `.deb` files.

Part of the [`tpt-deb-toolkit`](https://github.com/tpt-solutions/tpt-deb-toolkit)
workspace — a pure-Rust pipeline for Debian/Ubuntu package management.

Built on `tokio` + `reqwest` (rustls). Provides full index fetching with
compression fallback, `InRelease`/`Release` fetching, **resumable** partial
downloads, mirror failover, and PDiff (delta index) updates.

## Layer

**Layer 3 — Network.** Depends on `tpt-l-apt-keyring` only for detached-signature
verification (callers wire that up), otherwise self-contained for HTTP.

## Features

- `fetch_index` — tries `.xz`/`.gz`/raw `Packages` automatically.
- `fetch_release` / `fetch_release_gpg` — `InRelease` with `Release` fallback.
- `fetch_with_mirrors` — first-mirror-wins failover.
- `download_file` — streaming, incremental write with a progress callback.
- `download_file_resumable` — HTTP `Range` resume; falls back to full rewrite if the server ignores ranges.
- `fetch_range` / `fetch_if_modified_since` — partial fetches and `304 Not Modified` caching.
- `decompress_by_url` — transparent `.gz`/`.xz`/`.zst` decompression.
- `pdiff` module — apply PDiff rdiff deltas (see `docs/` / `pdiff` submodule); `release` module parses `Release` indices.

## Installation

```toml
[dependencies]
tpt-l-apt-transport = "0.1.0"
```

## Usage

```rust
use tpt_l_apt_transport::AptTransport;

# async fn run() -> Result<(), Box<dyn std::error::Error>> {
let transport = AptTransport::with_default_config()?;
let bytes = transport
    .fetch_bytes("https://deb.debian.org/debian/dists/stable/Release")
    .await?;
println!("{} bytes", bytes.len());
# Ok(()) }
```

### Resumable download with progress

```rust
# async fn run() -> Result<(), Box<dyn std::error::Error>> {
let t = AptTransport::with_default_config()?;
t.download_file_resumable(
    "https://deb.debian.org/debian/pool/main/c/curl/curl_8.2.1-1_amd64.deb",
    std::path::Path::new("./curl.deb"),
    Some(Box::new(|done, total| println!("{done}/{total}"))),
).await?;
# Ok(()) }
```

## API overview

- `AptTransport` — the client (`new`, `with_default_config`, `fetch_bytes`, `fetch_decompressed`, `fetch_index`, `fetch_release`, `fetch_release_gpg`, `fetch_with_mirrors`, `download_file`, `download_file_resumable`, `fetch_range`, `fetch_if_modified_since`).
- `TransportConfig` — connect/request timeouts and retry count.
- `decompress_by_url` — extension-based decompression.
- `pdiff` — `PdiffIndex`, `PdiffUpdate`, `apply_rdiff_delta`, `encode_rdiff_delta`, ….
- `release` — `ReleaseIndex`, `ReleaseFile` parsing.
- `TransportError` — HTTP/network/IO/decompression/pdiff failures.

## License

Dual-licensed under [MIT](../../LICENSE-MIT) or
[Apache-2.0](../../LICENSE-APACHE) at your option.

Copyright © 2026 TPT Solutions.
