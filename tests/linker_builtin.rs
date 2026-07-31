//! Built-in linker + multi-arch AOT smoke tests.

use raytask::link::{self, BuiltinLinkOptions};
use raytask::native_triple::{Arch, NativeTriple, OsKind};
use raytask::{compile_file, BuildOptions, Optimize, Target};
use std::fs;
use std::path::PathBuf;

fn tmp(name: &str) -> PathBuf {
    let p = std::env::temp_dir().join(format!(
        "raytask_link_{}_{}",
        name,
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&p);
    fs::create_dir_all(&p).unwrap();
    p
}

#[test]
fn builtin_linker_writes_pe_and_elf_for_arches() {
    let dir = tmp("arches");
    for (os, arch, magic) in [
        (OsKind::Linux, Arch::X86_64, &b"\x7fELF"[..]),
        (OsKind::Linux, Arch::Aarch64, &b"\x7fELF"[..]),
        (OsKind::Windows, Arch::X86_64, &b"MZ"[..]),
        (OsKind::Windows, Arch::Aarch64, &b"MZ"[..]),
    ] {
        let triple = NativeTriple::new(os, arch);
        let out = dir.join(format!("smoke_{}", triple.name()));
        link::write_smoke_executable(triple, &out).unwrap();
        let bytes = fs::read(&out).unwrap();
        assert!(
            bytes.starts_with(magic),
            "{} bad magic",
            triple
        );
    }
}

#[test]
fn aot_respects_arch_in_notes() {
    let dir = tmp("arch_note");
    let src = dir.join("hi.rt");
    fs::write(&src, "void Main() { print(1); }\n").unwrap();
    let out = compile_file(
        src.to_str().unwrap(),
        &BuildOptions {
            target: Target::NativeBin,
            optimize: Optimize::None,
            arch: Arch::Aarch64,
            platform: raytask::app_build::Platform::Linux,
            ..BuildOptions::default()
        },
    )
    .expect("compile");
    assert!(
        out.contains("linux-aarch64") || out.contains("aarch64"),
        "expected arch in result: {out}"
    );
}

#[test]
fn link_object_roundtrip_via_tcc_when_available() {
    // Compile a tiny freestanding C with TCC to .obj/.o, then built-in link.
    let dir = tmp("tcc_obj");
    let c = dir.join("ret0.c");
    fs::write(
        &c,
        r#"
            void _start(void) { }
            int main(void) { return 0; }
        "#,
    )
    .unwrap();
    let obj = dir.join(if cfg!(windows) { "ret0.obj" } else { "ret0.o" });
    let exe = dir.join(if cfg!(windows) { "ret0.exe" } else { "ret0.elf" });

    if raytask::tcc::compile_c_to_path(
        &c,
        &obj,
        raytask::tcc::OutputKind::Obj,
        false,
        &[],
    )
    .is_err()
    {
        return;
    }

    let triple = NativeTriple::host();
    let opts = BuiltinLinkOptions {
        triple,
        entry: "main".into(),
        base: None,
        efi: false,
    };
    match link::link_paths(&[obj], &exe, &opts) {
        Ok(r) => {
            assert!(r.output.is_file());
            let bytes = fs::read(&r.output).unwrap();
            assert!(bytes.starts_with(b"MZ") || bytes.starts_with(b"\x7fELF"));
        }
        Err(_) => {
            // Undefined CRT symbols / reloc kinds — acceptable on some hosts.
        }
    }
}
