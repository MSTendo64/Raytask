//! `project.rtp` / package manifest parsing and local package manager.

use crate::error::{CompileError, CompileResult};
use crate::span::Span;
use crate::{Optimize, Target};
use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Default)]
pub struct ProjectFile {
    pub name: String,
    pub version: String,
    pub author: String,
    pub description: String,
    pub dependencies: Vec<Dependency>,
    pub build: BuildConfig,
    pub entry: Option<PathBuf>,
    pub path: PathBuf,
}

#[derive(Debug, Clone)]
pub struct Dependency {
    pub name: String,
    pub version: String,
}

#[derive(Debug, Clone)]
pub struct BuildConfig {
    pub optimize: Optimize,
    pub target: Target,
    pub gc: bool,
    pub debug: bool,
}

impl Default for BuildConfig {
    fn default() -> Self {
        Self {
            optimize: Optimize::None,
            target: Target::Bytecode,
            gc: true,
            debug: false,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct PackageManifest {
    pub name: String,
    pub version: String,
    pub author: String,
    pub description: String,
    pub exports: Vec<String>,
    pub imports: Vec<String>,
    pub root: PathBuf,
}

/// Parse `project "Name" { ... }` or `package "Name" { ... }`.
pub fn parse_project_file(source: &str, path: &Path) -> CompileResult<ProjectFile> {
    let mut p = RtpParser::new(source);
    let kind = p.expect_ident()?;
    if kind != "project" && kind != "package" {
        return Err(CompileError::syntax(
            format!("expected 'project' or 'package', found '{}'", kind),
            Span::default(),
        ));
    }
    let name = p.expect_string()?;
    p.expect_char('{')?;
    let mut proj = ProjectFile {
        name,
        version: "0.1.0".into(),
        path: path.to_path_buf(),
        ..Default::default()
    };
    while !p.peek_char('}') && !p.eof() {
        let key = p.expect_ident()?;
        match key.as_str() {
            "version" => {
                p.expect_char('=')?;
                proj.version = p.expect_string()?;
            }
            "author" => {
                p.expect_char('=')?;
                proj.author = p.expect_string()?;
            }
            "description" => {
                p.expect_char('=')?;
                proj.description = p.expect_string()?;
            }
            "entry" => {
                p.expect_char('=')?;
                proj.entry = Some(PathBuf::from(p.expect_string()?));
            }
            "dependencies" => {
                p.expect_char('{')?;
                while !p.peek_char('}') && !p.eof() {
                    let dep_name = p.expect_string()?;
                    let mut ver = "*".into();
                    if p.try_ident("version") {
                        ver = p.expect_string()?;
                    } else if p.peek_char('=') {
                        p.expect_char('=')?;
                        ver = p.expect_string()?;
                    }
                    proj.dependencies.push(Dependency {
                        name: dep_name,
                        version: ver,
                    });
                }
                p.expect_char('}')?;
            }
            "build" => {
                p.expect_char('{')?;
                while !p.peek_char('}') && !p.eof() {
                    let k = p.expect_ident()?;
                    p.expect_char('=')?;
                    match k.as_str() {
                        "optimize" => {
                            let v = p.expect_string()?;
                            proj.build.optimize = match v.as_str() {
                                "speed" => Optimize::Speed,
                                "size" => Optimize::Size,
                                _ => Optimize::None,
                            };
                        }
                        "target" => {
                            let v = p.expect_string()?;
                            proj.build.target = Target::parse(&v).unwrap_or(Target::Bytecode);
                        }
                        "gc" => {
                            proj.build.gc = p.expect_bool()?;
                        }
                        "debug" => {
                            proj.build.debug = p.expect_bool()?;
                        }
                        _ => {
                            let _ = p.expect_value_skip()?;
                        }
                    }
                }
                p.expect_char('}')?;
            }
            "export" | "imports" | "import" => {
                // package.rtp fields — skip structured block / list
                if p.peek_char('{') {
                    p.expect_char('{')?;
                    let mut depth = 1;
                    while !p.eof() && depth > 0 {
                        if p.peek_char('{') {
                            depth += 1;
                            p.bump();
                        } else if p.peek_char('}') {
                            depth -= 1;
                            p.bump();
                        } else {
                            p.bump();
                        }
                    }
                } else {
                    let _ = p.expect_value_skip()?;
                }
            }
            _ => {
                if p.peek_char('=') {
                    p.expect_char('=')?;
                    let _ = p.expect_value_skip()?;
                } else if p.peek_char('{') {
                    p.expect_char('{')?;
                    let mut depth = 1;
                    while !p.eof() && depth > 0 {
                        if p.peek_char('{') {
                            depth += 1;
                            p.bump();
                        } else if p.peek_char('}') {
                            depth -= 1;
                            p.bump();
                        } else {
                            p.bump();
                        }
                    }
                }
            }
        }
    }
    p.expect_char('}')?;
    Ok(proj)
}

pub fn load_project(dir: &Path) -> CompileResult<ProjectFile> {
    let path = if dir.join("project.rtp").exists() {
        dir.join("project.rtp")
    } else if dir.is_file() && dir.extension().map(|e| e == "rtp").unwrap_or(false) {
        dir.to_path_buf()
    } else {
        return Err(CompileError::Io {
            message: format!("project.rtp not found in {}", dir.display()),
        });
    };
    let src = std::fs::read_to_string(&path).map_err(|e| CompileError::Io {
        message: format!("{}: {}", path.display(), e),
    })?;
    let mut proj = parse_project_file(&src, &path)?;
    if proj.entry.is_none() {
        let root = path.parent().unwrap_or(Path::new("."));
        if root.join("src/main.rt").exists() {
            proj.entry = Some(PathBuf::from("src/main.rt"));
        } else if root.join("main.rt").exists() {
            proj.entry = Some(PathBuf::from("main.rt"));
        }
    }
    Ok(proj)
}

pub fn packages_dir() -> PathBuf {
    PathBuf::from(".raytask").join("packages")
}

pub fn registry_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Ok(r) = std::env::var("RAYTASK_REGISTRY") {
        // Local path only (URLs handled via registry_url())
        if !r.starts_with("http://") && !r.starts_with("https://") {
            dirs.push(PathBuf::from(r));
        }
    }
    if let Some(home) = std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME")) {
        dirs.push(PathBuf::from(home).join(".raytask").join("registry"));
    }
    dirs.push(PathBuf::from("registry"));
    dirs
}

/// Remote registry base URL from `RAYTASK_REGISTRY_URL` or `RAYTASK_REGISTRY` if it is http(s).
pub fn registry_url() -> Option<String> {
    if let Ok(u) = std::env::var("RAYTASK_REGISTRY_URL") {
        return Some(u.trim_end_matches('/').to_string());
    }
    if let Ok(u) = std::env::var("RAYTASK_REGISTRY") {
        if u.starts_with("http://") || u.starts_with("https://") {
            return Some(u.trim_end_matches('/').to_string());
        }
    }
    None
}

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
pub struct RegistryIndex {
    pub packages: Vec<RegistryPackage>,
}

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
pub struct RegistryPackage {
    pub name: String,
    pub version: String,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
}

/// Fetch remote index: `GET {registry}/index.json`.
pub fn fetch_remote_index() -> Result<RegistryIndex, String> {
    let base = registry_url().ok_or_else(|| {
        "no remote registry — set RAYTASK_REGISTRY_URL (e.g. https://registry.example/raytask)"
            .to_string()
    })?;
    let url = format!("{}/index.json", base);
    let body = ureq::get(&url)
        .call()
        .map_err(|e| format!("registry fetch {}: {}", url, e))?
        .into_string()
        .map_err(|e| e.to_string())?;
    serde_json::from_str(&body).map_err(|e| format!("invalid index.json: {}", e))
}

pub fn search_packages(query: &str) -> Result<Vec<RegistryPackage>, String> {
    let q = query.to_ascii_lowercase();
    let mut found = Vec::new();

    // Remote
    if registry_url().is_some() {
        if let Ok(idx) = fetch_remote_index() {
            for p in idx.packages {
                if p.name.to_ascii_lowercase().contains(&q)
                    || p.description
                        .as_deref()
                        .unwrap_or("")
                        .to_ascii_lowercase()
                        .contains(&q)
                {
                    found.push(p);
                }
            }
        }
    }

    // Local registries
    for reg in registry_dirs() {
        if !reg.is_dir() {
            continue;
        }
        if let Ok(rd) = std::fs::read_dir(&reg) {
            for e in rd.flatten() {
                let name = e.file_name().to_string_lossy().to_string();
                if name.to_ascii_lowercase().contains(&q) {
                    let ver = read_package_version(&e.path()).unwrap_or_else(|| "0.1.0".into());
                    found.push(RegistryPackage {
                        name,
                        version: ver,
                        url: Some(e.path().display().to_string()),
                        description: Some("local registry".into()),
                    });
                }
            }
        }
    }
    Ok(found)
}

fn read_package_version(dir: &Path) -> Option<String> {
    let rtp = dir.join("package.rtp");
    let src = std::fs::read_to_string(rtp).ok()?;
    for line in src.lines() {
        let t = line.trim();
        if let Some(rest) = t.strip_prefix("version") {
            let rest = rest.trim().trim_start_matches('=').trim();
            if let Some(v) = rest.strip_prefix('"').and_then(|s| s.strip_suffix('"')) {
                return Some(v.to_string());
            }
        }
    }
    None
}

/// Download package tarball/zip from remote: `{url}/packages/{name}/{version}.zip`
/// or package `url` field. Extracts into `.raytask/packages/<name>`.
pub fn install_from_remote(name: &str, version: &str) -> Result<PathBuf, String> {
    let base = registry_url().ok_or_else(|| "no RAYTASK_REGISTRY_URL".to_string())?;
    let dest = packages_dir().join(name);
    if dest.exists() {
        std::fs::remove_dir_all(&dest).map_err(|e| e.to_string())?;
    }
    std::fs::create_dir_all(&dest).map_err(|e| e.to_string())?;

    // Prefer index entry URL
    let mut download_url = None;
    if let Ok(idx) = fetch_remote_index() {
        if let Some(p) = idx
            .packages
            .iter()
            .find(|p| p.name == name && (version == "*" || p.version == version))
        {
            download_url = p.url.clone();
        }
    }
    let url = download_url.unwrap_or_else(|| {
        format!("{}/packages/{}/{}.zip", base, name, version)
    });

    let resp = ureq::get(&url)
        .call()
        .map_err(|e| format!("download {}: {}", url, e))?;
    let mut bytes = Vec::new();
    resp.into_reader()
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;

    // ZIP: look for local PK header; otherwise treat as raw package tree JSON/text fallback
    if bytes.starts_with(b"PK") {
        extract_zip_naive(&bytes, &dest)?;
    } else if let Ok(text) = std::str::from_utf8(&bytes) {
        // Plain text package: single lib.rt body
        std::fs::create_dir_all(dest.join("src")).map_err(|e| e.to_string())?;
        std::fs::write(dest.join("src/lib.rt"), text).map_err(|e| e.to_string())?;
        std::fs::write(
            dest.join("package.rtp"),
            format!(
                "package \"{name}\" {{\n    version = \"{version}\"\n}}\n"
            ),
        )
        .map_err(|e| e.to_string())?;
    } else {
        return Err(format!("unsupported package payload from {}", url));
    }
    Ok(dest)
}

/// Minimal ZIP extractor (stored + deflate via std only for stored; deflate needs flate2).
/// Supports Store (method 0) entries only for zero extra deps; otherwise writes raw `.zip` and hints.
fn extract_zip_naive(data: &[u8], dest: &Path) -> Result<(), String> {
    // If we can't inflate, save the archive for the user
    let zip_path = dest.join("_package.zip");
    std::fs::write(&zip_path, data).map_err(|e| e.to_string())?;

    let mut i = 0usize;
    let mut extracted = 0usize;
    while i + 30 <= data.len() {
        if &data[i..i + 4] != b"PK\x03\x04" {
            break;
        }
        let method = u16::from_le_bytes([data[i + 8], data[i + 9]]);
        let comp_size = u32::from_le_bytes(data[i + 18..i + 22].try_into().unwrap()) as usize;
        let name_len = u16::from_le_bytes([data[i + 26], data[i + 27]]) as usize;
        let extra_len = u16::from_le_bytes([data[i + 28], data[i + 29]]) as usize;
        let name_start = i + 30;
        let name_end = name_start + name_len;
        if name_end + extra_len + comp_size > data.len() {
            break;
        }
        let name = String::from_utf8_lossy(&data[name_start..name_end]).to_string();
        let data_start = name_end + extra_len;
        let payload = &data[data_start..data_start + comp_size];
        if !name.ends_with('/') {
            if method == 0 {
                let out = dest.join(&name);
                if let Some(parent) = out.parent() {
                    std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
                }
                std::fs::write(&out, payload).map_err(|e| e.to_string())?;
                extracted += 1;
            }
        }
        i = data_start + comp_size;
    }
    if extracted == 0 {
        // Leave zip for manual unzip; still scaffold so resolve works
        std::fs::create_dir_all(dest.join("src")).ok();
        std::fs::write(
            dest.join("src/lib.rt"),
            "// Unzip _package.zip into this folder\nexport string PackageName() => \"pending\";\n",
        )
        .ok();
        return Err(format!(
            "downloaded zip (deflate?) — saved to {}; extract manually or use store-method zips",
            zip_path.display()
        ));
    }
    let _ = std::fs::remove_file(zip_path);
    Ok(())
}

/// Publish local package directory to remote registry (`POST {url}/packages/{name}/{version}`)
/// or copy into local `registry/` folder.
pub fn publish_package(dir: &Path) -> Result<String, String> {
    let rtp = if dir.join("package.rtp").exists() {
        dir.join("package.rtp")
    } else {
        return Err("package.rtp not found".into());
    };
    let src = std::fs::read_to_string(&rtp).map_err(|e| e.to_string())?;
    let name = src
        .lines()
        .find_map(|l| {
            let t = l.trim();
            if t.starts_with("package ") {
                t.trim_start_matches("package ")
                    .trim()
                    .trim_matches('"')
                    .split('{')
                    .next()
                    .map(|s| s.trim().trim_matches('"').to_string())
            } else {
                None
            }
        })
        .unwrap_or_else(|| {
            dir.file_name()
                .and_then(|s| s.to_str())
                .unwrap_or("pkg")
                .to_string()
        });
    let version = read_package_version(dir).unwrap_or_else(|| "0.1.0".into());

    if let Some(base) = registry_url() {
        let url = format!("{}/packages/{}/{}", base, name, version);
        // Pack as simple concatenated sources for MVP transport
        let mut body = String::new();
        let lib = dir.join("src/lib.rt");
        if lib.exists() {
            body = std::fs::read_to_string(lib).unwrap_or_default();
        }
        let resp = ureq::post(&url)
            .set("Content-Type", "text/plain")
            .send_string(&body);
        match resp {
            Ok(_) => return Ok(format!("published {}@{} → {}", name, version, url)),
            Err(e) => {
                return Err(format!("publish failed: {} (is the registry writable?)", e));
            }
        }
    }

    // Local registry copy
    let dest = PathBuf::from("registry").join(format!("{}-{}", name, version));
    if dest.exists() {
        std::fs::remove_dir_all(&dest).map_err(|e| e.to_string())?;
    }
    copy_dir(dir, &dest).map_err(|e| e.to_string())?;
    // Update local index.json
    let index_path = PathBuf::from("registry").join("index.json");
    let mut index = if index_path.exists() {
        serde_json::from_str(&std::fs::read_to_string(&index_path).unwrap_or_default())
            .unwrap_or(RegistryIndex {
                packages: vec![],
            })
    } else {
        RegistryIndex {
            packages: vec![],
        }
    };
    index.packages.retain(|p| !(p.name == name && p.version == version));
    index.packages.push(RegistryPackage {
        name: name.clone(),
        version: version.clone(),
        url: Some(dest.display().to_string()),
        description: None,
    });
    std::fs::create_dir_all("registry").ok();
    std::fs::write(
        &index_path,
        serde_json::to_string_pretty(&index).unwrap_or_default(),
    )
    .map_err(|e| e.to_string())?;
    Ok(format!(
        "published {}@{} → local registry {}",
        name,
        version,
        dest.display()
    ))
}

/// Install package into `.raytask/packages/<name>`.
/// Looks up local registry folders, then remote URL; otherwise scaffolds a stub package.
pub fn install_package(name: &str, version: Option<&str>) -> Result<PathBuf, String> {
    let ver = version.unwrap_or("0.1.0");
    let dest = packages_dir().join(name);
    if dest.exists() {
        return Ok(dest);
    }
    std::fs::create_dir_all(&dest).map_err(|e| e.to_string())?;

    // Copy from registry if present
    for reg in registry_dirs() {
        let src = reg.join(name);
        if src.is_dir() {
            copy_dir(&src, &dest).map_err(|e| e.to_string())?;
            return Ok(dest);
        }
        let src_ver = reg.join(format!("{}-{}", name, ver));
        if src_ver.is_dir() {
            copy_dir(&src_ver, &dest).map_err(|e| e.to_string())?;
            return Ok(dest);
        }
    }

    // Remote registry
    if registry_url().is_some() {
        match install_from_remote(name, ver) {
            Ok(p) => return Ok(p),
            Err(e) => {
                // Fall through to stub, but surface hint
                eprintln!("note: remote install failed: {}", e);
            }
        }
    }

    // Scaffold stub
    std::fs::write(
        dest.join("package.rtp"),
        format!(
            r#"package "{name}" {{
    version = "{ver}"
    author = ""
    description = "Local RayTask package"

    export {{
    }}

    import {{
    }}
}}
"#
        ),
    )
    .map_err(|e| e.to_string())?;
    std::fs::create_dir_all(dest.join("src")).map_err(|e| e.to_string())?;
    std::fs::write(
        dest.join("src/lib.rt"),
        format!(
            "// Package {name} {ver}\nexport string PackageName() => \"{name}\";\n"
        ),
    )
    .map_err(|e| e.to_string())?;
    Ok(dest)
}

