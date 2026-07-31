//! SSA → C for embedded / kernel targets.

use raytask::codegen_c::{CodegenOptions, RuntimeProfile};
use raytask::{compile_file, BuildOptions, Optimize, Target};
use std::fs;
use std::path::PathBuf;

fn emb(optimize: Optimize) -> BuildOptions {
    BuildOptions {
        target: Target::Embedded,
        optimize,
        gc: false,
        ..BuildOptions::default()
    }
}

#[test]
fn ssa_c_emits_block_gotos_and_mmio() {
    let out = compile_file(
        "examples/boards/stm32f103_bluepill/blink.rt",
        &emb(Optimize::Speed),
    )
    .expect("embedded SSA→C");
    let c = fs::read_to_string(&out).unwrap();
    assert!(
        c.contains("SSA") && c.contains("function bodies"),
        "expected SSA→C banner"
    );
    assert!(c.contains("goto bb"), "expected CFG as gotos");
    assert!(c.contains("MmioWrite32") || c.contains("MmioRead32"));
    assert!(c.contains("void LedInit") || c.contains("LedInit("));
    assert!(c.contains("void Main("));
    assert!(c.contains("Spin("));
}

#[test]
fn ssa_c_const_fold_visible_under_speed() {
    // 1+2 should fold in SSA; freestanding print becomes print_i64 of a constant.
    let src = r#"
        void Main() {
            print(1 + 2);
        }
    "#;
    let dir = std::env::temp_dir().join("raytask_ssa_c_fold");
    let _ = fs::create_dir_all(&dir);
    let path = dir.join("fold.rt");
    fs::write(&path, src).unwrap();
    let out = compile_file(path.to_str().unwrap(), &emb(Optimize::Speed)).unwrap();
    let c = fs::read_to_string(&out).unwrap();
    assert!(c.contains("goto bb") || c.contains("bb0:"));
    // Folded const 3 should appear in the C (as immediate).
    assert!(
        c.contains("print_i64(3)") || c.contains("= 3;") || c.contains("3; /*"),
        "expected folded constant in C:\n{}",
        &c[c.find("void Main").unwrap_or(0)..]
    );
}

#[test]
fn ssa_c_kernel_profile() {
    let src = r#"
        [export: "kmain"]
        void Main() {
            int x = 1 + 1;
        }
    "#;
    let dir = std::env::temp_dir().join("raytask_ssa_c_kern");
    let _ = fs::create_dir_all(&dir);
    let path = dir.join("k.rt");
    fs::write(&path, src).unwrap();
    let out = compile_file(
        path.to_str().unwrap(),
        &BuildOptions {
            target: Target::Kernel,
            optimize: Optimize::Size,
            gc: false,
            ..BuildOptions::default()
        },
    )
    .unwrap();
    let c = fs::read_to_string(&out).unwrap();
    assert!(c.contains("SSA") || c.contains("goto bb") || c.contains("bb0:"));
    let dir = PathBuf::from(&out).parent().unwrap().to_path_buf();
    assert!(dir.join("kernel.ld").is_file());
}

#[test]
fn boards_still_copy_link_ld_under_ssa_c() {
    let out = compile_file(
        "examples/boards/mps2_an385/uart_hello.rt",
        &emb(Optimize::None),
    )
    .unwrap();
    let dir = PathBuf::from(&out).parent().unwrap().to_path_buf();
    let link = fs::read_to_string(dir.join("link.ld")).unwrap();
    assert!(link.contains("ORIGIN = 0x00000000"));
    let c = fs::read_to_string(&out).unwrap();
    assert!(c.contains("UartInit") || c.contains("UartWriteHello"));
}

#[test]
fn ast_transpile_still_available() {
    // Host transpile_c remains AST path for non-embedded tooling.
    let c = raytask::transpile_c_with(
        "void Main() { int x = 1; }",
        CodegenOptions {
            profile: RuntimeProfile::Host,
            gc: true,
            freestanding: false,
        },
    )
    .unwrap();
    assert!(c.contains("void Main"));
    assert!(!c.contains("goto bb"));
}
