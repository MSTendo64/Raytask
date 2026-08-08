//! Emit freestanding C from optimized SSA (embedded / kernel).

use super::cfg::block_rpo;
use super::ir::*;
use crate::stdlib::ids;
use std::collections::HashMap;
use std::fmt::Write as _;

/// Emit C definitions for all non-script SSA functions.
pub fn emit_functions(ssa: &SsaModule) -> String {
    let mut out = String::new();
    out.push_str("/* ---- SSA → C function bodies ---- */\n");
    for f in &ssa.functions {
        if should_skip_function(&f.name) {
            continue;
        }
        out.push_str(&emit_function(ssa, f));
        out.push('\n');
    }
    out
}

fn should_skip_function(name: &str) -> bool {
    name == "<script>" || name.starts_with("lambda_")
}

/// C symbol for an SSA function / method name (`Foo.bar` → `Foo_bar`).
pub fn c_func_name(name: &str) -> String {
    name.replace('.', "_").replace('~', "_dtor_")
}

fn emit_function(ssa: &SsaModule, func: &SsaFunction) -> String {
    let mut out = String::new();
    let cname = c_func_name(&func.name);
    let ret = if function_returns_value(func) {
        "int64_t"
    } else {
        "void"
    };

    // Params: arity from SSA; names a0.. 
    let params: Vec<String> = (0..func.arity)
        .map(|i| format!("int64_t a{i}"))
        .collect();
    let plist = if params.is_empty() {
        "void".into()
    } else {
        params.join(", ")
    };
    let _ = writeln!(out, "{ret} {cname}({plist}) {{");

    // Declare all SSA values used as temps
    let mut declared = std::collections::HashSet::new();
    for b in func.blocks.values() {
        for inst in &b.insts {
            if declared.insert(inst.id.0) {
                match &inst.kind {
                    InstKind::Alloca { .. } => {
                        let _ = writeln!(out, "    int64_t slot_{};", inst.id.0);
                    }
                    InstKind::Store { .. } | InstKind::SetGlobal { .. } | InstKind::DefineGlobal { .. }
                    | InstKind::SetProperty { .. } | InstKind::SetIndex { .. }
                    | InstKind::SetUpvalue { .. } | InstKind::Print { .. } => {}
                    _ if inst.ty != SsaTy::Void => {
                        let _ = writeln!(out, "    int64_t v{} = 0;", inst.id.0);
                    }
                    _ => {}
                }
            }
        }
    }

    // Alloca slots get their own storage; Load/Store use &slot_id conceptually as ptr = slot id value holding address
    // We treat Alloca as: v_ptr = (int64_t)(intptr_t)&slot_N — but simpler: ptr ValueId maps to slot_{alloca_id}
    let mut alloca_of: HashMap<ValueId, ValueId> = HashMap::new();
    for b in func.blocks.values() {
        for inst in &b.insts {
            if matches!(inst.kind, InstKind::Alloca { .. }) {
                alloca_of.insert(inst.id, inst.id);
            }
        }
    }

    let order = block_rpo(func);
    let defs = collect_defs(func);

    for &bid in &order {
        let block = func.block(bid);
        let _ = writeln!(out, "  bb{}:", bid.0);
        for inst in &block.insts {
            out.push_str(&emit_inst(ssa, func, inst, &defs, &alloca_of));
        }
        out.push_str(&emit_term(&block.term, ret == "void"));
    }

    // Ensure non-void functions return
    if ret != "void" {
        out.push_str("  return 0;\n");
    }
    out.push_str("}\n");
    out
}

fn collect_defs(func: &SsaFunction) -> HashMap<ValueId, InstKind> {
    let mut defs = HashMap::new();
    for b in func.blocks.values() {
        for inst in &b.insts {
            defs.insert(inst.id, inst.kind.clone());
        }
    }
    defs
}

fn v(id: ValueId) -> String {
    format!("v{}", id.0)
}

