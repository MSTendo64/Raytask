//! Merge relocatable objects, resolve symbols, apply relocations, emit image.

use super::coff;
use super::elf;
use super::object::{RelocKind, Relocatable, Sec, SecKind, Sym, SymBind, SymType};
use crate::native_triple::{Arch, NativeTriple, OsKind};

#[derive(Debug)]
pub struct ResolveError(pub String);

impl std::fmt::Display for ResolveError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}
impl std::error::Error for ResolveError {}
impl From<String> for ResolveError {
    fn from(s: String) -> Self {
        Self(s)
    }
}
impl From<&str> for ResolveError {
    fn from(s: &str) -> Self {
        Self(s.into())
    }
}

fn align_up(v: u64, a: u64) -> u64 {
    if a == 0 {
        return v;
    }
    (v + a - 1) & !(a - 1)
}

/// Linked image ready to write.
#[derive(Debug)]
pub struct LinkedImage {
    pub arch: Arch,
    pub entry_va: u64,
    pub text: Vec<u8>,
    pub text_va: u64,
    pub rodata: Vec<u8>,
    pub rodata_va: u64,
    pub data: Vec<u8>,
    pub data_va: u64,
    pub notes: Vec<String>,
}

/// Merge many relocatables into one (same arch).
pub fn merge(objects: &[Relocatable]) -> Result<Relocatable, ResolveError> {
    if objects.is_empty() {
        return Err("no objects to link".into());
    }
    let arch = objects[0].arch;
    for o in objects {
        if o.arch != arch {
            return Err(format!(
                "arch mismatch: {} vs {}",
                arch.name(),
                o.arch.name()
            )
            .into());
        }
    }

    let mut out = Relocatable::new(arch);
    // section name → out index
    use std::collections::HashMap;
    let mut sec_map: HashMap<String, usize> = HashMap::new();

    for obj in objects {
        let mut local_sec_remap = vec![0usize; obj.sections.len()];
        let mut sec_offsets = vec![0u64; obj.sections.len()];

        for (i, sec) in obj.sections.iter().enumerate() {
            if let Some(&dst) = sec_map.get(&sec.name) {
                let align = out.sections[dst].align.max(sec.align) as u64;
                let cur = out.sections[dst].data.len() as u64;
                let pad = align_up(cur, align) - cur;
                out.sections[dst]
                    .data
                    .extend(std::iter::repeat(0u8).take(pad as usize));
                sec_offsets[i] = out.sections[dst].data.len() as u64;
                out.sections[dst].data.extend_from_slice(&sec.data);
                if sec.kind == SecKind::Bss {
                    let need = sec_offsets[i] + sec.size;
                    if need > out.sections[dst].size {
                        out.sections[dst].size = need;
                    }
                } else {
                    out.sections[dst].size = out.sections[dst].data.len() as u64;
                }
                out.sections[dst].align = align as u32;
                local_sec_remap[i] = dst;
            } else {
                let idx = out.sections.len();
                sec_map.insert(sec.name.clone(), idx);
                local_sec_remap[i] = idx;
                sec_offsets[i] = 0;
                out.sections.push(Sec {
                    name: sec.name.clone(),
                    kind: sec.kind,
                    data: sec.data.clone(),
                    align: sec.align,
                    size: sec.size.max(sec.data.len() as u64),
                    flags: sec.flags,
                });
            }
        }

        let sym_base = out.symbols.len();
        for sym in &obj.symbols {
            let section = sym.section.map(|s| local_sec_remap[s]);
            let value = match sym.section {
                Some(s) => sym.value + sec_offsets[s],
                None => sym.value,
            };
            out.symbols.push(Sym {
                name: sym.name.clone(),
                section,
                value,
                size: sym.size,
                bind: sym.bind,
                ty: sym.ty,
            });
        }
        for rel in &obj.relocs {
            let section = local_sec_remap[rel.section];
            let offset = rel.offset + sec_offsets[rel.section];
            out.relocs.push(super::object::Rel {
                section,
                offset,
                symbol: sym_base + rel.symbol,
                addend: rel.addend,
                kind: rel.kind,
            });
        }
        if out.entry.is_none() {
            out.entry = obj.entry.clone();
        }
    }

    // Resolve duplicate globals: keep first strong definition.
    let mut defined: HashMap<String, usize> = HashMap::new();
    for (i, sym) in out.symbols.iter().enumerate() {
        if sym.section.is_none() || sym.name.is_empty() {
            continue;
        }
        if matches!(sym.bind, SymBind::Local) {
            continue;
        }
        defined.entry(sym.name.clone()).or_insert(i);
    }
    // Point undefined symbols at the defining index via a redirect table used at reloc time.
    // We rewrite undefined symbol entries to copy the definition.
    for i in 0..out.symbols.len() {
        if out.symbols[i].section.is_some() {
            continue;
        }
        if out.symbols[i].name.is_empty() {
            continue;
        }
        if let Some(&def) = defined.get(&out.symbols[i].name) {
            out.symbols[i].section = out.symbols[def].section;
            out.symbols[i].value = out.symbols[def].value;
            out.symbols[i].size = out.symbols[def].size;
            out.symbols[i].ty = out.symbols[def].ty;
            out.symbols[i].bind = out.symbols[def].bind;
        }
    }

    Ok(out)
}

