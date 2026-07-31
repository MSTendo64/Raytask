//! Built-in object-file linker: ELF64 / COFF → PE / ELF executables.
//!
//! Pipeline: parse `.o`/`.obj` → merge → resolve symbols → apply relocs → emit image.

mod coff;
mod elf;
mod object;
mod resolve;

pub use object::{Rel, RelocKind, Relocatable, Sec, SecKind, Sym, SymBind, SymType};
pub use resolve::{LinkedImage, default_base};

use crate::native_triple::{NativeTriple, OsKind};
use std::fs;
use std::path::{Path, PathBuf};

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

#[derive(Debug, Clone)]
pub struct BuiltinLinkOptions {
    pub triple: NativeTriple,
    pub entry: String,
    pub base: Option<u64>,
    /// Treat as UEFI PE subsystem.
    pub efi: bool,
}

impl Default for BuiltinLinkOptions {
    fn default() -> Self {
        Self {
            triple: NativeTriple::host(),
            entry: "_start".into(),
            base: None,
            efi: false,
        }
    }
}

#[derive(Debug)]
pub struct BuiltinLinkResult {
    pub output: PathBuf,
    pub notes: Vec<String>,
    pub entry_va: u64,
}

/// Detect and parse a relocatable object file.
pub fn parse_object(bytes: &[u8]) -> Result<Relocatable, LinkError> {
    if bytes.len() >= 4 && &bytes[0..4] == b"\x7fELF" {
        return elf::parse_elf64(bytes).map_err(|e| LinkError { message: e.0 });
    }
    // COFF starts with machine word; common: 0x8664, 0x014c, 0xaa64
    if bytes.len() >= 20 {
        let m = u16::from_le_bytes([bytes[0], bytes[1]]);
        if matches!(m, 0x8664 | 0x014C | 0xAA64 | 0x01C4 | 0x01C0 | 0x01C2) {
            return coff::parse_coff(bytes).map_err(|e| LinkError { message: e.0 });
        }
    }
    Err("unrecognized object format (expected ELF64 or COFF .o/.obj)".into())
}

pub fn parse_object_file(path: &Path) -> Result<Relocatable, LinkError> {
    let bytes = fs::read(path).map_err(|e| e.to_string())?;
    parse_object(&bytes).map_err(|e| {
        LinkError {
            message: format!("{}: {}", path.display(), e.message),
        }
    })
}

/// Link one or more relocatable objects into an executable with the built-in linker.
pub fn link_objects(
    objects: &[Relocatable],
    out: &Path,
    opts: &BuiltinLinkOptions,
) -> Result<BuiltinLinkResult, LinkError> {
    if let Some(parent) = out.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let mut notes = Vec::new();
    notes.push(format!("built-in linker → {}", opts.triple));

    let merged = resolve::merge(objects).map_err(|e| LinkError { message: e.0 })?;
    let base = opts.base.unwrap_or_else(|| resolve::default_base(opts.triple));
    let image = resolve::layout_and_relocate(merged, base, &opts.entry)
        .map_err(|e| LinkError { message: e.0 })?;
    notes.extend(image.notes.clone());

    let efi = opts.efi || opts.triple.os == OsKind::Uefi;
    let bytes = resolve::emit_image(opts.triple, &image, efi)
        .map_err(|e| LinkError { message: e.0 })?;
    fs::write(out, &bytes).map_err(|e| e.to_string())?;

    #[cfg(unix)]
    if matches!(opts.triple.os, OsKind::Linux | OsKind::Freestanding | OsKind::Macos) {
        use std::os::unix::fs::PermissionsExt;
        if let Ok(meta) = fs::metadata(out) {
            let mut perms = meta.permissions();
            perms.set_mode(0o755);
            let _ = fs::set_permissions(out, perms);
        }
    }

    notes.push(format!(
        "entry={} @ {:#x}, text={}B rodata={}B data={}B",
        opts.entry,
        image.entry_va,
        image.text.len(),
        image.rodata.len(),
        image.data.len()
    ));

    Ok(BuiltinLinkResult {
        output: out.to_path_buf(),
        notes,
        entry_va: image.entry_va,
    })
}

