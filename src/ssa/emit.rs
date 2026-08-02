//! Emit SSA IR back to stack bytecode Module.

use super::cfg::block_rpo;
use super::ir::*;
use crate::bytecode::{Chunk, ClassInfo, Module, Op};
use crate::value::{FunctionRef, Value};
use std::collections::HashMap;
use std::rc::Rc;

pub fn emit_module(ssa: &SsaModule) -> Module {
    let func_meta: Vec<(String, usize)> = ssa
        .functions
        .iter()
        .map(|f| (f.name.clone(), f.arity))
        .collect();
    let mut chunks = Vec::new();
    for f in &ssa.functions {
        chunks.push(emit_function(f, &func_meta));
    }
    let classes = ssa
        .classes
        .iter()
        .map(|c| ClassInfo {
            name: c.name.clone(),
            kind: crate::bytecode::ClassKind::Class,
            fields: c.fields.clone(),
            field_types: c.fields.iter().map(|_| "dyn".into()).collect(),
            methods: c
                .methods
                .iter()
                .map(|(n, id)| (n.clone(), id.0 as usize))
                .collect(),
            constructor: c.constructor.map(|id| id.0 as usize),
            base: c.base,
            destructor: c.destructor.map(|id| id.0 as usize),
        })
        .collect();
    Module {
        chunks,
        main_chunk: ssa.main_chunk.0 as usize,
        globals: ssa.globals.clone(),
        classes,
        ffi: ssa.ffi.clone(),
        stdlib_enabled: ssa.stdlib_enabled,
    }
}

struct Emitter<'a> {
    #[allow(dead_code)]
    func: &'a SsaFunction,
    chunk: Chunk,
    alloca_slots: HashMap<ValueId, u8>,
    /// Spill slot for multi-use SSA values.
    spills: HashMap<ValueId, u8>,
    next_temp: u8,
    defs: HashMap<ValueId, &'a Inst>,
    func_meta: &'a [(String, usize)],
}

pub fn emit_function(func: &SsaFunction, func_meta: &[(String, usize)]) -> Chunk {
    let mut alloca_slots: HashMap<ValueId, u8> = HashMap::new();
    let mut defs: HashMap<ValueId, &Inst> = HashMap::new();
    for b in func.blocks.values() {
        for inst in &b.insts {
            defs.insert(inst.id, inst);
            if let InstKind::Alloca { slot, .. } = &inst.kind {
                alloca_slots.insert(inst.id, *slot as u8);
            }
        }
    }

    let mut chunk = Chunk::new(func.name.clone());
    chunk.arity = func.arity;
    chunk.local_count = func.local_count;
    chunk.is_async = func.is_async;
    chunk.source = func.source.clone();

    let next_temp = func.local_count.min(255) as u8;
    let mut em = Emitter {
        func,
        chunk,
        alloca_slots,
        spills: HashMap::new(),
        next_temp,
        defs,
        func_meta,
    };

    if func.blocks.is_empty() {
        em.chunk.emit_op(Op::Null, 0);
        em.chunk.emit_op(Op::Return, 0);
        return em.chunk;
    }

    let order = block_rpo(func);
    let mut block_start: HashMap<BlockId, usize> = HashMap::new();
    let mut pending: Vec<(usize, BlockId, bool)> = Vec::new(); // (operand_offset, target, is_loop_ok)

    for &bid in &order {
        block_start.insert(bid, em.chunk.code.len());
        let block = func.block(bid);

        for inst in &block.insts {
            em.emit_inst(inst);
        }

        match &block.term {
            Terminator::Br(t) => {
                let at = em.chunk.emit_jump(Op::Jump, 0);
                pending.push((at, *t, true));
            }
            Terminator::CondBr {
                cond,
                then_bb,
                else_bb,
            } => {
                em.push_value(*cond, 0);
                let jf = em.chunk.emit_jump(Op::JumpIfFalse, 0);
                pending.push((jf, *else_bb, false));
                let j = em.chunk.emit_jump(Op::Jump, 0);
                pending.push((j, *then_bb, true));
            }
            Terminator::Return(None) => {
                em.chunk.emit_op(Op::Null, 0);
                em.chunk.emit_op(Op::Return, 0);
            }
            Terminator::Return(Some(v)) => {
                em.push_value(*v, 0);
                em.chunk.emit_op(Op::Return, 0);
            }
            Terminator::Halt => em.chunk.emit_op(Op::Halt, 0),
            Terminator::Throw(v) => {
                em.push_value(*v, 0);
                em.chunk.emit_op(Op::Throw, 0);
            }
            Terminator::Unreachable => {
                em.chunk.emit_op(Op::Null, 0);
                em.chunk.emit_op(Op::Return, 0);
            }
        }
    }

    em.chunk.local_count = em.chunk.local_count.max(em.next_temp as usize);

    for (offset, target, loop_ok) in pending {
        let target_ip = block_start.get(&target).copied().unwrap_or(0);
        let after = offset + 2;
        if target_ip >= after {
            let jump = target_ip - after;
            em.chunk.code[offset] = ((jump >> 8) & 0xff) as u8;
            em.chunk.code[offset + 1] = (jump & 0xff) as u8;
        } else if loop_ok {
            let jump_op_ip = offset - 1;
            em.chunk.code[jump_op_ip] = Op::Loop as u8;
            let jump = after - target_ip;
            em.chunk.code[offset] = ((jump >> 8) & 0xff) as u8;
            em.chunk.code[offset + 1] = (jump & 0xff) as u8;
        } else {
            // Cannot encode backward JumpIfFalse; leave as forward 0 (dead)
            em.chunk.code[offset] = 0;
            em.chunk.code[offset + 1] = 0;
        }
    }

    em.chunk
}

