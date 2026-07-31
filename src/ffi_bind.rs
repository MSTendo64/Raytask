//! Expand `[DllImport:]` + `[bind:/include: ".h"]` into RayTask FFI function decls.
//! Enables typechecking and VM calls without gcc.

use crate::ast::*;
use crate::c_header::{self, CPrototype};
use crate::error::CompileResult;
use crate::ffi::{self, FfiType};
use crate::span::Span;
use std::collections::HashSet;
use std::path::Path;

/// Rewrite `program` in place: C header prototypes become bodyless function items.
pub fn expand_c_header_binds(program: &mut Program, entry: Option<&Path>) -> CompileResult<()> {
    let mut synthetic = Vec::new();
    let mut seen = HashSet::new();
    collect_existing_names(program, &mut seen);

    for item in &program.items {
        expand_item(item, entry, &mut synthetic, &mut seen, None)?;
    }

    if !synthetic.is_empty() {
        let mut out = synthetic;
        out.append(&mut program.items);
        program.items = out;
    }
    Ok(())
}

fn collect_existing_names(program: &Program, seen: &mut HashSet<String>) {
    for item in &program.items {
        collect_names_item(item, seen);
    }
}

fn collect_names_item(item: &Item, seen: &mut HashSet<String>) {
    match item {
        Item::Attribute(_, inner) => collect_names_item(inner, seen),
        Item::Namespace(ns) => {
            for i in &ns.items {
                collect_names_item(i, seen);
            }
        }
        Item::Function(f) => {
            seen.insert(f.name.clone());
        }
        _ => {}
    }
}

fn expand_item(
    item: &Item,
    entry: Option<&Path>,
    out: &mut Vec<Item>,
    seen: &mut HashSet<String>,
    inherited_lib: Option<String>,
) -> CompileResult<()> {
    match item {
        Item::Attribute(attr, inner) => {
            let key = attr.name.to_ascii_lowercase();
            let mut lib = inherited_lib;
            match key.as_str() {
                "dllimport" | "link" | "lib" => {
                    if let Some(s) = ffi::attr_string(attr) {
                        lib = Some(s);
                    }
                    expand_item(inner, entry, out, seen, lib)?;
                }
                "include" | "bind" | "cheader" => {
                    if let Some(header) = ffi::attr_string(attr) {
                        if let Some(ref lib_name) = lib {
                            push_header_decls(&header, lib_name, entry, out, seen)?;
                        }
                    }
                    expand_item(inner, entry, out, seen, lib)?;
                }
                _ => expand_item(inner, entry, out, seen, lib)?,
            }
        }
        Item::Namespace(ns) => {
            for i in &ns.items {
                expand_item(i, entry, out, seen, inherited_lib.clone())?;
            }
        }
        _ => {}
    }
    Ok(())
}

fn push_header_decls(
    header: &str,
    lib: &str,
    entry: Option<&Path>,
    out: &mut Vec<Item>,
    seen: &mut HashSet<String>,
) -> CompileResult<()> {
    let path = c_header::resolve_header(header, entry);
    if !path.is_file() {
        return Ok(());
    }
    let parsed = c_header::parse_header_file(&path)?;
    for proto in &parsed.prototypes {
        if !seen.insert(proto.name.clone()) {
            continue;
        }
        out.push(proto_to_item(proto, lib));
    }
    Ok(())
}

fn proto_to_item(proto: &CPrototype, lib: &str) -> Item {
    let span = Span::default();
    let params: Vec<Param> = proto
        .params
        .iter()
        .enumerate()
        .map(|(i, t)| Param {
            is_params: false,
            is_this: false,
            name: format!("a{i}"),
            ty: ffi_to_type_ref(t),
            default: None,
            span,
        })
        .collect();

    let f = FunctionDecl {
        access: Access::Export,
        is_async: false,
        is_unsafe: false,
        is_static: false,
        is_virtual: false,
        is_override: false,
        is_abstract: false,
        is_extension: false,
        return_type: ffi_to_type_ref(&proto.ret),
        name: proto.name.clone(),
        type_params: vec![],
        params,
        constraints: vec![],
        body: None,
        attributes: vec![Attribute {
            name: "DllImport".into(),
            value: Some(Expr::String(lib.to_string(), span)),
            span,
        }],
        span,
    };
    Item::Function(f)
}

fn ffi_to_type_ref(t: &FfiType) -> TypeRef {
    let name = match t {
        FfiType::Void => "void",
        FfiType::Bool => "bool",
        FfiType::I8 => "byte",
        FfiType::I16 => "short",
        FfiType::I32 => "int",
        FfiType::I64 => "long",
        FfiType::U8 => "ubyte",
        FfiType::U16 => "ushort",
        FfiType::U32 => "uint",
        FfiType::U64 => "ulong",
        FfiType::F32 => "float",
        FfiType::F64 => "double",
        FfiType::Ptr => "ptr",
        FfiType::CString => "string",
        FfiType::Struct(s) => return TypeRef::named(&s.name, Span::default()),
    };
    TypeRef::named(name, Span::default())
}
