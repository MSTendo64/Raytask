//! Relocatable object IR shared by ELF and COFF parsers.

use crate::native_triple::Arch;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SecKind {
    Text,
    Rodata,
    Data,
    Bss,
    Other,
}

#[derive(Debug, Clone)]
pub struct Sec {
    pub name: String,
    pub kind: SecKind,
    pub data: Vec<u8>,
    pub align: u32,
    /// Virtual size for BSS (may exceed `data.len()`).
    pub size: u64,
    pub flags: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SymBind {
    Local,
    Global,
    Weak,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SymType {
    None,
    Func,
    Object,
    Section,
    File,
}

#[derive(Debug, Clone)]
pub struct Sym {
    pub name: String,
    /// `None` = undefined (SHN_UNDEF / section number 0).
    pub section: Option<usize>,
    pub value: u64,
    pub size: u64,
    pub bind: SymBind,
    pub ty: SymType,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RelocKind {
    /// Absolute 64-bit address.
    Abs64,
    /// Absolute 32-bit address.
    Abs32,
    /// PC-relative 32-bit (S + A - P).
    Rel32,
    /// PC-relative 64-bit.
    Rel64,
    /// Absolute 32-bit signed (x86_64).
    Abs32S,
    /// AArch64 CALL26 / JUMP26 (imm26 << 2).
    Aarch64Call26,
    /// AArch64 ADR_PREL_PG_HI21.
    Aarch64AdrPrelPgHi21,
    /// AArch64 ADD_ABS_LO12_NC / LDST*_ABS_LO12_NC (page offset bits).
    Aarch64AddAbsLo12,
    /// COFF ADDR32NB (RVA without image base) — treated as Abs32 for static link.
    Addr32Nb,
}

#[derive(Debug, Clone)]
pub struct Rel {
    pub section: usize,
    pub offset: u64,
    pub symbol: usize,
    pub addend: i64,
    pub kind: RelocKind,
}

#[derive(Debug, Clone)]
pub struct Relocatable {
    pub arch: Arch,
    pub sections: Vec<Sec>,
    pub symbols: Vec<Sym>,
    pub relocs: Vec<Rel>,
    pub entry: Option<String>,
}

impl Relocatable {
    pub fn new(arch: Arch) -> Self {
        Self {
            arch,
            sections: Vec::new(),
            symbols: Vec::new(),
            relocs: Vec::new(),
            entry: None,
        }
    }

    pub fn find_symbol(&self, name: &str) -> Option<usize> {
        self.symbols.iter().position(|s| s.name == name)
    }
}
