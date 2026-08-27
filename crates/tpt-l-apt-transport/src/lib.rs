//! Async HTTP/HTTPS transport for fetching APT indices and `.deb` files.
//!
//! Features:
//! * Full index fetch ([`AptTransport::fetch_index`]) with `.xz`/`.gz`/raw fallback.
//! * `InRelease`/`Release` fetch with fallback.
//! * Partial downloads (`Range`, `If-Modified-Since`) and resumable file download.
//! * Mirror failover via [`AptTransport::fetch_with_mirrors`].
//! * Delta index updates (PDiff) — see the [`pdiff`] module and
//!   [`AptTransport::fetch_pdiff`].
//!
//! # Example
//!
//! ```no_run
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! use tpt_l_apt_transport::{AptTransport, TransportConfig};
//!
//! let transport = AptTransport::with_default_config()?;
//! let bytes = tokio::runtime::Runtime::new()?.block_on(async {
//!     transport
//!         .fetch_bytes("https://deb.debian.org/debian/dists/stable/Release")
//!         .await
//! })?;
//! println!("{} bytes", bytes.len());
//! # Ok(())
//! # }
//! ```

use std::io::Read as _;
use std::path::Path;
use std::time::Duration;

use chrono::{DateTime, Utc};
use thiserror::Error;
use tokio::io::AsyncWriteExt as _;

pub mod pdiff;
pub use pdiff::{
    apply_rdiff_delta, encode_rdiff_delta, packages_diff_index_url, resolve_chain,
    sources_diff_index_url, PdiffBasis, PdiffEntry, PdiffError, PdiffIndex, PdiffUpdate, RdiffOp,
};

// ─── Configuration ────────────────────────────────────────────────────────────

/// Configuration for [`AptTransport`].
#[derive(Debug, Clone)]
pub struct TransportConfig {
    /// Timeout for establishing a TCP connection, in seconds.
    pub connect_timeout_secs: u64,
    /// Timeout for the entire HTTP request/response cycle, in seconds.
    pub request_timeout_secs: u64,
    /// Maximum number of retry attempts on transient errors.
    pub max_retries: u32,
}

impl Default for TransportConfig {
    fn default() -> Self {
        Self {
            connect_timeout_secs: 30,
            request_timeout_secs: 60,
            max_retries: 3,
        }
    }
}

// ─── Errors ───────────────────────────────────────────────────────────────────

/// Errors that can occur in [`AptTransport`].
#[derive(Debug, Error)]
pub enum TransportError {
    /// The server returned a non-2xx HTTP status.
    #[error("HTTP error {0}")]
    Http(reqwest::StatusCode),
    /// A network-level error (DNS, TLS, connection refused, etc.).
    #[error("network error: {0}")]
    Network(#[from] reqwest::Error),
    /// An I/O error while writing to disk.
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    /// All mirrors were tried and all failed.
    #[error("all mirrors failed")]
    AllMirrorsFailed,
    /// The URL extension indicates a compression format we do not support.
    #[error("unsupported compression format: {0}")]
    UnsupportedCompression(String),
    /// The decompression step failed.
    #[error("decompression error: {0}")]
    DecompressError(String),
    /// A PDiff (delta index) update failed.
    #[error("pdiff error: {0}")]
    Pdiff(String),
}

// ─── Transport ────────────────────────────────────────────────────────────────

/// Async HTTP/HTTPS transport for fetching APT resources.
pub struct AptTransport {
    client: reqwest::Client,
    config: TransportConfig,
}

impl AptTransport {
    /// Create a new transport with the given configuration.
    pub fn new(config: TransportConfig) -> Result<Self, TransportError> {
        let client = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(config.connect_timeout_secs))
            .timeout(Duration::from_secs(config.request_timeout_secs))
            .user_agent("tpt-deb-toolkit/0.1 (APT-HTTP/1.3)")
            .build()?;
        Ok(Self { client, config })
    }

    /// Create a new transport using [`TransportConfig::default`].
    pub fn with_default_config() -> Result<Self, TransportError> {
        Self::new(TransportConfig::default())
    }

