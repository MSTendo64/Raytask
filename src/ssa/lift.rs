//! Lift stack bytecode chunks into SSA form.

use super::cfg::rebuild_edges;
use super::ir::*;
use crate::bytecode::{Chunk, Module, Op};
use crate::value::{FunctionRef, Value};
use std::collections::{HashMap, HashSet};

pub fn lift_module(module: &Module) -> SsaModule {
    let mut functions = Vec::new();
    for (i, chunk) in module.chunks.iter().enumerate() {
        functions.push(lift_chunk(FuncId(i as u32), chunk));
    }
    let classes = module
        .classes
        .iter()
        .map(|c| SsaClass {
            name: c.name.clone(),
            fields: c.fields.clone(),
            methods: c
                .methods
                .iter()
                .map(|(n, idx)| (n.clone(), FuncId(*idx as u32)))
                .collect(),
            constructor: c.constructor.map(|i| FuncId(i as u32)),
            base: c.base,
            destructor: c.destructor.map(|i| FuncId(i as u32)),
        })
        .collect();
    SsaModule {
        functions,
        main_chunk: FuncId(module.main_chunk as u32),
        globals: module.globals.clone(),
        classes,
        ffi: module.ffi.clone(),
        stdlib_enabled: module.stdlib_enabled,
        const_pool: Vec::new(),
    }
}

pub fn lift_chunk(id: FuncId, chunk: &Chunk) -> SsaFunction {
    let mut func = SsaFunction::new(id, chunk.name.clone(), chunk.arity);
    func.local_count = chunk.local_count.max(chunk.arity);
    func.is_async = chunk.is_async;
    func.source = chunk.source.clone();

    if chunk.code.is_empty() {
        let entry = func.entry;
        func.set_term(entry, Terminator::Return(None));
        rebuild_edges(&mut func);
        return func;
    }

    let leaders = find_leaders(chunk);
    let mut ip_to_block: HashMap<usize, BlockId> = HashMap::new();
    // First leader is entry (already BlockId(0))
    let mut sorted_leaders: Vec<usize> = leaders.iter().copied().collect();
    sorted_leaders.sort_unstable();
    for (i, &ip) in sorted_leaders.iter().enumerate() {
        let bid = if i == 0 {
            func.entry
        } else {
            func.create_block()
        };
        ip_to_block.insert(ip, bid);
    }

    // Alloca for each local slot
    let entry = func.entry;
    let mut slot_ptrs = Vec::new();
    for slot in 0..func.local_count {
        let vid = func.alloc_value();
        func.push_inst(
            entry,
            Inst {
                id: vid,
                kind: InstKind::Alloca {
                    ty: SsaTy::Dyn,
                    slot: slot as u32,
                },
                ty: SsaTy::Ref,
                line: 0,
                effectful: false,
            },
        );
        slot_ptrs.push(vid);
        // Params: store Param into alloca at entry (after all allocas conceptually)
    }
    for slot in 0..func.arity {
        let p = func.alloc_value();
        func.push_inst(
            entry,
            Inst {
                id: p,
                kind: InstKind::Param { index: slot as u32 },
                ty: SsaTy::Dyn,
                line: 0,
                effectful: false,
            },
        );
        let _sid = func.alloc_value();
        let _ = func.push_inst(
            entry,
            Inst {
                id: _sid,
                kind: InstKind::Store {
                    ptr: slot_ptrs[slot],
                    value: p,
                },
                ty: SsaTy::Void,
                line: 0,
                effectful: true,
            },
        );
    }

    // Translate each block
    for (li, &start_ip) in sorted_leaders.iter().enumerate() {
        let end_ip = sorted_leaders
            .get(li + 1)
            .copied()
            .unwrap_or(chunk.code.len());
        let bb = ip_to_block[&start_ip];
        translate_range(
            &mut func,
            chunk,
            start_ip,
            end_ip,
            bb,
            &ip_to_block,
            &slot_ptrs,
            li == 0,
        );
    }

    // Fallthrough: if a block doesn't terminate with a branch, add Br to next leader
    for (li, &start_ip) in sorted_leaders.iter().enumerate() {
        let bb = ip_to_block[&start_ip];
        let needs_fallthrough = matches!(
            func.block(bb).term,
            Terminator::Unreachable
        );
        if needs_fallthrough {
            if let Some(&next_ip) = sorted_leaders.get(li + 1) {
                let next_bb = ip_to_block[&next_ip];
                func.set_term(bb, Terminator::Br(next_bb));
            } else {
                // End of function without return — emit Return null
                let n = func.alloc_value();
                func.push_inst(
                    bb,
                    Inst {
                        id: n,
                        kind: InstKind::Const(ConstValue::Null),
                        ty: SsaTy::Dyn,
                        line: 0,
                        effectful: false,
                    },
                );
                func.set_term(bb, Terminator::Return(Some(n)));
            }
        }
    }

    rebuild_edges(&mut func);
    func
}