/// Link object files from disk.
pub fn link_paths(
    paths: &[PathBuf],
    out: &Path,
    opts: &BuiltinLinkOptions,
) -> Result<BuiltinLinkResult, LinkError> {
    let mut objs = Vec::new();
    for p in paths {
        objs.push(parse_object_file(p)?);
    }
    link_objects(&objs, out, opts)
}

/// Write a minimal freestanding ELF/PE that just returns (for smoke tests).
pub fn write_smoke_executable(triple: NativeTriple, out: &Path) -> Result<PathBuf, LinkError> {
    let (text, entry_name) = match triple.arch {
        crate::native_triple::Arch::X86_64 | crate::native_triple::Arch::I686 => {
            // xor eax,eax; ret
            (vec![0x31, 0xC0, 0xC3], "_start")
        }
        crate::native_triple::Arch::Aarch64 => {
            // mov w0, #0; ret
            (
                vec![0x00, 0x00, 0x80, 0x52, 0xC0, 0x03, 0x5F, 0xD6],
                "_start",
            )
        }
        crate::native_triple::Arch::Arm => {
            // mov r0, #0; bx lr
            (vec![0x00, 0x00, 0xA0, 0xE3, 0x1E, 0xFF, 0x2F, 0xE1], "_start")
        }
    };

    let mut obj = Relocatable::new(triple.arch);
    obj.sections.push(Sec {
        name: ".text".into(),
        kind: SecKind::Text,
        data: text,
        align: 16,
        size: 0,
        flags: 6,
    });
    obj.sections[0].size = obj.sections[0].data.len() as u64;
    obj.symbols.push(Sym {
        name: entry_name.into(),
        section: Some(0),
        value: 0,
        size: obj.sections[0].size,
        bind: SymBind::Global,
        ty: SymType::Func,
    });
    obj.entry = Some(entry_name.into());

    let opts = BuiltinLinkOptions {
        triple,
        entry: entry_name.into(),
        base: None,
        efi: triple.os == OsKind::Uefi,
    };
    let r = link_objects(&[obj], out, &opts)?;
    Ok(r.output)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::native_triple::{Arch, OsKind};

    #[test]
    fn smoke_elf_x86_64() {
        let dir = std::env::temp_dir().join(format!("rt_link_{}", std::process::id()));
        let _ = fs::create_dir_all(&dir);
        let out = dir.join("smoke.elf");
        let triple = NativeTriple::new(OsKind::Linux, Arch::X86_64);
        write_smoke_executable(triple, &out).unwrap();
        let bytes = fs::read(&out).unwrap();
        assert_eq!(&bytes[0..4], b"\x7fELF");
        assert_eq!(bytes[18], 62); // EM_X86_64
    }

    #[test]
    fn smoke_pe_x86_64() {
        let dir = std::env::temp_dir().join(format!("rt_link_pe_{}", std::process::id()));
        let _ = fs::create_dir_all(&dir);
        let out = dir.join("smoke.exe");
        let triple = NativeTriple::new(OsKind::Windows, Arch::X86_64);
        write_smoke_executable(triple, &out).unwrap();
        let bytes = fs::read(&out).unwrap();
        assert_eq!(&bytes[0..2], b"MZ");
    }

    #[test]
    fn smoke_elf_aarch64() {
        let dir = std::env::temp_dir().join(format!("rt_link_a64_{}", std::process::id()));
        let _ = fs::create_dir_all(&dir);
        let out = dir.join("smoke_a64.elf");
        let triple = NativeTriple::new(OsKind::Linux, Arch::Aarch64);
        write_smoke_executable(triple, &out).unwrap();
        let bytes = fs::read(&out).unwrap();
        assert_eq!(&bytes[0..4], b"\x7fELF");
        assert_eq!(u16::from_le_bytes([bytes[18], bytes[19]]), 183);
    }
}
