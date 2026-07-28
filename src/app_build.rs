//! Standalone application packaging: runtime stub + embedded bytecode.

use crate::bytecode::Module;
use crate::bytecode_format::{self, package_app, serialize_module};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Platform {
    Current,
    Windows,
    Linux,
    Macos,
    Uefi,
}

impl Platform {
    pub fn parse(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().as_str() {
            "current" | "host" | "native-app" => Some(Self::Current),
            "windows" | "win" | "win32" | "win64" => Some(Self::Windows),
            "linux" => Some(Self::Linux),
            "macos" | "mac" | "darwin" | "osx" => Some(Self::Macos),
            "uefi" | "efi" => Some(Self::Uefi),
            _ => None,
        }
    }

    pub fn rust_triple(self) -> &'static str {
        match self {
            Self::Current => host_triple(),
            Self::Windows => {
                if cfg!(target_arch = "aarch64") {
                    "aarch64-pc-windows-msvc"
                } else {
                    "x86_64-pc-windows-msvc"
                }
            }
            Self::Linux => {
                if cfg!(target_arch = "aarch64") {
                    "aarch64-unknown-linux-gnu"
                } else {
                    "x86_64-unknown-linux-gnu"
                }
            }
            Self::Macos => {
                if cfg!(target_arch = "aarch64") {
                    "aarch64-apple-darwin"
                } else {
                    "x86_64-apple-darwin"
                }
            }
            Self::Uefi => "x86_64-unknown-uefi",
        }
    }

    pub fn exe_suffix(self) -> &'static str {
        match self {
            Self::Windows => ".exe",
            Self::Uefi => ".efi",
            Self::Current if cfg!(windows) => ".exe",
            _ => "",
        }
    }

    pub fn is_host(self) -> bool {
        match self {
            Self::Current => true,
            Self::Windows => cfg!(windows),
            Self::Linux => cfg!(target_os = "linux"),
            Self::Macos => cfg!(target_os = "macos"),
            Self::Uefi => false,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Current => "current",
            Self::Windows => "windows",
            Self::Linux => "linux",
            Self::Macos => "macos",
            Self::Uefi => "uefi",
        }
    }
}

fn host_triple() -> &'static str {
    if cfg!(all(target_os = "windows", target_arch = "x86_64")) {
        "x86_64-pc-windows-msvc"
    } else if cfg!(all(target_os = "windows", target_arch = "aarch64")) {
        "aarch64-pc-windows-msvc"
    } else if cfg!(all(target_os = "linux", target_arch = "x86_64")) {
        "x86_64-unknown-linux-gnu"
    } else if cfg!(all(target_os = "linux", target_arch = "aarch64")) {
        "aarch64-unknown-linux-gnu"
    } else if cfg!(all(target_os = "macos", target_arch = "x86_64")) {
        "x86_64-apple-darwin"
    } else if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
        "aarch64-apple-darwin"
    } else {
        "x86_64-unknown-linux-gnu"
    }
}

pub struct AppBuildResult {
    pub output: PathBuf,
    pub bytecode_path: PathBuf,
    pub platform: Platform,
}

/// Build a standalone app: runtime stub with bytecode appended.
pub fn build_app(
    source_path: &Path,
    module: &Module,
    platform: Platform,
    output: Option<&Path>,
) -> Result<AppBuildResult, Box<dyn std::error::Error>> {
    let bytecode = serialize_module(module);
    let stem = source_path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("app");

    let out_dir = source_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join("dist");
    fs::create_dir_all(&out_dir)?;

    let bytecode_path = out_dir.join(format!("{}.rtbc", stem));
    fs::write(&bytecode_path, &bytecode)?;

    let exe_name = format!("{}{}", stem, platform.exe_suffix());
    let output_path = output
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| out_dir.join(&exe_name));

    // Also write a portable project for cross-machine rebuilds
    write_portable_project(&out_dir, stem, &bytecode, platform)?;

    let stub = ensure_stub(platform)?;
    let packaged = package_app(&stub, &bytecode);
    fs::write(&output_path, &packaged)?;

    // Make executable on Unix
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(&output_path)?.permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&output_path, perms)?;
    }

    Ok(AppBuildResult {
        output: output_path,
        bytecode_path,
        platform,
    })
}

fn write_portable_project(
    out_dir: &Path,
    stem: &str,
    bytecode: &[u8],
    platform: Platform,
) -> Result<(), Box<dyn std::error::Error>> {
    let proj = out_dir.join(format!("{}_app", stem));
    fs::create_dir_all(proj.join("src"))?;
    fs::write(proj.join("embedded.rtbc"), bytecode)?;

    let manifest = format!(
        r#"[package]
name = "{stem}-app"
version = "0.1.0"
edition = "2021"

[[bin]]
name = "{stem}"
path = "src/main.rs"

[dependencies]
raytask = {{ path = "{raytask_path}" }}
"#,
        stem = stem,
        raytask_path = escape_toml_path(&find_raytask_root()?)
    );
    fs::write(proj.join("Cargo.toml"), manifest)?;

    fs::write(
        proj.join("src/main.rs"),
        r#"//! Auto-generated RayTask standalone app (runtime + embedded bytecode).
fn main() {
    let bytes = include_bytes!("../embedded.rtbc");
    if let Err(e) = raytask::run_bytecode(bytes) {
        eprintln!("runtime error: {e}");
        std::process::exit(1);
    }
}
"#,
    )?;

    fs::write(
        proj.join("README.md"),
        format!(
            r#"# {stem} (RayTask app)

Standalone application with **RayTask runtime + bytecode** embedded.

## Quick run (prebuilt)

```
../{stem}{suffix}
```

## Rebuild for platform `{platform}`

```bash
cargo build --release --target {triple}
```

Bytecode: `embedded.rtbc`
"#,
            stem = stem,
            suffix = platform.exe_suffix(),
            platform = platform.name(),
            triple = platform.rust_triple(),
        ),
    )?;

    Ok(())
}