impl<'a> Emitter<'a> {
    fn spill_slot(&mut self, v: ValueId) -> u8 {
        if let Some(&s) = self.spills.get(&v) {
            return s;
        }
        let s = self.next_temp;
        self.next_temp = self.next_temp.saturating_add(1);
        self.spills.insert(v, s);
        s
    }

    fn emit_inst(&mut self, inst: &Inst) {
        let line = inst.line;
        match &inst.kind {
            InstKind::Alloca { .. } | InstKind::Param { .. } => {}
            InstKind::Const(c) => {
                self.emit_const(c, line);
                self.store_result(inst.id, line);
            }
            InstKind::Load { ptr } => {
                if let Some(&slot) = self.alloca_slots.get(ptr) {
                    self.chunk.emit_op(Op::GetLocal, line);
                    self.chunk.emit_byte(slot, line);
                } else {
                    self.chunk.emit_op(Op::Null, line);
                }
                self.store_result(inst.id, line);
            }
            InstKind::Store { ptr, value } => {
                self.push_value(*value, line);
                if let Some(&slot) = self.alloca_slots.get(ptr) {
                    self.chunk.emit_op(Op::SetLocal, line);
                    self.chunk.emit_byte(slot, line);
                    self.chunk.emit_op(Op::Pop, line);
                } else {
                    self.chunk.emit_op(Op::Pop, line);
                }
            }
            InstKind::BinOp { op, lhs, rhs } => {
                self.push_value(*lhs, line);
                self.push_value(*rhs, line);
                self.chunk.emit_op(binop_op(op), line);
                self.store_result(inst.id, line);
            }
            InstKind::UnOp { op, arg } => {
                self.push_value(*arg, line);
                self.chunk.emit_op(unop_op(op), line);
                self.store_result(inst.id, line);
            }
            InstKind::Call { callee, args, .. } => {
                self.push_value(*callee, line);
                for a in args {
                    self.push_value(*a, line);
                }
                self.chunk.emit_op(Op::Call, line);
                self.chunk.emit_byte(args.len() as u8, line);
                self.store_result(inst.id, line);
            }
            InstKind::GetGlobal { index } => {
                self.chunk.emit_get_global(*index as u16, line);
                self.store_result(inst.id, line);
            }
            InstKind::SetGlobal { index, value } => {
                self.push_value(*value, line);
                self.chunk.emit_set_global(*index as u16, line);
                self.chunk.emit_op(Op::Pop, line);
            }
            InstKind::DefineGlobal { index, value } => {
                self.push_value(*value, line);
                self.chunk.emit_define_global(*index as u16, line);
            }
            InstKind::GetProperty { object, name } => {
                self.push_value(*object, line);
                self.push_value(*name, line);
                self.chunk.emit_op(Op::GetProperty, line);
                self.store_result(inst.id, line);
            }
            InstKind::SetProperty {
                object,
                name,
                value,
            } => {
                self.push_value(*object, line);
                self.push_value(*name, line);
                self.push_value(*value, line);
                self.chunk.emit_op(Op::SetProperty, line);
            }
            InstKind::GetIndex { object, index } => {
                self.push_value(*object, line);
                self.push_value(*index, line);
                self.chunk.emit_op(Op::GetIndex, line);
                self.store_result(inst.id, line);
            }
            InstKind::SetIndex {
                object,
                index,
                value,
            } => {
                self.push_value(*object, line);
                self.push_value(*index, line);
                self.push_value(*value, line);
                self.chunk.emit_op(Op::SetIndex, line);
            }
            InstKind::NewObject { class_index, .. } => {
                self.chunk.emit_op(Op::NewObject, line);
                self.chunk.emit_byte(*class_index, line);
                self.store_result(inst.id, line);
            }
            InstKind::NewArray { elems } => {
                for e in elems {
                    self.push_value(*e, line);
                }
                self.chunk.emit_op(Op::NewArray, line);
                self.chunk.emit_byte(elems.len() as u8, line);
                self.store_result(inst.id, line);
            }
            InstKind::Print { value } => {
                self.push_value(*value, line);
                self.chunk.emit_op(Op::Print, line);
            }
            InstKind::Dup { value } => {
                self.push_value(*value, line);
                self.store_result(inst.id, line);
            }
            InstKind::Await { value } => {
                self.push_value(*value, line);
                self.chunk.emit_op(Op::Await, line);
                self.store_result(inst.id, line);
            }
            InstKind::GetUpvalue { index } => {
                self.chunk.emit_op(Op::GetUpvalue, line);
                self.chunk.emit_byte(*index, line);
                self.store_result(inst.id, line);
            }
            InstKind::SetUpvalue { index, value } => {
                self.push_value(*value, line);
                self.chunk.emit_op(Op::SetUpvalue, line);
                self.chunk.emit_byte(*index, line);
                self.chunk.emit_op(Op::Pop, line);
            }
            InstKind::MakeClosure { proto, captures } => {
                self.push_value(*proto, line);
                self.chunk.emit_op(Op::MakeClosure, line);
                self.chunk.emit_byte(captures.len() as u8, line);
                for (is_local, idx) in captures {
                    self.chunk.emit_byte(if *is_local { 1 } else { 0 }, line);
                    self.chunk.emit_byte(*idx, line);
                }
                self.store_result(inst.id, line);
            }
            InstKind::Phi { incomings } => {
                if let Some((_, v)) = incomings.first() {
                    self.push_value(*v, line);
                    self.store_result(inst.id, line);
                }
            }
        }
    }

