//! Linker: `ObjectFile` → PE / ELF / Mach-O / EFI / raw binaries.

use crate::app_build::{self, Platform};
use crate::bytecode::Module;
use crate::bytecode_format::{deserialize_module, package_app};
use crate::native_codegen::{
    codegen, codegen_to_dir, CodegenNativeOptions, LinkTarget, ObjectFile, SectionKind,
};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Debug)]
pub struct LinkError {
    pub message: String,
}

impl std::fmt::Display for LinkError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for LinkError {}

impl From<String> for LinkError {
    fn from(message: String) -> Self {
        Self { message }
    }
}

impl From<&str> for LinkError {
    fn from(message: &str) -> Self {
        Self {
            message: message.into(),
        }
    }
}

pub struct LinkResult {
    pub output: PathBuf,
    pub object_dir: PathBuf,
    pub notes: Vec<String>,
}

/// Link an object file to a native binary for `target`.
pub fn link(obj: &ObjectFile, target: LinkTarget, out: &Path) -> Result<PathBuf, LinkError> {
    if let Some(parent) = out.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }

    match target {
        LinkTarget::WindowsX64 => link_windows(obj, out),
        LinkTarget::LinuxX64 => link_linux(obj, out),
        LinkTarget::MacosX64 => link_macos(obj, out),
        LinkTarget::UefiX64 => link_uefi(obj, out),
        LinkTarget::RawX64 => link_raw(obj, out),
    }
}

/// Codegen + link a module into `out_dir/<name>.<ext>`.
pub fn link_module(
    module: &Module,
    target: LinkTarget,
    out_dir: &Path,
    name: &str,
) -> Result<LinkResult, LinkError> {
    fs::create_dir_all(out_dir).map_err(|e| e.to_string())?;
    let obj = codegen_to_dir(module, target, out_dir, name);
    let ext = target.default_ext();
    let out = out_dir.join(format!("{}.{}", name, ext));
    let mut notes = obj.notes.clone();
    let path = link(&obj, target, &out)?;
    notes.push(format!("linked {}", path.display()));
    Ok(LinkResult {
        output: path,
        object_dir: out_dir.join(format!("{}_native", name)),
        notes,
    })
}

/// Link from raw `.rtbc` bytes.
pub fn link_rtbc(
    rtbc: &[u8],
    target: LinkTarget,
    out: &Path,
    name: &str,
) -> Result<LinkResult, LinkError> {
    let module = deserialize_module(rtbc).map_err(|e| e.to_string())?;
    let out_dir = out
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .to_path_buf();
    let stem = out
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or(name);
    link_module(&module, target, &out_dir, stem)
}

fn link_windows(obj: &ObjectFile, out: &Path) -> Result<PathBuf, LinkError> {
    // Prefer packaging runtime stub + RTBC (runnable on host when stub builds).
    if let Ok(path) = package_with_stub(obj, Platform::Windows, out) {
        return Ok(path);
    }
    // Pure-Rust PE64 with embedded payload (not a full Win32 CRT app).
    let pe = write_pe64(&obj.rtbc, false)?;
    fs::write(out, pe).map_err(|e| e.to_string())?;
    Ok(out.to_path_buf())
}

fn link_linux(obj: &ObjectFile, out: &Path) -> Result<PathBuf, LinkError> {
    if let Ok(path) = package_with_stub(obj, Platform::Linux, out) {
        return Ok(path);
    }
    let elf = write_elf64_payload(&obj.rtbc)?;
    fs::write(out, elf).map_err(|e| e.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(out).map_err(|e| e.to_string())?.permissions();
        perms.set_mode(0o755);
        fs::set_permissions(out, perms).map_err(|e| e.to_string())?;
    }
    Ok(out.to_path_buf())
}

fn link_macos(obj: &ObjectFile, out: &Path) -> Result<PathBuf, LinkError> {
    if let Ok(path) = package_with_stub(obj, Platform::Macos, out) {
        return Ok(path);
    }
    let macho = write_macho64_payload(&obj.rtbc)?;
    fs::write(out, macho).map_err(|e| e.to_string())?;
    Ok(out.to_path_buf())
}

