//! ELF64 relocatable parse + executable emit.

use super::object::{Rel, RelocKind, Relocatable, Sec, SecKind, Sym, SymBind, SymType};
use crate::native_triple::Arch;

#[derive(Debug)]
pub struct ElfError(pub String);

impl std::fmt::Display for ElfError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}
impl std::error::Error for ElfError {}
impl From<String> for ElfError {
    fn from(s: String) -> Self {
        Self(s)
    }
}
impl From<&str> for ElfError {
    fn from(s: &str) -> Self {
        Self(s.into())
    }
}

fn r16(b: &[u8], o: usize) -> Result<u16, ElfError> {
    Ok(u16::from_le_bytes(
        b.get(o..o + 2)
            .ok_or("truncated ELF")?
            .try_into()
            .unwrap(),
    ))
}
fn r32(b: &[u8], o: usize) -> Result<u32, ElfError> {
    Ok(u32::from_le_bytes(
        b.get(o..o + 4)
            .ok_or("truncated ELF")?
            .try_into()
            .unwrap(),
    ))
}
fn r64(b: &[u8], o: usize) -> Result<u64, ElfError> {
    Ok(u64::from_le_bytes(
        b.get(o..o + 8)
            .ok_or("truncated ELF")?
            .try_into()
            .unwrap(),
    ))
}

fn cstr_at(table: &[u8], off: usize) -> String {
    let slice = table.get(off..).unwrap_or(&[]);
    let end = slice.iter().position(|&c| c == 0).unwrap_or(slice.len());
    String::from_utf8_lossy(&slice[..end]).into_owned()
}

fn arch_from_machine(m: u16) -> Result<Arch, ElfError> {
    match m {
        62 => Ok(Arch::X86_64),
        183 => Ok(Arch::Aarch64),
        40 => Ok(Arch::Arm),
        3 => Ok(Arch::I686),
        _ => Err(format!("unsupported ELF machine {m}").into()),
    }
}

fn sec_kind(name: &str, flags: u64, sh_type: u32) -> SecKind {
    if sh_type == 8 {
        // SHT_NOBITS
        return SecKind::Bss;
    }
    if name.starts_with(".text") || (flags & 4 != 0 && flags & 2 != 0) {
        // SHF_EXECINSTR | SHF_ALLOC often .text
        if name.starts_with(".text") || name.contains("text") {
            return SecKind::Text;
        }
    }
    if name.starts_with(".rodata") || name.starts_with(".rdata") {
        return SecKind::Rodata;
    }
    if name.starts_with(".data") {
        return SecKind::Data;
    }
    if name.starts_with(".bss") {
        return SecKind::Bss;
    }
    if flags & 4 != 0 {
        SecKind::Text
    } else if flags & 1 != 0 && flags & 2 == 0 {
        SecKind::Rodata
    } else if flags & 1 != 0 {
        SecKind::Data
    } else {
        SecKind::Other
    }
}