    /// After producing a value on stack, spill to temp so it can be rematerialized.
    fn store_result(&mut self, id: ValueId, line: usize) {
        let slot = self.spill_slot(id);
        self.chunk.emit_op(Op::SetLocal, line);
        self.chunk.emit_byte(slot, line);
        self.chunk.emit_op(Op::Pop, line);
    }

    fn push_value(&mut self, v: ValueId, line: usize) {
        if let Some(&slot) = self.spills.get(&v) {
            self.chunk.emit_op(Op::GetLocal, line);
            self.chunk.emit_byte(slot, line);
            return;
        }
        if let Some(inst) = self.defs.get(&v).copied() {
            match &inst.kind {
                InstKind::Const(c) => {
                    self.emit_const(c, line);
                    return;
                }
                InstKind::Load { ptr } => {
                    if let Some(&slot) = self.alloca_slots.get(ptr) {
                        self.chunk.emit_op(Op::GetLocal, line);
                        self.chunk.emit_byte(slot, line);
                        return;
                    }
                }
                InstKind::Param { index } => {
                    self.chunk.emit_op(Op::GetLocal, line);
                    self.chunk.emit_byte(*index as u8, line);
                    return;
                }
                InstKind::Alloca { slot, .. } => {
                    // Using alloca as value is a pointer — shouldn't happen on stack VM
                    let _ = slot;
                }
                _ => {
                    // Recompute once then spill
                    let id = inst.id;
                    // Avoid infinite recursion: emit computation inline without store_result first
                    self.recompute(inst);
                    let slot = self.spill_slot(id);
                    self.chunk.emit_op(Op::SetLocal, line);
                    self.chunk.emit_byte(slot, line);
                    // leave on stack (SetLocal leaves value)
                    return;
                }
            }
        }
        self.chunk.emit_op(Op::Null, line);
    }