fn find_leaders(chunk: &Chunk) -> HashSet<usize> {
    let mut leaders = HashSet::from([0usize]);
    let mut ip = 0usize;
    while ip < chunk.code.len() {
        let op = Op::from_byte(chunk.code[ip]).unwrap_or(Op::Halt);
        let line_ip = ip;
        ip += 1;
        match op {
            Op::Constant
            | Op::GetLocal
            | Op::SetLocal
            | Op::GetGlobal
            | Op::SetGlobal
            | Op::DefineGlobal
            | Op::Call
            | Op::NewObject
            | Op::NewArray
            | Op::GetUpvalue
            | Op::SetUpvalue
            | Op::IncLocal
            | Op::DecLocal => {
                ip += 1;
            }
            Op::Jump | Op::JumpIfFalse | Op::JumpIfTrue | Op::Loop | Op::TryBegin => {
                if ip + 1 < chunk.code.len() {
                    let off = u16::from_be_bytes([chunk.code[ip], chunk.code[ip + 1]]) as usize;
                    ip += 2;
                    let target = match op {
                        Op::Loop => line_ip.saturating_sub(off).saturating_add(0).max(0),
                        // Jump offset is from after the 2-byte operand
                        _ => ip + off,
                    };
                    // Loop: jump = code.len() - loop_start + 2 at emit time, target = ip_after_operand - jump
                    let target = if op == Op::Loop {
                        // At Loop emit: jump = (ip_of_operand_end) - loop_start
                        // Actually: emit_loop: jump = code.len() - loop_start + 2 after pushing op,
                        // then emit_u16. After full Loop instr, ip points after operand.
                        // Target = ip - jump.
                        ip - off
                    } else {
                        target
                    };
                    leaders.insert(target);
                    if matches!(op, Op::JumpIfFalse | Op::JumpIfTrue) {
                        leaders.insert(ip); // fallthrough
                    }
                    if op == Op::Jump {
                        // no fallthrough
                    } else if op == Op::Loop {
                        leaders.insert(target);
                    }
                }
            }
            Op::MakeClosure => {
                if ip < chunk.code.len() {
                    let n = chunk.code[ip] as usize;
                    ip += 1 + n * 2;
                }
            }
            Op::TryEnd => {}
            _ => {}
        }
        let _ = line_ip;
    }
    leaders
}

