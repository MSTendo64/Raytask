//! Product targets: web, mobile, wasm, embedded, kernel packaging.

use crate::ast::Program;
use crate::bytecode::Module;
use crate::bytecode_format::serialize_module;
use crate::codegen_c::{CCodegen, CodegenOptions, RuntimeProfile};
use crate::error::CompileResult;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Debug, Clone)]
pub struct TargetBuildResult {
    pub primary: PathBuf,
    pub artifacts: Vec<PathBuf>,
    pub notes: Vec<String>,
}

fn dist_dir(source: &Path, name: &str) -> PathBuf {
    let parent = source.parent().unwrap_or(Path::new("."));
    let stem = source
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("app");
    parent.join("dist").join(format!("{}_{}", stem, name))
}

fn write_c(
    program: &Program,
    opts: CodegenOptions,
    out_c: &Path,
) -> CompileResult<String> {
    let c = CCodegen::with_options(opts).generate(program)?;
    if let Some(parent) = out_c.parent() {
        fs::create_dir_all(parent).ok();
    }
    fs::write(out_c, &c).map_err(|e| crate::error::CompileError::Io {
        message: e.to_string(),
    })?;
    Ok(c)
}

fn write_c_ssa(
    program: &Program,
    opts: CodegenOptions,
    optimize: crate::Optimize,
    source: &Path,
    out_c: &Path,
) -> Result<(String, Vec<String>), Box<dyn std::error::Error>> {
    let src = source.to_string_lossy();
    let ssa = crate::ssa::build_ssa_for_c(program, optimize, true, Some(src.as_ref()))?;
    let c = CCodegen::with_options(opts).generate_with_ssa(program, &ssa)?;
    if let Some(parent) = out_c.parent() {
        fs::create_dir_all(parent).ok();
    }
    fs::write(out_c, &c)?;
    let mut notes = vec!["SSA → C (freestanding function bodies)".into()];
    match optimize {
        crate::Optimize::None => notes.push("optimize=none (lift + phi-elim only)".into()),
        crate::Optimize::Speed => notes.push("optimize=speed".into()),
        crate::Optimize::Size => notes.push("optimize=size".into()),
    }
    Ok((c, notes))
}

