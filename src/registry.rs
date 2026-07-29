//! Multi-repository package manager for RayTask.
//!
//! Configuration: `rtp.repos.yml` in the current directory or `~/.raytask/rtp.repos.yml`.
//!
//! Packages are installed to `external/<name>/` relative to the project root.
//! Each repo exposes an HTTP (or local-file) index and tarballs according to
//! the RayTask Registry Protocol defined in docs/REGISTRY_PROTOCOL.md.

use std::io::Read;
use std::path::{Path, PathBuf};

// ── YAML config structures ────────────────────────────────────────────────────

/// `rtp.repos.yml` — loaded from project root or `~/.raytask/`.
#[derive(Debug, Clone, serde::Deserialize, serde::Serialize, Default)]
pub struct ReposConfig {
    /// Ordered list of repositories (highest priority first, or specify `priority` field).
    #[serde(default)]
    pub repositories: Vec<RepoEntry>,
    /// Override where packages are installed (default: `external/`).
    #[serde(default)]
    pub install_dir: Option<String>,
}

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
pub struct RepoEntry {
    /// Human-readable name, e.g. "official", "company-internal".
    pub name: String,
    /// Base URL (https://...) or local path (file:///... or plain path).
    pub url: String,
    /// Higher number = higher priority. Default 0.
    #[serde(default)]
    pub priority: i64,
    /// Whether to require HTTPS for remote repos.
    #[serde(default)]
    pub secure: Option<bool>,
    /// Optional auth token sent as `Authorization: Bearer <token>`.
    #[serde(default)]
    pub token: Option<String>,
}

// ── Registry server protocol structures ──────────────────────────────────────

/// `GET {base}/index.json` — catalog of all packages in a repo.
#[derive(Debug, Clone, serde::Deserialize, serde::Serialize, Default)]
pub struct RepoIndex {
    /// Registry name (informational).
    #[serde(default)]
    pub registry: String,
    /// All packages in this registry.
    pub packages: Vec<IndexEntry>,
}

/// One package in the index.
#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
pub struct IndexEntry {
    /// Package name, e.g. "HttpClient".
    pub name: String,
    /// Latest stable version, e.g. "1.2.0".
    pub version: String,
    /// All available versions for this package, newest first.
    #[serde(default)]
    pub versions: Vec<String>,
    /// One-line description shown in `search`.
    #[serde(default)]
    pub description: Option<String>,
    /// Detailed description + install instructions shown with `install --info`.
    #[serde(default)]
    pub instructions: Option<String>,
    /// Author / maintainer.
    #[serde(default)]
    pub author: Option<String>,
    /// Homepage / source link.
    #[serde(default)]
    pub homepage: Option<String>,
    /// License identifier.
    #[serde(default)]
    pub license: Option<String>,
    /// Tags for search.
    #[serde(default)]
    pub tags: Vec<String>,
    /// Explicit tarball URL (overrides the default pattern).
    #[serde(default)]
    pub download_url: Option<String>,
}

// ── Config loading ────────────────────────────────────────────────────────────

/// Search order: `./rtp.repos.yml` then `~/.raytask/rtp.repos.yml`.
pub fn load_config() -> ReposConfig {
    let candidates = config_paths();
    for p in &candidates {
        if let Ok(src) = std::fs::read_to_string(p) {
            if let Ok(cfg) = serde_yaml::from_str::<ReposConfig>(&src) {
                return cfg;
            }
        }
    }
    ReposConfig::default()
}

fn config_paths() -> Vec<PathBuf> {
    let mut paths = vec![PathBuf::from("rtp.repos.yml")];
    if let Some(home) = std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME")) {
        paths.push(PathBuf::from(home).join(".raytask").join("rtp.repos.yml"));
    }
    paths
}

/// Where packages are installed (project-relative).
pub fn install_dir(cfg: &ReposConfig) -> PathBuf {
    PathBuf::from(cfg.install_dir.as_deref().unwrap_or("external"))
}

/// Sort repos: highest priority first, then by declaration order (stable sort).
pub fn sorted_repos(cfg: &ReposConfig) -> Vec<&RepoEntry> {
    let mut v: Vec<&RepoEntry> = cfg.repositories.iter().collect();
    v.sort_by(|a, b| b.priority.cmp(&a.priority));
    v
}

// ── Index fetching ────────────────────────────────────────────────────────────

/// Fetch and parse `{base}/index.json` for a single repo.
pub fn fetch_index(repo: &RepoEntry) -> Result<RepoIndex, String> {
    let url = format!("{}/index.json", repo.url.trim_end_matches('/'));
    fetch_json(&url, repo.token.as_deref())
}