fn map_reloc(arch: Arch, r_type: u32) -> Option<RelocKind> {
    match arch {
        Arch::X86_64 => match r_type {
            1 => Some(RelocKind::Abs64),     // R_X86_64_64
            2 => Some(RelocKind::Rel32),     // R_X86_64_PC32
            4 => Some(RelocKind::Rel32),     // R_X86_64_PLT32
            9 => Some(RelocKind::Rel32),     // R_X86_64_GOTPCREL (approx)
            10 => Some(RelocKind::Abs32),    // R_X86_64_32
            11 => Some(RelocKind::Abs32S),   // R_X86_64_32S
            24 => Some(RelocKind::Rel64),    // R_X86_64_PC64
            _ => None,
        },
        Arch::Aarch64 => match r_type {
            257 => Some(RelocKind::Abs64),              // ABS64
            258 => Some(RelocKind::Abs32),              // ABS32
            263 => Some(RelocKind::Rel32),              // PREL32
            275 => Some(RelocKind::Aarch64AdrPrelPgHi21),
            277 => Some(RelocKind::Aarch64AddAbsLo12),  // ADD_ABS_LO12_NC
            278 => Some(RelocKind::Aarch64AddAbsLo12),  // LDST8
            282 => Some(RelocKind::Aarch64Call26),      // JUMP26
            283 => Some(RelocKind::Aarch64Call26),      // CALL26
            284 => Some(RelocKind::Aarch64AddAbsLo12),  // LDST16
            285 => Some(RelocKind::Aarch64AddAbsLo12),  // LDST32
            286 => Some(RelocKind::Aarch64AddAbsLo12),  // LDST64
            _ => None,
        },
        Arch::I686 => match r_type {
            1 => Some(RelocKind::Abs32), // R_386_32
            2 => Some(RelocKind::Rel32), // R_386_PC32
            _ => None,
        },
        Arch::Arm => match r_type {
            2 => Some(RelocKind::Abs32), // R_ARM_ABS32
            3 => Some(RelocKind::Abs32), // R_ARM_REL32 → treat abs for static
            28 => Some(RelocKind::Rel32), // R_ARM_CALL (approx as Rel32)
            29 => Some(RelocKind::Rel32),
            _ => None,
        },
    }
}