fn emit_inst(
    ssa: &SsaModule,
    _func: &SsaFunction,
    inst: &Inst,
    defs: &HashMap<ValueId, InstKind>,
    alloca_of: &HashMap<ValueId, ValueId>,
) -> String {
    let line = inst.line;
    let mut s = String::new();
    match &inst.kind {
        InstKind::Const(c) => {
            let _ = writeln!(
                s,
                "    {} = {}; /* L{} */",
                v(inst.id),
                const_expr(c, ssa),
                line
            );
        }
        InstKind::Alloca { .. } => {
            // slot_{id} already declared; pointer value is unused — Load/Store use alloca id
            let _ = writeln!(s, "    /* alloca slot_{} */", inst.id.0);
        }
        InstKind::Load { ptr } => {
            let slot = alloca_of.get(ptr).copied().unwrap_or(*ptr);
            let _ = writeln!(s, "    {} = slot_{};", v(inst.id), slot.0);
        }
        InstKind::Store { ptr, value } => {
            let slot = alloca_of.get(ptr).copied().unwrap_or(*ptr);
            let _ = writeln!(s, "    slot_{} = {};", slot.0, v(*value));
        }
        InstKind::BinOp { op, lhs, rhs } => {
            let _ = writeln!(
                s,
                "    {} = {};",
                v(inst.id),
                bin_expr(op, &v(*lhs), &v(*rhs))
            );
        }
        InstKind::UnOp { op, arg } => {
            let _ = writeln!(s, "    {} = {};", v(inst.id), un_expr(op, &v(*arg)));
        }
        InstKind::Call { callee, args, .. } => {
            let argv: Vec<String> = args.iter().map(|a| v(*a)).collect();
            let (call, is_void) = resolve_call(ssa, defs, *callee, &argv);
            if is_void {
                let _ = writeln!(s, "    {call};");
                if inst.ty != SsaTy::Void {
                    let _ = writeln!(s, "    {} = 0;", v(inst.id));
                }
            } else if inst.ty == SsaTy::Void {
                let _ = writeln!(s, "    (void)({call});");
            } else {
                let _ = writeln!(s, "    {} = (int64_t)({call});", v(inst.id));
            }
        }
        InstKind::GetGlobal { index } => {
            let name = ssa
                .globals
                .get(*index as usize)
                .map(|s| s.as_str())
                .unwrap_or("/*bad*/");
            if let Some(native) = native_c_name_for_global(name) {
                let _ = writeln!(s, "    {} = 0; /* global fn {} → {} */", v(inst.id), name, native);
            } else if ssa.functions.iter().any(|f| f.name == name) {
                let _ = writeln!(
                    s,
                    "    {} = 0; /* fn {} */",
                    v(inst.id),
                    c_func_name(name)
                );
            } else {
                // FFI external function — no C variable exists.
                // The Call instruction will handle it directly.
                let _ = writeln!(s, "    {} = 0; /* FFI extern {} */", v(inst.id), name);
            }
        }
        InstKind::SetGlobal { index, value } | InstKind::DefineGlobal { index, value } => {
            let name = ssa
                .globals
                .get(*index as usize)
                .map(|s| s.as_str())
                .unwrap_or("g");
            if ssa.functions.iter().any(|f| f.name == name)
                || native_c_name_for_global(name).is_some()
            {
                let _ = writeln!(s, "    /* define {} */", name);
            } else {
                let _ = writeln!(s, "    {} = {};", sanitize_global(name), v(*value));
            }
        }
        InstKind::GetProperty { object, name } => {
            let _ = writeln!(
                s,
                "    {} = 0; /* getprop {} . {} */",
                v(inst.id),
                v(*object),
                v(*name)
            );
        }
        InstKind::SetProperty {
            object,
            name,
            value,
        } => {
            let _ = writeln!(
                s,
                "    (void){}; (void){}; (void){}; /* setprop */",
                v(*object),
                v(*name),
                v(*value)
            );
        }
        InstKind::GetIndex { object, index } => {
            let _ = writeln!(
                s,
                "    {} = 0; /* idx {}[{}] */",
                v(inst.id),
                v(*object),
                v(*index)
            );
        }
        InstKind::SetIndex {
            object,
            index,
            value,
        } => {
            let _ = writeln!(
                s,
                "    (void){}; (void){}; (void){}; /* setidx */",
                v(*object),
                v(*index),
                v(*value)
            );
        }
        InstKind::NewObject { .. } => {
            let _ = writeln!(s, "    {} = 0; /* new object */", v(inst.id));
        }
        InstKind::NewArray { elems } => {
            let _ = writeln!(
                s,
                "    {} = 0; /* new array len={} */",
                v(inst.id),
                elems.len()
            );
        }
        InstKind::Print { value } => {
            if matches!(
                defs.get(value),
                Some(InstKind::Const(ConstValue::String(_)))
            ) {
                let _ = writeln!(
                    s,
                    "    print((const char*)(uintptr_t){});",
                    v(*value)
                );
            } else {
                let _ = writeln!(s, "    print_i64({});", v(*value));
            }
        }
        InstKind::Dup { value } => {
            let _ = writeln!(s, "    {} = {};", v(inst.id), v(*value));
        }
        InstKind::Await { value } => {
            let _ = writeln!(s, "    {} = {}; /* await */", v(inst.id), v(*value));
        }
        InstKind::GetUpvalue { index } => {
            let _ = writeln!(s, "    {} = 0; /* upval {} */", v(inst.id), index);
        }
        InstKind::SetUpvalue { index, value } => {
            let _ = writeln!(s, "    (void){}; /* setupval {} */", v(*value), index);
        }
        InstKind::MakeClosure { .. } => {
            let _ = writeln!(s, "    {} = 0; /* closure */", v(inst.id));
        }
        InstKind::Phi { incomings } => {
            // Should be eliminated; fallback: pick first
            if let Some((_, val)) = incomings.first() {
                let _ = writeln!(s, "    {} = {}; /* phi */", v(inst.id), v(*val));
            } else {
                let _ = writeln!(s, "    {} = 0; /* phi empty */", v(inst.id));
            }
        }
        InstKind::Param { index } => {
            let _ = writeln!(s, "    {} = a{};", v(inst.id), index);
        }
    }
    s
}