pub fn uninstall_package(name: &str) -> Result<bool, String> {
    let dest = packages_dir().join(name);
    if dest.exists() {
        std::fs::remove_dir_all(&dest).map_err(|e| e.to_string())?;
        Ok(true)
    } else {
        Ok(false)
    }
}

pub fn update_packages(proj: &ProjectFile) -> Result<Vec<String>, String> {
    let mut updated = Vec::new();
    for dep in &proj.dependencies {
        let path = install_package(&dep.name, Some(&dep.version))?;
        updated.push(format!("{} @ {}", dep.name, path.display()));
    }
    Ok(updated)
}

/// Resolve dependency roots for compilation (`.rt` search paths).
pub fn resolve_dep_paths(proj: &ProjectFile) -> Result<Vec<PathBuf>, String> {
    let mut paths = Vec::new();
    for dep in &proj.dependencies {
        let p = packages_dir().join(&dep.name);
        if !p.exists() {
            install_package(&dep.name, Some(&dep.version))?;
        }
        if p.join("src").exists() {
            paths.push(p.join("src"));
        } else {
            paths.push(p);
        }
    }
    Ok(paths)
}

pub fn entry_path(proj: &ProjectFile) -> Result<PathBuf, String> {
    let root = proj.path.parent().unwrap_or(Path::new("."));
    let entry = proj
        .entry
        .clone()
        .ok_or_else(|| "no entry point in project.rtp".to_string())?;
    let full = if entry.is_absolute() {
        entry
    } else {
        root.join(entry)
    };
    if !full.exists() {
        return Err(format!("entry not found: {}", full.display()));
    }
    Ok(full)
}

