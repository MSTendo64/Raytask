//! COFF object parse + PE32+ executable emit.

use super::object::{Rel, RelocKind, Relocatable, Sec, SecKind, Sym, SymBind, SymType};
use crate::native_triple::Arch;

#[derive(Debug)]
pub struct CoffError(pub String);

impl std::fmt::Display for CoffError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}
impl std::error::Error for CoffError {}
impl From<String> for CoffError {
    fn from(s: String) -> Self {
        Self(s)
    }
}
impl From<&str> for CoffError {
    fn from(s: &str) -> Self {
        Self(s.into())
    }
}

fn r16(b: &[u8], o: usize) -> Result<u16, CoffError> {
    Ok(u16::from_le_bytes(
        b.get(o..o + 2)
            .ok_or("truncated COFF")?
            .try_into()
            .unwrap(),
    ))
}
fn r32(b: &[u8], o: usize) -> Result<u32, CoffError> {
    Ok(u32::from_le_bytes(
        b.get(o..o + 4)
            .ok_or("truncated COFF")?
            .try_into()
            .unwrap(),
    ))
}

fn arch_from_machine(m: u16) -> Result<Arch, CoffError> {
    match m {
        0x8664 => Ok(Arch::X86_64),
        0xAA64 => Ok(Arch::Aarch64),
        0x01C4 | 0x01C0 | 0x01C2 => Ok(Arch::Arm),
        0x014C => Ok(Arch::I686),
        _ => Err(format!("unsupported COFF machine 0x{m:x}").into()),
    }
}

fn sec_kind(name: &str, chars: u32) -> SecKind {
    let n = name.trim_start_matches('.');
    if n.starts_with("text") || (chars & 0x20 != 0 && !n.starts_with("data")) {
        SecKind::Text
    } else if n.starts_with("rdata") || n.starts_with("rodata") {
        SecKind::Rodata
    } else if n.starts_with("bss") || chars & 0x80 != 0 {
        SecKind::Bss
    } else if n.starts_with("data") {
        SecKind::Data
    } else {
        SecKind::Other
    }
}

fn map_reloc(arch: Arch, ty: u16) -> Option<(RelocKind, i64)> {
    match arch {
        Arch::X86_64 => match ty {
            0x0001 => Some((RelocKind::Abs64, 0)),
            0x0002 => Some((RelocKind::Abs32, 0)),
            0x0003 => Some((RelocKind::Addr32Nb, 0)),
            0x0004 => Some((RelocKind::Rel32, 0)),
            // REL32_1 .. REL32_5: displacement is relative to P+N
            n @ 0x0005..=0x0009 => Some((RelocKind::Rel32, -((n - 4) as i64))),
            _ => None,
        },
        Arch::I686 => match ty {
            0x0006 => Some((RelocKind::Abs32, 0)),
            0x0014 => Some((RelocKind::Rel32, 0)),
            _ => None,
        },
        Arch::Aarch64 => match ty {
            0x0014 => Some((RelocKind::Abs64, 0)),
            0x0002 => Some((RelocKind::Addr32Nb, 0)),
            0x0003 => Some((RelocKind::Aarch64Call26, 0)),
            _ => None,
        },
        Arch::Arm => match ty {
            0x0001 => Some((RelocKind::Abs32, 0)),
            _ => None,
        },
    }
}

fn read_name(raw: &[u8; 8], string_table: &[u8]) -> String {
    if raw[0] == 0 {
        let off = u32::from_le_bytes([raw[4], raw[5], raw[6], raw[7]]) as usize;
        let slice = string_table.get(off..).unwrap_or(&[]);
        let end = slice.iter().position(|&c| c == 0).unwrap_or(slice.len());
        String::from_utf8_lossy(&slice[..end]).into_owned()
    } else {
        let end = raw.iter().position(|&c| c == 0).unwrap_or(8);
        String::from_utf8_lossy(&raw[..end]).into_owned()
    }
}