fn translate_range(
    func: &mut SsaFunction,
    chunk: &Chunk,
    start: usize,
    end: usize,
    bb: BlockId,
    ip_to_block: &HashMap<usize, BlockId>,
    slot_ptrs: &[ValueId],
    is_entry: bool,
) {
    let mut stack: Vec<ValueId> = Vec::new();
    let mut ip = start;
    // Non-entry blocks: stack state is reconstructed poorly for stack-VM lift.
    // We use a pragmatic approach: each value pushed is independent; Jump doesn't
    // carry stack across blocks for RayTask's structured compiler (stack empty at
    // block boundaries for control flow). Assert empty at terminators when possible.

    while ip < end {
        let line = chunk.lines.get(ip).copied().unwrap_or(0);
        let op = match Op::from_byte(chunk.code[ip]) {
            Some(o) => o,
            None => break,
        };
        ip += 1;

        let mk = |id: ValueId, kind: InstKind, ty: SsaTy, effectful: bool| Inst {
            id,
            kind,
            ty,
            line,
            effectful,
        };

        match op {
            Op::Constant => {
                let idx = chunk.code[ip] as usize;
                ip += 1;
                let c = chunk.constants.get(idx).cloned().unwrap_or(Value::Null);
                let vid = func.alloc_value();
                let cv = value_to_const(&c);
                func.push_inst(bb, mk(vid, InstKind::Const(cv), SsaTy::Dyn, false));
                stack.push(vid);
            }
            Op::Null => {
                let vid = func.alloc_value();
                func.push_inst(bb, mk(vid, InstKind::Const(ConstValue::Null), SsaTy::Dyn, false));
                stack.push(vid);
            }
            Op::True => {
                let vid = func.alloc_value();
                func.push_inst(
                    bb,
                    mk(vid, InstKind::Const(ConstValue::Bool(true)), SsaTy::Bool, false),
                );
                stack.push(vid);
            }
            Op::False => {
                let vid = func.alloc_value();
                func.push_inst(
                    bb,
                    mk(
                        vid,
                        InstKind::Const(ConstValue::Bool(false)),
                        SsaTy::Bool,
                        false,
                    ),
                );
                stack.push(vid);
            }
            Op::Pop => {
                stack.pop();
            }
            Op::GetLocal => {
                let slot = chunk.code[ip] as usize;
                ip += 1;
                let ptr = slot_ptrs.get(slot).copied().unwrap_or(slot_ptrs[0]);
                let vid = func.alloc_value();
                func.push_inst(bb, mk(vid, InstKind::Load { ptr }, SsaTy::Dyn, false));
                stack.push(vid);
            }
            Op::SetLocal => {
                let slot = chunk.code[ip] as usize;
                ip += 1;
                let val = stack.last().copied().unwrap_or_else(|| {
                    let v = func.alloc_value();
                    func.push_inst(bb, mk(v, InstKind::Const(ConstValue::Null), SsaTy::Dyn, false));
                    v
                });
                let ptr = slot_ptrs.get(slot).copied().unwrap_or(slot_ptrs[0]);
                let _nid = func.alloc_value();
                let _ = func.push_inst(
                    bb,
                    mk(
                        _nid,
                        InstKind::Store { ptr, value: val },
                        SsaTy::Void,
                        true,
                    ),
                );
            }
            Op::IncLocal | Op::DecLocal => {
                let slot = chunk.code[ip] as usize;
                ip += 1;
                let ptr = slot_ptrs.get(slot).copied().unwrap_or(slot_ptrs[0]);
                let cur = func.alloc_value();
                func.push_inst(bb, mk(cur, InstKind::Load { ptr }, SsaTy::Dyn, false));
                let one = func.alloc_value();
                func.push_inst(
                    bb,
                    mk(one, InstKind::Const(ConstValue::Int(1)), SsaTy::Int, false),
                );
                let res = func.alloc_value();
                let bop = if op == Op::IncLocal {
                    BinOpKind::Add
                } else {
                    BinOpKind::Sub
                };
                func.push_inst(
                    bb,
                    mk(
                        res,
                        InstKind::BinOp {
                            op: bop,
                            lhs: cur,
                            rhs: one,
                        },
                        SsaTy::Dyn,
                        false,
                    ),
                );
                let _nid = func.alloc_value();
                let _ = func.push_inst(
                    bb,
                    mk(
                        _nid,
                        InstKind::Store { ptr, value: res },
                        SsaTy::Void,
                        true,
                    ),
                );
            }
            Op::GetGlobal => {
                let idx = chunk.code[ip] as u32;
                ip += 1;
                let vid = func.alloc_value();
                func.push_inst(bb, mk(vid, InstKind::GetGlobal { index: idx }, SsaTy::Dyn, false));
                stack.push(vid);
            }
            Op::SetGlobal => {
                let idx = chunk.code[ip] as u32;
                ip += 1;
                let val = stack.last().copied().unwrap_or_else(|| null_const(func, bb, line));
                let _nid = func.alloc_value();
                let _ = func.push_inst(
                    bb,
                    mk(
                        _nid,
                        InstKind::SetGlobal { index: idx, value: val },
                        SsaTy::Void,
                        true,
                    ),
                );
            }
            Op::DefineGlobal => {
                let idx = chunk.code[ip] as u32;
                ip += 1;
                let val = stack.pop().unwrap_or_else(|| null_const(func, bb, line));
                let _nid = func.alloc_value();
                let _ = func.push_inst(
                    bb,
                    mk(
                        _nid,
                        InstKind::DefineGlobal { index: idx, value: val },
                        SsaTy::Void,
                        true,
                    ),
                );
            }
            Op::GetProperty => {
                let name = stack.pop().unwrap_or_else(|| null_const(func, bb, line));
                let obj = stack.pop().unwrap_or_else(|| null_const(func, bb, line));
                let vid = func.alloc_value();
                func.push_inst(
                    bb,
                    mk(
                        vid,
                        InstKind::GetProperty {
                            object: obj,
                            name,
                        },
                        SsaTy::Dyn,
                        false,
                    ),
                );
                stack.push(vid);
            }
            Op::SetProperty => {
                let val = stack.pop().unwrap_or_else(|| null_const(func, bb, line));
                let name = stack.pop().unwrap_or_else(|| null_const(func, bb, line));
                let obj = stack.pop().unwrap_or_else(|| null_const(func, bb, line));
                let _nid = func.alloc_value();
                let _ = func.push_inst(
                    bb,
                    mk(
                        _nid,
                        InstKind::SetProperty {
                            object: obj,
                            name,
                            value: val,
                        },
                        SsaTy::Void,
                        true,
                    ),
                );
            }
            Op::GetIndex => {
                let index = stack.pop().unwrap_or_else(|| null_const(func, bb, line));
                let obj = stack.pop().unwrap_or_else(|| null_const(func, bb, line));
                let vid = func.alloc_value();
                func.push_inst(
                    bb,
                    mk(
                        vid,
                        InstKind::GetIndex {
                            object: obj,
                            index,
                        },
                        SsaTy::Dyn,
                        false,
                    ),
                );
                stack.push(vid);
            }
            Op::SetIndex => {
                let val = stack.pop().unwrap_or_else(|| null_const(func, bb, line));
                let index = stack.pop().unwrap_or_else(|| null_const(func, bb, line));
                let obj = stack.pop().unwrap_or_else(|| null_const(func, bb, line));
                let _nid = func.alloc_value();
                let _ = func.push_inst(
                    bb,
                    mk(
                        _nid,
                        InstKind::SetIndex {
                            object: obj,
                            index,
                            value: val,
                        },
                        SsaTy::Void,
                        true,
                    ),
                );
            }
            Op::Add
            | Op::Sub
            | Op::Mul
            | Op::Div
            | Op::Mod
            | Op::Eq
            | Op::Ne
            | Op::Lt
            | Op::Le
            | Op::Gt
            | Op::Ge
            | Op::And
            | Op::Or
            | Op::BitAnd
            | Op::BitOr
            | Op::BitXor
            | Op::Shl
            | Op::Shr
            | Op::NullCoalesce => {
                let rhs = stack.pop().unwrap_or_else(|| null_const(func, bb, line));
                let lhs = stack.pop().unwrap_or_else(|| null_const(func, bb, line));
                let vid = func.alloc_value();
                func.push_inst(
                    bb,
                    mk(
                        vid,
                        InstKind::BinOp {
                            op: binop_from_op(op),
                            lhs,
                            rhs,
                        },
                        SsaTy::Dyn,
                        false,
                    ),
                );
                stack.push(vid);
            }
            Op::Neg | Op::Not | Op::BitNot | Op::IsNull | Op::ToString => {
                let arg = stack.pop().unwrap_or_else(|| null_const(func, bb, line));
                let vid = func.alloc_value();
                func.push_inst(
                    bb,
                    mk(
                        vid,
                        InstKind::UnOp {
                            op: unop_from_op(op),
                            arg,
                        },
                        SsaTy::Dyn,
                        false,
                    ),
                );
                stack.push(vid);
            }
            Op::Jump => {
                let off = u16::from_be_bytes([chunk.code[ip], chunk.code[ip + 1]]) as usize;
                ip += 2;
                let target = ip + off;
                let tbb = *ip_to_block.get(&target).unwrap_or(&bb);
                func.set_term(bb, Terminator::Br(tbb));
                return;
            }
            Op::JumpIfFalse | Op::JumpIfTrue => {
                let off = u16::from_be_bytes([chunk.code[ip], chunk.code[ip + 1]]) as usize;
                ip += 2;
                let target = ip + off;
                let cond = stack.pop().unwrap_or_else(|| null_const(func, bb, line));
                let c = cond;
                if op == Op::JumpIfTrue {
                    // Invert: JumpIfTrue target = CondBr(not cond, fall, target) → CondBr(cond, target, fall)
                    // JumpIfTrue: if true jump to target, else fallthrough
                    let tbb = *ip_to_block.get(&target).unwrap_or(&bb);
                    let fbb = *ip_to_block.get(&ip).unwrap_or(&bb);
                    func.set_term(
                        bb,
                        Terminator::CondBr {
                            cond: c,
                            then_bb: tbb,
                            else_bb: fbb,
                        },
                    );
                } else {
                    // JumpIfFalse: if false jump to target
                    let tbb = *ip_to_block.get(&target).unwrap_or(&bb);
                    let fbb = *ip_to_block.get(&ip).unwrap_or(&bb);
                    func.set_term(
                        bb,
                        Terminator::CondBr {
                            cond: c,
                            then_bb: fbb,
                            else_bb: tbb,
                        },
                    );
                }
                let _ = c;
                return;
            }
            Op::Loop => {
                let off = u16::from_be_bytes([chunk.code[ip], chunk.code[ip + 1]]) as usize;
                ip += 2;
                let target = ip - off;
                let tbb = *ip_to_block.get(&target).unwrap_or(&bb);
                func.set_term(bb, Terminator::Br(tbb));
                return;
            }
            Op::Call => {
                let argc = chunk.code[ip] as usize;
                ip += 1;
                let mut args = Vec::new();
                for _ in 0..argc {
                    args.push(stack.pop().unwrap_or_else(|| null_const(func, bb, line)));
                }
                args.reverse();
                let callee = stack.pop().unwrap_or_else(|| null_const(func, bb, line));
                let vid = func.alloc_value();
                func.push_inst(
                    bb,
                    mk(
                        vid,
                        InstKind::Call {
                            callee,
                            args,
                            effectful: true,
                        },
                        SsaTy::Dyn,
                        true,
                    ),
                );
                stack.push(vid);
            }
            Op::Return => {
                let v = stack.pop();
                func.set_term(bb, Terminator::Return(v));
                return;
            }
            Op::Halt => {
                func.set_term(bb, Terminator::Halt);
                return;
            }
            Op::Throw => {
                let v = stack.pop().unwrap_or_else(|| null_const(func, bb, line));
                func.set_term(bb, Terminator::Throw(v));
                return;
            }
            Op::NewObject => {
                let ci = chunk.code[ip];
                ip += 1;
                let vid = func.alloc_value();
                func.push_inst(
                    bb,
                    mk(
                        vid,
                        InstKind::NewObject {
                            class_index: ci,
                            name: None,
                        },
                        SsaTy::Ref,
                        true,
                    ),
                );
                stack.push(vid);
            }
            Op::NewArray => {
                let n = chunk.code[ip] as usize;
                ip += 1;
                let mut elems = Vec::new();
                for _ in 0..n {
                    elems.push(stack.pop().unwrap_or_else(|| null_const(func, bb, line)));
                }
                elems.reverse();
                let vid = func.alloc_value();
                func.push_inst(bb, mk(vid, InstKind::NewArray { elems }, SsaTy::Ref, true));
                stack.push(vid);
            }
            Op::Print => {
                let v = stack.pop().unwrap_or_else(|| null_const(func, bb, line));
                let _pid = func.alloc_value();
                let _ = func.push_inst(
                    bb,
                    mk(_pid, InstKind::Print { value: v }, SsaTy::Void, true),
                );
            }
            Op::Dup => {
                if let Some(&top) = stack.last() {
                    stack.push(top);
                }
            }
            Op::Await => {
                let v = stack.pop().unwrap_or_else(|| null_const(func, bb, line));
                let vid = func.alloc_value();
                func.push_inst(bb, mk(vid, InstKind::Await { value: v }, SsaTy::Dyn, true));
                stack.push(vid);
            }
            Op::GetUpvalue => {
                let idx = chunk.code[ip];
                ip += 1;
                let vid = func.alloc_value();
                func.push_inst(
                    bb,
                    mk(vid, InstKind::GetUpvalue { index: idx }, SsaTy::Dyn, false),
                );
                stack.push(vid);
            }
            Op::SetUpvalue => {
                let idx = chunk.code[ip];
                ip += 1;
                let val = stack.last().copied().unwrap_or_else(|| null_const(func, bb, line));
                let _nid = func.alloc_value();
                let _ = func.push_inst(
                    bb,
                    mk(
                        _nid,
                        InstKind::SetUpvalue { index: idx, value: val },
                        SsaTy::Void,
                        true,
                    ),
                );
            }
            Op::MakeClosure => {
                let n = chunk.code[ip] as usize;
                ip += 1;
                let mut captures = Vec::new();
                for _ in 0..n {
                    let is_local = chunk.code[ip] != 0;
                    let idx = chunk.code[ip + 1];
                    ip += 2;
                    captures.push((is_local, idx));
                }
                let proto = stack.pop().unwrap_or_else(|| null_const(func, bb, line));
                let vid = func.alloc_value();
                func.push_inst(
                    bb,
                    mk(
                        vid,
                        InstKind::MakeClosure { proto, captures },
                        SsaTy::Ref,
                        true,
                    ),
                );
                stack.push(vid);
            }
            Op::TryBegin => {
                // Skip operand; treat as opaque barrier (no CFG exceptional edge)
                ip += 2;
            }
            Op::TryEnd => {}
        }
    }
    let _ = (is_entry, stack);
}