fn link_uefi(obj: &ObjectFile, out: &Path) -> Result<PathBuf, LinkError> {
    // Try clang freestanding build of generated C
    if let Some(c) = &obj.c_source {
        let dir = out.parent().unwrap_or_else(|| Path::new("."));
        let c_path = dir.join("uefi_build.c");
        fs::write(&c_path, c).map_err(|e| e.to_string())?;
        if try_clang_uefi(&c_path, out) {
            return Ok(out.to_path_buf());
        }
    }
    // Pure-Rust PE32+ EFI image (stub entry + RTBC in .rdata)
    let text = obj
        .section(SectionKind::Text)
        .map(|s| s.data.clone())
        .unwrap_or_else(|| vec![0x31, 0xC0, 0xC3]);
    let pe = write_pe_efi(&text, &obj.rtbc)?;
    fs::write(out, pe).map_err(|e| e.to_string())?;
    Ok(out.to_path_buf())
}

fn link_raw(obj: &ObjectFile, out: &Path) -> Result<PathBuf, LinkError> {
    let mut raw = Vec::new();
    let text = obj
        .section(SectionKind::Text)
        .map(|s| s.data.as_slice())
        .unwrap_or(&[]);
    let rodata = obj.rodata_payload();
    // Layout: [text][pad to 0x200][rodata]
    raw.extend_from_slice(text);
    while raw.len() % 16 != 0 {
        raw.push(0x90); // nop pad
    }
    let pad_to = 0x200usize;
    if raw.len() < pad_to {
        raw.resize(pad_to, 0);
    }
    raw.extend_from_slice(rodata);
    fs::write(out, raw).map_err(|e| e.to_string())?;
    Ok(out.to_path_buf())
}

fn package_with_stub(
    obj: &ObjectFile,
    platform: Platform,
    out: &Path,
) -> Result<PathBuf, LinkError> {
    // Use app_build's stub discovery via a temp module rebuild path:
    // Write rtbc and call into app packaging logic by reconstructing Module.
    let module = deserialize_module(&obj.rtbc).map_err(|e| e.to_string())?;
    let tmp_src = out.with_extension("rt_link_tmp");
    // build_app needs a source path for naming; use out stem
    let fake = out.with_extension("rt");
    let parent = out.parent().unwrap_or_else(|| Path::new("."));
    let _ = fs::create_dir_all(parent);
    // Directly package stub
    let stub = load_stub_bytes(platform)?;
    let packaged = package_app(&stub, &obj.rtbc);
    fs::write(out, packaged).map_err(|e| e.to_string())?;
    let _ = (module, tmp_src, fake);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Ok(meta) = fs::metadata(out) {
            let mut perms = meta.permissions();
            perms.set_mode(0o755);
            let _ = fs::set_permissions(out, perms);
        }
    }
    Ok(out.to_path_buf())
}

fn load_stub_bytes(platform: Platform) -> Result<Vec<u8>, LinkError> {
    app_build::load_runtime_stub(platform).map_err(|e| e.to_string().into())
}

fn try_clang_uefi(c_path: &Path, out: &Path) -> bool {
    let status = Command::new("clang")
        .arg("-target")
        .arg("x86_64-unknown-windows")
        .arg("-ffreestanding")
        .arg("-fno-stack-protector")
        .arg("-fno-stack-check")
        .arg("-mno-red-zone")
        .arg("-nostdlib")
        .arg("-Wl,-entry:efi_main")
        .arg("-Wl,-subsystem:efi_application")
        .arg("-o")
        .arg(out)
        .arg(c_path)
        .status();
    matches!(status, Ok(s) if s.success())
}

// ---- PE64 / PE EFI writers -------------------------------------------------

fn align_up(v: usize, a: usize) -> usize {
    (v + a - 1) & !(a - 1)
}

fn push_u16(out: &mut Vec<u8>, v: u16) {
    out.extend_from_slice(&v.to_le_bytes());
}
fn push_u32(out: &mut Vec<u8>, v: u32) {
    out.extend_from_slice(&v.to_le_bytes());
}
fn push_u64(out: &mut Vec<u8>, v: u64) {
    out.extend_from_slice(&v.to_le_bytes());
}

/// Minimal PE32+ EFI application: .text (entry) + .rdata (rtbc).
pub fn write_pe_efi(text: &[u8], rodata: &[u8]) -> Result<Vec<u8>, LinkError> {
    write_pe_image(text, rodata, true)
}

/// Minimal PE64 image (Windows subsystem console) embedding payload in .rdata.
pub fn write_pe64(rodata: &[u8], efi: bool) -> Result<Vec<u8>, LinkError> {
    let text = [0x31u8, 0xC0, 0xC3]; // xor eax,eax; ret
    write_pe_image(&text, rodata, efi)
}

