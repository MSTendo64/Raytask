//! Minimal C header parser — extract function prototypes for RayTask FFI.
//!
//! Supported subset:
//! - `//` and `/* */` comments
//! - `#include "local.h"` (relative; system `<...>` skipped)
//! - `typedef` aliases for scalar / pointer types
//! - Opaque `struct` / `union` tags registered as `Ptr` for FFI params
//! - Function prototypes: `Ret name(T a, U b);`
//! - Common stdint / Windows typedefs built-in

use crate::error::{CompileError, CompileResult};
use crate::ffi::FfiType;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct CPrototype {
    pub name: String,
    pub params: Vec<FfiType>,
    pub ret: FfiType,
    /// Original C parameter type strings (for diagnostics / codegen).
    pub param_names: Vec<String>,
}

#[derive(Debug, Clone, Default)]
pub struct CHeader {
    pub prototypes: Vec<CPrototype>,
    pub typedefs: HashMap<String, FfiType>,
    pub includes_resolved: Vec<PathBuf>,
}

/// Parse a header file (and local `#include "..."` recursively).
pub fn parse_header_file(path: &Path) -> CompileResult<CHeader> {
    let mut visited = HashSet::new();
    let mut header = CHeader::default();
    seed_builtins(&mut header.typedefs);
    parse_header_recursive(path, &mut header, &mut visited, 0)?;
    Ok(header)
}

/// Parse header text with an optional base directory for `#include "..."`.
pub fn parse_header_source(source: &str, base_dir: &Path) -> CompileResult<CHeader> {
    let mut visited = HashSet::new();
    let mut header = CHeader::default();
    seed_builtins(&mut header.typedefs);
    parse_source_into(source, base_dir, &mut header, &mut visited, 0)?;
    Ok(header)
}

fn seed_builtins(td: &mut HashMap<String, FfiType>) {
    let pairs = [
        ("void", FfiType::Void),
        ("bool", FfiType::Bool),
        ("_Bool", FfiType::Bool),
        ("char", FfiType::I8),
        ("signed", FfiType::I32),
        ("unsigned", FfiType::U32),
        ("short", FfiType::I16),
        ("int", FfiType::I32),
        ("long", FfiType::I64),
        ("float", FfiType::F32),
        ("double", FfiType::F64),
        ("size_t", FfiType::U64),
        ("ssize_t", FfiType::I64),
        ("ptrdiff_t", FfiType::I64),
        ("intptr_t", FfiType::I64),
        ("uintptr_t", FfiType::U64),
        ("int8_t", FfiType::I8),
        ("int16_t", FfiType::I16),
        ("int32_t", FfiType::I32),
        ("int64_t", FfiType::I64),
        ("uint8_t", FfiType::U8),
        ("uint16_t", FfiType::U16),
        ("uint32_t", FfiType::U32),
        ("uint64_t", FfiType::U64),
        ("BYTE", FfiType::U8),
        ("WORD", FfiType::U16),
        ("DWORD", FfiType::U32),
        ("QWORD", FfiType::U64),
        ("BOOL", FfiType::I32),
        ("BOOLEAN", FfiType::U8),
        ("HANDLE", FfiType::Ptr),
        ("HMODULE", FfiType::Ptr),
        ("HWND", FfiType::Ptr),
        ("LPVOID", FfiType::Ptr),
        ("PVOID", FfiType::Ptr),
        ("LPCSTR", FfiType::CString),
        ("LPSTR", FfiType::CString),
        ("LPCWSTR", FfiType::Ptr),
        ("LPWSTR", FfiType::Ptr),
    ];
    for (k, v) in pairs {
        td.insert(k.to_string(), v);
    }
}

fn parse_header_recursive(
    path: &Path,
    header: &mut CHeader,
    visited: &mut HashSet<PathBuf>,
    depth: usize,
) -> CompileResult<()> {
    if depth > 32 {
        return Err(CompileError::Io {
            message: format!("C header include depth exceeded at {}", path.display()),
        });
    }
    let canon = path
        .canonicalize()
        .unwrap_or_else(|_| path.to_path_buf());
    if !visited.insert(canon.clone()) {
        return Ok(());
    }
    let source = std::fs::read_to_string(path).map_err(|e| CompileError::Io {
        message: format!("cannot read C header '{}': {e}", path.display()),
    })?;
    header.includes_resolved.push(path.to_path_buf());
    let base = path.parent().unwrap_or_else(|| Path::new("."));
    parse_source_into(&source, base, header, visited, depth)
}

