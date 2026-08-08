//! Minimal C header parser — extract function prototypes and structs for RayTask FFI.
//!
//! Supported subset:
//! - `//` and `/* */` comments
//! - `#include "local.h"` (relative; system `<...>` skipped)
//! - `typedef` aliases for scalar / pointer / enum / struct types
//! - `typedef struct { … } name;` → RayTask `[repr: "C"]` structs
//! - Leading attribute macros (`BGFX_C_API`, `WINAPI`, …) skipped before prototypes
//! - Function prototypes: `Ret name(T a, U b);`
//! - Common stdint / Windows typedefs built-in

use crate::ast::{Access, Attribute, Expr, FieldDecl, Member, StructDecl, TypeRef};
use crate::error::{CompileError, CompileResult};
use crate::ffi::{FfiFieldLayout, FfiStructLayout, FfiType};
use crate::span::Span;
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
    /// Structs in declaration order (dependencies first).
    pub structs: Vec<StructDecl>,
    /// Enum / `#define` integer constants (for fixed array sizes).
    pub constants: HashMap<String, i64>,
    pub includes_resolved: Vec<PathBuf>,
}

/// Maximum total prototypes parsed across all includes.
const MAX_HEADER_PROTOS: usize = 1024;
/// Maximum loop iterations in a single parse_source_into call.
const MAX_LOOP_ITERS: usize = 4096;

fn check_header_limits(header: &CHeader) -> CompileResult<()> {
    let items = header.prototypes.len() + header.structs.len() + header.constants.len();
    if items > MAX_HEADER_PROTOS {
        return Ok(()); // silently stop — enough items parsed
    }
    Ok(())
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
        ("va_list", FfiType::Ptr),
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
    // Skip files exceeding 512 KiB of raw source (allow large single headers like raylib.h)
    if source.len() > 512 * 1024 {
        return Ok(());
    }
    if header.includes_resolved.len() > 8 {
        return Ok(()); // silently stop — too many includes
    }
    header.includes_resolved.push(path.to_path_buf());
    let base = path.parent().unwrap_or_else(|| Path::new("."));
    parse_source_into(&source, base, header, visited, depth)?;
    check_header_limits(header)?;
    Ok(())
}