fn write_pe_image(text: &[u8], rodata: &[u8], efi: bool) -> Result<Vec<u8>, LinkError> {
    let section_align = 0x1000usize;
    let file_align = 0x200usize;

    let text_raw = align_up(text.len().max(1), file_align);
    let rdata_raw = align_up(rodata.len().max(1), file_align);

    // Headers fit in first 0x200
    let headers_size = file_align;
    let text_vasize = align_up(text.len().max(1), section_align);
    let rdata_vasize = align_up(rodata.len().max(1), section_align);

    let image_base: u64 = if efi { 0 } else { 0x140000000 };
    let entry_rva = section_align as u32; // .text at 0x1000

    let size_of_image = (section_align + text_vasize + rdata_vasize) as u32;

    let mut out = Vec::new();

    // DOS header
    out.extend_from_slice(b"MZ");
    out.resize(0x3C, 0);
    push_u32(&mut out, 0x80); // e_lfanew
    out.resize(0x80, 0);

    // PE signature
    out.extend_from_slice(b"PE\0\0");

    // COFF header
    push_u16(&mut out, 0x8664); // Machine AMD64
    push_u16(&mut out, 2); // NumberOfSections
    push_u32(&mut out, 0); // TimeDateStamp
    push_u32(&mut out, 0); // PointerToSymbolTable
    push_u32(&mut out, 0); // NumberOfSymbols
    push_u16(&mut out, 0xF0); // SizeOfOptionalHeader (PE32+)
    let characteristics: u16 = if efi {
        0x2022 // EXECUTABLE_IMAGE | LARGE_ADDRESS_AWARE
    } else {
        0x0022
    };
    push_u16(&mut out, characteristics);

    // Optional header PE32+
    push_u16(&mut out, 0x20B); // Magic PE32+
    out.push(14); // MajorLinkerVersion
    out.push(0); // MinorLinkerVersion
    push_u32(&mut out, text_vasize as u32); // SizeOfCode
    push_u32(&mut out, rdata_vasize as u32); // SizeOfInitializedData
    push_u32(&mut out, 0); // SizeOfUninitializedData
    push_u32(&mut out, entry_rva); // AddressOfEntryPoint
    push_u32(&mut out, section_align as u32); // BaseOfCode
    push_u64(&mut out, image_base); // ImageBase
    push_u32(&mut out, section_align as u32); // SectionAlignment
    push_u32(&mut out, file_align as u32); // FileAlignment
    push_u16(&mut out, 6); // MajorOS
    push_u16(&mut out, 0);
    push_u16(&mut out, 0); // MajorImage
    push_u16(&mut out, 0);
    push_u16(&mut out, 6); // MajorSubsystem
    push_u16(&mut out, 0);
    push_u32(&mut out, 0); // Win32Version
    push_u32(&mut out, size_of_image);
    push_u32(&mut out, headers_size as u32);
    push_u32(&mut out, 0); // CheckSum
    let subsystem: u16 = if efi { 10 } else { 3 }; // EFI_APPLICATION=10, CONSOLE=3
    push_u16(&mut out, subsystem);
    push_u16(&mut out, 0x8160); // DllCharacteristics
    push_u64(&mut out, 0x100000); // SizeOfStackReserve
    push_u64(&mut out, 0x1000); // SizeOfStackCommit
    push_u64(&mut out, 0x100000); // SizeOfHeapReserve
    push_u64(&mut out, 0x1000); // SizeOfHeapCommit
    push_u32(&mut out, 0); // LoaderFlags
    push_u32(&mut out, 16); // NumberOfRvaAndSizes
    for _ in 0..16 {
        push_u32(&mut out, 0);
        push_u32(&mut out, 0);
    }

    // Section .text
    let text_file_off = headers_size;
    write_section_header(
        &mut out,
        b".text\0\0\0",
        text_vasize as u32,
        section_align as u32,
        text_raw as u32,
        text_file_off as u32,
        0x60000020, // CODE | EXECUTE | READ
    );
    // Section .rdata
    let rdata_file_off = text_file_off + text_raw;
    write_section_header(
        &mut out,
        b".rdata\0\0",
        rdata_vasize as u32,
        (section_align + text_vasize) as u32,
        rdata_raw as u32,
        rdata_file_off as u32,
        0x40000040, // INITIALIZED_DATA | READ
    );

    out.resize(headers_size, 0);

    // Section data
    let mut text_data = text.to_vec();
    text_data.resize(text_raw, 0);
    out.extend_from_slice(&text_data);

    let mut rdata_data = rodata.to_vec();
    rdata_data.resize(rdata_raw, 0);
    out.extend_from_slice(&rdata_data);

    Ok(out)
}