fn emit_term(term: &Terminator, void_ret: bool) -> String {
    match term {
        Terminator::Br(t) => format!("    goto bb{};\n", t.0),
        Terminator::CondBr {
            cond,
            then_bb,
            else_bb,
        } => format!(
            "    if ({}) goto bb{}; else goto bb{};\n",
            v(*cond),
            then_bb.0,
            else_bb.0
        ),
        Terminator::Return(None) => {
            if void_ret {
                "    return;\n".into()
            } else {
                "    return 0;\n".into()
            }
        }
        Terminator::Return(Some(val)) => {
            if void_ret {
                format!("    (void){}; return;\n", v(*val))
            } else {
                format!("    return {};\n", v(*val))
            }
        }
        Terminator::Halt => "    for(;;){}\n".into(),
        Terminator::Throw(val) => format!("    (void){}; for(;;){{}} /* throw */\n", v(*val)),
        Terminator::Unreachable => "    /* unreachable */\n".into(),
    }
}

fn const_expr(c: &ConstValue, ssa: &SsaModule) -> String {
    match c {
        ConstValue::Null => "0".into(),
        ConstValue::Bool(b) => if *b { "1" } else { "0" }.into(),
        ConstValue::Int(i) => i.to_string(),
        ConstValue::Float(f) => format!("{}", f.to_bits() as i64),
        ConstValue::String(s) => {
            // pointer as int — freestanding demos rarely need this in SSA path
            format!("(int64_t)(uintptr_t)\"{}\"", escape_c(s))
        }
        ConstValue::FuncRef(id) => {
            let name = ssa
                .functions
                .get(id.0 as usize)
                .map(|f| c_func_name(&f.name))
                .unwrap_or_else(|| "0".into());
            format!("/* fn {} */ 0", name)
        }
        ConstValue::Native(id) => format!("/* native {} */ 0", id),
        ConstValue::TypeModule(n) => format!("/* type {} */ 0", n),
    }
}

fn bin_expr(op: &BinOpKind, l: &str, r: &str) -> String {
    match op {
        BinOpKind::Add => format!("({l} + {r})"),
        BinOpKind::Sub => format!("({l} - {r})"),
        BinOpKind::Mul => format!("({l} * {r})"),
        BinOpKind::Div => format!("({r} == 0 ? 0 : ({l} / {r}))"),
        BinOpKind::Mod => format!("({r} == 0 ? 0 : ({l} % {r}))"),
        BinOpKind::Eq => format!("({l} == {r})"),
        BinOpKind::Ne => format!("({l} != {r})"),
        BinOpKind::Lt => format!("({l} < {r})"),
        BinOpKind::Le => format!("({l} <= {r})"),
        BinOpKind::Gt => format!("({l} > {r})"),
        BinOpKind::Ge => format!("({l} >= {r})"),
        BinOpKind::And => format!("({l} && {r})"),
        BinOpKind::Or => format!("({l} || {r})"),
        BinOpKind::BitAnd => format!("({l} & {r})"),
        BinOpKind::BitOr => format!("({l} | {r})"),
        BinOpKind::BitXor => format!("({l} ^ {r})"),
        BinOpKind::Shl => format!("({l} << {r})"),
        BinOpKind::Shr => format!("({l} >> {r})"),
        BinOpKind::NullCoalesce => format!("({l} ? {l} : {r})"),
    }
}