/// Parse a COFF relocatable object (`.obj`).
pub fn parse_coff(bytes: &[u8]) -> Result<Relocatable, CoffError> {
    if bytes.len() >= 2 && &bytes[0..2] == b"MZ" {
        return Err("PE images are not relocatable objects; pass a .obj".into());
    }
    if bytes.len() < 20 {
        return Err("truncated COFF header".into());
    }

    let machine = r16(bytes, 0)?;
    let arch = arch_from_machine(machine)?;
    let nsects = r16(bytes, 2)? as usize;
    let sym_off = r32(bytes, 8)? as usize;
    let nsyms = r32(bytes, 12)? as usize;
    let opt_hdr = r16(bytes, 16)? as usize;
    let sec_table = 20 + opt_hdr;

    let mut obj = Relocatable::new(arch);
    let mut reloc_ptrs: Vec<(u32, u16)> = Vec::with_capacity(nsects);

    for i in 0..nsects {
        let off = sec_table + i * 40;
        let sh = bytes.get(off..off + 40).ok_or("section header OOB")?;
        let mut name_raw = [0u8; 8];
        name_raw.copy_from_slice(&sh[0..8]);
        let name = if name_raw[0] == b'/' {
            String::from_utf8_lossy(&name_raw).into_owned()
        } else {
            let end = name_raw.iter().position(|&c| c == 0).unwrap_or(8);
            String::from_utf8_lossy(&name_raw[..end]).into_owned()
        };
        let vsize = r32(sh, 8)? as u64;
        let raw_size = r32(sh, 16)? as usize;
        let raw_ptr = r32(sh, 20)? as usize;
        let reloc_ptr = r32(sh, 24)?;
        let nreloc = r16(sh, 32)?;
        let chars = r32(sh, 36)?;
        let kind = sec_kind(&name, chars);
        let sec_data = if raw_size > 0 && raw_ptr > 0 {
            bytes
                .get(raw_ptr..raw_ptr + raw_size)
                .ok_or("section data OOB")?
                .to_vec()
        } else {
            Vec::new()
        };
        let size = if kind == SecKind::Bss {
            vsize.max(sec_data.len() as u64)
        } else {
            sec_data.len() as u64
        };
        obj.sections.push(Sec {
            name,
            kind,
            data: sec_data,
            align: 16,
            size,
            flags: chars as u64,
        });
        reloc_ptrs.push((reloc_ptr, nreloc));
    }

    let str_off = sym_off.saturating_add(nsyms.saturating_mul(18));
    let string_table = if str_off + 4 <= bytes.len() {
        let len = r32(bytes, str_off)? as usize;
        bytes
            .get(str_off..str_off + len.max(4))
            .unwrap_or(&[])
            .to_vec()
    } else {
        Vec::new()
    };

    for sec in &mut obj.sections {
        if let Some(rest) = sec.name.strip_prefix('/') {
            if let Ok(off) = rest.trim_end_matches('\0').parse::<usize>() {
                let slice = string_table.get(off..).unwrap_or(&[]);
                let end = slice.iter().position(|&c| c == 0).unwrap_or(slice.len());
                sec.name = String::from_utf8_lossy(&slice[..end]).into_owned();
                sec.kind = sec_kind(&sec.name, sec.flags as u32);
            }
        }
    }

    // Compact symbol list + COFF index map (aux entries skipped).
    let mut coff_to_ours: Vec<Option<usize>> = vec![None; nsyms];
    let mut i = 0usize;
    while i < nsyms {
        let e = bytes
            .get(sym_off + i * 18..sym_off + i * 18 + 18)
            .ok_or("symbol OOB")?;
        let mut name_raw = [0u8; 8];
        name_raw.copy_from_slice(&e[0..8]);
        let name = read_name(&name_raw, &string_table);
        let value = r32(e, 8)? as u64;
        let sec_num = r16(e, 12)? as i16;
        let typ = r16(e, 14)?;
        let storage = e[16];
        let naux = e[17] as usize;

        let section = if sec_num > 0 {
            Some((sec_num as usize) - 1)
        } else {
            None
        };
        let bind = match storage {
            2 => SymBind::Global,
            105 => SymBind::Weak,
            _ => SymBind::Local,
        };
        let ty = if typ == 0x20 {
            SymType::Func
        } else if section.is_some() {
            SymType::Object
        } else {
            SymType::None
        };

        coff_to_ours[i] = Some(obj.symbols.len());
        obj.symbols.push(Sym {
            name,
            section,
            value,
            size: 0,
            bind,
            ty,
        });
        i += 1 + naux;
    }

    for (sec_i, &(reloc_ptr, nreloc)) in reloc_ptrs.iter().enumerate() {
        if nreloc == 0 || reloc_ptr == 0 {
            continue;
        }
        for r in 0..nreloc as usize {
            let off = reloc_ptr as usize + r * 10;
            let re = bytes.get(off..off + 10).ok_or("reloc OOB")?;
            let r_off = r32(re, 0)? as u64;
            let sym_idx = r32(re, 4)? as usize;
            let r_type = r16(re, 8)?;
            let Some((kind, addend)) = map_reloc(arch, r_type) else {
                continue;
            };
            let Some(Some(s)) = coff_to_ours.get(sym_idx).copied() else {
                continue;
            };
            obj.relocs.push(Rel {
                section: sec_i,
                offset: r_off,
                symbol: s,
                addend,
                kind,
            });
        }
    }

    Ok(obj)
}

