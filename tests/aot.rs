//! True AOT: `--target native-bin` / `native` without RTBC interpreter.

use raytask::{compile_file, BuildOptions, Optimize, Target};
use std::fs;
use std::path::PathBuf;
use std::process::Command;

fn tmp(name: &str) -> PathBuf {
    let p = std::env::temp_dir().join(format!("raytask_aot_{}_{}", name, std::process::id()));
    let _ = fs::remove_dir_all(&p);
    fs::create_dir_all(&p).unwrap();
    p
}

#[test]
fn native_bin_aot_emits_ssa_c_without_rtbc_banner() {
    let dir = tmp("bin");
    let src = dir.join("hi.rt");
    fs::write(
        &src,
        r#"
            void Main() {
                print(1 + 2);
            }
        "#,
    )
    .unwrap();

    let out = compile_file(
        src.to_str().unwrap(),
        &BuildOptions {
            target: Target::NativeBin,
            optimize: Optimize::Speed,
            ..BuildOptions::default()
        },
    )
    .expect("native-bin AOT");

    // Message mentions AOT
    assert!(
        out.contains("AOT") || out.contains("aot"),
        "expected AOT in result: {out}"
    );

    // Generated C lives under dist/*_aot/
    let c_candidates: Vec<_> = fs::read_dir(dir.join("dist"))
        .unwrap()
        .filter_map(|e| e.ok())
        .flat_map(|e| fs::read_dir(e.path()).into_iter().flatten().filter_map(|x| x.ok()))
        .filter(|e| e.path().extension().and_then(|x| x.to_str()) == Some("c"))
        .collect();
    assert!(
        !c_candidates.is_empty(),
        "expected generated .c under dist"
    );
    let c = fs::read_to_string(c_candidates[0].path()).unwrap();
    assert!(
        c.contains("True AOT") || c.contains("no RTBC interpreter"),
        "missing AOT banner"
    );
    assert!(c.contains("goto bb") || c.contains("bb0:"), "expected SSA CFG");
    assert!(!c.contains("RTBCAP"), "must not embed app magic in C source");
}

#[test]
fn native_aot_binary_has_no_rtbc_magic_when_linked() {
    let dir = tmp("exe");
    let src = dir.join("add.rt");
    fs::write(
        &src,
        r#"
            void Main() {
                print(40 + 2);
            }
        "#,
    )
    .unwrap();
    let exe_out = dir.join(if cfg!(windows) { "add.exe" } else { "add" });

    let result = compile_file(
        src.to_str().unwrap(),
        &BuildOptions {
            target: Target::Native,
            optimize: Optimize::Speed,
            output: Some(exe_out.clone()),
            ..BuildOptions::default()
        },
    );
    let Ok(out) = result else {
        // No C toolchain — skip binary check
        return;
    };

    let path = PathBuf::from(out.split_whitespace().next().unwrap_or(&out));
    if !path.is_file() {
        return;
    }
    // If we only got .c back, toolchain missing
    if path.extension().and_then(|e| e.to_str()) == Some("c") {
        return;
    }

    let bytes = fs::read(&path).unwrap();
    // Packaged apps end with APP_MAGIC / contain RTBC trailer — AOT must not.
    assert!(
        !bytes.windows(6).any(|w| w == b"RTBCAP"),
        "AOT exe must not contain RTBCAP stub trailer"
    );
    // Optional smoke: run the binary
    let _ = Command::new(&path).status();
}

#[test]
fn app_target_still_packages_rtbc() {
    // Contrasting path: --target app keeps stub + bytecode.
    let dir = tmp("app");
    let src = dir.join("hi.rt");
    fs::write(&src, "void Main() { print(1); }\n").unwrap();
    let out = compile_file(
        src.to_str().unwrap(),
        &BuildOptions {
            target: Target::App,
            ..BuildOptions::default()
        },
    );
    // May fail without stub binary — that's OK; just ensure we don't claim AOT.
    if let Ok(msg) = out {
        assert!(
            !msg.contains("native-bin AOT"),
            "app must not be reported as AOT: {msg}"
        );
    }
}