fn parse_source_into(
    source: &str,
    base_dir: &Path,
    header: &mut CHeader,
    visited: &mut HashSet<PathBuf>,
    depth: usize,
) -> CompileResult<()> {
    let text = strip_comments(source);
    // Individual source after comment strip must fit within reasonable bounds
    if text.len() > 512 * 1024 {
        return Ok(()); // silently skip huge files
    }
    let mut i = 0;
    let bytes = text.as_bytes();
    let mut loop_iters = 0usize;
    while i < bytes.len() {
        if loop_iters >= MAX_LOOP_ITERS {
            return Ok(()); // too many declarations — stop gracefully
        }
        loop_iters += 1;
        skip_ws(&text, &mut i);
        if i >= bytes.len() {
            break;
        }
        // preprocessor
        if bytes[i] == b'#' {
            let line_end = find_preprocessor_line_end(&text, i);
            let directive = text[i..line_end].trim();
            // Strip leading '#' and optional spaces
            let rest = directive.trim_start_matches('#').trim();
            if let Some(inc) = rest.strip_prefix("include") {
                let rest = inc.trim();
                if let Some(name) = rest.strip_prefix('"').and_then(|s| s.strip_suffix('"')) {
                    let path = base_dir.join(name);
                    if path.is_file() {
                        parse_header_recursive(&path, header, visited, depth + 1)?;
                    }
                }
            } else if let Some(def) = rest.strip_prefix("define") {
                parse_define_line(def.trim(), &mut header.constants);
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
            let start = i;
            i += "typedef".len();
            skip_ws(&text, &mut i);
            if !parse_typedef_into(&text, &mut i, header) {
                i = start + "typedef".len();
                skip_ws(&text, &mut i);
                skip_decl_or_def(&text, &mut i);
            }
            continue;
        }

        // struct/union: register tag; parse body when present
        if starts_with_word(&text, i, "struct") || starts_with_word(&text, i, "union") {
            let is_struct = starts_with_word(&text, i, "struct");
            i += if is_struct {
                "struct".len()
            } else {
                "union".len()
            };
            skip_ws(&text, &mut i);
            let tag = read_ident(&text, &mut i);
            skip_ws(&text, &mut i);
            let b = text.as_bytes();
            if i < b.len() && b[i] == b'{' {
                if let Some(tag) = tag.clone() {
                    if is_struct {
                        if let Some(layout) = parse_struct_body(&text, &mut i, &tag, header) {
                            register_struct(header, layout);
                        }
                    } else {
                        skip_decl_or_def(&text, &mut i);
                        header.typedefs.entry(tag).or_insert(FfiType::Ptr);
                    }
                } else {
                    skip_decl_or_def(&text, &mut i);
                }
            } else {
                if let Some(tag) = tag {
                    header.typedefs.entry(tag).or_insert(FfiType::Ptr);
                }
                skip_decl_or_def(&text, &mut i);
            }
            continue;
        }

        // enum definitions (non-typedef)
        if starts_with_word(&text, i, "enum") {
            i += "enum".len();
            skip_ws(&text, &mut i);
            let _ = read_ident(&text, &mut i);
            skip_ws(&text, &mut i);
            if i < bytes.len() && bytes[i] == b'{' {
                parse_enum_body(&text, &mut i, &mut header.constants);
            }
            skip_ws(&text, &mut i);
            let _ = read_ident(&text, &mut i);
            skip_ws(&text, &mut i);
            if i < bytes.len() && bytes[i] == b';' {
                i += 1;
            }
            continue;
        }

        if starts_with_word(&text, i, "extern") {
            i += "extern".len();
            skip_ws(&text, &mut i);
            // extern "C" { ... } — skip the entire linkage block
            let b = text.as_bytes();
            if i < b.len() && b[i] == b'"' {
                i += 1;
                while i < b.len() && b[i] != b'"' {
                    i += 1;
                }
                if i < b.len() {
                    i += 1;
                }
                skip_ws(&text, &mut i);
                if i < b.len() && b[i] == b'{' {
                    i += 1;
                    let mut depth = 1u32;
                    while i < b.len() && depth > 0 {
                        if b[i] == b'{' {
                            depth += 1;
                        } else if b[i] == b'}' {
                            depth -= 1;
                        }
                        i += 1;
                    }
                    continue;
                }
            }
            // fall through to prototype
        }

        if bytes[i] == b';' {
            i += 1;
            continue;
        }

        let start = i;
        if let Some(proto) = try_parse_prototype(&text, &mut i, &header.typedefs) {
            if !header.prototypes.iter().any(|p| p.name == proto.name) {
                header.prototypes.push(proto);
                // Check limits periodically
                if header.prototypes.len() + header.structs.len() > MAX_HEADER_PROTOS {
                    return Ok(()); // silently stop — enough declarations parsed
                }
            }
            continue;
        }
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

fn find_preprocessor_line_end(text: &str, start: usize) -> usize {
    let b = text.as_bytes();
    let mut i = start;
    while i < b.len() {
        if b[i] == b'\\' && i + 1 < b.len() && (b[i + 1] == b'\n' || b[i + 1] == b'\r') {
            i += 2;
            if i < b.len() && b[i - 1] == b'\r' && b[i] == b'\n' {
                i += 1;
            }
            continue;
        }
        if b[i] == b'\n' {
            return i;
        }
        i += 1;
    }
    text.len()
}

fn parse_define_line(rest: &str, constants: &mut HashMap<String, i64>) {
    let rest = rest.trim();
    if rest.is_empty() {
        return;
    }
    // Skip function-like macros: NAME(
    let mut chars = rest.char_indices();
    let Some((_, c0)) = chars.next() else {
        return;
    };
    if !(c0.is_ascii_alphabetic() || c0 == '_') {
        return;
    }
    let mut end = 1;
    for (idx, c) in chars {
        if c.is_ascii_alphanumeric() || c == '_' {
            end = idx + c.len_utf8();
        } else {
            break;
        }
    }
    let name = &rest[..end];
    let after = rest[end..].trim_start();
    if after.starts_with('(') {
        return; // function-like
    }
    if after.is_empty() {
        return;
    }
    // Only record simple integer literals
    let lit = after
        .split_whitespace()
        .next()
        .unwrap_or("")
        .trim_end_matches('u')
        .trim_end_matches('U')
        .trim_end_matches('l')
        .trim_end_matches('L');
    if let Some(v) = parse_c_int_literal(lit) {
        constants.insert(name.to_string(), v);
    }
}

fn parse_c_int_literal(s: &str) -> Option<i64> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    // UINT64_C(0x…) / UINT32_C(…)
    if let Some(inner) = s
        .strip_prefix("UINT64_C(")
        .or_else(|| s.strip_prefix("UINT32_C("))
        .or_else(|| s.strip_prefix("INT64_C("))
        .or_else(|| s.strip_prefix("INT32_C("))
        .and_then(|x| x.strip_suffix(')'))
    {
        return parse_c_int_literal(inner);
    }
    let (neg, s) = if let Some(rest) = s.strip_prefix('-') {
        (true, rest)
    } else {
        (false, s)
    };
    let v = if let Some(hex) = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
        i64::from_str_radix(hex, 16).ok()?
    } else {
        s.parse::<i64>().ok()?
    };
    Some(if neg { -v } else { v })
}

fn parse_typedef_into(text: &str, i: &mut usize, header: &mut CHeader) -> bool {
    skip_ws(text, i);
    // typedef enum …
    if starts_with_word(text, *i, "enum") {
        *i += "enum".len();
        skip_ws(text, i);
        let _ = read_ident(text, i);
        skip_ws(text, i);
        let b = text.as_bytes();
        if *i < b.len() && b[*i] == b'{' {
            parse_enum_body(text, i, &mut header.constants);
        }
        skip_ws(text, i);
        let Some(alias) = read_ident(text, i) else {
            return false;
        };
        skip_ws(text, i);
        if *i < b.len() && b[*i] == b';' {
            *i += 1;
        }
        header.typedefs.insert(alias, FfiType::I32);
        return true;
    }

    // typedef struct / union …
    if starts_with_word(text, *i, "struct") || starts_with_word(text, *i, "union") {
        let is_struct = starts_with_word(text, *i, "struct");
        *i += if is_struct {
            "struct".len()
        } else {
            "union".len()
        };
        skip_ws(text, i);
        let tag = read_ident(text, i);
        skip_ws(text, i);
        let b = text.as_bytes();
        if *i < b.len() && b[*i] == b'{' {
            if !is_struct {
                skip_decl_or_def(text, i);
                skip_ws(text, i);
                if let Some(alias) = read_ident(text, i) {
                    header.typedefs.insert(alias, FfiType::Ptr);
                }
                skip_ws(text, i);
                if *i < b.len() && b[*i] == b';' {
                    *i += 1;
                }
                return true;
            }
            let temp_name = tag
                .clone()
                .unwrap_or_else(|| format!("__anon_struct_{}", header.structs.len()));
            let Some(mut layout) = parse_struct_body(text, i, &temp_name, header) else {
                return false;
            };
            skip_ws(text, i);
            let alias = read_ident(text, i).unwrap_or(temp_name.clone());
            layout.name = alias.clone();
            skip_ws(text, i);
            if *i < b.len() && b[*i] == b';' {
                *i += 1;
            }
            if let Some(tag) = tag {
                if tag != alias {
                    header
                        .typedefs
                        .insert(tag, FfiType::Struct(layout.clone()));
                }
            }
            register_struct(header, layout);
            return true;
        }
        // typedef struct Tag Alias;  (opaque)
        skip_ws(text, i);
        let Some(alias) = read_ident(text, i) else {
            return false;
        };
        skip_ws(text, i);
        if *i < b.len() && b[*i] == b';' {
            *i += 1;
        }
        let ty = if let Some(tag) = tag {
            header
                .typedefs
                .get(&tag)
                .cloned()
                .unwrap_or(FfiType::Ptr)
        } else {
            FfiType::Ptr
        };
        header.typedefs.insert(alias, ty);
        return true;
    }

    // typedef void (*name)(…);
    let save = *i;
    if let Some(ret) = parse_c_type(text, i, &header.typedefs) {
        skip_ws(text, i);
        let b = text.as_bytes();
        if *i < b.len() && b[*i] == b'(' {
            *i += 1;
            skip_ws(text, i);
            while *i < b.len() && b[*i] == b'*' {
                *i += 1;
                skip_ws(text, i);
            }
            let Some(alias) = read_ident(text, i) else {
                *i = save;
                return false;
            };
            skip_ws(text, i);
            if *i >= b.len() || b[*i] != b')' {
                *i = save;
                return false;
            }
            *i += 1;
            skip_ws(text, i);
            if *i < b.len() && b[*i] == b'(' {
                // skip parameter list
                let mut depth = 0i32;
                while *i < b.len() {
                    let c = b[*i];
                    *i += 1;
                    match c {
                        b'(' => depth += 1,
                        b')' => {
                            depth -= 1;
                            if depth == 0 {
                                break;
                            }
                        }
                        _ => {}
                    }
                }
            }
            skip_ws(text, i);
            if *i < b.len() && b[*i] == b';' {
                *i += 1;
            }
            let _ = ret;
            header.typedefs.insert(alias, FfiType::Ptr);
            return true;
        }
        *i = save;
    }

    // typedef TYPE [*]alias;
    let Some(ty) = parse_c_type(text, i, &header.typedefs) else {
        return false;
    };
    skip_ws(text, i);
    let b = text.as_bytes();
    let mut ptr = ty;
    while *i < b.len() && b[*i] == b'*' {
        ptr = pointerize(ptr);
        *i += 1;
        skip_ws(text, i);
    }
    let Some(alias) = read_ident(text, i) else {
        return false;
    };
    skip_ws(text, i);
    // reject arrays / leftover junk
    if *i < b.len() && b[*i] == b'[' {
        return false;
    }
    if *i < b.len() && b[*i] == b';' {
        *i += 1;
        header.typedefs.insert(alias, ptr);
        return true;
    }
    false
}

fn register_struct(header: &mut CHeader, layout: FfiStructLayout) {
    let decl = layout_to_struct_decl(&layout);
    header.typedefs.insert(
        layout.name.clone(),
        FfiType::Struct(layout.clone()),
    );
    // Replace existing decl with same name
    if let Some(pos) = header.structs.iter().position(|s| s.name == layout.name) {
        header.structs[pos] = decl;
    } else {
        header.structs.push(decl);
    }
}

fn layout_to_struct_decl(layout: &FfiStructLayout) -> StructDecl {
    let members: Vec<Member> = layout
        .fields
        .iter()
        .map(|f| {
            Member::Field(FieldDecl {
                access: Access::Export,
                is_static: false,
                is_const: false,
                ty: Some(ffi_to_type_ref_ast(&f.ty)),
                name: f.name.clone(),
                init: None,
                span: Span::default(),
            })
        })
        .collect();
    StructDecl {
        access: Access::Export,
        name: layout.name.clone(),
        type_params: vec![],
        members,
        attributes: vec![Attribute {
            name: "repr".into(),
            value: Some(Expr::String("C".into(), Span::default())),
            span: Span::default(),
        }],
        packed: layout.packed,
        align: None,
        repr_c: true,
        span: Span::default(),
    }
}

fn ffi_to_type_ref_ast(t: &FfiType) -> TypeRef {
    match t {
        FfiType::Void => TypeRef::named("void", Span::default()),
        FfiType::Bool => TypeRef::named("bool", Span::default()),
        FfiType::I8 => TypeRef::named("byte", Span::default()),
        FfiType::I16 => TypeRef::named("short", Span::default()),
        FfiType::I32 => TypeRef::named("int", Span::default()),
        FfiType::I64 => TypeRef::named("long", Span::default()),
        FfiType::U8 => TypeRef::named("byte", Span::default()),
        FfiType::U16 => TypeRef::named("ushort", Span::default()),
        FfiType::U32 => TypeRef::named("uint", Span::default()),
        FfiType::U64 => TypeRef::named("ulong", Span::default()),
        FfiType::F32 => TypeRef::named("float", Span::default()),
        FfiType::F64 => TypeRef::named("double", Span::default()),
        FfiType::Ptr | FfiType::CString => TypeRef::named("ptr", Span::default()),
        FfiType::Struct(s) => TypeRef::named(&s.name, Span::default()),
        FfiType::StructPtr(s) => {
            let mut tr = TypeRef::named("ptr", Span::default());
            tr.args.push(TypeRef::named(&s.name, Span::default()));
            tr
        }
    }
}

fn parse_enum_body(text: &str, i: &mut usize, constants: &mut HashMap<String, i64>) {
    let b = text.as_bytes();
    if *i >= b.len() || b[*i] != b'{' {
        return;
    }
    *i += 1;
    let mut value: i64 = 0;
    while *i < b.len() {
        skip_ws(text, i);
        if *i < b.len() && b[*i] == b'}' {
            *i += 1;
            break;
        }
        let Some(name) = read_ident(text, i) else {
            // skip junk
            if *i < b.len() {
                *i += 1;
            }
            continue;
        };
        skip_ws(text, i);
        if *i < b.len() && b[*i] == b'=' {
            *i += 1;
            skip_ws(text, i);
            let start = *i;
            while *i < b.len()
                && b[*i] != b','
                && b[*i] != b'}'
                && !b[*i].is_ascii_whitespace()
            {
                *i += 1;
            }
            if let Some(v) = parse_c_int_literal(&text[start..*i]) {
                value = v;
            }
        }
        constants.insert(name, value);
        value += 1;
        skip_ws(text, i);
        if *i < b.len() && b[*i] == b',' {
            *i += 1;
        }
    }
}

fn parse_struct_body(
    text: &str,
    i: &mut usize,
    name: &str,
    header: &mut CHeader,
) -> Option<FfiStructLayout> {
    let b = text.as_bytes();
    if *i >= b.len() || b[*i] != b'{' {
        return None;
    }
    *i += 1;
    let mut fields: Vec<(String, FfiType)> = Vec::new();
    let mut anon = 0usize;
    while *i < b.len() {
        skip_ws(text, i);
        if *i < b.len() && b[*i] == b'}' {
            *i += 1;
            break;
        }
        // function-pointer field: ret (*name)(…);
        if let Some((fname, fty)) = try_parse_fn_ptr_field(text, i, &header.typedefs) {
            fields.push((fname, fty));
            continue;
        }
        let save = *i;
        let Some(mut ty) = parse_c_type(text, i, &header.typedefs) else {
            // recovery inside struct: skip to ;
            if let Some(n) = text[*i..].find(';') {
                *i += n + 1;
                continue;
            }
            *i = save;
            return None;
        };
        skip_ws(text, i);
        while *i < b.len() && b[*i] == b'*' {
            ty = pointerize(ty);
            *i += 1;
            skip_ws(text, i);
        }
        let fname = read_ident(text, i).unwrap_or_else(|| {
            anon += 1;
            format!("__anon{anon}")
        });
        skip_ws(text, i);
        // arrays
        if *i < b.len() && b[*i] == b'[' {
            let count = parse_array_count(text, i, &header.constants).unwrap_or(1);
            let (esz, eal) = ffi_size_align(&ty);
            let total = esz.saturating_mul(count.max(1) as usize);
            let blob_name = format!("{name}_{fname}");
            let blob = make_blob_layout(&blob_name, total, eal);
            // Ensure blob type exists for nested field type name
            if !header.typedefs.contains_key(&blob_name) {
                register_struct(header, blob.clone());
            }
            fields.push((fname, FfiType::Struct(blob)));
        } else {
            fields.push((fname, ty));
        }
        skip_ws(text, i);
        if *i < b.len() && b[*i] == b';' {
            *i += 1;
        }
    }
    Some(layout_fields(name.to_string(), fields, false))
}

fn try_parse_fn_ptr_field(
    text: &str,
    i: &mut usize,
    typedefs: &HashMap<String, FfiType>,
) -> Option<(String, FfiType)> {
    let checkpoint = *i;
    let parsed = (|| {
        let _ = parse_c_type(text, i, typedefs)?;
        skip_ws(text, i);
        let b = text.as_bytes();
        // Optional pointers before (*name): e.g. `bgfx_x_t* (*foo)(...)`
        while *i < b.len() && b[*i] == b'*' {
            *i += 1;
            skip_ws(text, i);
        }
        if *i >= b.len() || b[*i] != b'(' {
            return None;
        }
        *i += 1;
        skip_ws(text, i);
        while *i < b.len() && b[*i] == b'*' {
            *i += 1;
            skip_ws(text, i);
        }
        let name = read_ident(text, i)?;
        skip_ws(text, i);
        if *i >= b.len() || b[*i] != b')' {
            return None;
        }
        *i += 1;
        skip_ws(text, i);
        if *i >= b.len() || b[*i] != b'(' {
            return None;
        }
        let mut depth = 0i32;
        while *i < b.len() {
            let c = b[*i];
            *i += 1;
            match c {
                b'(' => depth += 1,
                b')' => {
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                }
                _ => {}
            }
        }
        skip_ws(text, i);
        if *i < b.len() && b[*i] == b';' {
            *i += 1;
        }
        Some((name, FfiType::Ptr))
    })();
    if parsed.is_none() {
        *i = checkpoint;
    }
    parsed
}

fn parse_array_count(
    text: &str,
    i: &mut usize,
    constants: &HashMap<String, i64>,
) -> Option<u32> {
    let b = text.as_bytes();
    if *i >= b.len() || b[*i] != b'[' {
        return None;
    }
    *i += 1;
    skip_ws(text, i);
    let start = *i;
    while *i < b.len() && b[*i] != b']' {
        *i += 1;
    }
    let inner = text[start..*i].trim();
    if *i < b.len() {
        *i += 1;
    }
    if inner.is_empty() {
        return Some(0);
    }
    if let Some(v) = parse_c_int_literal(inner) {
        return Some(v.max(0) as u32);
    }
    constants.get(inner).map(|v| (*v).max(0) as u32)
}

fn make_blob_layout(name: &str, size: usize, align: usize) -> FfiStructLayout {
    // Represent opaque bytes as ulong/ubyte fields so abi::layout_struct matches.
    let mut fields = Vec::new();
    let mut offset = 0usize;
    let mut idx = 0usize;
    let align = align.max(1);
    while offset + 8 <= size {
        fields.push(FfiFieldLayout {
            name: format!("_{idx}"),
            offset,
            ty: FfiType::U64,
        });
        offset += 8;
        idx += 1;
    }
    while offset < size {
        fields.push(FfiFieldLayout {
            name: format!("_{idx}"),
            offset,
            ty: FfiType::U8,
        });
        offset += 1;
        idx += 1;
    }
    let size = align_up(size, align);
    FfiStructLayout {
        name: name.to_string(),
        size,
        align,
        fields,
        packed: false,
    }
}

fn layout_fields(name: String, fields: Vec<(String, FfiType)>, packed: bool) -> FfiStructLayout {
    let mut out = Vec::new();
    let mut offset = 0usize;
    let mut max_align = 1usize;
    for (fname, ty) in fields {
        let (sz, al) = ffi_size_align(&ty);
        let al = if packed { 1 } else { al.max(1) };
        if !packed {
            max_align = max_align.max(al);
            offset = align_up(offset, al);
        }
        out.push(FfiFieldLayout {
            name: fname,
            offset,
            ty,
        });
        offset += sz;
    }
    let size = if packed {
        offset
    } else {
        align_up(offset, max_align.max(1))
    };
    FfiStructLayout {
        name,
        size,
        align: max_align.max(1),
        fields: out,
        packed,
    }
}

fn align_up(off: usize, align: usize) -> usize {
    if align == 0 {
        return off;
    }
    (off + align - 1) & !(align - 1)
}

fn ffi_size_align(t: &FfiType) -> (usize, usize) {
    match t {
        FfiType::Void => (0, 1),
        FfiType::Bool | FfiType::I8 | FfiType::U8 => (1, 1),
        FfiType::I16 | FfiType::U16 => (2, 2),
        FfiType::I32 | FfiType::U32 | FfiType::F32 => (4, 4),
        FfiType::I64 | FfiType::U64 | FfiType::F64 | FfiType::Ptr | FfiType::CString => (8, 8),
        // Pointer-to-struct is still a pointer in a field.
        FfiType::StructPtr(_) => (8, 8),
        FfiType::Struct(s) => (s.size, s.align.max(1)),
    }
}

fn pointerize(ty: FfiType) -> FfiType {
    match ty {
        FfiType::Struct(s) => FfiType::StructPtr(s),
        FfiType::I8 | FfiType::U8 => FfiType::CString,
        FfiType::StructPtr(_) => FfiType::Ptr,
        _ => FfiType::Ptr,
    }
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
    after >= b.len() || !(b[after].is_ascii_alphanumeric() || b[after] == b'_')
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

fn is_type_keyword(name: &str) -> bool {
    matches!(
        name,
        "void"
            | "bool"
            | "_Bool"
            | "char"
            | "short"
            | "int"
            | "long"
            | "float"
            | "double"
            | "signed"
            | "unsigned"
            | "struct"
            | "union"
            | "enum"
            | "const"
            | "volatile"
            | "restrict"
    )
}

fn skip_leading_macros_and_storage(text: &str, i: &mut usize, typedefs: &HashMap<String, FfiType>) {
    loop {
        skip_ws(text, i);
        if starts_with_word(text, *i, "static")
            || starts_with_word(text, *i, "inline")
            || starts_with_word(text, *i, "__inline")
            || starts_with_word(text, *i, "extern")
        {
            let _ = read_ident(text, i);
            skip_ws(text, i);
            // extern "C"
            let b = text.as_bytes();
            if *i < b.len() && b[*i] == b'"' {
                *i += 1;
                while *i < b.len() && b[*i] != b'"' {
                    *i += 1;
                }
                if *i < b.len() {
                    *i += 1;
                }
            }
            continue;
        }
        if starts_with_word(text, *i, "__declspec") {
            *i += "__declspec".len();
            skip_ws(text, i);
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
            continue;
        }
        // Attribute / empty macros: IDENT followed by a type keyword or known typedef
        let save = *i;
        let Some(id) = read_ident(text, i) else {
            *i = save;
            break;
        };
        if is_type_keyword(&id) || typedefs.contains_key(&id) {
            *i = save;
            break;
        }
        skip_ws(text, i);
        // Peek next token
        let save2 = *i;
        let next = read_ident(text, i);
        *i = save2;
        let next_is_type = next
            .as_ref()
            .map(|n| is_type_keyword(n) || typedefs.contains_key(n) || n.ends_with("_t"))
            .unwrap_or(false);
        if next_is_type {
            // skipped macro `id`
            continue;
        }
        // Not a macro — rewind
        *i = save;
        break;
    }
}

fn try_parse_prototype(
    text: &str,
    i: &mut usize,
    typedefs: &HashMap<String, FfiType>,
) -> Option<CPrototype> {
    let checkpoint = *i;
    skip_leading_macros_and_storage(text, i, typedefs);

    let mut ret = match parse_c_type(text, i, typedefs) {
        Some(t) => t,
        None => {
            *i = checkpoint;
            return None;
        }
    };
    skip_ws(text, i);
    let b = text.as_bytes();
    while *i < b.len() && b[*i] == b'*' {
        ret = pointerize(ret);
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
    skip_ws(text, i);
    if *i >= b.len() || b[*i] != b'(' {
        *i = checkpoint;
        return None;
    }
    *i += 1;

    let mut params = Vec::new();
    let mut param_names = Vec::new();
    skip_ws(text, i);
    if starts_with_word(text, *i, "void") {
        let after = *i + 4;
        let b2 = text.as_bytes();
        let void_only =
            after >= b2.len() || (!b2[after].is_ascii_alphanumeric() && b2[after] != b'_');
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
            pty = pointerize(pty);
            *i += 1;
            skip_ws(text, i);
        }
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
        // C `T*` to known struct → StructPtr for RayTask `ptr<T>` packing
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
        let _ = read_ident(text, i); // struct|enum|union
        let tag = read_ident(text, i);
        if let Some(tag) = tag {
            if let Some(t) = typedefs.get(&tag) {
                base = Some(t.clone());
            } else {
                base = Some(FfiType::Ptr);
            }
        } else {
            base = Some(FfiType::Ptr);
        }
    } else if let Some(id) = {
        let save = *i;
        let id = read_ident(text, i);
        if let Some(ref name) = id {
            skip_ws(text, i);
            let b = text.as_bytes();
            if *i < b.len() && b[*i] == b'(' {
                // `foo(` → function name (rewind). `Type (*foo)` → keep as type.
                let mut j = *i + 1;
                while j < b.len() && b[j].is_ascii_whitespace() {
                    j += 1;
                }
                if j < b.len() && b[j] == b'*' {
                    Some(name.clone())
                } else {
                    *i = save;
                    None
                }
            } else if typedefs.contains_key(name)
                || name.ends_with("_t")
                || name.chars().all(|c| c.is_ascii_uppercase() || c == '_')
            {
                Some(name.clone())
            } else if unsigned || (signed && (long_count > 0 || short || saw_int)) {
                *i = save;
                None
            } else {
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
            base = Some(FfiType::I32);
        }
    }

    let _ = signed;
    base
}

// ── .rtbnd cache ────────────────────────────────────────────────────────

/// Cache file extension: `<header>.rtbnd`

fn cache_path(header: &Path) -> PathBuf {
    let mut s = header.as_os_str().to_os_string();
    s.push(".rtbnd");
    PathBuf::from(s)
}

fn write_ffi_type(buf: &mut Vec<u8>, t: &FfiType) {
    match t {
        FfiType::Void => buf.push(0),
        FfiType::Bool => buf.push(1),
        FfiType::I8 => buf.push(2),
        FfiType::I16 => buf.push(3),
        FfiType::I32 => buf.push(4),
        FfiType::I64 => buf.push(5),
        FfiType::U8 => buf.push(6),
        FfiType::U16 => buf.push(7),
        FfiType::U32 => buf.push(8),
        FfiType::U64 => buf.push(9),
        FfiType::F32 => buf.push(10),
        FfiType::F64 => buf.push(11),
        FfiType::Ptr => buf.push(12),
        FfiType::CString => buf.push(13),
        FfiType::Struct(s) => {
            buf.push(14);
            let b = s.name.as_bytes();
            buf.extend_from_slice(&(b.len() as u16).to_le_bytes());
            buf.extend_from_slice(b);
        }
        FfiType::StructPtr(s) => {
            buf.push(15);
            let b = s.name.as_bytes();
            buf.extend_from_slice(&(b.len() as u16).to_le_bytes());
            buf.extend_from_slice(b);
        }
    }
}

fn read_ffi_type(data: &[u8], pos: &mut usize) -> Option<FfiType> {
    if *pos >= data.len() { return None; }
    let disc = data[*pos]; *pos += 1;
    match disc {
        0 => Some(FfiType::Void),
        1 => Some(FfiType::Bool),
        2 => Some(FfiType::I8),
        3 => Some(FfiType::I16),
        4 => Some(FfiType::I32),
        5 => Some(FfiType::I64),
        6 => Some(FfiType::U8),
        7 => Some(FfiType::U16),
        8 => Some(FfiType::U32),
        9 => Some(FfiType::U64),
        10 => Some(FfiType::F32),
        11 => Some(FfiType::F64),
        12 => Some(FfiType::Ptr),
        13 => Some(FfiType::CString),
        14 | 15 => {
            if *pos + 2 > data.len() { return None; }
            let len = u16::from_le_bytes([data[*pos], data[*pos+1]]) as usize;
            *pos += 2;
            if *pos + len > data.len() { return None; }
            let name = String::from_utf8_lossy(&data[*pos..*pos+len]).into_owned();
            *pos += len;
            let layout = crate::ffi::FfiStructLayout { name, size: 0, align: 0, fields: vec![], packed: false };
            if disc == 14 { Some(FfiType::Struct(layout)) } else { Some(FfiType::StructPtr(layout)) }
        }
        _ => None,
    }
}

fn write_str(buf: &mut Vec<u8>, s: &str) {
    let b = s.as_bytes();
    buf.extend_from_slice(&(b.len() as u16).to_le_bytes());
    buf.extend_from_slice(b);
}

fn read_str(data: &[u8], pos: &mut usize) -> Option<String> {
    if *pos + 2 > data.len() { return None; }
    let len = u16::from_le_bytes([data[*pos], data[*pos+1]]) as usize;
    *pos += 2;
    if *pos + len > data.len() { return None; }
    let s = String::from_utf8_lossy(&data[*pos..*pos+len]).into_owned();
    *pos += len;
    Some(s)
}

/// Serialize only prototypes + constants to binary cache.
/// Structs are excluded (they're few and complex to round-trip through ast::StructDecl).
pub fn write_bind_cache(header: &CHeader, header_path: &Path) -> std::io::Result<()> {
    let path = cache_path(header_path);
    let mut buf: Vec<u8> = Vec::with_capacity(65536);

    // Magic + version
    buf.extend_from_slice(b"RTBD");
    buf.extend_from_slice(&1u32.to_le_bytes());

    // Prototypes
    buf.extend_from_slice(&(header.prototypes.len() as u16).to_le_bytes());
    for p in &header.prototypes {
        write_str(&mut buf, &p.name);
        write_ffi_type(&mut buf, &p.ret);
        buf.push(p.params.len() as u8);
        for param in &p.params {
            write_ffi_type(&mut buf, param);
        }
    }

    // Constants
    buf.extend_from_slice(&(header.constants.len() as u16).to_le_bytes());
    for (name, val) in &header.constants {
        write_str(&mut buf, name);
        buf.extend_from_slice(&val.to_le_bytes());
    }

    // Empty structs section (0)
    buf.extend_from_slice(&0u16.to_le_bytes());

    std::fs::write(&path, &buf)?;
    Ok(())
}

/// Try to read a .rtbnd cache. Returns `None` if cache is missing or corrupt.
pub fn read_bind_cache(header_path: &Path) -> Option<CHeader> {
    let path = cache_path(header_path);
    // Cache must be newer than the header file
    let hdr_meta = std::fs::metadata(header_path).ok()?;
    let cache_meta = std::fs::metadata(&path).ok()?;
    if cache_meta.modified().ok()? < hdr_meta.modified().ok()? {
        return None; // header changed since cache was created
    }

    let data = std::fs::read(&path).ok()?;
    if data.len() < 10 { return None; }
    if &data[..4] != b"RTBD" { return None; }
    let _version = u32::from_le_bytes([data[4], data[5], data[6], data[7]]);
    let mut pos = 8usize;

    // Prototypes
    if pos + 2 > data.len() { return None; }
    let nprotos = u16::from_le_bytes([data[pos], data[pos+1]]) as usize;
    pos += 2;
    let mut prototypes = Vec::with_capacity(nprotos.min(1024));
    for _ in 0..nprotos {
        let name = read_str(&data, &mut pos)?;
        let ret = read_ffi_type(&data, &mut pos)?;
        if pos >= data.len() { return None; }
        let nparams = data[pos] as usize; pos += 1;
        let mut params = Vec::with_capacity(nparams);
        for _ in 0..nparams {
            params.push(read_ffi_type(&data, &mut pos)?);
        }
        prototypes.push(CPrototype { name, params, ret, param_names: vec![] });
    }

    // Constants
    if pos + 2 > data.len() { return None; }
    let nconsts = u16::from_le_bytes([data[pos], data[pos+1]]) as usize;
    pos += 2;
    let mut constants = HashMap::with_capacity(nconsts);
    for _ in 0..nconsts {
        let name = read_str(&data, &mut pos)?;
        if pos + 8 > data.len() { return None; }
        let val = i64::from_le_bytes([
            data[pos], data[pos+1], data[pos+2], data[pos+3],
            data[pos+4], data[pos+5], data[pos+6], data[pos+7],
        ]);
        pos += 8;
        constants.insert(name, val);
    }

    Some(CHeader {
        prototypes,
        constants,
        structs: vec![],
        typedefs: HashMap::new(),
        includes_resolved: vec![],
    })
}

/// Read cache, check which names are missing, parse the header to fill gaps,
/// then write back the merged cache.
pub fn parse_header_with_cache(header_path: &Path, needed_names: &HashSet<String>) -> CompileResult<CHeader> {
    // 1. Try cache first
    let cached = read_bind_cache(header_path);

    // 2. Check coverage
    let cache_misses: Vec<String> = if let Some(ref c) = cached {
        needed_names.iter()
            .filter(|n| !c.prototypes.iter().any(|p| &p.name == *n) && !c.constants.contains_key(*n))
            .cloned()
            .collect()
    } else {
        needed_names.iter().cloned().collect()
    };

    // 3. If cache covers everything, return it
    if cache_misses.is_empty() && cached.is_some() {
        return Ok(cached.unwrap());
    }

    // 4. Parse header (with limits)
    let fresh = parse_header_file(header_path)?;

    // 5. Merge: keep all fresh structs, merge prototypes/constants
    let merged = if let Some(mut c) = cached {
        // Add fresh prototypes not in cache
        for p in &fresh.prototypes {
            if !c.prototypes.iter().any(|cp| cp.name == p.name) {
                c.prototypes.push(p.clone());
            }
        }
        for (k, v) in &fresh.constants {
            c.constants.entry(k.clone()).or_insert(*v);
        }
        // structs always from fresh parse
        c.structs = fresh.structs;
        c
    } else {
        fresh
    };

    // 6. Write cache
    let _ = write_bind_cache(&merged, header_path);

    Ok(merged)
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
        FfiType::U8 => "byte".into(),
        FfiType::U16 => "ushort".into(),
        FfiType::U32 => "uint".into(),
        FfiType::U64 => "ulong".into(),
        FfiType::F32 => "float".into(),
        FfiType::F64 => "double".into(),
        FfiType::Ptr => "ptr".into(),
        FfiType::CString => "string".into(),
        FfiType::Struct(s) => s.name.clone(),
        FfiType::StructPtr(s) => format!("ptr<{}>", s.name),
    }
}

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