fn write_section_header(
    out: &mut Vec<u8>,
    name: &[u8; 8],
    virtual_size: u32,
    virtual_addr: u32,
    raw_size: u32,
    raw_ptr: u32,
    characteristics: u32,
) {
    out.extend_from_slice(name);
    push_u32(out, virtual_size);
    push_u32(out, virtual_addr);
    push_u32(out, raw_size);
    push_u32(out, raw_ptr);
    push_u32(out, 0); // Relocs
    push_u32(out, 0); // LineNums
    push_u16(out, 0);
    push_u16(out, 0);
    push_u32(out, characteristics);
}

// ---- ELF64 -----------------------------------------------------------------

/// Minimal ELF64 ET_EXEC with .note.rtbc payload in a PT_LOAD (not runnable without real entry).
pub fn write_elf64_payload(rodata: &[u8]) -> Result<Vec<u8>, LinkError> {
    // Tiny x86_64 _start: mov eax,60; xor edi,edi; syscall  (exit 0) — Linux
    let text: &[u8] = &[
        0xB8, 0x3C, 0x00, 0x00, 0x00, // mov eax, 60
        0x31, 0xFF, // xor edi, edi
        0x0F, 0x05, // syscall
    ];

    let ehdr_size = 64usize;
    let phdr_size = 56usize;
    let phnum = 2usize;
    let header_end = ehdr_size + phdr_size * phnum;

    let text_off = align_up(header_end, 16);
    let text_addr = 0x400000u64 + text_off as u64;
    let ro_off = align_up(text_off + text.len(), 16);
    let ro_addr = 0x400000u64 + ro_off as u64;

    let mut out = Vec::new();
    // ELF header
    out.extend_from_slice(&[
        0x7f, b'E', b'L', b'F', // magic
        2,    // 64-bit
        1,    // little endian
        1,    // version
        0,    // System V
        0, 0, 0, 0, 0, 0, 0, 0,
    ]);
    push_u16(&mut out, 2); // ET_EXEC
    push_u16(&mut out, 0x3E); // EM_X86_64
    push_u32(&mut out, 1); // version
    push_u64(&mut out, text_addr); // entry
    push_u64(&mut out, ehdr_size as u64); // phoff
    push_u64(&mut out, 0); // shoff
    push_u32(&mut out, 0); // flags
    push_u16(&mut out, ehdr_size as u16);
    push_u16(&mut out, phdr_size as u16);
    push_u16(&mut out, phnum as u16);
    push_u16(&mut out, 0); // shentsize
    push_u16(&mut out, 0); // shnum
    push_u16(&mut out, 0); // shstrndx

    // PHDR 0: text
    push_u32(&mut out, 1); // PT_LOAD
    push_u32(&mut out, 5); // R|X
    push_u64(&mut out, text_off as u64);
    push_u64(&mut out, text_addr);
    push_u64(&mut out, text_addr);
    push_u64(&mut out, text.len() as u64);
    push_u64(&mut out, text.len() as u64);
    push_u64(&mut out, 0x1000);

    // PHDR 1: rodata (rtbc)
    push_u32(&mut out, 1); // PT_LOAD
    push_u32(&mut out, 4); // R
    push_u64(&mut out, ro_off as u64);
    push_u64(&mut out, ro_addr);
    push_u64(&mut out, ro_addr);
    push_u64(&mut out, rodata.len() as u64);
    push_u64(&mut out, rodata.len() as u64);
    push_u64(&mut out, 0x1000);

    out.resize(text_off, 0);
    out.extend_from_slice(text);
    out.resize(ro_off, 0);
    out.extend_from_slice(rodata);
    Ok(out)
}

// ---- Mach-O 64 ------------------------------------------------------------