/// `--target wasm`: C → WebAssembly (+ HTML shell).
pub fn build_wasm(
    source: &Path,
    program: &Program,
    gc: bool,
) -> Result<TargetBuildResult, Box<dyn std::error::Error>> {
    let dir = dist_dir(source, "wasm");
    fs::create_dir_all(&dir)?;
    let stem = source
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("app");
    let c_path = dir.join(format!("{}.c", stem));
    let wasm_path = dir.join(format!("{}.wasm", stem));
    let js_path = dir.join(format!("{}.js", stem));
    let html_path = dir.join("index.html");

    write_c(
        program,
        CodegenOptions {
            profile: RuntimeProfile::Wasm,
            gc,
            freestanding: false,
        },
        &c_path,
    )?;

    fs::write(
        &html_path,
        format!(
            r#"<!DOCTYPE html>
<html lang="en">
<head>
  <meta charset="utf-8"/>
  <title>{stem} — RayTask WASM</title>
  <style>
    body {{ font-family: ui-sans-serif, system-ui; margin: 2rem; background: #0f1419; color: #e7ecf1; }}
    #out {{ white-space: pre-wrap; background: #1a2332; padding: 1rem; border-radius: 8px; }}
  </style>
</head>
<body>
  <h1>{stem}</h1>
  <p>RayTask WebAssembly target</p>
  <div id="out">Loading…</div>
  <script src="./{stem}.js"></script>
  <script>
    (async () => {{
      const el = document.getElementById('out');
      try {{
        if (typeof createRayTaskModule === 'function') {{
          const Module = await createRayTaskModule();
          el.textContent = 'WASM module loaded. Check console for print() output.';
          if (Module._Main) Module._Main();
          else if (Module.ccall) Module.ccall('Main', null, [], []);
        }} else {{
          const res = await fetch('./{stem}.wasm');
          const bytes = await res.arrayBuffer();
          const {{ instance }} = await WebAssembly.instantiate(bytes, {{ env: {{}} }});
          el.textContent = 'Raw WASM instantiated (' + bytes.byteLength + ' bytes).\\nExport keys: ' +
            Object.keys(instance.exports).join(', ');
          if (instance.exports.Main) instance.exports.Main();
          if (instance.exports.kmain) instance.exports.kmain();
        }}
      }} catch (e) {{
        el.textContent = 'WASM load error: ' + e + '\\n\\nBuild with emcc or clang --target=wasm32 (see build_wasm.sh).';
      }}
    }})();
  </script>
</body>
</html>
"#
        ),
    )?;

    fs::write(
        dir.join("build_wasm.sh"),
        format!(
            r#"#!/bin/sh
# Build RayTask WASM (requires emcc or clang+wasi-sdk)
set -e
SRC="{stem}.c"
if command -v emcc >/dev/null 2>&1; then
  emcc "$SRC" -O2 -s WASM=1 -s EXPORTED_FUNCTIONS='["_Main","_main"]' \
    -s EXPORTED_RUNTIME_METHODS='["ccall","cwrap"]' \
    -o {stem}.js
  echo "Built {stem}.js + {stem}.wasm via emcc"
elif command -v clang >/dev/null 2>&1; then
  clang --target=wasm32 -O2 -nostdlib -Wl,--no-entry -Wl,--export-all -o {stem}.wasm "$SRC" || \
  clang --target=wasm32-wasi -O2 -o {stem}.wasm "$SRC"
  echo "Built {stem}.wasm via clang"
else
  echo "Install emscripten (emcc) or wasm32 clang to compile $SRC"
  exit 1
fi
"#
        ),
    )?;

    // Minimal JS stub so the page doesn't 404 before emcc
    if !js_path.exists() {
        fs::write(
            &js_path,
            "// Generated after emcc. Placeholder until WASM is built.\n\
             console.warn('Run build_wasm.sh to produce the real module');\n\
             async function createRayTaskModule(){ throw new Error('WASM not built yet'); }\n",
        )?;
    }

    let mut notes = vec![
        "Open index.html after building WASM (build_wasm.sh)".into(),
        format!("C source: {}", c_path.display()),
    ];
    let mut artifacts = vec![c_path.clone(), html_path.clone(), js_path.clone()];

    // Try emcc / clang automatically
    if try_emcc(&c_path, &js_path) || try_clang_wasm(&c_path, &wasm_path) {
        notes.push("WASM binary compiled successfully".into());
        if wasm_path.exists() {
            artifacts.push(wasm_path);
        }
    } else {
        notes.push("Compiler for WASM not found — C + HTML scaffold written".into());
    }

    Ok(TargetBuildResult {
        primary: dir,
        artifacts,
        notes,
    })
}

fn try_emcc(c_path: &Path, js_out: &Path) -> bool {
    let status = Command::new("emcc")
        .arg(c_path)
        .arg("-O2")
        .arg("-s")
        .arg("WASM=1")
        .arg("-s")
        .arg("EXPORTED_FUNCTIONS=[\"_Main\",\"_main\"]")
        .arg("-s")
        .arg("EXPORTED_RUNTIME_METHODS=[\"ccall\",\"cwrap\"]")
        .arg("-o")
        .arg(js_out)
        .status();
    matches!(status, Ok(s) if s.success())
}

fn try_clang_wasm(c_path: &Path, wasm_out: &Path) -> bool {
    let status = Command::new("clang")
        .arg("--target=wasm32-wasi")
        .arg("-O2")
        .arg("-o")
        .arg(wasm_out)
        .arg(c_path)
        .status();
    matches!(status, Ok(s) if s.success())
}

/// `--target web`: web app bundle (WASM + static host + optional bytecode payload).
pub fn build_web(
    source: &Path,
    program: &Program,
    module: &Module,
    gc: bool,
) -> Result<TargetBuildResult, Box<dyn std::error::Error>> {
    let mut result = build_wasm(source, program, gc)?;
    let dir = dist_dir(source, "web");
    fs::create_dir_all(&dir)?;

    // Copy wasm scaffold pieces into web/
    for art in &result.artifacts {
        if let Some(name) = art.file_name() {
            let dest = dir.join(name);
            if art.exists() {
                let _ = fs::copy(art, &dest);
            }
        }
    }

    let bytes = serialize_module(module);
    let b64 = base64_encode(&bytes);
    fs::write(dir.join("embedded.rtbc"), &bytes)?;
    fs::write(
        dir.join("rtbc.js"),
        format!(
            "// RayTask bytecode payload (for host runtimes)\n\
             export const RTBC_BASE64 = \"{b64}\";\n\
             export const RTBC_BYTES = Uint8Array.from(atob(RTBC_BASE64), c => c.charCodeAt(0));\n"
        ),
    )?;

    fs::write(
        dir.join("README.md"),
        "# RayTask Web target\n\n\
         1. Run `build_wasm.sh` (or install emscripten) to produce `.wasm`\n\
         2. Serve this folder: `python -m http.server`\n\
         3. Open `index.html`\n\n\
         `embedded.rtbc` / `rtbc.js` carry the VM bytecode for alternative hosts.\n",
    )?;

    result.primary = dir.clone();
    result.artifacts.push(dir.join("embedded.rtbc"));
    result.notes.push("Web bundle in dist/*_web".into());
    Ok(result)
}

/// `--target mobile`: Android + iOS project scaffolds with embedded bytecode.
pub fn build_mobile(
    source: &Path,
    module: &Module,
) -> Result<TargetBuildResult, Box<dyn std::error::Error>> {
    let dir = dist_dir(source, "mobile");
    let android = dir.join("android");
    let ios = dir.join("ios");
    fs::create_dir_all(android.join("app/src/main/assets"))?;
    fs::create_dir_all(android.join("app/src/main/java/com/raytask/app"))?;
    fs::create_dir_all(ios.join("RayTaskApp"))?;

    let bytes = serialize_module(module);
    fs::write(android.join("app/src/main/assets/embedded.rtbc"), &bytes)?;
    fs::write(ios.join("RayTaskApp/embedded.rtbc"), &bytes)?;

    let stem = source
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("app");

    fs::write(
        android.join("README.md"),
        format!(
            "# {stem} — Android scaffold\n\n\
             Place a RayTask native/JNI runtime next to `embedded.rtbc` and load it from assets.\n\n\
             ```kotlin\n\
             // Example: copy assets/embedded.rtbc and call RayTask.run(bytes)\n\
             ```\n"
        ),
    )?;
    fs::write(
        android.join("app/src/main/java/com/raytask/app/MainActivity.kt"),
        r#"package com.raytask.app

// Scaffold — wire to RayTask Android runtime / JNI.
class MainActivity {
    fun loadBytecode(assetBytes: ByteArray) {
        // RayTaskNative.run(assetBytes)
    }
}
"#,
    )?;

    fs::write(
        ios.join("README.md"),
        format!(
            "# {stem} — iOS scaffold\n\n\
             Bundle `RayTaskApp/embedded.rtbc` and run via RayTask iOS runtime.\n"
        ),
    )?;
    fs::write(
        ios.join("RayTaskApp/App.swift"),
        r#"import Foundation

// Scaffold — load embedded.rtbc and start RayTask runtime.
@main
struct RayTaskAppMain {
    static func main() {
        if let url = Bundle.main.url(forResource: "embedded", withExtension: "rtbc"),
           let data = try? Data(contentsOf: url) {
            // RayTaskRuntime.run(data)
            print("Loaded \(data.count) bytecode bytes")
        }
    }
}
"#,
    )?;

    fs::write(
        dir.join("README.md"),
        "# RayTask Mobile target\n\n\
         - `android/` — Kotlin scaffold + `embedded.rtbc` in assets\n\
         - `ios/` — Swift scaffold + bundled bytecode\n\n\
         Link against a RayTask mobile runtime (JNI / Swift bridge) to execute.\n",
    )?;

    Ok(TargetBuildResult {
        primary: dir.clone(),
        artifacts: vec![
            android.join("app/src/main/assets/embedded.rtbc"),
            ios.join("RayTaskApp/embedded.rtbc"),
        ],
        notes: vec!["Mobile scaffolds generated (Android + iOS)".into()],
    })
}

/// `--target embedded`: freestanding C via **SSA → C** (shared optimizer with VM).
///
/// If the source directory contains board assets (`link.ld`, `startup.c`,
/// `build.sh`, …), they are copied into the output folder so MCU kits under
/// `examples/boards/` build with a real memory map.
pub fn build_embedded(
    source: &Path,
    program: &Program,
    gc: bool,
    optimize: crate::Optimize,
) -> Result<TargetBuildResult, Box<dyn std::error::Error>> {
    let dir = dist_dir(source, "embedded");
    fs::create_dir_all(&dir)?;
    let stem = source
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("app");
    let c_path = dir.join(format!("{}.c", stem));
    let (_c, mut notes) = write_c_ssa(
        program,
        CodegenOptions {
            profile: RuntimeProfile::Embedded,
            gc,
            freestanding: true,
        },
        optimize,
        source,
        &c_path,
    )?;
    notes.insert(0, "Embedded freestanding C generated".into());

    let src_dir = source.parent().unwrap_or(Path::new("."));
    let mut artifacts = vec![c_path.clone()];

    let board_link = src_dir.join("link.ld");
    if board_link.is_file() {
        let dest = dir.join("link.ld");
        fs::copy(&board_link, &dest)?;
        artifacts.push(dest);
        notes.push(format!("using board link.ld from {}", src_dir.display()));
    } else {
        let dest = dir.join("link.ld");
        fs::write(
            &dest,
            r#"/* Minimal linker script scaffold for MCU / bare metal */
MEMORY
{
  FLASH (rx) : ORIGIN = 0x08000000, LENGTH = 128K
  RAM (rwx)  : ORIGIN = 0x20000000, LENGTH = 20K
}
ENTRY(Main)
SECTIONS {
  .text : { *(.text*) *(.text.isr.*) } > FLASH
  .rodata : { *(.rodata*) } > FLASH
  .data : { *(.data*) } > RAM AT> FLASH
  .bss : { *(.bss*) *(COMMON) } > RAM
}
"#,
        )?;
        artifacts.push(dest);
    }

    for name in [
        "startup.c",
        "startup.S",
        "startup.s",
        "build.sh",
        "build.ps1",
        "openocd.cfg",
        "README.md",
    ] {
        let src = src_dir.join(name);
        if src.is_file() {
            let dest = dir.join(name);
            fs::copy(&src, &dest)?;
            artifacts.push(dest);
        }
    }

    if !dir.join("build.sh").is_file() {
        fs::write(
            dir.join("build.sh"),
            format!(
                r#"#!/bin/sh
# Cross-compile example (adjust toolchain / CPU)
arm-none-eabi-gcc -ffreestanding -nostdlib -O2 -T link.ld -o {stem}.elf {stem}.c
"#
            ),
        )?;
    }
    if !dir.join("README.md").is_file() {
        fs::write(
            dir.join("README.md"),
            "# RayTask Embedded target\n\n\
             Freestanding C output (`--no-gc` recommended). Use your MCU toolchain + `link.ld`.\n\
             Board kits: `examples/boards/` (STM32 Blue Pill, MPS2 AN385 / QEMU).\n\
             Attributes: `[address:]` MMIO, `[interrupt:]` ISR stubs, `[export:]` symbols.\n",
        )?;
    }

    Ok(TargetBuildResult {
        primary: c_path,
        artifacts,
        notes,
    })
}

/// `--target kernel`: freestanding kernel image C via **SSA → C**.
pub fn build_kernel(
    source: &Path,
    program: &Program,
    optimize: crate::Optimize,
) -> Result<TargetBuildResult, Box<dyn std::error::Error>> {
    let dir = dist_dir(source, "kernel");
    fs::create_dir_all(&dir)?;
    let stem = source
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("kernel");
    let c_path = dir.join(format!("{}.c", stem));
    let (_c, mut notes) = write_c_ssa(
        program,
        CodegenOptions {
            profile: RuntimeProfile::Kernel,
            gc: false,
            freestanding: true,
        },
        optimize,
        source,
        &c_path,
    )?;
    notes.insert(0, "Kernel freestanding C generated (SSA → C)".into());

    fs::write(
        dir.join("kernel.ld"),
        r#"ENTRY(kmain)
SECTIONS {
  . = 0x100000;
  .text : { *(.text.boot) *(.text*) }
  .rodata : { *(.rodata*) }
  .data : { *(.data*) }
  .bss : { *(.bss*) *(COMMON) }
}
"#,
    )?;
    fs::write(
        dir.join("README.md"),
        "# RayTask Kernel target\n\n\
         No GC, freestanding, **SSA → C** bodies. Prefer `[export: \"kmain\"]` as entry.\n\
         `[interrupt: N]` emits ISR section markers for your IDT/IVT setup.\n\
         Pass `--optimize speed|size` to run the SSA pass manager before C emit.\n",
    )?;

    Ok(TargetBuildResult {
        primary: c_path.clone(),
        artifacts: vec![c_path, dir.join("kernel.ld")],
        notes,
    })
}

fn base64_encode(data: &[u8]) -> String {
    const T: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity((data.len() + 2) / 3 * 4);
    for chunk in data.chunks(3) {
        let mut n = (chunk[0] as u32) << 16;
        if chunk.len() > 1 {
            n |= (chunk[1] as u32) << 8;
        }
        if chunk.len() > 2 {
            n |= chunk[2] as u32;
        }
        out.push(T[((n >> 18) & 63) as usize] as char);
        out.push(T[((n >> 12) & 63) as usize] as char);
        out.push(if chunk.len() > 1 {
            T[((n >> 6) & 63) as usize] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            T[(n & 63) as usize] as char
        } else {
            '='
        });
    }
    out
}

/// Result of true AOT host build (`native-bin` / SSA→C→TCC|gcc).
#[derive(Debug, Clone)]
pub struct AotBuildResult {
    pub exe: PathBuf,
    pub c_path: PathBuf,
    pub notes: Vec<String>,
}

/// True AOT for host/cross: SSA → C (Host runtime) → TCC/gcc/clang executable.
/// The output binary contains **no** RTBC payload and **no** VM interpreter.
pub fn build_aot_native(
    source: &Path,
    program: &Program,
    optimize: crate::Optimize,
    gc: bool,
    debug: bool,
    output: Option<&Path>,
    triple: crate::native_triple::NativeTriple,
    link_builtin: bool,
) -> Result<AotBuildResult, Box<dyn std::error::Error>> {
    let dir = dist_dir(source, "aot");
    fs::create_dir_all(&dir)?;
    let stem = source
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("app");
    let c_path = dir.join(format!("{}.c", stem));
    let src = source.to_string_lossy();
    let ssa = crate::ssa::build_ssa_for_c(program, optimize, true, Some(src.as_ref()))?;
    let c = CCodegen::with_options(CodegenOptions {
        profile: RuntimeProfile::Host,
        gc,
        freestanding: false,
    })
    .set_source_dir(
        &source.parent()
            .and_then(|p| {
                if p.as_os_str().is_empty() {
                    std::env::current_dir().ok()
                } else if p.is_absolute() {
                    Some(p.to_path_buf())
                } else {
                    std::env::current_dir().ok().map(|cwd| cwd.join(p))
                }
            })
            .unwrap_or_else(|| PathBuf::from("."))
            // Normalize: remove . components
            .components()
            .collect::<PathBuf>()
    )
    .generate_with_ssa(program, &ssa)?;
    fs::write(&c_path, &c)?;

    let default_exe = dir.join(match triple.os {
        crate::native_triple::OsKind::Windows | crate::native_triple::OsKind::Uefi => {
            format!("{}.{}", stem, triple.default_ext())
        }
        _ => {
            if cfg!(windows) && triple.matches_host() {
                format!("{}.exe", stem)
            } else {
                stem.to_string()
            }
        }
    });
    let exe = output
        .map(|p| p.to_path_buf())
        .unwrap_or(default_exe);

    let mut link_libs = crate::codegen_c::collect_link_libs(program);
    // Resolve relative link library paths (e.g. "raylib/lib/raylib.dll") to absolute
    // paths based on the source file's directory.
    // For .dll files, try the corresponding .lib (import library) first,
    // which TCC can use for linking on Windows.
    if let Some(parent) = source.parent() {
        let parent = if parent.is_absolute() {
            parent.to_path_buf()
        } else {
            std::env::current_dir().ok().map(|cwd| cwd.join(parent)).unwrap_or_else(|| parent.to_path_buf())
        };
        for i in 0..link_libs.len() {
            let lib = &link_libs[i];
            if lib.starts_with('<') || lib.starts_with('-') || Path::new(lib).is_absolute() {
                continue;
            }
            let abs = parent.join(&**lib);
            if abs.exists() {
                link_libs[i] = abs.display().to_string();
            } else if lib.ends_with(".dll") {
                // Try the .lib import library instead.
                let lib_path = Path::new(&**lib).with_extension("lib");
                let abs_lib = parent.join(&lib_path);
                if abs_lib.exists() {
                    link_libs[i] = abs_lib.display().to_string();
                } else {
                    // Try the raw path relative to CWD.
                    link_libs[i] = abs.display().to_string();
                }
            }
        }
    }
    let mut notes = vec![
        format!(
            "True AOT: SSA → C → native ({}) (no RTBC interpreter)",
            triple
        ),
    ];
    match optimize {
        crate::Optimize::None => notes.push("optimize=none (lift + phi-elim)".into()),
        crate::Optimize::Speed => notes.push("optimize=speed".into()),
        crate::Optimize::Size => notes.push("optimize=size".into()),
    }

    // Include the source file's directory so relative #include paths work.
    // Use absolute path for TCC to find headers from any C file location.
    let include_dirs: Vec<PathBuf> = source
        .parent()
        .and_then(|p| {
            if p.as_os_str().is_empty() {
                std::env::current_dir().ok()
            } else if p.is_absolute() {
                Some(p.to_path_buf())
            } else {
                std::env::current_dir().ok().map(|cwd| cwd.join(p))
            }
        })
        .into_iter()
        .collect();

    match compile_native_c(&c_path, &exe, debug, &link_libs, triple, link_builtin, &include_dirs) {
        Ok(how) => {
            notes.push(how);
            // Copy referenced DLLs next to the executable so Windows can find them.
            for lib in &link_libs {
                if lib.ends_with(".dll") && Path::new(lib).is_file() {
                    if let Some(dll_name) = Path::new(lib).file_name() {
                        let dest = exe.with_file_name(dll_name);
                        if std::fs::copy(lib, &dest).is_ok() {
                            notes.push(format!("copied {} → {}", lib, dest.display()));
                        }
                    }
                }
            }
        }
        Err(e) => {
            notes.push(format!("C toolchain failed ({e}); leaving {}", c_path.display()));
            fs::write(
                dir.join("README.md"),
                format!(
                    "# RayTask True AOT\n\n\
                     Target: `{triple}` (`{}`)\n\n\
                     Generated `{}` via SSA → C. Compile with a C toolchain:\n\n\
                     ```\nclang -target {} -O2 -o {} {}\n```\n\
                     Or host: `gcc -O2 -o {} {}`\n\n\
                     This path does **not** embed an RTBC interpreter.\n\
                     Use `--link-builtin` after `-c` for the built-in ELF/COFF linker.\n",
                    triple.clang_target(),
                    c_path.file_name().unwrap().to_string_lossy(),
                    triple.clang_target(),
                    exe.file_name().unwrap().to_string_lossy(),
                    c_path.file_name().unwrap().to_string_lossy(),
                    exe.file_name().unwrap().to_string_lossy(),
                    c_path.file_name().unwrap().to_string_lossy(),
                ),
            )?;
            return Ok(AotBuildResult {
                exe: c_path.clone(),
                c_path,
                notes,
            });
        }
    }

    fs::write(
        dir.join("README.md"),
        format!(
            "# RayTask True AOT\n\n\
             Host/cross executable built from **SSA → C → TCC/gcc/clang** for `{triple}`.\n\
             No RTBC bytecode and no VM stub are linked into the binary.\n\
             Use `--target app` if you want stub + embedded `.rtbc` instead.\n\
             Object files can be linked with `raytask link foo.o --platform … --arch …`.\n"
        ),
    )?;

    Ok(AotBuildResult {
        exe,
        c_path,
        notes,
    })
}

/// Compile Host C to an object file for `triple`.
pub fn compile_c_to_object(
    c_path: &Path,
    obj_path: &Path,
    debug: bool,
    triple: crate::native_triple::NativeTriple,
) -> Result<String, String> {
    if let Some(parent) = obj_path.parent() {
        fs::create_dir_all(parent).ok();
    }
    if triple.matches_host() {
        match crate::tcc::compile_c_to_path(
            c_path,
            obj_path,
            crate::tcc::OutputKind::Obj,
            debug,
            &[],
            &[],
        ) {
            Ok(()) => return Ok("object via embedded TCC".into()),
            Err(_) => {}
        }
    }
    for cc in ["clang", "gcc"] {
        let mut cmd = Command::new(cc);
        cmd.arg(c_path).arg("-c").arg("-o").arg(obj_path);
        if debug {
            cmd.arg("-g").arg("-O0");
        } else {
            cmd.arg("-O2");
        }
        if !triple.matches_host() {
            if cc != "clang" {
                continue; // gcc cross needs a prefixed toolchain; skip
            }
            cmd.arg("-target").arg(triple.clang_target());
        }
        if let Ok(st) = cmd.status() {
            if st.success() {
                return Ok(format!("object via {cc} ({})", triple.clang_target()));
            }
        }
    }
    // Retry clang without requiring target when host
    if triple.matches_host() {
        let mut cmd = Command::new("clang");
        cmd.arg(c_path).arg("-c").arg("-o").arg(obj_path);
        if debug {
            cmd.arg("-g").arg("-O0");
        } else {
            cmd.arg("-O2");
        }
        if let Ok(st) = cmd.status() {
            if st.success() {
                return Ok("object via clang".into());
            }
        }
        let mut cmd = Command::new("gcc");
        cmd.arg(c_path).arg("-c").arg("-o").arg(obj_path);
        if debug {
            cmd.arg("-g").arg("-O0");
        } else {
            cmd.arg("-O2");
        }
        if let Ok(st) = cmd.status() {
            if st.success() {
                return Ok("object via gcc".into());
            }
        }
    }
    Err(format!(
        "no C compiler for object ({})",
        triple.clang_target()
    ))
}

/// Compile + link Host C for a native triple.
pub fn compile_native_c(
    c_path: &Path,
    exe: &Path,
    debug: bool,
    link_libs: &[String],
    triple: crate::native_triple::NativeTriple,
    link_builtin: bool,
    include_dirs: &[PathBuf],
) -> Result<String, String> {
    // Built-in linker path: .o → PE/ELF (freestanding / explicit --link-builtin).
    if link_builtin
        || matches!(
            triple.os,
            crate::native_triple::OsKind::Freestanding | crate::native_triple::OsKind::Uefi
        )
    {
        let obj = c_path.with_extension(if cfg!(windows) { "obj" } else { "o" });
        match compile_c_to_object(c_path, &obj, debug, triple) {
            Ok(how) => {
                let entry = if triple.os == crate::native_triple::OsKind::Uefi {
                    "efi_main"
                } else if link_builtin {
                    "main"
                } else {
                    "_start"
                };
                let opts = crate::link::BuiltinLinkOptions {
                    triple,
                    entry: entry.into(),
                    base: None,
                    efi: triple.os == crate::native_triple::OsKind::Uefi,
                };
                match crate::link::link_paths(&[obj.clone()], exe, &opts) {
                    Ok(r) => {
                        return Ok(format!("{how}; built-in linker ({})", r.notes.join("; ")));
                    }
                    Err(e) if link_builtin => return Err(e.message),
                    Err(e) => {
                        // Freestanding without CRT entry — keep going to cross/host tools.
                        let _ = e;
                    }
                }
            }
            Err(e) if link_builtin => return Err(e),
            Err(_) => {}
        }
    }

    if triple.matches_host() {
        return compile_host_c(c_path, exe, debug, link_libs, include_dirs);
    }

    // Cross: clang -target <triple>
    if let Some(parent) = exe.parent() {
        fs::create_dir_all(parent).ok();
    }
    for cc in ["clang", "zig"] {
        let mut cmd = Command::new(cc);
        if cc == "zig" {
            cmd.arg("cc");
        }
        cmd.arg(c_path);
        if debug {
            cmd.arg("-g").arg("-O0");
        } else {
            cmd.arg("-O2");
        }
        cmd.arg("-target").arg(triple.clang_target());
        cmd.arg("-o").arg(exe);
        for lib in link_libs {
            if lib.ends_with(".c")
                || lib.ends_with(".o")
                || lib.ends_with(".obj")
                || lib.ends_with(".a")
                || lib.ends_with(".so")
                || lib.ends_with(".dylib")
                || lib.ends_with(".dll")
                || lib.ends_with(".lib")
            {
                cmd.arg(lib);
            } else if lib.starts_with("lib") {
                cmd.arg(format!("-l{}", lib.trim_start_matches("lib")));
            } else {
                cmd.arg(format!("-l{}", lib.trim_end_matches(".dll")));
            }
        }
        if let Ok(st) = cmd.status() {
            if st.success() {
                return Ok(format!(
                    "cross-linked with {cc} -target {}",
                    triple.clang_target()
                ));
            }
        }
    }

    // Last resort: emit .o for the user + built-in link attempt
    let obj = c_path.with_extension("o");
    let how = compile_c_to_object(c_path, &obj, debug, triple)?;
    let opts = crate::link::BuiltinLinkOptions {
        triple,
        entry: "main".into(),
        base: None,
        efi: false,
    };
    let r = crate::link::link_paths(&[obj], exe, &opts).map_err(|e| e.message)?;
    Ok(format!(
        "{how}; built-in linker fallback ({})",
        r.notes.join("; ")
    ))
}

/// Compile a generated Host C file to an executable (TCC, then gcc/clang/cl).
pub fn compile_host_c(
    c_path: &Path,
    exe: &Path,
    debug: bool,
    link_libs: &[String],
    include_dirs: &[PathBuf],
) -> Result<String, String> {
    if let Some(parent) = exe.parent() {
        fs::create_dir_all(parent).ok();
    }
    match crate::tcc::compile_c_to_path(
        c_path,
        exe,
        crate::tcc::OutputKind::Exe,
        debug,
        link_libs,
        include_dirs,
    ) {
        Ok(()) => return Ok("linked with embedded TCC".into()),
        Err(_err) => {
            // TCC failed — fall through to gcc/clang.
        }
    }

    // Fallback to gcc/clang/cl.

    let out_str = c_path.display().to_string();
    let exe_str = exe.display().to_string();
    for cc in ["gcc", "clang", "cl"] {
        let mut cmd = Command::new(cc);
        if cc == "cl" {
            cmd.arg(&out_str).arg(format!("/Fe:{}", exe_str));
            if debug {
                cmd.arg("/Zi").arg("/Od");
            }
            for lib in link_libs {
                if lib.ends_with(".dll") || lib.ends_with(".lib") {
                    cmd.arg(lib);
                } else {
                    cmd.arg(format!("{}.lib", lib));
                }
            }
        } else {
            cmd.arg(&out_str);
            if debug {
                cmd.arg("-g").arg("-O0");
            } else {
                cmd.arg("-O2");
            }
            cmd.arg("-o").arg(&exe_str);
            for lib in link_libs {
                if lib.ends_with(".c") {
                    cmd.arg(lib);
                } else if lib.ends_with(".so")
                    || lib.ends_with(".dylib")
                    || lib.ends_with(".a")
                    || lib.ends_with(".dll")
                {
                    cmd.arg(lib);
                } else if lib.starts_with("lib") {
                    cmd.arg(format!("-l{}", lib.trim_start_matches("lib")));
                } else {
                    cmd.arg(format!("-l{}", lib.trim_end_matches(".dll")));
                }
            }
        }
        if let Ok(st) = cmd.status() {
            if st.success() {
                return Ok(format!("linked with {cc}"));
            }
        }
    }

    if cfg!(windows) {
        let mut cmd = Command::new("wsl");
        cmd.arg("gcc").arg(out_str.replace('\\', "/"));
        if debug {
            cmd.arg("-g").arg("-O0");
        } else {
            cmd.arg("-O2");
        }
        let wsl_out = exe_str.replace('\\', "/").trim_end_matches(".exe").to_string();
        if let Ok(st) = cmd.arg("-o").arg(&wsl_out).status() {
            if st.success() {
                return Ok("linked with wsl gcc".into());
            }
        }
    }

    Err("no working C compiler (tcc/gcc/clang/cl)".into())
}