fn copy_dir(src: &Path, dst: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let ty = entry.file_type()?;
        let to = dst.join(entry.file_name());
        if ty.is_dir() {
            copy_dir(&entry.path(), &to)?;
        } else {
            std::fs::copy(entry.path(), to)?;
        }
    }
    Ok(())
}

struct RtpParser<'a> {
    src: &'a str,
    i: usize,
}

impl<'a> RtpParser<'a> {
    fn new(src: &'a str) -> Self {
        Self { src, i: 0 }
    }

    fn eof(&self) -> bool {
        self.i >= self.src.len()
    }

    fn skip_ws(&mut self) {
        while let Some(c) = self.peek() {
            if c.is_whitespace() {
                self.bump();
            } else if self.src[self.i..].starts_with("//") {
                while let Some(c) = self.peek() {
                    self.bump();
                    if c == '\n' {
                        break;
                    }
                }
            } else {
                break;
            }
        }
    }

    fn peek(&self) -> Option<char> {
        self.src[self.i..].chars().next()
    }

    fn bump(&mut self) -> Option<char> {
        let c = self.peek()?;
        self.i += c.len_utf8();
        Some(c)
    }

    fn peek_char(&mut self, ch: char) -> bool {
        self.skip_ws();
        self.peek() == Some(ch)
    }

