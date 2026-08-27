//! Command-line interface for the `tpt-deb-toolkit` APT layer.
//!
//! Provides `update`, `install`, `search`, `show`, `list`, and `completions`
//! subcommands built on the layer crates. Offline commands (`search`, `show`,
//! `list --installed`) work against locally cached indices and the dpkg status
//! database; `update` and `install` perform network I/O.

use std::path::PathBuf;

use anyhow::{anyhow, Context, Result};
use clap::CommandFactory;
use serde::Serialize;

use tpt_l_apt_solver::{Resolver, Universe};
use tpt_l_control_file::BinaryPackage;
use tpt_l_deb_format::DebFile;
use tpt_l_dpkg_db::StatusDb;
use tpt_l_sources_list::{SourceEntry, SourcesList};

/// Default cache directory name (created under the user's cache root).
const CACHE_DIR_NAME: &str = "tpt-apt";

/// High-level APT CLI application state.
pub struct Apt {
    /// Directory holding cached `Packages` indices.
    pub cache_dir: PathBuf,
    /// Path to the dpkg status database.
    pub status_path: PathBuf,
    /// Optional path to a `sources.list` file.
    pub sources_path: Option<PathBuf>,
    /// When `true`, mutating commands only print what they would do.
    pub dry_run: bool,
    /// When `true`, long-running loops (`update`, `install`) render an
    /// `indicatif` progress bar to stderr. Suppressed automatically when the
    /// `--json` flag is set so machine-readable output stays clean.
    pub progress: bool,
}

/// A search result row.
#[derive(Debug, Clone, Serialize)]
pub struct SearchHit {
    pub name: String,
    pub version: String,
    pub description: String,
}

impl Apt {
    /// Construct an [`Apt`] with default locations derived from the OS.
    pub fn new(dry_run: bool) -> Self {
        let cache_dir = default_cache_dir();
        let status_path = if cfg!(unix) {
            PathBuf::from("/var/lib/dpkg/status")
        } else {
            cache_dir.join("status")
        };
        Self {
            cache_dir,
            status_path,
            sources_path: None,
            dry_run,
            progress: true,
        }
    }

    /// Override the dpkg status database path.
    pub fn with_status(mut self, path: PathBuf) -> Self {
        self.status_path = path;
        self
    }

    /// Override the sources list path.
    pub fn with_sources(mut self, path: Option<PathBuf>) -> Self {
        self.sources_path = path;
        self
    }

    /// Override the cache directory.
    pub fn with_cache_dir(mut self, dir: PathBuf) -> Self {
        self.cache_dir = dir;
        self
    }

    /// Load every cached `Packages` index into typed [`BinaryPackage`]s.
    fn load_index(&self) -> Result<Vec<BinaryPackage>> {
        let mut all = Vec::new();
        if !self.cache_dir.exists() {
            return Ok(all);
        }
        for entry in std::fs::read_dir(&self.cache_dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("packages") {
                continue;
            }
            let content = std::fs::read_to_string(&path)?;
            for pkg in BinaryPackage::parse_packages_index(&content)
                .into_iter()
                .filter_map(Result::ok)
            {
                all.push(pkg);
            }
        }
        Ok(all)
    }

    /// Search cached indices for packages whose name or description contains
    /// `query` (case-insensitive).
    pub fn search(&self, query: &str) -> Result<Vec<SearchHit>> {
        let q = query.to_lowercase();
        let mut hits = Vec::new();
        for pkg in self.load_index()? {
            let hay = format!(
                "{} {}",
                pkg.name.to_lowercase(),
                pkg.description.to_lowercase()
            );
            if hay.contains(&q) {
                hits.push(SearchHit {
                    name: pkg.name,
                    version: pkg.version_str,
                    description: pkg.description,
                });
            }
        }
        hits.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(hits)
    }

    /// Show the metadata for the package named exactly `name` (first match).
    pub fn show(&self, name: &str) -> Result<Option<BinaryPackage>> {
        for pkg in self.load_index()? {
            if pkg.name == name {
                return Ok(Some(pkg));
            }
        }
        Ok(None)
    }