fn fetch_json<T: serde::de::DeserializeOwned>(url: &str, token: Option<&str>) -> Result<T, String> {
    if url.starts_with("file://") {
        let path = url.trim_start_matches("file://");
        let src = std::fs::read_to_string(path)
            .map_err(|e| format!("read {path}: {e}"))?;
        return serde_json::from_str(&src)
            .map_err(|e| format!("parse {path}: {e}"));
    }
    // Local-path repo (no scheme)
    if !url.starts_with("http://") && !url.starts_with("https://") {
        let src = std::fs::read_to_string(url)
            .map_err(|e| format!("read {url}: {e}"))?;
        return serde_json::from_str(&src)
            .map_err(|e| format!("parse {url}: {e}"));
    }
    let mut req = ureq::get(url);
    if let Some(tok) = token {
        req = req.set("Authorization", &format!("Bearer {tok}"));
    }
    let body = req
        .call()
        .map_err(|e| format!("GET {url}: {e}"))?
        .into_string()
        .map_err(|e| e.to_string())?;
    serde_json::from_str(&body).map_err(|e| format!("invalid JSON from {url}: {e}"))
}

// ── Resolver ─────────────────────────────────────────────────────────────────

/// A resolved package candidate with its source repo.
#[derive(Debug, Clone)]
pub struct Candidate {
    pub repo: RepoEntry,
    pub entry: IndexEntry,
    /// The specific version we will install.
    pub resolved_version: String,
}

/// Find the best candidate across all repos:
/// - If `version` is Some, find that exact version.
/// - If None, pick the newest version across repos.
/// - When same version exists in multiple repos, use the highest-priority one.
pub fn resolve(
    name: &str,
    version: Option<&str>,
    cfg: &ReposConfig,
) -> Result<Candidate, String> {
    let repos = sorted_repos(cfg); // highest priority first
    let mut best: Option<Candidate> = None;

    for repo in repos {
        let index = match fetch_index(repo) {
            Ok(i) => i,
            Err(e) => {
                eprintln!("  [registry] skip {} — {}", repo.name, e);
                continue;
            }
        };

        for entry in index.packages {
            if entry.name.to_ascii_lowercase() != name.to_ascii_lowercase() {
                continue;
            }

            let target_ver = match version {
                Some(v) => {
                    // Exact version requested
                    let available: Vec<&str> = if entry.versions.is_empty() {
                        vec![entry.version.as_str()]
                    } else {
                        entry.versions.iter().map(|s| s.as_str()).collect()
                    };
                    if !available.contains(&v) {
                        continue; // this repo doesn't have the requested version
                    }
                    v.to_string()
                }
                None => {
                    // Pick newest from this repo (versions[0] or `version` field)
                    if entry.versions.is_empty() {
                        entry.version.clone()
                    } else {
                        entry.versions[0].clone()
                    }
                }
            };

            // Compare with current best
            let use_this = match &best {
                None => true,
                Some(b) => {
                    if b.resolved_version == target_ver {
                        // Same version — keep higher-priority repo (already sorted, but
                        // we may encounter a better same-priority later; use first found)
                        false
                    } else if version.is_none() {
                        // No specific version: pick lexicographically newer semver
                        semver_gt(&target_ver, &b.resolved_version)
                    } else {
                        false
                    }
                }
            };

            if use_this {
                best = Some(Candidate {
                    repo: repo.clone(),
                    entry: entry.clone(),
                    resolved_version: target_ver,
                });
            }
        }
    }

    best.ok_or_else(|| {
        if let Some(v) = version {
            format!("package '{name}@{v}' not found in any configured repository")
        } else {
            format!("package '{name}' not found in any configured repository")
        }
    })
}

/// Naive semver comparison: split by '.' and compare numeric parts.
fn semver_gt(a: &str, b: &str) -> bool {
    let parse = |s: &str| -> Vec<u64> {
        s.split('.')
            .map(|p| p.split(['-', '+']).next().unwrap_or(p).parse().unwrap_or(0))
            .collect()
    };
    let av = parse(a);
    let bv = parse(b);
    for (x, y) in av.iter().zip(bv.iter()) {
        if x != y {
            return x > y;
        }
    }
    av.len() > bv.len()
}

// ── Installer ─────────────────────────────────────────────────────────────────

