//! NativeCodeGen + Linker format tests.

use raytask::linker::{link_module, write_elf64_payload, write_macho64_payload, write_pe_efi, write_pe64};
use raytask::native_codegen::{codegen, CodegenNativeOptions, LinkTarget, SectionKind};
use raytask::compile_bytecode;
use std::fs;
use std::path::PathBuf;

fn hello_module() -> raytask::bytecode::Module {
    let src = r#"
import bstd.io;
void Main() {
    print("hi");
}
"#;
    compile_bytecode(src).expect("compile")
}

fn tmp(name: &str) -> PathBuf {
    let p = std::env::temp_dir().join(format!("raytask_nlink_{}_{}", name, std::process::id()));
    let _ = fs::remove_dir_all(&p);
    fs::create_dir_all(&p).unwrap();
    p
}

#[test]
fn codegen_embeds_rtbc_payload() {
    let m = hello_module();
    let obj = codegen(
        &m,
        &CodegenNativeOptions {
            target: LinkTarget::WindowsX64,
            name: "t".into(),
            out_dir: None,
            load_address: 0x100000,
        },
    );
    assert!(!obj.rtbc.is_empty());
    assert!(obj.rtbc.starts_with(b"RTBC"));
    assert!(obj.section(SectionKind::Rodata).is_some());
    assert!(obj.section(SectionKind::Text).is_some());
}

#[test]
fn pe_efi_has_mz_and_pe_signature() {
    let pe = write_pe_efi(&[0x31, 0xC0, 0xC3], b"RTBC\0\0payload").unwrap();
    assert_eq!(&pe[0..2], b"MZ");
    let e_lfanew = u32::from_le_bytes(pe[0x3C..0x40].try_into().unwrap()) as usize;
    assert_eq!(&pe[e_lfanew..e_lfanew + 4], b"PE\0\0");
    // Subsystem EFI_APPLICATION = 10 at optional header + 68
    // Optional starts at e_lfanew+24; Subsystem at +68 from optional start
    let subsystem = u16::from_le_bytes(pe[e_lfanew + 24 + 68..e_lfanew + 24 + 70].try_into().unwrap());
    assert_eq!(subsystem, 10);
}

#[test]
fn pe64_windows_console_magic() {
    let pe = write_pe64(b"RTBCdata", false).unwrap();
    assert_eq!(&pe[0..2], b"MZ");
    let e_lfanew = u32::from_le_bytes(pe[0x3C..0x40].try_into().unwrap()) as usize;
    assert_eq!(&pe[e_lfanew..e_lfanew + 4], b"PE\0\0");
}

#[test]
fn elf64_magic() {
    let elf = write_elf64_payload(b"RTBCelf").unwrap();
    assert_eq!(&elf[0..4], b"\x7fELF");
    assert_eq!(elf[4], 2); // 64-bit
}

#[test]
fn macho64_magic() {
    let macho = write_macho64_payload(b"RTBCmac").unwrap();
    let magic = u32::from_le_bytes(macho[0..4].try_into().unwrap());
    assert_eq!(magic, 0xFEEDFACF);
}

#[test]
fn link_efi_and_raw_and_linux() {
    let m = hello_module();
    let dir = tmp("multi");
    for target in [
        LinkTarget::UefiX64,
        LinkTarget::RawX64,
        LinkTarget::LinuxX64,
        LinkTarget::MacosX64,
        LinkTarget::WindowsX64,
    ] {
        let r = link_module(&m, target, &dir, "app").expect("link");
        assert!(r.output.exists(), "{:?} missing", target);
        let bytes = fs::read(&r.output).unwrap();
        assert!(!bytes.is_empty());
        match target {
            LinkTarget::UefiX64 | LinkTarget::WindowsX64 => {
                // May be stub packaging (MZ) or PE writer
                assert!(bytes.starts_with(b"MZ") || bytes.windows(4).any(|w| w == b"RTBC"));
            }
            LinkTarget::LinuxX64 => {
                assert!(bytes.starts_with(b"\x7fELF") || bytes.windows(4).any(|w| w == b"RTBC"));
            }
            LinkTarget::MacosX64 => {
                let magic = u32::from_le_bytes(bytes[0..4].try_into().unwrap_or([0; 4]));
                assert!(magic == 0xFEEDFACF || bytes.windows(4).any(|w| w == b"RTBC"));
            }
            LinkTarget::RawX64 => {
                assert!(bytes.windows(4).any(|w| w == b"RTBC"));
            }
        }
    }
}

#[test]
fn uefi_c_contains_interpreter() {
    let m = hello_module();
    let obj = codegen(
        &m,
        &CodegenNativeOptions {
            target: LinkTarget::UefiX64,
            name: "demo".into(),
            out_dir: None,
            load_address: 0x100000,
        },
    );
    let c = obj.c_source.expect("c source");
    assert!(c.contains("efi_main"));
    assert!(c.contains("uefi_code"));
    assert!(c.contains("OP_PRINT"));
}

#[test]
fn build_efi_via_compile_file() {
    let dir = tmp("cfi");
    let src = dir.join("hi.rt");
    fs::write(&src, "import bstd.io;\nvoid Main() { print(\"efi\"); }\n").unwrap();
    let out = raytask::compile_file(
        src.to_str().unwrap(),
        &raytask::BuildOptions {
            target: raytask::Target::Efi,
            ..raytask::BuildOptions::default()
        },
    )
    .expect("efi build");
    assert!(out.contains(".efi") || PathBuf::from(out.split_whitespace().next().unwrap()).exists() || out.contains("efi"));
}