    /// List packages from the dpkg status database.
    ///
    /// When `installed_only` is `true`, only `installed` packages are returned.
    pub fn list(&self, installed_only: bool) -> Result<Vec<tpt_l_dpkg_db::InstalledPackage>> {
        let db = StatusDb::open(&self.status_path)
            .with_context(|| format!("opening status db {}", self.status_path.display()))?;
        let mut out: Vec<_> = if installed_only {
            db.installed_packages().cloned().collect()
        } else {
            db.packages().to_vec()
        };
        out.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(out)
    }

    /// Fetch and cache `Packages` indices from all configured sources.
    ///
    /// Requires network access. Each fetched index is written to the cache
    /// directory as `<host>_<suite>_<component>.packages`.
    pub async fn update(&self) -> Result<usize> {
        let sources = self.load_sources()?;
        std::fs::create_dir_all(&self.cache_dir)?;
        let transport = tpt_l_apt_transport::AptTransport::with_default_config()?;

        // Collect every (url, destination) pair first so we know the total.
        let mut tasks: Vec<(String, PathBuf)> = Vec::new();
        for entry in sources
            .entries()
            .filter(|e| e.enabled && e.source_type == tpt_l_sources_list::SourceType::Binary)
        {
            for component in &entry.components {
                let url = packages_url(entry, component, "amd64");
                let slug = url_slug(&entry.uri, &entry.suite, component);
                let out = self.cache_dir.join(format!("{slug}.packages"));
                tasks.push((url, out));
            }
        }

        let pb = make_progress(
            self.progress,
            tasks.len() as u64,
            "Fetching indices [{bar:20.cyan}] {pos}/{len} ({eta})",
        );

        let mut fetched = 0;
        for (url, out) in &tasks {
            if let Some(pb) = &pb {
                pb.set_message(url.clone());
            }
            let bytes = transport
                .fetch_bytes(url)
                .await
                .with_context(|| format!("fetching {url}"))?;
            let bytes = maybe_gunzip(&bytes);
            std::fs::write(out, bytes)?;
            fetched += 1;
            if let Some(pb) = &pb {
                pb.inc(1);
            }
        }
        if let Some(pb) = &pb {
            pb.finish_and_clear();
        }
        Ok(fetched)
    }

    fn load_sources(&self) -> Result<SourcesList> {
        let path = self
            .sources_path
            .clone()
            .or_else(default_sources_path)
            .ok_or_else(|| anyhow!("no sources list configured"))?;
        SourcesList::load_file(&path).with_context(|| format!("parsing {}", path.display()))
    }

    /// Resolve and install `packages` (best-effort single-pass).
    ///
    /// Builds the dependency universe from cached indices, resolves a plan with
    /// the SAT solver, downloads each `.deb`, extracts it, and runs the
    /// `postinst` maintainer script. With `dry_run`, only the resolved plan is
    /// printed.
    pub async fn install(&self, packages: &[String]) -> Result<Vec<String>> {
        let index = self.load_index()?;
        if index.is_empty() {
            return Err(anyhow!(
                "no cached package indices; run `update` first (or point --config at a sources list)"
            ));
        }
        let universe = Universe::from_binary_packages(&index)
            .map_err(|e| anyhow!("building solver universe: {e}"))?;
        let resolver = Resolver::new(universe);
        let requests: Vec<&str> = packages.iter().map(String::as_str).collect();
        let plan = resolver
            .resolve(&requests)
            .map_err(|e| anyhow!("dependency resolution failed: {e}"))?;

        let to_install: Vec<(String, tpt_l_deb_version::Version)> = plan.install.clone();
        if self.dry_run {
            for (name, version) in &to_install {
                println!("Would install {} {}", name, version);
            }
            return Ok(to_install.iter().map(|(n, _)| n.clone()).collect());
        }

        let transport = tpt_l_apt_transport::AptTransport::with_default_config()?;
        let mut installed = Vec::new();
        let pb = make_progress(
            self.progress,
            to_install.len() as u64,
            "Installing {msg} [{bar:20.cyan}] {pos}/{len}",
        );
        for (name, version) in &to_install {
            if let Some(pb) = &pb {
                pb.set_message(format!("{name} {version}"));
            }
            let binary = index
                .iter()
                .find(|b| b.name == *name)
                .ok_or_else(|| anyhow!("resolved package {} not found in index", name))?;
            let filename = binary
                .filename
                .as_ref()
                .ok_or_else(|| anyhow!("package {} has no download filename", name))?;
            let uri = self
                .index_source_uri(&index, name)
                .ok_or_else(|| anyhow!("cannot determine download URI for {}", name))?;
            let url = format!(
                "{}/{}",
                uri.trim_end_matches('/'),
                filename.trim_start_matches('/')
            );
            let deb_bytes = transport
                .fetch_bytes(&url)
                .await
                .with_context(|| format!("downloading {url}"))?;

            let dest = self.cache_dir.join("unpack").join(name);
            std::fs::create_dir_all(&dest)?;
            let deb = DebFile::parse(&deb_bytes).map_err(|e| anyhow!("parsing .deb: {e}"))?;
            deb.extract(&dest)
                .map_err(|e| anyhow!("extracting {}: {e}", name))?;

            let control_dir = dest.join("DEBIAN");
            if control_dir.exists() {
                use tpt_l_maintainer_scripts::{PackageRef, ScriptRunner};
                let runner = ScriptRunner::unrestricted(
                    control_dir,
                    PackageRef {
                        name: name.clone(),
                        version: version.to_string(),
                        arch: binary.architecture.clone(),
                    },
                );
                if let Ok(outcome) = runner.run_postinst("configure") {
                    if !outcome.success() {
                        tracing::warn!(package = %name, code = outcome.exit_code, "postinst exited non-zero");
                    }
                }
            }
            installed.push(name.clone());
            if let Some(pb) = &pb {
                pb.inc(1);
            }
        }
        if let Some(pb) = &pb {
            pb.finish_and_clear();
        }
        Ok(installed)
    }

