use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn main() {
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let tcc_dir = manifest_dir.join("tcc");
    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());

    println!("cargo:rerun-if-changed={}", tcc_dir.display());
    println!("cargo:rerun-if-changed=build.rs");

    if !tcc_dir.exists() {
        panic!("vendored tcc directory not found at {}", tcc_dir.display());
    }

    let version = fs::read_to_string(tcc_dir.join("VERSION"))
        .unwrap_or_else(|_| "unknown".into())
        .trim()
        .to_string();

    let runtime_dir = out_dir.join("tcc-runtime");
    stage_runtime(&tcc_dir, &runtime_dir);

    let config_h = out_dir.join("config.h");
    fs::write(&config_h, make_config_h(&version, &runtime_dir)).expect("write tcc config.h");

    let target_os = env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    let target_arch = env::var("CARGO_CFG_TARGET_ARCH").unwrap_or_default();
    let defs = target_defines(&target_os, &target_arch);

    // Static libtcc linked into RayTask.
    let mut build = cc::Build::new();
    build
        .file(tcc_dir.join("libtcc.c"))
        .include(&tcc_dir)
        .include(&out_dir)
        .warnings(false)
        .define("ONE_SOURCE", "1")
        .define("TCC_IS_NATIVE", "1")
        .define("CONFIG_TCC_STATIC", "1");
    for (k, v) in &defs {
        build.define(k, v.as_deref());
    }
    build.compile("raytask_tcc");

    // Host tcc.exe used to bootstrap libtcc1 / CRT objects into the runtime dir.
    if let Err(err) = bootstrap_runtime_with_host_tcc(&tcc_dir, &out_dir, &runtime_dir, &defs) {
        println!("cargo:warning=tcc runtime bootstrap failed: {err}");
        println!(
            "cargo:warning=embedded TCC may still work for -run / memory mode; EXE linking can need libtcc1"
        );
    }

    println!(
        "cargo:rustc-env=RAYTASK_VENDORED_TCC_ROOT={}",
        tcc_dir.display()
    );
    println!(
        "cargo:rustc-env=RAYTASK_TCC_RUNTIME={}",
        runtime_dir.display()
    );

    if target_os == "linux" || target_os == "android" {
        println!("cargo:rustc-link-lib=dl");
        println!("cargo:rustc-link-lib=pthread");
        println!("cargo:rustc-link-lib=m");
    } else if target_os == "macos" {
        println!("cargo:rustc-link-lib=c");
    } else if target_os == "windows" {
        println!("cargo:rustc-link-lib=kernel32");
        println!("cargo:rustc-link-lib=user32");
        println!("cargo:rustc-link-lib=ws2_32");
    }
}

fn target_defines(target_os: &str, target_arch: &str) -> Vec<(String, Option<String>)> {
    let mut defs = Vec::new();
    if target_os == "windows" {
        defs.push(("CONFIG_WIN32".into(), Some("1".into())));
        defs.push(("TCC_TARGET_PE".into(), Some("1".into())));
    }
    if target_os == "macos" {
        defs.push(("CONFIG_OSX".into(), Some("1".into())));
        defs.push(("TCC_TARGET_MACHO".into(), Some("1".into())));
    }
    match target_arch {
        "x86_64" => defs.push(("TCC_TARGET_X86_64".into(), Some("1".into()))),
        "x86" | "i386" | "i686" => defs.push(("TCC_TARGET_I386".into(), Some("1".into()))),
        "aarch64" => defs.push(("TCC_TARGET_ARM64".into(), Some("1".into()))),
        "arm" => defs.push(("TCC_TARGET_ARM".into(), Some("1".into()))),
        "riscv64" => defs.push(("TCC_TARGET_RISCV64".into(), Some("1".into()))),
        other => panic!("unsupported target arch for vendored tcc: {other}"),
    }
    defs
}

fn make_config_h(version: &str, runtime_dir: &Path) -> String {
    let runtime = runtime_dir
        .display()
        .to_string()
        .replace('\\', "/")
        .replace('"', "\\\"");
    format!("#define TCC_VERSION \"{version}\"\n#define CONFIG_TCCDIR \"{runtime}\"\n")
}

fn stage_runtime(tcc_dir: &Path, runtime_dir: &Path) {
    let include_dir = runtime_dir.join("include");
    let lib_dir = runtime_dir.join("lib");
    fs::create_dir_all(&include_dir).expect("create tcc runtime include");
    fs::create_dir_all(&lib_dir).expect("create tcc runtime lib");

    copy_dir_files(&tcc_dir.join("include"), &include_dir);
    if cfg!(windows) {
        copy_dir_files(&tcc_dir.join("win32").join("include"), &include_dir);
        let win_lib = tcc_dir.join("win32").join("lib");
        for name in ["msvcrt.def", "kernel32.def", "user32.def", "gdi32.def", "ws2_32.def"] {
            let src = win_lib.join(name);
            if src.exists() {
                let _ = fs::copy(&src, lib_dir.join(name));
            }
        }
        // Keep a nested include/winapi tree if present.
        let winapi_src = tcc_dir.join("win32").join("include").join("winapi");
        if winapi_src.exists() {
            let winapi_dst = include_dir.join("winapi");
            fs::create_dir_all(&winapi_dst).ok();
            copy_dir_files(&winapi_src, &winapi_dst);
        }
        for nest in ["sys", "sec_api", "tcc"] {
            let src = tcc_dir.join("win32").join("include").join(nest);
            if src.exists() {
                let dst = include_dir.join(nest);
                fs::create_dir_all(&dst).ok();
                copy_dir_files(&src, &dst);
            }
        }
    }
    let _ = fs::copy(tcc_dir.join("tcclib.h"), include_dir.join("tcclib.h"));
    let _ = fs::copy(tcc_dir.join("libtcc.h"), runtime_dir.join("libtcc.h"));
}