/// Install a candidate into `external/<name>/`.
/// Returns the installed directory path.
pub fn install_candidate(
    cand: &Candidate,
    cfg: &ReposConfig,
) -> Result<PathBuf, String> {
    let dest = install_dir(cfg).join(&cand.entry.name);
    if dest.exists() {
        std::fs::remove_dir_all(&dest).map_err(|e| e.to_string())?;
    }
    std::fs::create_dir_all(&dest).map_err(|e| e.to_string())?;

    let url = build_download_url(&cand.repo, &cand.entry, &cand.resolved_version);
    download_and_extract(&url, cand.repo.token.as_deref(), &dest)?;

    // Write a lock manifest next to the package
    let lock = PackageLock {
        name: cand.entry.name.clone(),
        version: cand.resolved_version.clone(),
        repo: cand.repo.name.clone(),
        repo_url: cand.repo.url.clone(),
    };
    let lock_path = dest.join("rtp.lock.yml");
    let lock_src = serde_yaml::to_string(&lock).unwrap_or_default();
    std::fs::write(lock_path, lock_src).ok();

    Ok(dest)
}

/// Lock file written into `external/<name>/rtp.lock.yml`.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct PackageLock {
    pub name: String,
    pub version: String,
    pub repo: String,
    pub repo_url: String,
}

fn build_download_url(repo: &RepoEntry, entry: &IndexEntry, version: &str) -> String {
    if let Some(u) = &entry.download_url {
        return u.clone();
    }
    // Default pattern: {base}/packages/{name}/{version}.zip
    format!(
        "{}/packages/{}/{}.zip",
        repo.url.trim_end_matches('/'),
        entry.name,
        version
    )
}

fn download_and_extract(url: &str, token: Option<&str>, dest: &Path) -> Result<(), String> {
    // Local file path
    if url.starts_with("file://") {
        let path = url.trim_start_matches("file://");
        let bytes = std::fs::read(path).map_err(|e| format!("read {path}: {e}"))?;
        return extract_package(&bytes, dest);
    }
    if !url.starts_with("http://") && !url.starts_with("https://") {
        let bytes = std::fs::read(url).map_err(|e| format!("read {url}: {e}"))?;
        return extract_package(&bytes, dest);
    }

    let mut req = ureq::get(url);
    if let Some(tok) = token {
        req = req.set("Authorization", &format!("Bearer {tok}"));
    }
    let resp = req.call().map_err(|e| format!("GET {url}: {e}"))?;
    let mut bytes = Vec::new();
    resp.into_reader()
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;

    extract_package(&bytes, dest)
}

/// Extract a zip payload using flate2 (deflate) or fall back to stored entries.
fn extract_package(data: &[u8], dest: &Path) -> Result<(), String> {
    if data.starts_with(b"PK") {
        return extract_zip(data, dest);
    }
    // Plain text: treat as a single lib.rt
    if let Ok(text) = std::str::from_utf8(data) {
        std::fs::create_dir_all(dest.join("src")).map_err(|e| e.to_string())?;
        std::fs::write(dest.join("src/lib.rt"), text).map_err(|e| e.to_string())?;
        return Ok(());
    }
    Err("unknown package format (expected .zip or plain-text .rt)".into())
}

fn extract_zip(data: &[u8], dest: &Path) -> Result<(), String> {
    use flate2::read::DeflateDecoder;
    let mut i = 0usize;
    let mut extracted = 0usize;

    while i + 30 <= data.len() {
        if &data[i..i + 4] != b"PK\x03\x04" {
            break;
        }
        let method = u16::from_le_bytes([data[i + 8], data[i + 9]]);
        let comp_size = u32::from_le_bytes(data[i + 18..i + 22].try_into().unwrap()) as usize;
        let uncomp_size = u32::from_le_bytes(data[i + 22..i + 26].try_into().unwrap()) as usize;
        let name_len = u16::from_le_bytes([data[i + 26], data[i + 27]]) as usize;
        let extra_len = u16::from_le_bytes([data[i + 28], data[i + 29]]) as usize;
        let name_start = i + 30;
        let name_end = name_start + name_len;
        if name_end + extra_len + comp_size > data.len() {
            break;
        }
        let entry_name = String::from_utf8_lossy(&data[name_start..name_end]).to_string();
        let data_start = name_end + extra_len;
        let payload = &data[data_start..data_start + comp_size];

        if !entry_name.ends_with('/') {
            let out = dest.join(&entry_name);
            if let Some(parent) = out.parent() {
                std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
            }
            match method {
                0 => {
                    std::fs::write(&out, payload).map_err(|e| e.to_string())?;
                }
                8 => {
                    let mut dec = DeflateDecoder::new(payload);
                    let mut buf = Vec::with_capacity(uncomp_size);
                    dec.read_to_end(&mut buf).map_err(|e| e.to_string())?;
                    std::fs::write(&out, buf).map_err(|e| e.to_string())?;
                }
                _ => {
                    return Err(format!("unsupported zip method {method} for {entry_name}"));
                }
            }
            extracted += 1;
        }
        i = data_start + comp_size;
    }

    if extracted == 0 {
        return Err("zip contained no extractable files".into());
    }
    Ok(())
}