/// Parse an ELF64 relocatable object (ET_REL).
pub fn parse_elf64(bytes: &[u8]) -> Result<Relocatable, ElfError> {
    if bytes.len() < 64 || &bytes[0..4] != b"\x7fELF" {
        return Err("not an ELF file".into());
    }
    if bytes[4] != 2 {
        return Err("only ELF64 supported".into());
    }
    if bytes[5] != 1 {
        return Err("only little-endian ELF supported".into());
    }
    let e_type = r16(bytes, 16)?;
    if e_type != 1 {
        return Err(format!("expected ET_REL (1), got {e_type}").into());
    }
    let machine = r16(bytes, 18)?;
    let arch = arch_from_machine(machine)?;
    let e_shoff = r64(bytes, 40)? as usize;
    let e_shentsize = r16(bytes, 58)? as usize;
    let e_shnum = r16(bytes, 60)? as usize;
    let e_shstrndx = r16(bytes, 62)? as usize;

    if e_shentsize < 64 || e_shnum == 0 {
        return Err("invalid section header table".into());
    }

    let sh_at = |i: usize| -> Result<&[u8], ElfError> {
        let off = e_shoff + i * e_shentsize;
        bytes
            .get(off..off + 64)
            .ok_or_else(|| "section header OOB".into())
    };

    let shstr = {
        let sh = sh_at(e_shstrndx)?;
        let off = r64(sh, 24)? as usize;
        let size = r64(sh, 32)? as usize;
        bytes
            .get(off..off + size)
            .ok_or("shstrtab OOB")?
            .to_vec()
    };

    // Map ELF section index → our section index (skip null / non-alloc metadata).
    let mut elf_to_ours: Vec<Option<usize>> = vec![None; e_shnum];
    let mut obj = Relocatable::new(arch);
    // Keep placeholder for section 0
    let mut symtab_idx: Option<usize> = None;
    let mut rela_list: Vec<(usize, usize, usize, bool)> = Vec::new(); // (elf_sec, offset, size, is_rela)

    for i in 1..e_shnum {
        let sh = sh_at(i)?;
        let name_off = r32(sh, 0)? as usize;
        let sh_type = r32(sh, 4)?;
        let flags = r64(sh, 8)?;
        let offset = r64(sh, 24)? as usize;
        let size = r64(sh, 32)? as usize;
        let link = r32(sh, 40)? as usize;
        let _info = r32(sh, 44)?;
        let addralign = r64(sh, 48)? as u32;
        let name = cstr_at(&shstr, name_off);

        match sh_type {
            1 | 8 => {
                // SHT_PROGBITS / SHT_NOBITS
                let data = if sh_type == 8 {
                    Vec::new()
                } else {
                    bytes
                        .get(offset..offset + size)
                        .ok_or("section data OOB")?
                        .to_vec()
                };
                let kind = sec_kind(&name, flags, sh_type);
                let idx = obj.sections.len();
                obj.sections.push(Sec {
                    name,
                    kind,
                    data,
                    align: addralign.max(1),
                    size: size as u64,
                    flags,
                });
                elf_to_ours[i] = Some(idx);
            }
            2 => {
                // SHT_SYMTAB
                symtab_idx = Some(i);
                let _ = (link, size, offset);
            }
            4 => {
                // SHT_RELA
                rela_list.push((i, offset, size, true));
            }
            9 => {
                // SHT_REL
                rela_list.push((i, offset, size, false));
            }
            _ => {}
        }
    }

    // Symbols
    if let Some(si) = symtab_idx {
        let sh = sh_at(si)?;
        let offset = r64(sh, 24)? as usize;
        let size = r64(sh, 32)? as usize;
        let link = r32(sh, 40)? as usize; // strtab
        let entsize = r64(sh, 56)? as usize;
        if entsize < 24 {
            return Err("bad symtab entsize".into());
        }
        let str_sh = sh_at(link)?;
        let stroff = r64(str_sh, 24)? as usize;
        let strsz = r64(str_sh, 32)? as usize;
        let strtab = bytes
            .get(stroff..stroff + strsz)
            .ok_or("strtab OOB")?;

        let count = size / entsize;
        for i in 0..count {
            let e = &bytes[offset + i * entsize..offset + i * entsize + 24];
            let st_name = r32(e, 0)? as usize;
            let st_info = e[4];
            let _st_other = e[5];
            let st_shndx = r16(e, 6)? as usize;
            let st_value = r64(e, 8)?;
            let st_size = r64(e, 16)?;
            let bind = match st_info >> 4 {
                0 => SymBind::Local,
                1 => SymBind::Global,
                2 => SymBind::Weak,
                _ => SymBind::Global,
            };
            let ty = match st_info & 0xf {
                1 => SymType::Object,
                2 => SymType::Func,
                3 => SymType::Section,
                4 => SymType::File,
                _ => SymType::None,
            };
            let name = cstr_at(strtab, st_name);
            let section = if st_shndx == 0 || st_shndx >= 0xff00 {
                None
            } else {
                elf_to_ours.get(st_shndx).copied().flatten()
            };
            obj.symbols.push(Sym {
                name,
                section,
                value: st_value,
                size: st_size,
                bind,
                ty,
            });
        }
    }

    // Relocations — info field of RELA section is target section
    for (ri, offset, size, is_rela) in rela_list {
        let sh = sh_at(ri)?;
        let info = r32(sh, 44)? as usize; // target section
        let target = match elf_to_ours.get(info).copied().flatten() {
            Some(t) => t,
            None => continue,
        };
        let entsize = if is_rela { 24 } else { 16 };
        let count = size / entsize;
        for i in 0..count {
            let e = &bytes[offset + i * entsize..offset + i * entsize + entsize];
            let r_offset = r64(e, 0)?;
            let r_info = r64(e, 8)?;
            let addend = if is_rela {
                r64(e, 16)? as i64
            } else {
                // REL: addend in place
                0
            };
            let sym_idx = (r_info >> 32) as usize;
            let r_type = (r_info & 0xffff_ffff) as u32;
            let Some(kind) = map_reloc(arch, r_type) else {
                continue; // skip unsupported
            };
            // For REL, read implicit addend from section data
            let addend = if !is_rela {
                let sec = &obj.sections[target];
                let off = r_offset as usize;
                match kind {
                    RelocKind::Abs64 | RelocKind::Rel64 => {
                        if off + 8 <= sec.data.len() {
                            i64::from_le_bytes(sec.data[off..off + 8].try_into().unwrap())
                        } else {
                            0
                        }
                    }
                    _ => {
                        if off + 4 <= sec.data.len() {
                            i32::from_le_bytes(sec.data[off..off + 4].try_into().unwrap()) as i64
                        } else {
                            0
                        }
                    }
                }
            } else {
                addend
            };
            if sym_idx >= obj.symbols.len() {
                continue;
            }
            obj.relocs.push(Rel {
                section: target,
                offset: r_offset,
                symbol: sym_idx,
                addend,
                kind,
            });
        }
    }

    Ok(obj)
}