    /// Fetch the raw bytes at `url`.
    ///
    /// Retries up to [`TransportConfig::max_retries`] times on transient
    /// failures.
    pub async fn fetch_bytes(&self, url: &str) -> Result<Vec<u8>, TransportError> {
        let mut last_err = None;
        for attempt in 0..=self.config.max_retries {
            if attempt > 0 {
                tracing::debug!("retry {} for {}", attempt, url);
            }
            match self.fetch_once(url).await {
                Ok(bytes) => return Ok(bytes),
                Err(e) => {
                    tracing::warn!("fetch attempt {} failed for {}: {}", attempt, url, e);
                    last_err = Some(e);
                }
            }
        }
        Err(last_err.unwrap())
    }

    async fn fetch_once(&self, url: &str) -> Result<Vec<u8>, TransportError> {
        let resp = self.client.get(url).send().await?;
        if !resp.status().is_success() {
            return Err(TransportError::Http(resp.status()));
        }
        let bytes = resp.bytes().await?;
        Ok(bytes.to_vec())
    }

    /// Fetch `url` and decompress based on the file extension in the URL.
    ///
    /// Supported: `.gz`, `.xz`, `.zst`. Uncompressed URLs are returned as-is.
    pub async fn fetch_decompressed(&self, url: &str) -> Result<Vec<u8>, TransportError> {
        let raw = self.fetch_bytes(url).await?;
        decompress_by_url(url, raw)
    }

    /// Fetch a binary-package index (`Packages`) for the given coordinates.
    ///
    /// Tries `.xz` first, then `.gz`, then uncompressed.
    pub async fn fetch_index(
        &self,
        base_url: &str,
        suite: &str,
        component: &str,
        arch: &str,
    ) -> Result<Vec<u8>, TransportError> {
        let prefix = format!(
            "{}/dists/{}/{}/binary-{}/Packages",
            base_url.trim_end_matches('/'),
            suite,
            component,
            arch
        );
        for url in &[
            format!("{}.xz", prefix),
            format!("{}.gz", prefix),
            prefix.clone(),
        ] {
            match self.fetch_decompressed(url).await {
                Ok(data) => {
                    tracing::debug!("fetched index from {}", url);
                    return Ok(data);
                }
                Err(TransportError::Http(s)) if s == reqwest::StatusCode::NOT_FOUND => {
                    tracing::debug!("index not found at {}, trying next", url);
                }
                Err(e) => return Err(e),
            }
        }
        Err(TransportError::Http(reqwest::StatusCode::NOT_FOUND))
    }

    /// Fetch an `InRelease` file, falling back to `Release` if not found.
    pub async fn fetch_release(
        &self,
        base_url: &str,
        suite: &str,
    ) -> Result<Vec<u8>, TransportError> {
        let base = format!("{}/dists/{}", base_url.trim_end_matches('/'), suite);
        let inrelease = format!("{}/InRelease", base);
        match self.fetch_bytes(&inrelease).await {
            Ok(data) => {
                tracing::debug!("fetched InRelease from {}", inrelease);
                return Ok(data);
            }
            Err(TransportError::Http(s)) if s == reqwest::StatusCode::NOT_FOUND => {
                tracing::debug!("InRelease not found, falling back to Release");
            }
            Err(e) => return Err(e),
        }
        let release_url = format!("{}/Release", base);
        self.fetch_bytes(&release_url).await
    }

    /// Try each URL in `urls` in order, returning the first success.
    pub async fn fetch_with_mirrors(&self, urls: &[String]) -> Result<Vec<u8>, TransportError> {
        for url in urls {
            match self.fetch_bytes(url).await {
                Ok(data) => return Ok(data),
                Err(e) => {
                    tracing::warn!("mirror {} failed: {}", url, e);
                }
            }
        }
        Err(TransportError::AllMirrorsFailed)
    }