    /// Find the repository URI that provides `package_name`, by consulting the
    /// sources list. Returns the first binary source's URI as a heuristic.
    fn index_source_uri(&self, _index: &[BinaryPackage], _package_name: &str) -> Option<String> {
        let sources = self.load_sources().ok()?;
        let entries: Vec<_> = sources.entries().collect();
        entries
            .into_iter()
            .find(|e| e.enabled && e.source_type == tpt_l_sources_list::SourceType::Binary)
            .map(|e| e.uri.clone())
    }
}

fn packages_url(entry: &SourceEntry, component: &str, arch: &str) -> String {
    format!(
        "{}/dists/{}/{}/binary-{}/Packages",
        entry.uri.trim_end_matches('/'),
        entry.suite,
        component,
        arch
    )
}

fn url_slug(uri: &str, suite: &str, component: &str) -> String {
    let host = uri
        .trim_end_matches('/')
        .split("://")
        .nth(1)
        .unwrap_or(uri)
        .replace(['/', ':'], "_");
    format!("{}_{}_{}", host, suite, component)
}

/// Create an `indicatif` progress bar over `len` items, or `None` when
/// progress reporting is disabled (e.g. JSON mode). The bar writes to stderr
/// so it never corrupts structured output on stdout.
fn make_progress(enabled: bool, len: u64, template: &str) -> Option<indicatif::ProgressBar> {
    if !enabled {
        return None;
    }
    let pb = indicatif::ProgressBar::new(len);
    pb.set_style(
        indicatif::ProgressStyle::with_template(template)
            .unwrap_or_else(|_| indicatif::ProgressStyle::default_bar()),
    );
    Some(pb)
}

/// Decompress `bytes` if they look like gzip; otherwise return them as-is.
fn maybe_gunzip(bytes: &[u8]) -> Vec<u8> {
    if bytes.len() > 2 && bytes[0] == 0x1f && bytes[1] == 0x8b {
        use std::io::Read;
        let mut dec = flate2::read::GzDecoder::new(bytes);
        let mut out = Vec::new();
        if dec.read_to_end(&mut out).is_ok() {
            return out;
        }
    }
    bytes.to_vec()
}

/// Compute the default cache directory for the current OS.
fn default_cache_dir() -> PathBuf {
    if let Some(cache) = std::env::var_os("XDG_CACHE_HOME") {
        return PathBuf::from(cache).join(CACHE_DIR_NAME);
    }
    if let Some(home) = std::env::var_os("HOME") {
        return PathBuf::from(home).join(".cache").join(CACHE_DIR_NAME);
    }
    if let Some(local) = std::env::var_os("LOCALAPPDATA") {
        return PathBuf::from(local).join(CACHE_DIR_NAME);
    }
    PathBuf::from(format!(".{CACHE_DIR_NAME}"))
}