fn align_up(v: usize, a: usize) -> usize {
    if a == 0 {
        return v;
    }
    (v + a - 1) & !(a - 1)
}

fn push_u16(o: &mut Vec<u8>, v: u16) {
    o.extend_from_slice(&v.to_le_bytes());
}
fn push_u32(o: &mut Vec<u8>, v: u32) {
    o.extend_from_slice(&v.to_le_bytes());
}
fn push_u64(o: &mut Vec<u8>, v: u64) {
    o.extend_from_slice(&v.to_le_bytes());
}

/// Emit a static ELF64 ET_EXEC from laid-out sections.
/// `sections`: (name, data, vaddr, flags) where flags: PF_X=1 PF_W=2 PF_R=4
pub fn write_elf64_exec(
    arch: Arch,
    entry: u64,
    loads: &[(Vec<u8>, u64, u32)], // (data, vaddr, p_flags)
) -> Result<Vec<u8>, ElfError> {
    let page = 0x1000usize;
    let phnum = loads.len() as u16;
    let ehdr_size = 64usize;
    let phdr_size = 56usize;
    let ph_off = ehdr_size;
    let header_end = align_up(ehdr_size + phdr_size * phnum as usize, page);

    // File layout: headers, then each segment at file offset congruent to vaddr mod page
    let mut file_offs = Vec::with_capacity(loads.len());
    let mut cursor = header_end;
    for (data, vaddr, _) in loads {
        let va = *vaddr as usize;
        // Want file_off ≡ vaddr (mod page)
        let want = (cursor & !(page - 1)) + (va & (page - 1));
        let file_off = if want < cursor { want + page } else { want };
        file_offs.push(file_off);
        cursor = file_off + data.len();
    }
    let file_size = cursor;

    let mut out = Vec::with_capacity(file_size);
    out.extend_from_slice(b"\x7fELF");
    out.push(2); // ELF64
    out.push(1); // LE
    out.push(1); // version
    out.push(0); // System V
    out.extend_from_slice(&[0u8; 8]);
    push_u16(&mut out, 2); // ET_EXEC
    push_u16(&mut out, arch.elf_machine());
    push_u32(&mut out, 1); // version
    push_u64(&mut out, entry);
    push_u64(&mut out, ph_off as u64);
    push_u64(&mut out, 0); // shoff
    push_u32(&mut out, 0); // flags
    push_u16(&mut out, 64);
    push_u16(&mut out, 56);
    push_u16(&mut out, phnum);
    push_u16(&mut out, 0);
    push_u16(&mut out, 0);
    push_u16(&mut out, 0);

    out.resize(ph_off, 0);
    for (i, (data, vaddr, flags)) in loads.iter().enumerate() {
        let file_off = file_offs[i];
        push_u32(&mut out, 1); // PT_LOAD
        push_u32(&mut out, *flags);
        push_u64(&mut out, file_off as u64);
        push_u64(&mut out, *vaddr);
        push_u64(&mut out, *vaddr);
        push_u64(&mut out, data.len() as u64);
        push_u64(&mut out, data.len() as u64);
        push_u64(&mut out, page as u64);
    }
    out.resize(file_size, 0);
    for (i, (data, _, _)) in loads.iter().enumerate() {
        let off = file_offs[i];
        out[off..off + data.len()].copy_from_slice(data);
    }
    Ok(out)
}