pub fn write_macho64_payload(rodata: &[u8]) -> Result<Vec<u8>, LinkError> {
    // Minimal unsigned Mach-O 64 with one __TEXT segment containing ret + payload note.
    // Not a fully working dyld binary; format-valid for tooling tests.
    let magic: u32 = 0xFEEDFACF; // MH_MAGIC_64
    let cputype: u32 = 0x01000007; // CPU_TYPE_X86_64
    let cpusubtype: u32 = 3;
    let filetype: u32 = 2; // MH_EXECUTE
    let ncmds: u32 = 1;
    let text: &[u8] = &[0x31, 0xC0, 0xC3];

    let mut seg = Vec::new();
    // segment_command_64
    push_u32(&mut seg, 0x19); // LC_SEGMENT_64
    let cmdsize = 72u32 + 80 * 2; // seg + 2 sections
    push_u32(&mut seg, cmdsize);
    seg.extend_from_slice(b"__TEXT\0\0\0\0\0\0\0\0\0\0"); // 16 bytes
    push_u64(&mut seg, 0x100000000); // vmaddr
    let vmsize = align_up(text.len() + rodata.len() + 0x1000, 0x1000) as u64;
    push_u64(&mut seg, vmsize);
    push_u64(&mut seg, 0); // fileoff (patched conceptually; we put data after header)
    push_u64(&mut seg, (text.len() + rodata.len()) as u64);
    push_u32(&mut seg, 7); // maxprot rwx
    push_u32(&mut seg, 5); // initprot rx
    push_u32(&mut seg, 2); // nsects
    push_u32(&mut seg, 0); // flags

    // section __text
    seg.extend_from_slice(b"__text\0\0\0\0\0\0\0\0\0\0");
    seg.extend_from_slice(b"__TEXT\0\0\0\0\0\0\0\0\0\0");
    push_u64(&mut seg, 0x100000000);
    push_u64(&mut seg, text.len() as u64);
    let header_size = 32 + cmdsize as usize;
    push_u32(&mut seg, header_size as u32); // offset
    push_u32(&mut seg, 0); // align 2^0
    push_u32(&mut seg, 0);
    push_u32(&mut seg, 0);
    push_u32(&mut seg, 0x80000400); // S_ATTR_PURE_INSTRUCTIONS
    push_u32(&mut seg, 0);
    push_u32(&mut seg, 0);
    push_u32(&mut seg, 0);

    // section __const (rtbc)
    seg.extend_from_slice(b"__const\0\0\0\0\0\0\0\0\0");
    seg.extend_from_slice(b"__TEXT\0\0\0\0\0\0\0\0\0\0");
    push_u64(&mut seg, 0x100000000 + text.len() as u64);
    push_u64(&mut seg, rodata.len() as u64);
    push_u32(&mut seg, (header_size + text.len()) as u32);
    push_u32(&mut seg, 0);
    push_u32(&mut seg, 0);
    push_u32(&mut seg, 0);
    push_u32(&mut seg, 0);
    push_u32(&mut seg, 0);
    push_u32(&mut seg, 0);
    push_u32(&mut seg, 0);

    let sizeofcmds = seg.len() as u32;
    let mut out = Vec::new();
    push_u32(&mut out, magic);
    push_u32(&mut out, cputype);
    push_u32(&mut out, cpusubtype);
    push_u32(&mut out, filetype);
    push_u32(&mut out, ncmds);
    push_u32(&mut out, sizeofcmds);
    push_u32(&mut out, 1); // MH_NOUNDEFS
    push_u32(&mut out, 0); // reserved
    out.extend_from_slice(&seg);
    out.extend_from_slice(text);
    out.extend_from_slice(rodata);
    Ok(out)
}

/// Map CLI platform string / app Platform to LinkTarget.
pub fn link_target_from_platform(platform: &str) -> Option<LinkTarget> {
    LinkTarget::parse(platform)
}

pub fn link_target_from_app_platform(p: Platform) -> LinkTarget {
    match p {
        Platform::Windows => LinkTarget::WindowsX64,
        Platform::Linux => LinkTarget::LinuxX64,
        Platform::Macos => LinkTarget::MacosX64,
        Platform::Current => LinkTarget::host(),
        Platform::Uefi => LinkTarget::UefiX64,
    }
}

/// High-level: Module → binary for build pipeline.
pub fn build_native_bin(
    source_path: &Path,
    module: &Module,
    target: LinkTarget,
    output: Option<&Path>,
) -> Result<LinkResult, Box<dyn std::error::Error>> {
    let stem = source_path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("app");
    let out_dir = source_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join("dist");
    fs::create_dir_all(&out_dir)?;
    let mut result = link_module(module, target, &out_dir, stem)?;
    if let Some(user_out) = output {
        if user_out != result.output {
            if let Some(parent) = user_out.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::copy(&result.output, user_out)?;
            result.output = user_out.to_path_buf();
        }
    }
    // Also write codegen artifacts
    let _ = codegen(
        module,
        &CodegenNativeOptions {
            target,
            name: stem.into(),
            out_dir: Some(result.object_dir.clone()),
            load_address: 0x100000,
        },
    );
    Ok(result)
}