/// Best-effort default `sources.list` location.
fn default_sources_path() -> Option<PathBuf> {
    if cfg!(unix) {
        let p = PathBuf::from("/etc/apt/sources.list");
        if p.exists() {
            return Some(p);
        }
        let d = PathBuf::from("/etc/apt/sources.list.d");
        if d.exists() {
            return Some(d);
        }
    }
    None
}

/// Shared CLI argument definitions.
#[derive(clap::Parser)]
#[command(name = "tpt-l-apt", about = "tpt-deb-toolkit APT layer CLI")]
pub struct Cli {
    /// Path to a sources list or apt config (used by `update`/`install`).
    #[arg(long)]
    pub config: Option<PathBuf>,
    /// Only print what would be done; do not mutate the system.
    #[arg(long)]
    pub dry_run: bool,
    /// Emit verbose logs.
    #[arg(long)]
    pub verbose: bool,
    /// Emit machine-readable JSON instead of human text.
    #[arg(long)]
    pub json: bool,
    #[command(subcommand)]
    pub command: Command,
}

/// Subcommands.
#[derive(clap::Subcommand)]
pub enum Command {
    /// Fetch and cache package indices from configured sources.
    Update,
    /// Resolve dependencies and install packages.
    Install {
        /// Package names to install.
        packages: Vec<String>,
    },
    /// Search cached indices for a query string.
    Search {
        /// Substring to search for (matched against name/description).
        query: String,
    },
    /// Show metadata for a single package.
    Show {
        /// Exact package name.
        package: String,
    },
    /// List packages from the dpkg status database.
    List {
        /// Only list installed packages.
        #[arg(long)]
        installed: bool,
    },
    /// Generate a shell completion script (bash, zsh, or fish).
    Completions {
        /// Target shell: `bash`, `zsh`, or `fish`.
        #[arg(long, default_value = "bash")]
        shell: String,
        /// Write the script to this path instead of stdout.
        #[arg(long)]
        output: Option<PathBuf>,
    },
}

/// Run the CLI. This is the shared entry point used by `main`.
pub async fn run(cli: Cli) -> Result<()> {
    let filter = if cli.verbose {
        "tpt_deb_toolkit=debug,tpt_l=debug"
    } else {
        "info"
    };
    let _ = tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_env_filter(filter)
        .try_init();

    let mut apt = Apt::new(cli.dry_run).with_sources(cli.config.clone());
    if cli.json {
        apt.progress = false;
    }
    if let Some(cfg) = &cli.config {
        if cfg.is_dir() {
            apt = apt.with_cache_dir(cfg.join("cache"));
        }
    }

    match &cli.command {
        Command::Update => {
            let n = apt.update().await?;
            if cli.json {
                println!("{}", serde_json::json!({ "fetched": n }));
            } else {
                println!(
                    "Fetched {n} package indices into {}",
                    apt.cache_dir.display()
                );
            }
        }
        Command::Install { packages } => {
            let installed = apt.install(packages).await?;
            if cli.json {
                println!("{}", serde_json::json!({ "installed": installed }));
            } else {
                for p in &installed {
                    println!("Installed {p}");
                }
            }
        }
        Command::Search { query } => {
            let hits = apt.search(query)?;
            if cli.json {
                println!("{}", serde_json::to_string(&hits)?);
            } else if hits.is_empty() {
                println!("No packages matched.");
            } else {
                for h in &hits {
                    println!("{}  {}  {}", h.name, h.version, h.description);
                }
            }
        }
        Command::Show { package } => {
            let pkg = apt.show(package)?;
            match pkg {
                Some(p) => {
                    if cli.json {
                        let value = serde_json::json!({
                            "name": p.name,
                            "version": p.version_str,
                            "architecture": p.architecture,
                            "description": p.description,
                            "depends": p.depends,
                            "filename": p.filename,
                        });
                        println!("{value}");
                    } else {
                        println!("Package: {}", p.name);
                        println!("Version: {}", p.version_str);
                        println!("Architecture: {}", p.architecture);
                        if !p.description.is_empty() {
                            println!("Description: {}", p.description);
                        }
                    }
                }
                None => println!("Package {} not found in cached indices.", package),
            }
        }
        Command::List { installed } => {
            let pkgs = apt.list(*installed)?;
            if cli.json {
                let values: Vec<_> = pkgs
                    .iter()
                    .map(|p| {
                        serde_json::json!({
                            "name": p.name,
                            "version": p.version,
                            "architecture": p.architecture,
                            "status": p.status.to_string(),
                        })
                    })
                    .collect();
                println!("{}", serde_json::to_string(&values)?);
            } else if pkgs.is_empty() {
                println!("No packages found.");
            } else {
                for p in &pkgs {
                    println!("{} {}", p.name, p.version);
                }
            }
        }
        Command::Completions { shell, output } => {
            let mut cmd = Cli::command();
            match shell.as_str() {
                "bash" => write_completion(clap_complete::shells::Bash, &mut cmd, output)?,
                "zsh" => write_completion(clap_complete::shells::Zsh, &mut cmd, output)?,
                "fish" => write_completion(clap_complete::shells::Fish, &mut cmd, output)?,
                other => return Err(anyhow!("unsupported shell for completions: {other}")),
            }
        }
    }
    Ok(())
}