fn copy_dir_files(src: &Path, dst: &Path) {
    let Ok(entries) = fs::read_dir(src) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_file() {
            if let Some(name) = path.file_name() {
                let _ = fs::copy(&path, dst.join(name));
            }
        }
    }
}

fn bootstrap_runtime_with_host_tcc(
    tcc_dir: &Path,
    out_dir: &Path,
    runtime_dir: &Path,
    defs: &[(String, Option<String>)],
) -> Result<(), String> {
    let host = out_dir.join(if cfg!(windows) {
        "tcc_host.exe"
    } else {
        "tcc_host"
    });
    let marker = runtime_dir.join("lib").join(if cfg!(windows) {
        "libtcc1.a"
    } else {
        "libtcc1.a"
    });
    if marker.exists() && host.exists() {
        return Ok(());
    }

    let build = cc::Build::new();
    let compiler = build.get_compiler();
    let mut cmd = compiler.to_command();
    cmd.arg(tcc_dir.join("tcc.c"));
    cmd.arg(format!("-I{}", tcc_dir.display()));
    cmd.arg(format!("-I{}", out_dir.display()));
    cmd.arg("-DONE_SOURCE=1");
    cmd.arg("-DTCC_IS_NATIVE=1");
    for (k, v) in defs {
        if let Some(val) = v {
            cmd.arg(format!("-D{k}={val}"));
        } else {
            cmd.arg(format!("-D{k}"));
        }
    }
    if compiler.is_like_msvc() {
        cmd.arg(format!("/Fe:{}", host.display()));
        cmd.arg("/O2");
        cmd.arg("/W0");
        cmd.arg("/nologo");
    } else {
        cmd.arg("-o").arg(&host);
        cmd.arg("-O2");
        cmd.arg("-w");
    }

    let status = cmd.status().map_err(|e| format!("spawn host tcc compiler: {e}"))?;
    if !status.success() {
        return Err(format!("host tcc compile failed with {status}"));
    }

    let lib_dir = runtime_dir.join("lib");
    fs::create_dir_all(&lib_dir).map_err(|e| e.to_string())?;

    // Compile runtime objects with the freshly built host tcc.
    let mut objects: Vec<PathBuf> = Vec::new();
    // Keep runmain.c / bt-*.c as separate support objects (not inside libtcc1.a),
    // matching TinyCC's Windows packaging. Putting runmain into the archive pulls in
    // exit()/__rt_exit and breaks normal PE executables.
    let mut sources: Vec<PathBuf> = vec![
        tcc_dir.join("lib").join("libtcc1.c"),
        tcc_dir.join("lib").join("builtin.c"),
        tcc_dir.join("lib").join("stdatomic.c"),
    ];
    if cfg!(windows) {
        let win_lib = tcc_dir.join("win32").join("lib");
        sources.extend([
            win_lib.join("crt1.c"),
            win_lib.join("crt1w.c"),
            win_lib.join("wincrt1.c"),
            win_lib.join("wincrt1w.c"),
            win_lib.join("dllcrt1.c"),
            win_lib.join("dllmain.c"),
            win_lib.join("winex.c"),
            win_lib.join("chkstk.S"),
            tcc_dir.join("lib").join("alloca.S"),
            tcc_dir.join("lib").join("alloca-bt.S"),
            tcc_dir.join("lib").join("atomic.S"),
        ]);
    } else {
        sources.extend([
            tcc_dir.join("lib").join("alloca.S"),
            tcc_dir.join("lib").join("alloca-bt.S"),
            tcc_dir.join("lib").join("atomic.S"),
            tcc_dir.join("lib").join("dsohandle.c"),
            tcc_dir.join("lib").join("va_list.c"),
        ]);
    }

    for src in sources {
        if !src.exists() {
            continue;
        }
        let stem = src
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("obj")
            .to_string();
        let obj = lib_dir.join(format!("{stem}.o"));
        let status = Command::new(&host)
            .arg(format!("-B{}", runtime_dir.display()))
            .arg("-c")
            .arg(&src)
            .arg("-o")
            .arg(&obj)
            .status()
            .map_err(|e| format!("run host tcc for {}: {e}", src.display()))?;
        if status.success() && obj.exists() {
            objects.push(obj);
        }
    }

    if objects.is_empty() {
        return Err("host tcc produced no runtime objects".into());
    }

    let mut ar = Command::new(&host);
    ar.arg("-ar").arg(&marker);
    for obj in &objects {
        ar.arg(obj);
    }
    let status = ar
        .status()
        .map_err(|e| format!("host tcc -ar failed: {e}"))?;
    if !status.success() || !marker.exists() {
        return Err(format!("failed to create {}", marker.display()));
    }

    // Extra helper objects expected next to libtcc1 on Windows builds.
    for extra in ["bt-exe.c", "bt-log.c", "bt-dll.c", "bcheck.c", "runmain.c"] {
        let src = tcc_dir.join("lib").join(extra);
        if !src.exists() {
            continue;
        }
        let stem = src.file_stem().unwrap().to_string_lossy().into_owned();
        let obj = lib_dir.join(format!("{stem}.o"));
        let mut cmd = Command::new(&host);
        cmd.arg(format!("-B{}", runtime_dir.display()))
            .arg("-c")
            .arg(&src)
            .arg("-o")
            .arg(&obj)
            .arg("-I")
            .arg(tcc_dir);
        let _ = cmd.status();
    }

    Ok(())
}