    fn expect_char(&mut self, ch: char) -> CompileResult<()> {
        self.skip_ws();
        if self.peek() == Some(ch) {
            self.bump();
            Ok(())
        } else {
            Err(CompileError::syntax(
                format!("expected '{}'", ch),
                Span::default(),
            ))
        }
    }

    fn expect_ident(&mut self) -> CompileResult<String> {
        self.skip_ws();
        let start = self.i;
        let Some(c) = self.peek() else {
            return Err(CompileError::syntax("expected identifier", Span::default()));
        };
        if !(c.is_ascii_alphabetic() || c == '_') {
            return Err(CompileError::syntax(
                format!("expected identifier, found '{}'", c),
                Span::default(),
            ));
        }
        self.bump();
        while let Some(c) = self.peek() {
            if c.is_ascii_alphanumeric() || c == '_' {
                self.bump();
            } else {
                break;
            }
        }
        Ok(self.src[start..self.i].to_string())
    }

    fn try_ident(&mut self, want: &str) -> bool {
        self.skip_ws();
        let save = self.i;
        if let Ok(id) = self.expect_ident() {
            if id == want {
                return true;
            }
        }
        self.i = save;
        false
    }

    fn expect_string(&mut self) -> CompileResult<String> {
        self.skip_ws();
        if self.peek() != Some('"') {
            return Err(CompileError::syntax("expected string", Span::default()));
        }
        self.bump();
        let mut out = String::new();
        while let Some(c) = self.peek() {
            self.bump();
            if c == '"' {
                break;
            }
            if c == '\\' {
                if let Some(n) = self.bump() {
                    out.push(n);
                }
            } else {
                out.push(c);
            }
        }
        Ok(out)
    }

    fn expect_bool(&mut self) -> CompileResult<bool> {
        self.skip_ws();
        let id = self.expect_ident()?;
        match id.as_str() {
            "true" => Ok(true),
            "false" => Ok(false),
            _ => Err(CompileError::syntax("expected true/false", Span::default())),
        }
    }

    fn expect_value_skip(&mut self) -> CompileResult<()> {
        self.skip_ws();
        if self.peek() == Some('"') {
            let _ = self.expect_string()?;
            return Ok(());
        }
        if self.peek().map(|c| c.is_ascii_digit()).unwrap_or(false) {
            while self.peek().map(|c| c.is_ascii_digit() || c == '.').unwrap_or(false) {
                self.bump();
            }
            return Ok(());
        }
        let _ = self.expect_ident()?;
        Ok(())
    }
}

/// Merge dependency search paths into a map name → root (for resolve).
pub fn dep_aliases(proj: &ProjectFile) -> HashMap<String, PathBuf> {
    let mut m = HashMap::new();
    for dep in &proj.dependencies {
        let p = packages_dir().join(&dep.name);
        if p.exists() {
            m.insert(dep.name.clone(), p);
        }
    }
    m
}