/// Render a completion script for `shell` and write it to `output` (or stdout).
fn write_completion<S: clap_complete::Generator>(
    shell: S,
    cmd: &mut clap::Command,
    output: &Option<PathBuf>,
) -> Result<()> {
    let mut buf: Vec<u8> = Vec::new();
    clap_complete::generate(shell, cmd, "tpt-l-apt", &mut buf);
    match output {
        Some(path) => std::fs::write(path, &buf)?,
        None => {
            use std::io::Write;
            std::io::stdout().write_all(&buf)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const PACKAGES: &str = "\
Package: curl
Version: 8.2.1-1
Architecture: amd64
Description: command line tool for transferring data
Filename: pool/main/c/curl/curl_8.2.1-1_amd64.deb

Package: libcurl4
Version: 8.2.1-1
Architecture: amd64
Description: easy-to-use client-side URL transfer library
Filename: pool/main/c/curl/libcurl4_8.2.1-1_amd64.deb
";

    const STATUS: &str = "\
Package: bash
Status: install ok installed
Architecture: amd64
Version: 5.1-6

Package: removed
Status: deinstall ok config-files
Architecture: amd64
Version: 1.0
";

    #[test]
    fn search_matches_name_and_description() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("test.packages"), PACKAGES).unwrap();
        let apt = Apt::new(false).with_cache_dir(dir.path().to_path_buf());
        let hits = apt.search("curl").unwrap();
        assert_eq!(hits.len(), 2);
        let hits = apt.search("transfer library").unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].name, "libcurl4");
    }

    #[test]
    fn show_returns_exact_package() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("test.packages"), PACKAGES).unwrap();
        let apt = Apt::new(false).with_cache_dir(dir.path().to_path_buf());
        let pkg = apt.show("curl").unwrap();
        assert!(pkg.is_some());
        assert_eq!(pkg.unwrap().version_str, "8.2.1-1");
        assert!(apt.show("nope").unwrap().is_none());
    }

    #[test]
    fn list_filters_installed() {
        let dir = tempfile::tempdir().unwrap();
        let status = dir.path().join("status");
        std::fs::write(&status, STATUS).unwrap();
        let apt = Apt::new(false)
            .with_cache_dir(dir.path().to_path_buf())
            .with_status(status);
        let all = apt.list(false).unwrap();
        assert_eq!(all.len(), 2);
        let installed = apt.list(true).unwrap();
        assert_eq!(installed.len(), 1);
        assert_eq!(installed[0].name, "bash");
    }

    #[test]
    fn completions_generate_bash() {
        use clap::CommandFactory;
        let mut cmd = Cli::command();
        let mut buf = Vec::new();
        clap_complete::generate(clap_complete::shells::Bash, &mut cmd, "tpt-l-apt", &mut buf);
        let script = String::from_utf8(buf).unwrap();
        assert!(script.contains("tpt-l-apt"));
        assert!(script.contains("update"));
        assert!(script.contains("install"));
    }
}