fn patch_u32(data: &mut [u8], off: usize, v: u32) -> Result<(), ResolveError> {
    if off + 4 > data.len() {
        return Err(format!("reloc OOB at {off}").into());
    }
    data[off..off + 4].copy_from_slice(&v.to_le_bytes());
    Ok(())
}

fn patch_u64(data: &mut [u8], off: usize, v: u64) -> Result<(), ResolveError> {
    if off + 8 > data.len() {
        return Err(format!("reloc OOB at {off}").into());
    }
    data[off..off + 8].copy_from_slice(&v.to_le_bytes());
    Ok(())
}

fn patch_i32(data: &mut [u8], off: usize, v: i32) -> Result<(), ResolveError> {
    patch_u32(data, off, v as u32)
}

/// Assign VAs and apply relocations.
pub fn layout_and_relocate(
    mut obj: Relocatable,
    base: u64,
    entry_name: &str,
) -> Result<LinkedImage, ResolveError> {
    let mut notes = Vec::new();

    // Order: text, rodata, data, bss/other alloc
    let mut order: Vec<usize> = Vec::new();
    for kind in [SecKind::Text, SecKind::Rodata, SecKind::Data, SecKind::Bss, SecKind::Other] {
        for (i, s) in obj.sections.iter().enumerate() {
            if s.kind == kind && !order.contains(&i) {
                // Skip non-alloc-ish debug
                if s.name.starts_with(".debug")
                    || s.name.starts_with(".comment")
                    || s.name.starts_with(".note")
                    || s.name.starts_with(".symtab")
                    || s.name.starts_with(".strtab")
                    || s.name.starts_with(".reloc")
                {
                    continue;
                }
                order.push(i);
            }
        }
    }

    let page = 0x1000u64;
    let mut va = align_up(base, page);
    let mut sec_va = vec![0u64; obj.sections.len()];
    for &i in &order {
        let align = obj.sections[i].align.max(1) as u64;
        va = align_up(va, align.max(16));
        sec_va[i] = va;
        let size = obj.sections[i]
            .size
            .max(obj.sections[i].data.len() as u64)
            .max(1);
        // Ensure data buffer covers size for BSS padding in image
        if obj.sections[i].data.len() < size as usize && obj.sections[i].kind != SecKind::Bss {
            obj.sections[i].data.resize(size as usize, 0);
        }
        va = align_up(va + size, page);
    }

    let sym_addr = |obj: &Relocatable, idx: usize| -> Result<u64, ResolveError> {
        let s = obj
            .symbols
            .get(idx)
            .ok_or_else(|| format!("bad symbol index {idx}"))?;
        match s.section {
            Some(sec) => Ok(sec_va[sec] + s.value),
            None => {
                if s.name.is_empty() {
                    Ok(s.value)
                } else {
                    Err(format!("undefined symbol '{}'", s.name).into())
                }
            }
        }
    };

    // Apply relocs into section data
    let relocs = obj.relocs.clone();
    for rel in &relocs {
        let s_addr = sym_addr(&obj, rel.symbol)?;
        let p = sec_va[rel.section] + rel.offset;
        let data = &mut obj.sections[rel.section].data;
        let off = rel.offset as usize;
        match rel.kind {
            RelocKind::Abs64 => {
                patch_u64(data, off, (s_addr as i64 + rel.addend) as u64)?;
            }
            RelocKind::Abs32 | RelocKind::Abs32S | RelocKind::Addr32Nb => {
                let v = (s_addr as i64 + rel.addend) as u32;
                patch_u32(data, off, v)?;
            }
            RelocKind::Rel32 => {
                let v = (s_addr as i64 + rel.addend - p as i64) as i32;
                patch_i32(data, off, v)?;
            }
            RelocKind::Rel64 => {
                let v = (s_addr as i64 + rel.addend - p as i64) as u64;
                patch_u64(data, off, v)?;
            }
            RelocKind::Aarch64Call26 => {
                if off + 4 > data.len() {
                    return Err("CALL26 OOB".into());
                }
                let imm = (s_addr as i64 + rel.addend - p as i64) >> 2;
                if !(-0x200_0000..=0x1ff_ffff).contains(&imm) {
                    return Err("CALL26 out of range".into());
                }
                let mut insn = u32::from_le_bytes(data[off..off + 4].try_into().unwrap());
                insn = (insn & 0xFC00_0000) | ((imm as u32) & 0x03FF_FFFF);
                patch_u32(data, off, insn)?;
            }
            RelocKind::Aarch64AdrPrelPgHi21 => {
                let page_s = s_addr & !0xfff;
                let page_p = p & !0xfff;
                let imm = ((page_s as i64 + rel.addend) - page_p as i64) >> 12;
                if off + 4 > data.len() {
                    return Err("ADR_PREL OOB".into());
                }
                let mut insn = u32::from_le_bytes(data[off..off + 4].try_into().unwrap());
                let immlo = (imm as u32) & 3;
                let immhi = ((imm as u32) >> 2) & 0x1f_ffff;
                insn = (insn & 0x9F00_001F) | (immlo << 29) | (immhi << 5);
                patch_u32(data, off, insn)?;
            }
            RelocKind::Aarch64AddAbsLo12 => {
                let imm = ((s_addr as i64 + rel.addend) & 0xfff) as u32;
                if off + 4 > data.len() {
                    return Err("LO12 OOB".into());
                }
                let mut insn = u32::from_le_bytes(data[off..off + 4].try_into().unwrap());
                insn = (insn & !(0xfff << 10)) | (imm << 10);
                patch_u32(data, off, insn)?;
            }
        }
    }

    // Concatenate by kind for image writers
    let mut text = Vec::new();
    let mut text_va = 0u64;
    let mut rodata = Vec::new();
    let mut rodata_va = 0u64;
    let mut data = Vec::new();
    let mut data_va = 0u64;

    for &i in &order {
        let s = &obj.sections[i];
        match s.kind {
            SecKind::Text => {
                if text.is_empty() {
                    text_va = sec_va[i];
                }
                // pad to section VA relative
                let pad = (sec_va[i] - text_va) as usize;
                if text.len() < pad {
                    text.resize(pad, 0x90);
                }
                text.extend_from_slice(&s.data);
            }
            SecKind::Rodata | SecKind::Other => {
                if rodata.is_empty() {
                    rodata_va = sec_va[i];
                }
                let pad = (sec_va[i].saturating_sub(rodata_va)) as usize;
                if rodata.len() < pad {
                    rodata.resize(pad, 0);
                }
                rodata.extend_from_slice(&s.data);
            }
            SecKind::Data | SecKind::Bss => {
                if data.is_empty() {
                    data_va = sec_va[i];
                }
                let pad = (sec_va[i].saturating_sub(data_va)) as usize;
                if data.len() < pad {
                    data.resize(pad, 0);
                }
                if s.kind == SecKind::Bss {
                    data.resize(data.len() + s.size as usize, 0);
                } else {
                    data.extend_from_slice(&s.data);
                }
            }
        }
    }

    let entry_va = if let Some(idx) = obj.find_symbol(entry_name) {
        sym_addr(&obj, idx)?
    } else if let Some(idx) = obj.find_symbol("_start") {
        notes.push("using _start as entry".into());
        sym_addr(&obj, idx)?
    } else if let Some(idx) = obj.find_symbol("main") {
        notes.push("using main as entry".into());
        sym_addr(&obj, idx)?
    } else if let Some(idx) = obj.find_symbol("efi_main") {
        notes.push("using efi_main as entry".into());
        sym_addr(&obj, idx)?
    } else {
        return Err(format!(
            "entry symbol '{entry_name}' not found (also tried _start, main, efi_main)"
        )
        .into());
    };

    // Silence unused SymType warning by referencing in notes if needed
    let _ = SymType::None;

    Ok(LinkedImage {
        arch: obj.arch,
        entry_va,
        text,
        text_va,
        rodata,
        rodata_va,
        data,
        data_va,
        notes,
    })
}