// ── High-level install ────────────────────────────────────────────────────────

pub struct InstallOptions<'a> {
    pub name: &'a str,
    pub version: Option<&'a str>,
    /// If true, print info + instructions and prompt before installing.
    pub show_info: bool,
}

/// Main entry point called from CLI.
pub fn install(opts: InstallOptions<'_>) -> Result<PathBuf, String> {
    let cfg = load_config();
    let candidate = resolve(opts.name, opts.version, &cfg)?;

    if opts.show_info {
        print_info(&candidate);
        if !prompt_confirm("Install this package? [y/N] ") {
            return Err("installation cancelled by user".into());
        }
    }

    let dest = install_candidate(&candidate, &cfg)?;
    Ok(dest)
}

fn print_info(cand: &Candidate) {
    let e = &cand.entry;
    println!();
    println!("  Package : {} v{}", e.name, cand.resolved_version);
    if let Some(a) = &e.author {
        println!("  Author  : {a}");
    }
    if let Some(l) = &e.license {
        println!("  License : {l}");
    }
    if let Some(h) = &e.homepage {
        println!("  Homepage: {h}");
    }
    println!("  From    : {} ({})", cand.repo.name, cand.repo.url);
    if !e.tags.is_empty() {
        println!("  Tags    : {}", e.tags.join(", "));
    }
    if let Some(d) = &e.description {
        println!();
        println!("  {d}");
    }
    if let Some(inst) = &e.instructions {
        println!();
        println!("── Installation instructions ─────────────────────────────────");
        println!("{inst}");
        println!("─────────────────────────────────────────────────────────────");
    }
    println!();
}

fn prompt_confirm(msg: &str) -> bool {
    use std::io::Write;
    print!("{msg}");
    let _ = std::io::stdout().flush();
    let mut line = String::new();
    let _ = std::io::stdin().read_line(&mut line);
    matches!(line.trim().to_ascii_lowercase().as_str(), "y" | "yes")
}

// ── Search across all repos ───────────────────────────────────────────────────

pub struct SearchResult {
    pub repo_name: String,
    pub entry: IndexEntry,
}

pub fn search(query: &str) -> Vec<SearchResult> {
    let cfg = load_config();
    let q = query.to_ascii_lowercase();
    let mut results: Vec<SearchResult> = Vec::new();

    for repo in sorted_repos(&cfg) {
        let Ok(index) = fetch_index(repo) else { continue };
        for entry in index.packages {
            let matches = entry.name.to_ascii_lowercase().contains(&q)
                || entry
                    .description
                    .as_deref()
                    .unwrap_or("")
                    .to_ascii_lowercase()
                    .contains(&q)
                || entry.tags.iter().any(|t| t.to_ascii_lowercase().contains(&q));
            if matches {
                results.push(SearchResult {
                    repo_name: repo.name.clone(),
                    entry,
                });
            }
        }
    }
    results
}

// ── Uninstall ─────────────────────────────────────────────────────────────────

pub fn uninstall(name: &str) -> Result<bool, String> {
    let cfg = load_config();
    let dest = install_dir(&cfg).join(name);
    if dest.exists() {
        std::fs::remove_dir_all(&dest).map_err(|e| e.to_string())?;
        Ok(true)
    } else {
        Ok(false)
    }
}

// ── List installed ────────────────────────────────────────────────────────────

pub struct InstalledPackage {
    pub name: String,
    pub version: String,
    pub repo: String,
}

pub fn list_installed() -> Vec<InstalledPackage> {
    let cfg = load_config();
    let dir = install_dir(&cfg);
    let mut result = Vec::new();
    if let Ok(rd) = std::fs::read_dir(&dir) {
        for e in rd.flatten() {
            if !e.path().is_dir() {
                continue;
            }
            let name = e.file_name().to_string_lossy().to_string();
            let lock_path = e.path().join("rtp.lock.yml");
            let (version, repo) = if let Ok(src) = std::fs::read_to_string(lock_path) {
                if let Ok(lock) = serde_yaml::from_str::<PackageLock>(&src) {
                    (lock.version, lock.repo)
                } else {
                    ("?".into(), "?".into())
                }
            } else {
                ("?".into(), "?".into())
            };
            result.push(InstalledPackage { name, version, repo });
        }
    }
    result
}