    /// Stream-download `url` to `dest`.
    ///
    /// The response body is written to `dest` incrementally as chunks arrive
    /// (it is *not* buffered entirely in memory first), and `progress` is
    /// invoked after each chunk with `(bytes_downloaded, total_bytes)`.
    /// `total_bytes` is `0` when the server omits `Content-Length`.
    pub async fn download_file(
        &self,
        url: &str,
        dest: &Path,
        progress: Option<Box<dyn Fn(u64, u64) + Send>>,
    ) -> Result<(), TransportError> {
        use futures_util::StreamExt;
        use tokio::fs::File;

        let resp = self.client.get(url).send().await?;
        if !resp.status().is_success() {
            return Err(TransportError::Http(resp.status()));
        }
        let total = resp.content_length().unwrap_or(0);
        let mut file = File::create(dest).await?;
        let mut downloaded: u64 = 0;
        let mut stream = resp.bytes_stream();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk?;
            file.write_all(&chunk).await?;
            downloaded += chunk.len() as u64;
            if let Some(ref cb) = progress {
                cb(downloaded, total.max(downloaded));
            }
        }
        file.flush().await?;
        Ok(())
    }

    /// Fetch a single byte range of `url` using an HTTP `Range` request.
    ///
    /// `end` is inclusive when `Some`, and open-ended (`bytes=START-`) when
    /// `None`. Servers that honor the header reply `206 Partial Content`;
    /// servers that ignore it and return `200` with the whole body are also
    /// accepted (the caller simply receives more than requested).
    pub async fn fetch_range(
        &self,
        url: &str,
        start: u64,
        end: Option<u64>,
    ) -> Result<Vec<u8>, TransportError> {
        let range = match end {
            Some(e) => format!("bytes={}-{}", start, e),
            None => format!("bytes={}-", start),
        };
        let resp = self
            .client
            .get(url)
            .header(reqwest::header::RANGE, range)
            .send()
            .await?;
        if !resp.status().is_success() {
            return Err(TransportError::Http(resp.status()));
        }
        let bytes = resp.bytes().await?;
        Ok(bytes.to_vec())
    }

    /// Fetch `url` only if it has changed since `since`.
    ///
    /// Sends `If-Modified-Since` and returns `Ok(None)` when the server replies
    /// `304 Not Modified`, otherwise the fresh body. This lets callers skip
    /// re-downloading index files they already have cached.
    pub async fn fetch_if_modified_since(
        &self,
        url: &str,
        since: DateTime<Utc>,
    ) -> Result<Option<Vec<u8>>, TransportError> {
        let resp = self
            .client
            .get(url)
            .header(reqwest::header::IF_MODIFIED_SINCE, since.to_rfc2822())
            .send()
            .await?;
        if resp.status() == reqwest::StatusCode::NOT_MODIFIED {
            return Ok(None);
        }
        if !resp.status().is_success() {
            return Err(TransportError::Http(resp.status()));
        }
        let bytes = resp.bytes().await?;
        Ok(Some(bytes.to_vec()))
    }

    /// Resume downloading `url` into `dest`.
    ///
    /// If `dest` already exists, only the bytes starting at its current length
    /// are requested (HTTP `Range`), and the remainder is appended. If the
    /// server does not honor `Range` (replies `200` with the whole body), the
    /// existing content is discarded and the file is rewritten from scratch.
    /// `progress` reports `(total_written, server_reported_total)` after each
    /// chunk, where `total_written` includes the bytes already present on disk.
    pub async fn download_file_resumable(
        &self,
        url: &str,
        dest: &Path,
        progress: Option<Box<dyn Fn(u64, u64) + Send>>,
    ) -> Result<(), TransportError> {
        use futures_util::StreamExt;
        use tokio::fs::OpenOptions;

        let existing = tokio::fs::metadata(dest)
            .await
            .map(|m| m.len())
            .unwrap_or(0);

        let resp = if existing > 0 {
            self.client
                .get(url)
                .header(reqwest::header::RANGE, format!("bytes={}-", existing))
                .send()
                .await?
        } else {
            self.client.get(url).send().await?
        };

        let resume = resp.status() == reqwest::StatusCode::PARTIAL_CONTENT;
        let server_total = resp.content_length().unwrap_or(0);
        let total = if resume {
            server_total.saturating_add(existing)
        } else {
            server_total
        };

        if resume {
            let mut file = OpenOptions::new()
                .write(true)
                .create(true)
                .append(true)
                .open(dest)
                .await?;
            let mut written = existing;
            let mut stream = resp.bytes_stream();
            while let Some(chunk) = stream.next().await {
                let chunk = chunk?;
                file.write_all(&chunk).await?;
                written += chunk.len() as u64;
                if let Some(ref cb) = progress {
                    cb(written, total.max(written));
                }
            }
            file.flush().await?;
        } else {
            // Server ignored Range: overwrite from the beginning.
            let mut file = OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(true)
                .open(dest)
                .await?;
            let mut written: u64 = 0;
            let mut stream = resp.bytes_stream();
            while let Some(chunk) = stream.next().await {
                let chunk = chunk?;
                file.write_all(&chunk).await?;
                written += chunk.len() as u64;
                if let Some(ref cb) = progress {
                    cb(written, total.max(written));
                }
            }
            file.flush().await?;
        }
        Ok(())
    }
}

