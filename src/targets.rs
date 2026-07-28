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

/// `--target embedded`: freestanding C, no GC by default, bare-metal friendly.
pub fn build_embedded(
    source: &Path,
    program: &Program,
    gc: bool,
) -> Result<TargetBuildResult, Box<dyn std::error::Error>> {
    let dir = dist_dir(source, "embedded");
    fs::create_dir_all(&dir)?;
    let stem = source
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("app");
    let c_path = dir.join(format!("{}.c", stem));
    write_c(
        program,
        CodegenOptions {
            profile: RuntimeProfile::Embedded,
            gc,
            freestanding: true,
        },
        &c_path,
    )?;

    fs::write(
        dir.join("link.ld"),
        r#"/* Minimal linker script scaffold for MCU / bare metal */
ENTRY(Main)
SECTIONS {
  .text : { *(.text*) }
  .rodata : { *(.rodata*) }
  .data : { *(.data*) }
  .bss : { *(.bss*) }
}
"#,
    )?;
    fs::write(
        dir.join("build.sh"),
        format!(
            r#"#!/bin/sh
# Cross-compile example (adjust toolchain)
arm-none-eabi-gcc -ffreestanding -nostdlib -O2 -T link.ld -o {stem}.elf {stem}.c
"#
        ),
    )?;
    fs::write(
        dir.join("README.md"),
        "# RayTask Embedded target\n\n\
         Freestanding C output (`--no-gc` recommended). Use your MCU toolchain + `link.ld`.\n\
         Attributes: `[address:]` MMIO, `[interrupt:]` ISR stubs, `[export:]` symbols.\n",
    )?;

    Ok(TargetBuildResult {
        primary: c_path.clone(),
        artifacts: vec![c_path, dir.join("link.ld")],
        notes: vec!["Embedded freestanding C generated".into()],
    })
}

/// `--target kernel`: freestanding kernel image C (kmain / interrupt handlers).
pub fn build_kernel(
    source: &Path,
    program: &Program,
) -> Result<TargetBuildResult, Box<dyn std::error::Error>> {
    let dir = dist_dir(source, "kernel");
    fs::create_dir_all(&dir)?;
    let stem = source
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("kernel");
    let c_path = dir.join(format!("{}.c", stem));
    write_c(
        program,
        CodegenOptions {
            profile: RuntimeProfile::Kernel,
            gc: false,
            freestanding: true,
        },
        &c_path,
    )?;

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
         No GC, freestanding. Prefer `[export: \"kmain\"]` as entry.\n\
         `[interrupt: N]` emits ISR section markers for your IDT/IVT setup.\n",
    )?;

    Ok(TargetBuildResult {
        primary: c_path.clone(),
        artifacts: vec![c_path, dir.join("kernel.ld")],
        notes: vec!["Kernel freestanding C generated (GC disabled)".into()],
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