fn un_expr(op: &UnOpKind, a: &str) -> String {
    match op {
        UnOpKind::Neg => format!("(-{a})"),
        UnOpKind::Not => format!("(!{a})"),
        UnOpKind::BitNot => format!("(~{a})"),
        UnOpKind::IsNull => format!("({a} == 0)"),
        UnOpKind::ToString => format!("({a}) /* tostring */"),
    }
}

fn resolve_call(
    ssa: &SsaModule,
    defs: &HashMap<ValueId, InstKind>,
    callee: ValueId,
    args: &[String],
) -> (String, bool) {
    let alist = args.join(", ");
    match defs.get(&callee) {
        Some(InstKind::Const(ConstValue::FuncRef(id))) => {
            let name = ssa
                .functions
                .get(id.0 as usize)
                .map(|f| c_func_name(&f.name))
                .unwrap_or_else(|| "unknown_fn".into());
            let is_void = ssa
                .functions
                .get(id.0 as usize)
                .map(|f| !function_returns_value(f))
                .unwrap_or(true);
            (format!("{name}({alist})"), is_void)
        }
        Some(InstKind::Const(ConstValue::Native(nid))) => {
            let name = native_c_name(*nid).unwrap_or("/*native*/");
            let is_void = matches!(
                *nid,
                ids::PRINT
                    | ids::WRITE
                    | ids::SLEEP
                    | ids::MMIO_WRITE32
                    | ids::SPIN
                    | ids::FREE
            );
            (format!("{name}({alist})"), is_void)
        }
        Some(InstKind::GetGlobal { index }) => {
            let gname = ssa
                .globals
                .get(*index as usize)
                .map(|s| s.as_str())
                .unwrap_or("fn");
            if let Some(n) = native_c_name_for_global(gname) {
                let is_void = matches!(
                    gname,
                    "print" | "write" | "sleep" | "MmioWrite32" | "Spin" | "free"
                );
                (format!("{n}({alist})"), is_void)
            } else if let Some(f) = ssa.functions.iter().find(|f| f.name == gname) {
                (
                    format!("{}({alist})", c_func_name(gname)),
                    !function_returns_value(f),
                )
            } else {
                (format!("{}({alist})", c_func_name(gname)), true)
            }
        }
        _ => (format!("/* dyn call */ 0 /* {alist} */"), false),
    }
}

fn function_returns_value(func: &SsaFunction) -> bool {
    let defs = collect_defs(func);
    for b in func.blocks.values() {
        if let Terminator::Return(Some(vid)) = &b.term {
            match defs.get(vid) {
                Some(InstKind::Const(ConstValue::Null)) => continue,
                _ => return true,
            }
        }
    }
    false
}

fn native_c_name(id: usize) -> Option<&'static str> {
    Some(match id {
        ids::PRINT => "print_i64",
        ids::WRITE => "rt_write",
        ids::SLEEP => "rt_sleep",
        ids::MMIO_READ32 => "MmioRead32",
        ids::MMIO_WRITE32 => "MmioWrite32",
        ids::SPIN => "Spin",
        ids::IS_FREESTANDING => "IsFreestanding",
        ids::MALLOC => "rt_malloc",
        ids::FREE => "rt_free",
        _ => return None,
    })
}

fn native_c_name_for_global(name: &str) -> Option<&'static str> {
    match name {
        "print" => Some("print_i64"),
        "write" => Some("rt_write"),
        "sleep" => Some("rt_sleep"),
        "MmioRead32" => Some("MmioRead32"),
        "MmioWrite32" => Some("MmioWrite32"),
        "Spin" => Some("Spin"),
        "IsFreestanding" => Some("IsFreestanding"),
        "malloc" => Some("rt_malloc"),
        "free" => Some("rt_free"),
        _ => None,
    }
}

fn sanitize_global(name: &str) -> String {
    name.chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect()
}

fn escape_c(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
        .replace('\t', "\\t")
}

/// Names of SSA functions that replace AST bodies (for prototype-only AST emit).
pub fn body_function_names(ssa: &SsaModule) -> Vec<String> {
    ssa.functions
        .iter()
        .filter(|f| !should_skip_function(&f.name))
        .map(|f| f.name.clone())
        .collect()
}