// ─── Decompression ────────────────────────────────────────────────────────────

/// Decompress `raw` based on the file extension detected in `url`.
pub fn decompress_by_url(url: &str, raw: Vec<u8>) -> Result<Vec<u8>, TransportError> {
    let path = url.split('?').next().unwrap_or(url);
    if path.ends_with(".gz") {
        let mut decoder = flate2::read::GzDecoder::new(raw.as_slice());
        let mut out = Vec::new();
        decoder
            .read_to_end(&mut out)
            .map_err(|e| TransportError::DecompressError(e.to_string()))?;
        Ok(out)
    } else if path.ends_with(".xz") {
        let mut decoder = xz2::read::XzDecoder::new(raw.as_slice());
        let mut out = Vec::new();
        decoder
            .read_to_end(&mut out)
            .map_err(|e| TransportError::DecompressError(e.to_string()))?;
        Ok(out)
    } else if path.ends_with(".zst") {
        let mut decoder = zstd::Decoder::new(raw.as_slice())
            .map_err(|e| TransportError::DecompressError(e.to_string()))?;
        let mut out = Vec::new();
        decoder
            .read_to_end(&mut out)
            .map_err(|e| TransportError::DecompressError(e.to_string()))?;
        Ok(out)
    } else {
        Ok(raw)
    }
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_values() {
        let cfg = TransportConfig::default();
        assert_eq!(cfg.connect_timeout_secs, 30);
        assert_eq!(cfg.request_timeout_secs, 60);
        assert_eq!(cfg.max_retries, 3);
    }

    #[test]
    fn decompress_gz_roundtrip() {
        use flate2::write::GzEncoder;
        use flate2::Compression;
        use std::io::Write;

        let original = b"hello from gzip compression test";
        let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(original).unwrap();
        let compressed = encoder.finish().unwrap();

        let decompressed = decompress_by_url("http://example.com/Packages.gz", compressed).unwrap();
        assert_eq!(decompressed, original);
    }

    #[test]
    fn decompress_xz_roundtrip() {
        use std::io::Write;
        use xz2::write::XzEncoder;

        let original = b"hello from xz compression test";
        let mut encoder = XzEncoder::new(Vec::new(), 6);
        encoder.write_all(original).unwrap();
        let compressed = encoder.finish().unwrap();

        let decompressed = decompress_by_url("http://example.com/Packages.xz", compressed).unwrap();
        assert_eq!(decompressed, original);
    }

    #[test]
    fn decompress_unknown_extension_passthrough() {
        let data = b"raw data".to_vec();
        let result = decompress_by_url("http://example.com/Packages", data.clone()).unwrap();
        assert_eq!(result, data);
    }

    #[test]
    fn transport_construction_succeeds() {
        let t = AptTransport::with_default_config();
        assert!(t.is_ok());
    }

    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate};

    #[tokio::test]
    async fn fetch_bytes_from_mock_server() {
        let mock = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/Packages"))
            .respond_with(ResponseTemplate::new(200).set_body_string("a: 1\nb: 2\n"))
            .mount(&mock)
            .await;

        let t = AptTransport::with_default_config().unwrap();
        let body = t
            .fetch_bytes(&format!("{}/Packages", mock.uri()))
            .await
            .unwrap();
        assert_eq!(body, b"a: 1\nb: 2\n");
    }

    #[tokio::test]
    async fn fetch_range_sends_range_header() {
        let mock = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/pkg.deb"))
            .and(header("range", "bytes=2-5"))
            .respond_with(ResponseTemplate::new(206).set_body_bytes(b"llo"))
            .mount(&mock)
            .await;

        let t = AptTransport::with_default_config().unwrap();
        let part = t
            .fetch_range(&format!("{}/pkg.deb", mock.uri()), 2, Some(5))
            .await
            .unwrap();
        assert_eq!(part, b"llo");
    }

    #[tokio::test]
    async fn fetch_range_open_ended() {
        let mock = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/pkg.deb"))
            .and(header("range", "bytes=3-"))
            .respond_with(ResponseTemplate::new(206).set_body_bytes(b"lo world"))
            .mount(&mock)
            .await;

        let t = AptTransport::with_default_config().unwrap();
        let part = t
            .fetch_range(&format!("{}/pkg.deb", mock.uri()), 3, None)
            .await
            .unwrap();
        assert_eq!(part, b"lo world");
    }

    /// Responds `304` when an `If-Modified-Since` header is present, else `200`.
    struct IfModifiedResponder;

    impl Respond for IfModifiedResponder {
        fn respond(&self, request: &Request) -> ResponseTemplate {
            if request.headers.get("if-modified-since").is_some() {
                ResponseTemplate::new(304)
            } else {
                ResponseTemplate::new(200).set_body_string("changed")
            }
        }
    }

    #[tokio::test]
    async fn fetch_if_modified_since_returns_none_on_304() {
        let mock = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/Release"))
            .respond_with(IfModifiedResponder)
            .mount(&mock)
            .await;

        let t = AptTransport::with_default_config().unwrap();
        let since = chrono::Utc::now() - chrono::Duration::hours(1);
        let res = t
            .fetch_if_modified_since(&format!("{}/Release", mock.uri()), since)
            .await
            .unwrap();
        assert!(res.is_none());
    }

    #[tokio::test]
    async fn fetch_if_modified_since_returns_body_on_200() {
        let mock = MockServer::start().await;
        let since = chrono::Utc::now() - chrono::Duration::hours(1);
        Mock::given(method("GET"))
            .and(path("/Release"))
            .respond_with(ResponseTemplate::new(200).set_body_string("Origin: Debian\n"))
            .mount(&mock)
            .await;

        let t = AptTransport::with_default_config().unwrap();
        let res = t
            .fetch_if_modified_since(&format!("{}/Release", mock.uri()), since)
            .await
            .unwrap();
        assert_eq!(res.unwrap(), b"Origin: Debian\n");
    }

    #[tokio::test]
    async fn fetch_with_mirrors_failover() {
        let bad = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/x"))
            .respond_with(ResponseTemplate::new(500))
            .mount(&bad)
            .await;
        let good = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/x"))
            .respond_with(ResponseTemplate::new(200).set_body_string("ok"))
            .mount(&good)
            .await;

        let t = AptTransport::with_default_config().unwrap();
        let body = t
            .fetch_with_mirrors(&[format!("{}/x", bad.uri()), format!("{}/x", good.uri())])
            .await
            .unwrap();
        assert_eq!(body, b"ok");
    }

    /// A responder that honors HTTP `Range` by slicing its fixed payload.
    struct RangeFile {
        data: Vec<u8>,
    }

    impl Respond for RangeFile {
        fn respond(&self, request: &Request) -> ResponseTemplate {
            let range = request
                .headers
                .get("range")
                .and_then(|v| v.to_str().ok())
                .unwrap_or("");
            let body: Vec<u8> = if let Some(rest) = range.strip_prefix("bytes=") {
                let (s, e) = match rest.split_once('-') {
                    Some((s, e)) => {
                        let start = s.parse::<usize>().unwrap_or(0);
                        let end = if e.is_empty() {
                            self.data.len()
                        } else {
                            e.parse::<usize>()
                                .unwrap_or(self.data.len())
                                .min(self.data.len())
                        };
                        (start, end)
                    }
                    None => (0, self.data.len()),
                };
                self.data.get(s..e).unwrap_or(&[]).to_vec()
            } else {
                self.data.clone()
            };
            if range.is_empty() {
                ResponseTemplate::new(200).set_body_bytes(body)
            } else {
                ResponseTemplate::new(206).set_body_bytes(body)
            }
        }
    }

    #[tokio::test]
    async fn download_file_resumable_appends() {
        let mock = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/big.bin"))
            .respond_with(RangeFile {
                data: b"hello world, this is a big file".to_vec(),
            })
            .mount(&mock)
            .await;

        let t = AptTransport::with_default_config().unwrap();
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("big.bin");

        t.download_file_resumable(&format!("{}/big.bin", mock.uri()), &dest, None)
            .await
            .unwrap();
        assert_eq!(
            tokio::fs::read(&dest).await.unwrap(),
            b"hello world, this is a big file"
        );

        // Simulate a partial download, then resume.
        tokio::fs::write(&dest, b"hello world").await.unwrap();
        t.download_file_resumable(&format!("{}/big.bin", mock.uri()), &dest, None)
            .await
            .unwrap();
        assert_eq!(
            tokio::fs::read(&dest).await.unwrap(),
            b"hello world, this is a big file"
        );
    }
}
