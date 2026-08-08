//! Expand `[DllImport:]` + `[bind:/include: ".h"]` into RayTask FFI decls.
//! Enables typechecking and VM calls without gcc.

use crate::ast::*;
use crate::c_header::{self, CPrototype};
use crate::error::CompileResult;
use crate::ffi::{self, FfiType};
use crate::span::Span;
use std::collections::HashSet;
use std::path::Path;

/// Rewrite `program` in place: C header prototypes/structs become RayTask items.
pub fn expand_c_header_binds(program: &mut Program, entry: Option<&Path>) -> CompileResult<()> {
    let mut synthetic = Vec::new();
    let mut seen = HashSet::new();
    collect_existing_names(program, &mut seen);
    
    // Collect names referenced in user code to filter header prototypes
    let mut used_names = HashSet::new();
    collect_referenced_names(program, &mut used_names);

    for item in &program.items {
        expand_item(item, entry, &mut synthetic, &mut seen, None, &used_names)?;
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

/// Collect every identifier referenced in expressions (calls, field access, etc.)
/// so we can filter header prototypes to only actually-used functions.
fn collect_referenced_names(program: &Program, used: &mut HashSet<String>) {
    for item in &program.items {
        collect_referenced_item(item, used);
    }
}

fn collect_referenced_item(item: &Item, used: &mut HashSet<String>) {
    match item {
        Item::Attribute(_, inner) => collect_referenced_item(inner, used),
        Item::Namespace(ns) => {
            for i in &ns.items {
                collect_referenced_item(i, used);
            }
        }
        Item::Function(f) => {
            if let Some(body) = &f.body {
                match body {
                    FunctionBody::Block(b) => collect_referenced_in_block(b, used),
                    FunctionBody::Expr(e) => collect_referenced_in_expr(e, used),
                }
            }
        }
        Item::Struct(s) => {
            for m in &s.members {
                if let Member::Field(f) = m {
                    if let Some(init) = &f.init {
                        collect_referenced_in_expr(init, used);
                    }
                }
            }
        }
        Item::Class(c) => {
            for m in &c.members {
                match m {
                    Member::Field(f) => {
                        if let Some(init) = &f.init {
                            collect_referenced_in_expr(init, used);
                        }
                    }
                    Member::Method(m) => {
                        if let Some(body) = &m.body {
                            match body {
                                FunctionBody::Block(b) => collect_referenced_in_block(b, used),
                                FunctionBody::Expr(e) => collect_referenced_in_expr(e, used),
                            }
                        }
                    }
                    Member::Property(p) => {
                        if let Some(b) = &p.getter { collect_referenced_in_block(b, used); }
                        if let Some(b) = &p.setter { collect_referenced_in_block(b, used); }
                    }
                    Member::Constructor(ct) => collect_referenced_in_block(&ct.body, used),
                    _ => {}
                }
            }
        }
        _ => {}
    }
}

fn collect_referenced_in_block(block: &Block, used: &mut HashSet<String>) {
    collect_referenced_in_stmts(&block.stmts, used);
}

fn collect_referenced_in_stmts(stmts: &[Stmt], used: &mut HashSet<String>) {
    for stmt in stmts {
        collect_referenced_in_stmt(stmt, used);
    }
}

fn collect_referenced_in_stmt(stmt: &Stmt, used: &mut HashSet<String>) {
    match stmt {
        Stmt::Expr(e) => collect_referenced_in_expr(e, used),
        Stmt::Decl(d) => {
            if let Some(init) = &d.init {
                collect_referenced_in_expr(init, used);
            }
        }
        Stmt::Return(e, _) => { if let Some(e) = e { collect_referenced_in_expr(e, used); } }
        Stmt::If { cond, then_block, else_branch, .. } => {
            collect_referenced_in_expr(cond, used);
            collect_referenced_in_block(then_block, used);
            match else_branch {
                Some(ElseBranch::Block(b)) => collect_referenced_in_block(b, used),
                Some(ElseBranch::If(s)) => collect_referenced_in_stmt(s, used),
                None => {}
            }
        }
        Stmt::While { cond, body, .. } | Stmt::DoWhile { cond, body, .. } => {
            collect_referenced_in_expr(cond, used);
            collect_referenced_in_block(body, used);
        }
        Stmt::For { init, cond, step, body, .. } => {
            if let Some(init) = init { collect_referenced_in_stmt(init, used); }
            if let Some(cond) = cond { collect_referenced_in_expr(cond, used); }
            if let Some(step) = step { collect_referenced_in_expr(step, used); }
            collect_referenced_in_block(body, used);
        }
        Stmt::Foreach { iter, body, .. } => {
            collect_referenced_in_expr(iter, used);
            collect_referenced_in_block(body, used);
        }
        Stmt::Switch { expr, cases, .. } => {
            collect_referenced_in_expr(expr, used);
            for case in cases { collect_referenced_in_stmts(&case.body, used); }
        }
        Stmt::Match { expr, arms, .. } => {
            collect_referenced_in_expr(expr, used);
            for arm in arms { collect_referenced_in_expr(&arm.body, used); }
        }
        Stmt::Try { body, catches, finally, .. } => {
            collect_referenced_in_block(body, used);
            for c in catches { collect_referenced_in_block(&c.body, used); }
            if let Some(f) = finally { collect_referenced_in_block(f, used); }
        }
        Stmt::Using { decl, body, .. } => {
            if let Some(init) = &decl.init {
                collect_referenced_in_expr(init, used);
            }
            collect_referenced_in_block(body, used);
        }
        Stmt::Block(b) => collect_referenced_in_block(b, used),
        Stmt::Unsafe(b, _) => collect_referenced_in_block(b, used),
        Stmt::Throw(e, _) => collect_referenced_in_expr(e, used),
        Stmt::Const(_) | Stmt::Break(_) | Stmt::Continue(_) | Stmt::Asm { .. } => {}
    }
}

fn collect_referenced_in_expr(expr: &Expr, used: &mut HashSet<String>) {
    match expr {
        Expr::Call { callee, args, .. } => {
            if let Expr::Ident(name, _) = callee.as_ref() {
                used.insert(name.clone());
            } else {
                collect_referenced_in_expr(callee, used);
            }
            for a in args { collect_referenced_in_expr(&a.value, used); }
        }
        Expr::Member { object, .. } | Expr::PtrMember { object, .. } => collect_referenced_in_expr(object, used),
        Expr::Index { object, indices, .. } => {
            collect_referenced_in_expr(object, used);
            for idx in indices { collect_referenced_in_expr(idx, used); }
        }
        Expr::Binary { left, right, .. } => {
            collect_referenced_in_expr(left, used);
            collect_referenced_in_expr(right, used);
        }
        Expr::Ternary { cond, then_expr, else_expr, .. } => {
            collect_referenced_in_expr(cond, used);
            collect_referenced_in_expr(then_expr, used);
            collect_referenced_in_expr(else_expr, used);
        }
        Expr::Unary { expr: e, .. } | Expr::Await(e, _) | Expr::Deref(e, _)
        | Expr::AddressOf(e, _) | Expr::Grouped(e, _) | Expr::Try(e, _)
        | Expr::Cast { expr: e, .. } => collect_referenced_in_expr(e, used),
        Expr::Assign { target, value, .. } => {
            collect_referenced_in_expr(target, used);
            collect_referenced_in_expr(value, used);
        }
        Expr::Lambda { body, .. } => match body {
            FunctionBody::Block(b) => collect_referenced_in_block(b, used),
            FunctionBody::Expr(e) => collect_referenced_in_expr(e, used),
        },
        Expr::New { args, .. } => {
            for a in args { collect_referenced_in_expr(&a.value, used); }
        }
        Expr::ArrayLit(elems, _) => {
            for e in elems { collect_referenced_in_expr(e, used); }
        }
        Expr::Interpolated(parts, _) => {
            for p in parts {
                if let InterpPart::Expr(e) = p { collect_referenced_in_expr(e, used); }
            }
        }
        Expr::Is { expr: e, .. } | Expr::As { expr: e, .. } => collect_referenced_in_expr(e, used),
        // Leaf expressions — nothing to recurse into
        Expr::Ident(_, _) | Expr::Int(_, _) | Expr::UInt(_, _) | Expr::Float(_, _)
        | Expr::Decimal(_, _) | Expr::Bool(_, _) | Expr::Char(_, _)
        | Expr::String(_, _) | Expr::Null(_) | Expr::This(_) | Expr::Base(_)
        | Expr::TypeOf(_, _) | Expr::NameOf(_, _) | Expr::SizeOf(_, _)
        | Expr::OffsetOf { .. } => {}
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
        Item::Struct(s) => {
            seen.insert(s.name.clone());
        }
        Item::Class(c) => {
            seen.insert(c.name.clone());
        }
        Item::Union(u) => {
            seen.insert(u.name.clone());
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
    used_names: &HashSet<String>,
) -> CompileResult<()> {
    match item {
        Item::Attribute(attr, inner) => {
            let key = attr.name.to_ascii_lowercase();
            let mut lib = inherited_lib.clone();
            match key.as_str() {
                "dllimport" | "link" | "lib" => {
                    if let Some(s) = ffi::attr_string(attr) {
                        lib = Some(s);
                    }
                    expand_item(inner, entry, out, seen, lib, used_names)?;
                }
                "include" | "bind" | "cheader" => {
                    if let Some(header) = ffi::attr_string(attr) {
                        if let Some(ref lib_name) = lib {
                            push_header_decls(&header, lib_name, entry, out, seen, used_names)?;
                        }
                    }
                    expand_item(inner, entry, out, seen, lib, used_names)?;
                }
                _ => expand_item(inner, entry, out, seen, lib, used_names)?,
            }
        }
        Item::Namespace(ns) => {
            for i in &ns.items {
                expand_item(i, entry, out, seen, inherited_lib.clone(), used_names)?;
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
    used_names: &HashSet<String>,
) -> CompileResult<()> {
    let path = c_header::resolve_header(header, entry);
    if !path.is_file() {
        return Ok(());
    }
    // Use cache-aware parser — on first run parses the header, on subsequent runs reads .rtbnd cache
    let parsed = c_header::parse_header_with_cache(&path, used_names)?;
    for s in &parsed.structs {
        if !seen.insert(s.name.clone()) {
            continue;
        }
        out.push(Item::Struct(s.clone()));
    }
    for (name, value) in &parsed.constants {
        if !seen.insert(name.clone()) {
            continue;
        }
        out.push(Item::Const(ConstDecl {
            name: name.clone(),
            ty: TypeRef::named("int", Span::default()),
            value: Expr::Int(*value, Span::default()),
            span: Span::default(),
        }));
    }
    let filtered: Vec<_> = if used_names.is_empty() {
        // No user code references found — include everything (small headers, etc.)
        parsed.prototypes.iter().collect()
    } else {
        parsed.prototypes.iter().filter(|p| used_names.contains(&p.name)).collect()
    };
    for proto in filtered {
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
        FfiType::U8 => "byte",
        FfiType::U16 => "ushort",
        FfiType::U32 => "uint",
        FfiType::U64 => "ulong",
        FfiType::F32 => "float",
        FfiType::F64 => "double",
        FfiType::Ptr => "ptr",
        FfiType::CString => "string",
        FfiType::Struct(s) => return TypeRef::named(&s.name, Span::default()),
        FfiType::StructPtr(s) => {
            let mut tr = TypeRef::named("ptr", Span::default());
            tr.args.push(TypeRef::named(&s.name, Span::default()));
            return tr;
        }
    };
    TypeRef::named(name, Span::default())
}