fn align_up(v: usize, a: usize) -> usize {
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

/// Write a PE32+ console (or EFI) image from section blobs.
pub fn write_pe64(
    arch: Arch,
    entry_rva: u32,
    text: &[u8],
    rdata: &[u8],
    data: &[u8],
    efi: bool,
) -> Result<Vec<u8>, CoffError> {
    let section_align = 0x1000usize;
    let file_align = 0x200usize;
    let image_base: u64 = if efi { 0 } else { 0x140000000 };

    let mut sections: Vec<(&str, &[u8], u32)> = Vec::new();
    if !text.is_empty() {
        sections.push((".text", text, 0x60000020));
    }
    if !rdata.is_empty() {
        sections.push((".rdata", rdata, 0x40000040));
    }
    if !data.is_empty() {
        sections.push((".data", data, 0xC0000040));
    }
    if sections.is_empty() {
        sections.push((".text", &[0xC3], 0x60000020));
    }

    let nsect = sections.len() as u16;
    let headers_size = align_up(0x80 + 4 + 20 + 0xF0 + nsect as usize * 40, file_align);

    let mut va = section_align;
    let mut layouts = Vec::new();
    for (name, blob, chars) in &sections {
        let vsize = align_up(blob.len().max(1), section_align);
        let raw = align_up(blob.len().max(1), file_align);
        layouts.push((*name, *blob, *chars, va, vsize, raw));
        va += vsize;
    }
    let size_of_image = va as u32;

    let mut out = Vec::new();
    out.extend_from_slice(b"MZ");
    out.resize(0x3C, 0);
    push_u32(&mut out, 0x80);
    out.resize(0x80, 0);
    out.extend_from_slice(b"PE\0\0");

    push_u16(&mut out, arch.coff_machine());
    push_u16(&mut out, nsect);
    push_u32(&mut out, 0);
    push_u32(&mut out, 0);
    push_u32(&mut out, 0);
    push_u16(&mut out, 0xF0);
    push_u16(&mut out, if efi { 0x2022 } else { 0x0022 });

    push_u16(&mut out, 0x20B);
    push_u16(&mut out, 0);
    let text_raw: u32 = layouts
        .iter()
        .find(|l| l.0 == ".text")
        .map(|l| l.5 as u32)
        .unwrap_or(0);
    push_u32(&mut out, text_raw);
    push_u32(&mut out, 0);
    push_u32(&mut out, 0);
    push_u32(&mut out, entry_rva);
    push_u32(
        &mut out,
        layouts
            .iter()
            .find(|l| l.0 == ".text")
            .map(|l| l.3 as u32)
            .unwrap_or(section_align as u32),
    );
    push_u64(&mut out, image_base);
    push_u32(&mut out, section_align as u32);
    push_u32(&mut out, file_align as u32);
    push_u16(&mut out, 6);
    push_u16(&mut out, 0);
    push_u16(&mut out, 0);
    push_u16(&mut out, 0);
    push_u16(&mut out, 6);
    push_u16(&mut out, 0);
    push_u32(&mut out, 0);
    push_u32(&mut out, size_of_image);
    push_u32(&mut out, headers_size as u32);
    push_u32(&mut out, 0);
    push_u16(&mut out, if efi { 10 } else { 3 });
    push_u16(&mut out, 0x8160);
    push_u64(&mut out, 0x100000);
    push_u64(&mut out, 0x1000);
    push_u64(&mut out, 0x100000);
    push_u64(&mut out, 0x1000);
    push_u32(&mut out, 0);
    push_u32(&mut out, 16);
    for _ in 0..16 {
        push_u32(&mut out, 0);
        push_u32(&mut out, 0);
    }

    let mut file_off = headers_size;
    for (name, _blob, chars, va, vsize, raw) in &layouts {
        let mut nm = [0u8; 8];
        let nb = name.as_bytes();
        nm[..nb.len().min(8)].copy_from_slice(&nb[..nb.len().min(8)]);
        out.extend_from_slice(&nm);
        push_u32(&mut out, *vsize as u32);
        push_u32(&mut out, *va as u32);
        push_u32(&mut out, *raw as u32);
        push_u32(&mut out, file_off as u32);
        push_u32(&mut out, 0);
        push_u32(&mut out, 0);
        push_u16(&mut out, 0);
        push_u16(&mut out, 0);
        push_u32(&mut out, *chars);
        file_off += raw;
    }
    out.resize(headers_size, 0);

    for (_name, blob, _c, _va, _vs, raw) in &layouts {
        let mut padded = blob.to_vec();
        padded.resize(*raw, 0);
        out.extend_from_slice(&padded);
    }
    Ok(out)
}