fn escape_toml_path(p: &Path) -> String {
    p.display().to_string().replace('\\', "/")
}

fn find_raytask_root() -> Result<PathBuf, Box<dyn std::error::Error>> {
    // Prefer the directory containing this compiler's Cargo.toml
    if let Ok(exe) = env::current_exe() {
        // target/release/raytask -> repo root
        if let Some(root) = exe
            .parent() // release
            .and_then(|p| p.parent()) // target
            .and_then(|p| p.parent())
        {
            if root.join("Cargo.toml").exists() && root.join("src").join("lib.rs").exists() {
                return Ok(root.to_path_buf());
            }
        }
    }
    let cwd = env::current_dir()?;
    if cwd.join("Cargo.toml").exists() {
        return Ok(cwd);
    }
    Err("cannot locate raytask source tree for portable app project".into())
}

fn ensure_stub(platform: Platform) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    let root = find_raytask_root().unwrap_or_else(|_| env::current_dir().unwrap());
    let stub_path = stub_path_for(&root, platform);
    if stub_needs_rebuild(&stub_path, &root) {
        build_stub(&root, platform)?;
    }
    if !stub_path.exists() {
        return Err(format!(
            "runtime stub not found at {} — run: cargo build --release --bin raytask-stub{}",
            stub_path.display(),
            if platform.is_host() {
                String::new()
            } else {
                format!(" --target {}", platform.rust_triple())
            }
        )
        .into());
    }

    Ok(fs::read(&stub_path)?)
}

fn stub_path_for(root: &Path, platform: Platform) -> PathBuf {
    let triple = platform.rust_triple();
    let stub_name = if platform.exe_suffix() == ".exe" || triple.contains("windows") {
        "raytask-stub.exe"
    } else {
        "raytask-stub"
    };
    if platform.is_host() {
        root.join("target").join("release").join(stub_name)
    } else {
        root.join("target")
            .join(triple)
            .join("release")
            .join(stub_name)
    }
}

/// Rebuild stub when missing or older than format / stub sources (avoids
/// "unsupported .rtbc version N" from a stale release stub).
fn stub_needs_rebuild(stub_path: &Path, root: &Path) -> bool {
    if !stub_path.exists() {
        return true;
    }
    let Ok(stub_meta) = fs::metadata(stub_path) else {
        return true;
    };
    let Ok(stub_mtime) = stub_meta.modified() else {
        return true;
    };
    let watch = [
        root.join("src/bytecode_format.rs"),
        root.join("src/bin/raytask_stub.rs"),
        root.join("src/lib.rs"),
        root.join("src/vm.rs"),
        root.join("src/value.rs"),
    ];
    for p in watch {
        if let Ok(meta) = fs::metadata(&p) {
            if let Ok(mtime) = meta.modified() {
                if mtime > stub_mtime {
                    return true;
                }
            }
        }
    }
    false
}

/// Public entry for Linker / tools: load (and refresh) the runtime stub bytes.
pub fn load_runtime_stub(platform: Platform) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    ensure_stub(platform)
}

fn build_stub(root: &Path, platform: Platform) -> Result<(), Box<dyn std::error::Error>> {
    let mut cmd = Command::new("cargo");
    cmd.arg("build")
        .arg("--release")
        .arg("--bin")
        .arg("raytask-stub")
        .current_dir(root);

    if !platform.is_host() {
        let triple = platform.rust_triple();
        cmd.arg("--target").arg(triple);
        // Ensure target is installed (best-effort)
        let _ = Command::new("rustup")
            .args(["target", "add", triple])
            .status();
    }

    let status = cmd.status()?;
    if !status.success() {
        return Err(format!(
            "failed to build runtime stub for platform '{}' ({})",
            platform.name(),
            platform.rust_triple()
        )
        .into());
    }
    Ok(())
}

/// Generate a C source that embeds bytecode as a byte array (for inspection / custom loaders).
pub fn generate_embedded_c(module: &Module, app_name: &str) -> String {
    let bytes = serialize_module(module);
    let mut out = String::new();
    out.push_str("/* Auto-generated RayTask embedded bytecode */\n");
    out.push_str("#include <stdint.h>\n");
    out.push_str("#include <stddef.h>\n\n");
    out.push_str(&format!(
        "/* App: {} | {} bytes of .rtbc */\n",
        app_name,
        bytes.len()
    ));
    out.push_str("static const uint8_t RAYTASK_BYTECODE[] = {\n");
    for (i, b) in bytes.iter().enumerate() {
        if i % 16 == 0 {
            out.push_str("  ");
        }
        out.push_str(&format!("0x{:02x},", b));
        if i % 16 == 15 {
            out.push('\n');
        } else {
            out.push(' ');
        }
    }
    if bytes.len() % 16 != 0 {
        out.push('\n');
    }
    out.push_str("};\n");
    out.push_str(&format!(
        "static const size_t RAYTASK_BYTECODE_LEN = {};\n",
        bytes.len()
    ));
    out.push_str(
        r#"
/* Link with the RayTask runtime stub, or load RAYTASK_BYTECODE via raytask-stub.
 * Standalone apps are normally produced with:
 *   raytask build app.rt --target app --platform <windows|linux|macos>
 */
"#,
    );
    out
}

pub use bytecode_format::extract_app_payload;