fn parse_source_into(
    source: &str,
    base_dir: &Path,
    header: &mut CHeader,
    visited: &mut HashSet<PathBuf>,
    depth: usize,
) -> CompileResult<()> {
    let text = strip_comments(source);
    let mut i = 0;
    let bytes = text.as_bytes();
    while i < bytes.len() {
        skip_ws(&text, &mut i);
        if i >= bytes.len() {
            break;
        }
        // preprocessor
        if bytes[i] == b'#' {
            let line_end = text[i..]
                .find('\n')
                .map(|n| i + n)
                .unwrap_or(text.len());
            let directive = text[i..line_end].trim();
            if let Some(rest) = directive.strip_prefix("#include") {
                let rest = rest.trim();
                if let Some(name) = rest.strip_prefix('"').and_then(|s| s.strip_suffix('"')) {
                    let inc = base_dir.join(name);
                    if inc.is_file() {
                        parse_header_recursive(&inc, header, visited, depth + 1)?;
                    }
                }
                // skip <system.h>
            }
            i = if line_end < text.len() {
                line_end + 1
            } else {
                text.len()
            };
            continue;
        }

        // typedef
        if starts_with_word(&text, i, "typedef") {
            i += "typedef".len();
            skip_ws(&text, &mut i);
            if let Some((alias, ty)) = parse_typedef(&text, &mut i, &header.typedefs) {
                header.typedefs.insert(alias, ty);
            } else {
                // skip until ;
                if let Some(n) = text[i..].find(';') {
                    i += n + 1;
                } else {
                    break;
                }
            }
            continue;
        }

        // struct/union: register tag as opaque Ptr, then skip body/definition
        if starts_with_word(&text, i, "struct") || starts_with_word(&text, i, "union") {
            let is_struct = starts_with_word(&text, i, "struct");
            i += if is_struct { "struct".len() } else { "union".len() };
            skip_ws(&text, &mut i);
            if let Some(tag) = read_ident(&text, &mut i) {
                header.typedefs.entry(tag).or_insert(FfiType::Ptr);
            }
            skip_decl_or_def(&text, &mut i);
            continue;
        }

        // skip enum definitions and lone ;
        if starts_with_word(&text, i, "enum")
            || starts_with_word(&text, i, "extern")
        {
            // May be `extern Ret name(...);` — handle extern by skipping keyword
            if starts_with_word(&text, i, "extern") {
                i += "extern".len();
                skip_ws(&text, &mut i);
                // fall through to prototype parse
            } else {
                skip_decl_or_def(&text, &mut i);
                continue;
            }
        }

        if bytes[i] == b';' {
            i += 1;
            continue;
        }

        // Try function prototype
        let start = i;
        if let Some(proto) = try_parse_prototype(&text, &mut i, &header.typedefs) {
            // Dedup by name (first wins)
            if !header.prototypes.iter().any(|p| p.name == proto.name) {
                header.prototypes.push(proto);
            }
            continue;
        }
        // Recovery: advance one token / to next ;
        i = start;
        if let Some(n) = text[i..].find(';') {
            i += n + 1;
        } else if let Some(n) = text[i..].find('\n') {
            i += n + 1;
        } else {
            break;
        }
    }
    Ok(())
}

fn strip_comments(src: &str) -> String {
    let mut out = String::with_capacity(src.len());
    let b = src.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if i + 1 < b.len() && b[i] == b'/' && b[i + 1] == b'/' {
            i += 2;
            while i < b.len() && b[i] != b'\n' {
                i += 1;
            }
            continue;
        }
        if i + 1 < b.len() && b[i] == b'/' && b[i + 1] == b'*' {
            i += 2;
            while i + 1 < b.len() && !(b[i] == b'*' && b[i + 1] == b'/') {
                i += 1;
            }
            i = (i + 2).min(b.len());
            out.push(' ');
            continue;
        }
        // keep strings intact enough for #include "..."
        if b[i] == b'"' {
            out.push('"');
            i += 1;
            while i < b.len() {
                out.push(b[i] as char);
                if b[i] == b'\\' && i + 1 < b.len() {
                    out.push(b[i + 1] as char);
                    i += 2;
                    continue;
                }
                if b[i] == b'"' {
                    i += 1;
                    break;
                }
                i += 1;
            }
            continue;
        }
        out.push(b[i] as char);
        i += 1;
    }
    out
}