    fn recompute(&mut self, inst: &Inst) {
        let line = inst.line;
        match &inst.kind {
            InstKind::BinOp { op, lhs, rhs } => {
                self.push_value(*lhs, line);
                self.push_value(*rhs, line);
                self.chunk.emit_op(binop_op(op), line);
            }
            InstKind::UnOp { op, arg } => {
                self.push_value(*arg, line);
                self.chunk.emit_op(unop_op(op), line);
            }
            InstKind::Call { callee, args, .. } => {
                self.push_value(*callee, line);
                for a in args {
                    self.push_value(*a, line);
                }
                self.chunk.emit_op(Op::Call, line);
                self.chunk.emit_byte(args.len() as u8, line);
            }
            InstKind::GetGlobal { index } => {
                self.chunk.emit_get_global(*index as u16, line);
            }
            InstKind::GetProperty { object, name } => {
                self.push_value(*object, line);
                self.push_value(*name, line);
                self.chunk.emit_op(Op::GetProperty, line);
            }
            InstKind::GetIndex { object, index } => {
                self.push_value(*object, line);
                self.push_value(*index, line);
                self.chunk.emit_op(Op::GetIndex, line);
            }
            InstKind::NewObject { class_index, .. } => {
                self.chunk.emit_op(Op::NewObject, line);
                self.chunk.emit_byte(*class_index, line);
            }
            InstKind::NewArray { elems } => {
                for e in elems {
                    self.push_value(*e, line);
                }
                self.chunk.emit_op(Op::NewArray, line);
                self.chunk.emit_byte(elems.len() as u8, line);
            }
            InstKind::Await { value } => {
                self.push_value(*value, line);
                self.chunk.emit_op(Op::Await, line);
            }
            InstKind::GetUpvalue { index } => {
                self.chunk.emit_op(Op::GetUpvalue, line);
                self.chunk.emit_byte(*index, line);
            }
            InstKind::MakeClosure { proto, captures } => {
                self.push_value(*proto, line);
                self.chunk.emit_op(Op::MakeClosure, line);
                self.chunk.emit_byte(captures.len() as u8, line);
                for (is_local, idx) in captures {
                    self.chunk.emit_byte(if *is_local { 1 } else { 0 }, line);
                    self.chunk.emit_byte(*idx, line);
                }
            }
            InstKind::Dup { value } => self.push_value(*value, line),
            InstKind::Phi { incomings } => {
                if let Some((_, v)) = incomings.first() {
                    self.push_value(*v, line);
                } else {
                    self.chunk.emit_op(Op::Null, line);
                }
            }
            _ => self.chunk.emit_op(Op::Null, line),
        }
    }

    fn emit_const(&mut self, c: &ConstValue, line: usize) {
        match c {
            ConstValue::Null => self.chunk.emit_op(Op::Null, line),
            ConstValue::Bool(true) => self.chunk.emit_op(Op::True, line),
            ConstValue::Bool(false) => self.chunk.emit_op(Op::False, line),
            ConstValue::Int(i) => self.chunk.emit_constant(Value::Int(*i), line),
            ConstValue::Float(f) => self.chunk.emit_constant(Value::Float(*f), line),
            ConstValue::String(s) => {
                self.chunk
                    .emit_constant(Value::String(Rc::from(s.as_str())), line)
            }
            ConstValue::FuncRef(fid) => {
                let (name, arity) = self
                    .func_meta
                    .get(fid.0 as usize)
                    .cloned()
                    .unwrap_or_else(|| (String::from("<fn>"), 0));
                self.chunk.emit_constant(
                    Value::Function(FunctionRef::plain(name, fid.0 as usize, arity)),
                    line,
                );
            }
            ConstValue::Native(id) => self.chunk.emit_constant(Value::Native(*id), line),
            ConstValue::TypeModule(n) => {
                self.chunk
                    .emit_constant(Value::TypeModule(Rc::from(n.as_str())), line)
            }
        }
    }
}

fn binop_op(op: &BinOpKind) -> Op {
    match op {
        BinOpKind::Add => Op::Add,
        BinOpKind::Sub => Op::Sub,
        BinOpKind::Mul => Op::Mul,
        BinOpKind::Div => Op::Div,
        BinOpKind::Mod => Op::Mod,
        BinOpKind::Eq => Op::Eq,
        BinOpKind::Ne => Op::Ne,
        BinOpKind::Lt => Op::Lt,
        BinOpKind::Le => Op::Le,
        BinOpKind::Gt => Op::Gt,
        BinOpKind::Ge => Op::Ge,
        BinOpKind::And => Op::And,
        BinOpKind::Or => Op::Or,
        BinOpKind::BitAnd => Op::BitAnd,
        BinOpKind::BitOr => Op::BitOr,
        BinOpKind::BitXor => Op::BitXor,
        BinOpKind::Shl => Op::Shl,
        BinOpKind::Shr => Op::Shr,
        BinOpKind::NullCoalesce => Op::NullCoalesce,
    }
}

fn unop_op(op: &UnOpKind) -> Op {
    match op {
        UnOpKind::Neg => Op::Neg,
        UnOpKind::Not => Op::Not,
        UnOpKind::BitNot => Op::BitNot,
        UnOpKind::IsNull => Op::IsNull,
        UnOpKind::ToString => Op::ToString,
    }
}
