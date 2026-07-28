//! Native code generation: RTBC `Module` → relocatable `ObjectFile`.

use crate::bytecode::{Module, Op};
use crate::bytecode_format::serialize_module;
use crate::value::Value;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Arch {
    X86_64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinkTarget {
    WindowsX64,
    LinuxX64,
    MacosX64,
    /// PE32+ EFI application (.efi)
    UefiX64,
    /// Flat raw binary
    RawX64,
}

impl LinkTarget {
    pub fn parse(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().as_str() {
            "windows" | "win" | "win64" | "windows-x64" => Some(Self::WindowsX64),
            "linux" | "linux-x64" => Some(Self::LinuxX64),
            "macos" | "mac" | "darwin" | "macos-x64" | "osx" => Some(Self::MacosX64),
            "uefi" | "efi" | "uefi-x64" => Some(Self::UefiX64),
            "raw" | "bin" | "flat" => Some(Self::RawX64),
            "current" | "host" => Some(Self::host()),
            _ => None,
        }
    }

    pub fn host() -> Self {
        if cfg!(windows) {
            Self::WindowsX64
        } else if cfg!(target_os = "macos") {
            Self::MacosX64
        } else {
            Self::LinuxX64
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::WindowsX64 => "windows-x64",
            Self::LinuxX64 => "linux-x64",
            Self::MacosX64 => "macos-x64",
            Self::UefiX64 => "uefi-x64",
            Self::RawX64 => "raw-x64",
        }
    }

    pub fn is_freestanding(self) -> bool {
        matches!(self, Self::UefiX64 | Self::RawX64)
    }

    pub fn default_ext(self) -> &'static str {
        match self {
            Self::WindowsX64 => "exe",
            Self::LinuxX64 => "elf",
            Self::MacosX64 => "macho",
            Self::UefiX64 => "efi",
            Self::RawX64 => "bin",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SectionKind {
    Text,
    Rodata,
    Data,
    Bss,
}

#[derive(Debug, Clone)]
pub struct Section {
    pub name: String,
    pub kind: SectionKind,
    pub data: Vec<u8>,
    pub align: u32,
    pub vma: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SymbolKind {
    Func,
    Object,
    Absolute,
}

#[derive(Debug, Clone)]
pub struct Symbol {
    pub name: String,
    pub section: Option<usize>,
    pub offset: u64,
    pub size: u64,
    pub kind: SymbolKind,
    pub global: bool,
}

#[derive(Debug, Clone)]
pub struct Reloc {
    pub section: usize,
    pub offset: u64,
    pub symbol: String,
    pub addend: i64,
}

#[derive(Debug, Clone)]
pub struct ObjectFile {
    pub arch: Arch,
    pub target: LinkTarget,
    pub sections: Vec<Section>,
    pub symbols: Vec<Symbol>,
    pub relocs: Vec<Reloc>,
    /// Serialized RTBC payload (always present).
    pub rtbc: Vec<u8>,
    /// Generated C source for host or UEFI (when applicable).
    pub c_source: Option<String>,
    /// Entry symbol name.
    pub entry: String,
    /// Notes for the linker / user.
    pub notes: Vec<String>,
}

impl ObjectFile {
    pub fn section(&self, kind: SectionKind) -> Option<&Section> {
        self.sections.iter().find(|s| s.kind == kind)
    }

    pub fn section_mut(&mut self, kind: SectionKind) -> Option<&mut Section> {
        self.sections.iter_mut().find(|s| s.kind == kind)
    }

    pub fn rodata_payload(&self) -> &[u8] {
        &self.rtbc
    }
}

/// Codegen options.
#[derive(Debug, Clone)]
pub struct CodegenNativeOptions {
    pub target: LinkTarget,
    pub name: String,
    /// Write generated `.c` under this directory when set.
    pub out_dir: Option<PathBuf>,
    /// Raw / UEFI load address (default 0x100000).
    pub load_address: u64,
}

impl Default for CodegenNativeOptions {
    fn default() -> Self {
        Self {
            target: LinkTarget::host(),
            name: "app".into(),
            out_dir: None,
            load_address: 0x100000,
        }
    }
}

/// Lower a bytecode module to an `ObjectFile` for the given link target.
pub fn codegen(module: &Module, opts: &CodegenNativeOptions) -> ObjectFile {
    let rtbc = serialize_module(module);
    let mut obj = ObjectFile {
        arch: Arch::X86_64,
        target: opts.target,
        sections: Vec::new(),
        symbols: Vec::new(),
        relocs: Vec::new(),
        rtbc: rtbc.clone(),
        c_source: None,
        entry: if opts.target.is_freestanding() {
            "efi_main".into()
        } else {
            "main".into()
        },
        notes: Vec::new(),
    };

    // .rodata — RTBC payload
    let rodata_idx = obj.sections.len();
    obj.sections.push(Section {
        name: ".rodata".into(),
        kind: SectionKind::Rodata,
        data: rtbc.clone(),
        align: 16,
        vma: opts.load_address + 0x2000,
    });
    obj.symbols.push(Symbol {
        name: "rtbc_payload".into(),
        section: Some(rodata_idx),
        offset: 0,
        size: rtbc.len() as u64,
        kind: SymbolKind::Object,
        global: true,
    });
    obj.symbols.push(Symbol {
        name: "rtbc_len".into(),
        section: None,
        offset: rtbc.len() as u64,
        size: 8,
        kind: SymbolKind::Absolute,
        global: true,
    });

    match opts.target {
        LinkTarget::UefiX64 | LinkTarget::RawX64 => {
            let c = generate_uefi_c_from_module(module, &rtbc, &opts.name);
            obj.c_source = Some(c.clone());
            let text = uefi_stub_text();
            let text_idx = obj.sections.len();
            obj.sections.push(Section {
                name: ".text".into(),
                kind: SectionKind::Text,
                data: text,
                align: 16,
                vma: opts.load_address,
            });
            obj.symbols.push(Symbol {
                name: "efi_main".into(),
                section: Some(text_idx),
                offset: 0,
                size: 6,
                kind: SymbolKind::Func,
                global: true,
            });
            obj.notes.push(
                "UEFI: freestanding C interpreter generated; link with clang for full runtime"
                    .into(),
            );
            maybe_write_c(&opts.out_dir, &opts.name, "uefi", &c);
        }
        LinkTarget::WindowsX64 | LinkTarget::LinuxX64 | LinkTarget::MacosX64 => {
            let c = generate_host_c(&rtbc, &opts.name);
            obj.c_source = Some(c.clone());
            // Host .text placeholder (real binary uses runtime stub packaging)
            let text_idx = obj.sections.len();
            obj.sections.push(Section {
                name: ".text".into(),
                kind: SectionKind::Text,
                data: host_stub_text(),
                align: 16,
                vma: 0x1000,
            });
            obj.symbols.push(Symbol {
                name: "main".into(),
                section: Some(text_idx),
                offset: 0,
                size: 1,
                kind: SymbolKind::Func,
                global: true,
            });
            obj.notes.push(
                "Host: ObjectFile embeds RTBC; Linker packages runtime stub + payload".into(),
            );
            maybe_write_c(&opts.out_dir, &opts.name, "host", &c);
        }
    }

    obj
}

fn maybe_write_c(out_dir: &Option<PathBuf>, name: &str, kind: &str, c: &str) {
    let Some(dir) = out_dir else { return };
    let _ = std::fs::create_dir_all(dir);
    let path = dir.join(format!("{}_{}.c", name, kind));
    let _ = std::fs::write(path, c);
}

/// Minimal x86_64: xor eax,eax; ret  (return 0)
fn host_stub_text() -> Vec<u8> {
    vec![0x31, 0xC0, 0xC3]
}

/// Minimal EFI entry: xor eax,eax; ret (EFI_SUCCESS = 0)
fn uefi_stub_text() -> Vec<u8> {
    vec![0x31, 0xC0, 0xC3]
}

fn c_byte_array(bytes: &[u8], name: &str) -> String {
    let mut out = format!("static const unsigned char {}[] = {{\n", name);
    for (i, b) in bytes.iter().enumerate() {
        if i % 16 == 0 {
            out.push_str("  ");
        }
        out.push_str(&format!("0x{:02x},", b));
        if i % 16 == 15 {
            out.push('\n');
        } else {
            out.push(' ');
        }
    }
    if !bytes.is_empty() && bytes.len() % 16 != 0 {
        out.push('\n');
    }
    out.push_str("};\n");
    out.push_str(&format!(
        "static const unsigned long {}_len = {};\n",
        name,
        bytes.len()
    ));
    out
}

fn generate_host_c(rtbc: &[u8], name: &str) -> String {
    let mut s = String::new();
    s.push_str("/* Generated by RayTask NativeCodeGen — host runner */\n");
    s.push_str(&format!("/* app: {} */\n", name));
    s.push_str("#include <stdio.h>\n#include <stdint.h>\n#include <stddef.h>\n\n");
    s.push_str(&c_byte_array(rtbc, "rtbc_payload"));
    s.push_str(
        r#"
/* Prefer linking via: raytask build --target native-bin
 * This C file documents the embedded payload; the Linker packages the
 * RayTask runtime stub with rtbc_payload for a runnable executable.
 */
int main(void) {
    printf("RayTask native-bin payload: %lu bytes\n", (unsigned long)rtbc_payload_len);
    printf("Run via raytask-stub or --target native-bin packaging.\n");
    return 0;
}
"#,
    );
    s
}

fn generate_uefi_c_from_module(module: &Module, rtbc: &[u8], name: &str) -> String {
    let mut s = String::from(include_str!("native_rt/uefi_main.c.template"));
    s = s.replace("{{APP_NAME}}", name);
    s = s.replace("{{RTBC_ARRAY}}", &c_byte_array(rtbc, "rtbc_payload"));
    s = s.replace("{{UEFI_PROGRAM}}", &generate_uefi_program_tables(module));
    s
}

fn escape_c_string(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 32 => out.push_str(&format!("\\x{:02x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// Emit constant pool + main chunk code for the UEFI mini-interpreter.
fn generate_uefi_program_tables(module: &Module) -> String {
    let chunk = module
        .chunks
        .get(module.main_chunk)
        .or_else(|| module.chunks.first());
    let Some(chunk) = chunk else {
        return String::from(
            "static const unsigned char uefi_code[] = {0};\n\
             static const unsigned long uefi_code_len = 0;\n\
             static const unsigned uefi_local_count = 0;\n\
             static const unsigned uefi_const_count = 0;\n\
             static const unsigned uefi_str_count = 0;\n\
             static const int uefi_const_tag[1] = {0};\n\
             static const long long uefi_const_i[1] = {0};\n\
             static const char* uefi_strings[1] = {\"\"};\n",
        );
    };

    let mut strings: Vec<String> = Vec::new();
    let mut tags: Vec<i32> = Vec::new();
    let mut ints: Vec<i64> = Vec::new();

    for c in &chunk.constants {
        match c {
            Value::Null => {
                tags.push(0);
                ints.push(0);
            }
            Value::Bool(b) => {
                tags.push(1);
                ints.push(if *b { 1 } else { 0 });
            }
            Value::Int(n) => {
                tags.push(2);
                ints.push(*n);
            }
            Value::Float(f) => {
                tags.push(2);
                ints.push(*f as i64);
            }
            Value::String(s) => {
                tags.push(3);
                let idx = strings.len() as i64;
                strings.push(s.to_string());
                ints.push(idx);
            }
            _ => {
                tags.push(0);
                ints.push(0);
            }
        }
    }

    if tags.is_empty() {
        tags.push(0);
        ints.push(0);
    }
    if strings.is_empty() {
        strings.push(String::new());
    }

    let mut out = String::new();
    out.push_str("static const unsigned char uefi_code[] = {\n");
    for (i, b) in chunk.code.iter().enumerate() {
        if i % 16 == 0 {
            out.push_str("  ");
        }
        out.push_str(&format!("0x{:02x},", b));
        if i % 16 == 15 {
            out.push('\n');
        } else {
            out.push(' ');
        }
    }
    if !chunk.code.is_empty() && chunk.code.len() % 16 != 0 {
        out.push('\n');
    }
    out.push_str("};\n");
    out.push_str(&format!(
        "static const unsigned long uefi_code_len = {};\n",
        chunk.code.len()
    ));
    out.push_str(&format!(
        "static const unsigned uefi_local_count = {};\n",
        chunk.local_count.max(8)
    ));
    out.push_str(&format!(
        "static const unsigned uefi_const_count = {};\n",
        tags.len()
    ));
    out.push_str(&format!(
        "static const unsigned uefi_str_count = {};\n",
        strings.len()
    ));
    out.push_str("static const int uefi_const_tag[] = {");
    for (i, t) in tags.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        out.push_str(&format!("{}", t));
    }
    out.push_str("};\n");
    out.push_str("static const long long uefi_const_i[] = {");
    for (i, n) in ints.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        out.push_str(&format!("{}", n));
    }
    out.push_str("};\n");
    out.push_str("static const char* uefi_strings[] = {\n");
    for s in &strings {
        out.push_str(&format!("  {},\n", escape_c_string(s)));
    }
    out.push_str("};\n");
    let _ = Op::Halt; // keep Op imported used
    out
}

/// Convenience: codegen into `out_dir/<name>_native/`.
pub fn codegen_to_dir(
    module: &Module,
    target: LinkTarget,
    out_dir: &Path,
    name: &str,
) -> ObjectFile {
    let dir = out_dir.join(format!("{}_native", name));
    let _ = std::fs::create_dir_all(&dir);
    let obj = codegen(
        module,
        &CodegenNativeOptions {
            target,
            name: name.to_string(),
            out_dir: Some(dir.clone()),
            load_address: 0x100000,
        },
    );
    // Persist RTBC beside sources
    let _ = std::fs::write(dir.join(format!("{}.rtbc", name)), &obj.rtbc);
    let _ = std::fs::write(
        dir.join("object.json"),
        format!(
            "{{\n  \"target\": \"{}\",\n  \"entry\": \"{}\",\n  \"rtbc_bytes\": {},\n  \"sections\": {}\n}}\n",
            target.name(),
            obj.entry,
            obj.rtbc.len(),
            obj.sections.len()
        ),
    );
    obj
}