fn skip_ws(text: &str, i: &mut usize) {
    let b = text.as_bytes();
    while *i < b.len() && b[*i].is_ascii_whitespace() {
        *i += 1;
    }
}

fn starts_with_word(text: &str, i: usize, word: &str) -> bool {
    let b = text.as_bytes();
    if i + word.len() > b.len() {
        return false;
    }
    if text[i..i + word.len()] != *word {
        return false;
    }
    let after = i + word.len();
    after >= b.len()
        || !(b[after].is_ascii_alphanumeric() || b[after] == b'_')
}

fn read_ident(text: &str, i: &mut usize) -> Option<String> {
    skip_ws(text, i);
    let b = text.as_bytes();
    if *i >= b.len() {
        return None;
    }
    if !(b[*i].is_ascii_alphabetic() || b[*i] == b'_') {
        return None;
    }
    let start = *i;
    *i += 1;
    while *i < b.len() && (b[*i].is_ascii_alphanumeric() || b[*i] == b'_') {
        *i += 1;
    }
    Some(text[start..*i].to_string())
}

fn parse_typedef(
    text: &str,
    i: &mut usize,
    typedefs: &HashMap<String, FfiType>,
) -> Option<(String, FfiType)> {
    // typedef TYPE alias;
    // typedef TYPE *alias;
    let ty = parse_c_type(text, i, typedefs)?;
    skip_ws(text, i);
    let b = text.as_bytes();
    let mut ptr = ty;
    while *i < b.len() && b[*i] == b'*' {
        ptr = FfiType::Ptr;
        *i += 1;
        skip_ws(text, i);
    }
    let alias = read_ident(text, i)?;
    skip_ws(text, i);
    if *i < b.len() && b[*i] == b';' {
        *i += 1;
        return Some((alias, ptr));
    }
    None
}

fn skip_decl_or_def(text: &str, i: &mut usize) {
    let b = text.as_bytes();
    let mut depth = 0i32;
    while *i < b.len() {
        let c = b[*i];
        *i += 1;
        match c {
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    skip_ws(text, i);
                    if *i < b.len() && b[*i] == b';' {
                        *i += 1;
                    }
                    return;
                }
            }
            b';' if depth == 0 => return,
            _ => {}
        }
    }
}

