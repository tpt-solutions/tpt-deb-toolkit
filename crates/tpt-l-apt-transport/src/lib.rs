//! Async HTTP/HTTPS transport for fetching APT indices and `.deb` files.
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

use thiserror::Error;
use tokio::io::AsyncWriteExt as _;

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
    /// `progress` is called with `(bytes_downloaded, total_bytes)`.
    /// `total_bytes` is `0` when the server omits `Content-Length`.
    pub async fn download_file(
        &self,
        url: &str,
        dest: &Path,
        progress: Option<Box<dyn Fn(u64, u64) + Send>>,
    ) -> Result<(), TransportError> {
        use tokio::fs::File;

        let resp = self.client.get(url).send().await?;
        if !resp.status().is_success() {
            return Err(TransportError::Http(resp.status()));
        }
        let total = resp.content_length().unwrap_or(0);
        let data = resp.bytes().await?;
        let downloaded = data.len() as u64;
        let mut file = File::create(dest).await?;
        file.write_all(&data).await?;
        file.flush().await?;
        if let Some(ref cb) = progress {
            cb(downloaded, total.max(downloaded));
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
}