/// Emit bytes for the given OS triple from a linked image.
pub fn emit_image(triple: NativeTriple, image: &LinkedImage, efi: bool) -> Result<Vec<u8>, ResolveError> {
    match triple.os {
        OsKind::Linux | OsKind::Freestanding => {
            let mut loads = Vec::new();
            // PF_R|PF_X = 5, PF_R = 4, PF_R|PF_W = 6
            if !image.text.is_empty() {
                loads.push((image.text.clone(), image.text_va, 5u32));
            }
            if !image.rodata.is_empty() {
                loads.push((image.rodata.clone(), image.rodata_va, 4u32));
            }
            if !image.data.is_empty() {
                loads.push((image.data.clone(), image.data_va, 6u32));
            }
            elf::write_elf64_exec(image.arch, image.entry_va, &loads)
                .map_err(|e| ResolveError(e.0))
        }
        OsKind::Windows | OsKind::Uefi => {
            let entry_rva = if image.text_va >= 0x140000000 && !efi {
                (image.entry_va - 0x140000000) as u32
            } else if image.text_va >= 0x1000 {
                // Prefer RVA relative to preferred image base used in writer
                let base = if efi { 0 } else { 0x140000000u64 };
                if image.entry_va >= base {
                    (image.entry_va - base) as u32
                } else {
                    (image.entry_va - image.text_va + 0x1000) as u32
                }
            } else {
                0x1000
            };
            // When we layout with base 0x140001000, entry_rva = entry - image_base
            let image_base = if efi { 0u64 } else { 0x140000000 };
            let entry_rva = if image.entry_va >= image_base {
                (image.entry_va - image_base) as u32
            } else {
                entry_rva
            };
            coff::write_pe64(
                image.arch,
                entry_rva,
                &image.text,
                &image.rodata,
                &image.data,
                efi || triple.os == OsKind::Uefi,
            )
            .map_err(|e| ResolveError(e.0))
        }
        OsKind::Macos => Err(
            "built-in Mach-O emit is limited; use clang -target for macOS host link".into(),
        ),
    }
}

/// Default VMA base for a triple.
pub fn default_base(triple: NativeTriple) -> u64 {
    match triple.os {
        OsKind::Windows => 0x140001000,
        OsKind::Uefi => 0x1000,
        OsKind::Linux | OsKind::Freestanding => 0x400000,
        OsKind::Macos => 0x100000000,
    }
}