fn try_parse_prototype(
    text: &str,
    i: &mut usize,
    typedefs: &HashMap<String, FfiType>,
) -> Option<CPrototype> {
    let checkpoint = *i;
    // optional storage: static inline …
    loop {
        skip_ws(text, i);
        if starts_with_word(text, *i, "static")
            || starts_with_word(text, *i, "inline")
            || starts_with_word(text, *i, "__inline")
            || starts_with_word(text, *i, "__declspec")
        {
            if starts_with_word(text, *i, "__declspec") {
                *i += "__declspec".len();
                skip_ws(text, i);
                // skip (dllimport)
                let b = text.as_bytes();
                if *i < b.len() && b[*i] == b'(' {
                    *i += 1;
                    while *i < b.len() && b[*i] != b')' {
                        *i += 1;
                    }
                    if *i < b.len() {
                        *i += 1;
                    }
                }
            } else if starts_with_word(text, *i, "static") {
                *i += "static".len();
            } else if starts_with_word(text, *i, "inline") {
                *i += "inline".len();
            } else {
                *i += "__inline".len();
            }
            continue;
        }
        break;
    }

    let mut ret = parse_c_type(text, i, typedefs)?;
    skip_ws(text, i);
    let b = text.as_bytes();
    // pointers before name: int *foo(
    while *i < b.len() && b[*i] == b'*' {
        ret = if ret == FfiType::I8 || ret == FfiType::U8 {
            // char* → CString preference
            FfiType::CString
        } else {
            FfiType::Ptr
        };
        *i += 1;
        skip_ws(text, i);
    }

    let name = match read_ident(text, i) {
        Some(n) => n,
        None => {
            *i = checkpoint;
            return None;
        }
    };
    // Skip if this looks like a variable: name;
    skip_ws(text, i);
    if *i >= b.len() || b[*i] != b'(' {
        *i = checkpoint;
        return None;
    }
    *i += 1; // (

    let mut params = Vec::new();
    let mut param_names = Vec::new();
    skip_ws(text, i);
    // empty / void parameter list
    if starts_with_word(text, *i, "void") {
        let after = *i + 4;
        let b2 = text.as_bytes();
        let void_only = after >= b2.len()
            || (!b2[after].is_ascii_alphanumeric() && b2[after] != b'_');
        if void_only {
            *i = after;
            skip_ws(text, i);
            if *i < b.len() && b[*i] == b')' {
                *i += 1;
            }
            skip_ws(text, i);
            if *i < b.len() && b[*i] == b'{' {
                skip_decl_or_def(text, i);
                return Some(CPrototype {
                    name,
                    params,
                    ret,
                    param_names,
                });
            }
            if *i < b.len() && b[*i] == b';' {
                *i += 1;
                return Some(CPrototype {
                    name,
                    params,
                    ret,
                    param_names,
                });
            }
            *i = checkpoint;
            return None;
        }
    }

    while *i < b.len() && b[*i] != b')' {
        skip_ws(text, i);
        if *i < b.len() && b[*i] == b')' {
            break;
        }
        if *i < b.len() && b[*i] == b'.' {
            // varargs — stop, treat remaining as unsupported (ignore ...)
            while *i < b.len() && b[*i] != b')' {
                *i += 1;
            }
            break;
        }
        let pty = match parse_c_type(text, i, typedefs) {
            Some(t) => t,
            None => {
                *i = checkpoint;
                return None;
            }
        };
        skip_ws(text, i);
        let mut pty = pty;
        while *i < b.len() && b[*i] == b'*' {
            pty = if matches!(pty, FfiType::I8 | FfiType::U8) {
                FfiType::CString
            } else {
                FfiType::Ptr
            };
            *i += 1;
            skip_ws(text, i);
        }
        // optional param name / array brackets
        let _ = read_ident(text, i);
        skip_ws(text, i);
        while *i < b.len() && b[*i] == b'[' {
            while *i < b.len() && b[*i] != b']' {
                *i += 1;
            }
            if *i < b.len() {
                *i += 1;
            }
            pty = FfiType::Ptr;
            skip_ws(text, i);
        }
        params.push(pty);
        param_names.push(String::new());
        skip_ws(text, i);
        if *i < b.len() && b[*i] == b',' {
            *i += 1;
            continue;
        }
        break;
    }
    skip_ws(text, i);
    if *i >= b.len() || b[*i] != b')' {
        *i = checkpoint;
        return None;
    }
    *i += 1;
    skip_ws(text, i);
    // prototype ends with ;  (skip function bodies)
    if *i < b.len() && b[*i] == b'{' {
        skip_decl_or_def(text, i);
        return Some(CPrototype {
            name,
            params,
            ret,
            param_names,
        });
    }
    if *i < b.len() && b[*i] == b';' {
        *i += 1;
        return Some(CPrototype {
            name,
            params,
            ret,
            param_names,
        });
    }
    *i = checkpoint;
    None
}