fn null_const(func: &mut SsaFunction, bb: BlockId, line: usize) -> ValueId {
    let vid = func.alloc_value();
    func.push_inst(
        bb,
        Inst {
            id: vid,
            kind: InstKind::Const(ConstValue::Null),
            ty: SsaTy::Dyn,
            line,
            effectful: false,
        },
    );
    vid
}

fn value_to_const(v: &Value) -> ConstValue {
    match v {
        Value::Null => ConstValue::Null,
        Value::Bool(b) => ConstValue::Bool(*b),
        Value::Int(i) => ConstValue::Int(*i),
        Value::Float(f) => ConstValue::Float(*f),
        Value::String(s) => ConstValue::String(s.to_string()),
        Value::Function(FunctionRef { chunk_index, .. }) => {
            ConstValue::FuncRef(FuncId(*chunk_index as u32))
        }
        Value::Native(id) => ConstValue::Native(*id),
        Value::TypeModule(n) => ConstValue::TypeModule(n.to_string()),
        _ => ConstValue::Null,
    }
}

fn binop_from_op(op: Op) -> BinOpKind {
    match op {
        Op::Add => BinOpKind::Add,
        Op::Sub => BinOpKind::Sub,
        Op::Mul => BinOpKind::Mul,
        Op::Div => BinOpKind::Div,
        Op::Mod => BinOpKind::Mod,
        Op::Eq => BinOpKind::Eq,
        Op::Ne => BinOpKind::Ne,
        Op::Lt => BinOpKind::Lt,
        Op::Le => BinOpKind::Le,
        Op::Gt => BinOpKind::Gt,
        Op::Ge => BinOpKind::Ge,
        Op::And => BinOpKind::And,
        Op::Or => BinOpKind::Or,
        Op::BitAnd => BinOpKind::BitAnd,
        Op::BitOr => BinOpKind::BitOr,
        Op::BitXor => BinOpKind::BitXor,
        Op::Shl => BinOpKind::Shl,
        Op::Shr => BinOpKind::Shr,
        Op::NullCoalesce => BinOpKind::NullCoalesce,
        _ => BinOpKind::Add,
    }
}

fn unop_from_op(op: Op) -> UnOpKind {
    match op {
        Op::Neg => UnOpKind::Neg,
        Op::Not => UnOpKind::Not,
        Op::BitNot => UnOpKind::BitNot,
        Op::IsNull => UnOpKind::IsNull,
        Op::ToString => UnOpKind::ToString,
        _ => UnOpKind::Not,
    }
}