fn parse_c_type(
    text: &str,
    i: &mut usize,
    typedefs: &HashMap<String, FfiType>,
) -> Option<FfiType> {
    skip_ws(text, i);
    // qualifiers
    loop {
        if starts_with_word(text, *i, "const")
            || starts_with_word(text, *i, "volatile")
            || starts_with_word(text, *i, "restrict")
        {
            let _ = read_ident(text, i);
            skip_ws(text, i);
            continue;
        }
        break;
    }

    let mut signed = true;
    let mut unsigned = false;
    let mut long_count = 0;
    let mut short = false;
    let mut saw_int = false;
    let mut base: Option<FfiType> = None;

    // Collect size/sign keywords, then one base type token.
    loop {
        skip_ws(text, i);
        if starts_with_word(text, *i, "unsigned") {
            unsigned = true;
            signed = false;
            *i += "unsigned".len();
            continue;
        }
        if starts_with_word(text, *i, "signed") {
            signed = true;
            unsigned = false;
            *i += "signed".len();
            continue;
        }
        if starts_with_word(text, *i, "long") {
            long_count += 1;
            *i += "long".len();
            continue;
        }
        if starts_with_word(text, *i, "short") {
            short = true;
            *i += "short".len();
            continue;
        }
        break;
    }

    skip_ws(text, i);
    if starts_with_word(text, *i, "int") {
        saw_int = true;
        *i += "int".len();
    } else if starts_with_word(text, *i, "char") {
        *i += "char".len();
        base = Some(if unsigned { FfiType::U8 } else { FfiType::I8 });
    } else if starts_with_word(text, *i, "float") {
        *i += "float".len();
        base = Some(FfiType::F32);
    } else if starts_with_word(text, *i, "double") {
        *i += "double".len();
        base = Some(FfiType::F64);
    } else if starts_with_word(text, *i, "void") {
        *i += "void".len();
        base = Some(FfiType::Void);
    } else if starts_with_word(text, *i, "struct")
        || starts_with_word(text, *i, "enum")
        || starts_with_word(text, *i, "union")
    {
        let _ = read_ident(text, i);
        let _ = read_ident(text, i);
        base = Some(FfiType::Ptr);
    } else if let Some(id) = {
        // Peek: only consume ident if it is a known type / looks like a type name
        // (not followed immediately by '(' which would mean we ate the function name).
        let save = *i;
        let id = read_ident(text, i);
        if let Some(ref name) = id {
            skip_ws(text, i);
            let b = text.as_bytes();
            if *i < b.len() && b[*i] == b'(' {
                // This was the function name — rewind
                *i = save;
                None
            } else if typedefs.contains_key(name)
                || name.ends_with("_t")
                || name.chars().all(|c| c.is_ascii_uppercase() || c == '_')
            {
                Some(name.clone())
            } else if unsigned || signed && (long_count > 0 || short || saw_int) {
                // already have enough from modifiers; treat unknown as rewind
                *i = save;
                None
            } else {
                // Assume typedef-like identifier type
                Some(name.clone())
            }
        } else {
            None
        }
    } {
        if let Some(t) = typedefs.get(&id) {
            base = Some(t.clone());
        } else {
            base = Some(FfiType::Ptr);
        }
    }

    if base.is_none() {
        if short {
            base = Some(if unsigned { FfiType::U16 } else { FfiType::I16 });
        } else if long_count >= 1 {
            base = Some(if unsigned { FfiType::U64 } else { FfiType::I64 });
        } else if saw_int || unsigned {
            base = Some(if unsigned { FfiType::U32 } else { FfiType::I32 });
        } else if signed && !unsigned && long_count == 0 && !short {
            // bare "signed" → int
            base = Some(FfiType::I32);
        }
    }

    // Ignore unused `signed` warning path
    let _ = signed;

    base
}

/// Emit RayTask-facing FFI decls text for documentation / `raytask bind`.
pub fn prototypes_to_raytask(lib: &str, protos: &[CPrototype]) -> String {
    let mut out = format!("[DllImport: \"{}\"]\n", lib);
    for p in protos {
        let ret = ffi_type_to_rt(&p.ret);
        let args: Vec<String> = p
            .params
            .iter()
            .enumerate()
            .map(|(i, t)| format!("a{}: {}", i, ffi_type_to_rt(t)))
            .collect();
        out.push_str(&format!(
            "{} {}({});\n",
            ret,
            p.name,
            args.join(", ")
        ));
    }
    out
}

fn ffi_type_to_rt(t: &FfiType) -> String {
    match t {
        FfiType::Void => "void".into(),
        FfiType::Bool => "bool".into(),
        FfiType::I8 => "byte".into(),
        FfiType::I16 => "short".into(),
        FfiType::I32 => "int".into(),
        FfiType::I64 => "long".into(),
        FfiType::U8 => "ubyte".into(),
        FfiType::U16 => "ushort".into(),
        FfiType::U32 => "uint".into(),
        FfiType::U64 => "ulong".into(),
        FfiType::F32 => "float".into(),
        FfiType::F64 => "double".into(),
        FfiType::Ptr => "ptr".into(),
        FfiType::CString => "string".into(),
        FfiType::Struct(s) => s.name.clone(),
    }
}

/// Resolve a header path relative to a source file / working dir.
pub fn resolve_header(name: &str, from_file: Option<&Path>) -> PathBuf {
    let p = PathBuf::from(name);
    if p.is_file() {
        return p;
    }
    if let Some(f) = from_file {
        if let Some(dir) = f.parent() {
            let c = dir.join(name);
            if c.is_file() {
                return c;
            }
        }
    }
    p
}
