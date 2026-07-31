//! Sparse conditional constant propagation.

use crate::ssa::cfg::rebuild_edges;
use crate::ssa::ir::*;
use crate::ssa::pass::Pass;
use std::collections::{HashMap, HashSet, VecDeque};

#[derive(Clone, Debug, PartialEq)]
enum Lat {
    Undef,
    Const(ConstValue),
    Overdef,
}

pub struct Sccp;

impl Pass for Sccp {
    fn name(&self) -> &'static str {
        "sccp"
    }

    fn run(&mut self, module: &mut SsaModule) -> bool {
        let mut changed = false;
        for func in &mut module.functions {
            rebuild_edges(func);
            if run_sccp(func) {
                changed = true;
            }
        }
        changed
    }
}

fn run_sccp(func: &mut SsaFunction) -> bool {
    let mut values: HashMap<ValueId, Lat> = HashMap::new();
    let mut executable: HashSet<(BlockId, BlockId)> = HashSet::new(); // (from,to) edges; entry uses (entry,entry)
    let mut cfg_work: VecDeque<BlockId> = VecDeque::new();
    let mut ssa_work: VecDeque<ValueId> = VecDeque::new();

    executable.insert((func.entry, func.entry));
    cfg_work.push_back(func.entry);

    // Seed params/consts
    for (_, inst) in func.values() {
        match &inst.kind {
            InstKind::Const(c) => {
                values.insert(inst.id, Lat::Const(c.clone()));
            }
            InstKind::Param { .. } => {
                values.insert(inst.id, Lat::Overdef);
            }
            _ => {
                values.insert(inst.id, Lat::Undef);
            }
        }
    }

    while !cfg_work.is_empty() || !ssa_work.is_empty() {
        while let Some(bid) = cfg_work.pop_front() {
            let insts = func.block(bid).insts.clone();
            for inst in &insts {
                visit_inst(func, &inst, &mut values, &mut ssa_work);
            }
            match func.block(bid).term.clone() {
                Terminator::Br(t) => {
                    if executable.insert((bid, t)) {
                        cfg_work.push_back(t);
                    }
                }
                Terminator::CondBr {
                    cond,
                    then_bb,
                    else_bb,
                } => match values.get(&cond).cloned().unwrap_or(Lat::Undef) {
                    Lat::Const(ConstValue::Bool(true)) => {
                        if executable.insert((bid, then_bb)) {
                            cfg_work.push_back(then_bb);
                        }
                    }
                    Lat::Const(ConstValue::Bool(false)) => {
                        if executable.insert((bid, else_bb)) {
                            cfg_work.push_back(else_bb);
                        }
                    }
                    Lat::Overdef | Lat::Const(_) => {
                        if executable.insert((bid, then_bb)) {
                            cfg_work.push_back(then_bb);
                        }
                        if executable.insert((bid, else_bb)) {
                            cfg_work.push_back(else_bb);
                        }
                    }
                    Lat::Undef => {}
                },
                _ => {}
            }
        }
        while let Some(vid) = ssa_work.pop_front() {
            if let Some(inst) = func.find_def(vid).cloned() {
                visit_inst(func, &inst, &mut values, &mut ssa_work);
            }
        }
    }

    // Apply constants
    let mut changed = false;
    let const_map: Vec<(ValueId, ConstValue)> = values
        .iter()
        .filter_map(|(id, lat)| match lat {
            Lat::Const(c) => Some((*id, c.clone())),
            _ => None,
        })
        .collect();

    for (id, c) in &const_map {
        if let Some(inst) = func.find_def(*id) {
            if !matches!(&inst.kind, InstKind::Const(_)) && !inst.effectful {
                let bid = find_block(func, *id);
                if let Some(bid) = bid {
                    if let Some(slot) = func.block_mut(bid).insts.iter_mut().find(|i| i.id == *id)
                    {
                        slot.kind = InstKind::Const(c.clone());
                        changed = true;
                    }
                }
            }
        }
    }

    // Fold constant branches
    let bids: Vec<_> = func.blocks.keys().copied().collect();
    for bid in bids {
        if let Terminator::CondBr {
            cond,
            then_bb,
            else_bb,
        } = func.block(bid).term.clone()
        {
            if let Some(Lat::Const(ConstValue::Bool(b))) = values.get(&cond) {
                let t = if *b { then_bb } else { else_bb };
                func.set_term(bid, Terminator::Br(t));
                changed = true;
            }
        }
    }
    rebuild_edges(func);
    changed
}

fn find_block(func: &SsaFunction, id: ValueId) -> Option<BlockId> {
    for (bid, b) in &func.blocks {
        if b.insts.iter().any(|i| i.id == id) {
            return Some(*bid);
        }
    }
    None
}

fn visit_inst(
    func: &SsaFunction,
    inst: &Inst,
    values: &mut HashMap<ValueId, Lat>,
    ssa_work: &mut VecDeque<ValueId>,
) {
    let new = eval(func, inst, values);
    let old = values.get(&inst.id).cloned().unwrap_or(Lat::Undef);
    if meet_changed(&old, &new) {
        values.insert(inst.id, meet(&old, &new));
        ssa_work.push_back(inst.id);
        // Also users — approximate by pushing all
    }
}

fn eval(_func: &SsaFunction, inst: &Inst, values: &HashMap<ValueId, Lat>) -> Lat {
    match &inst.kind {
        InstKind::Const(c) => Lat::Const(c.clone()),
        InstKind::Param { .. } => Lat::Overdef,
        InstKind::BinOp { op, lhs, rhs } => {
            let l = values.get(lhs).cloned().unwrap_or(Lat::Undef);
            let r = values.get(rhs).cloned().unwrap_or(Lat::Undef);
            match (l, r) {
                (Lat::Const(a), Lat::Const(b)) => fold_bin_lat(op, &a, &b),
                (Lat::Overdef, _) | (_, Lat::Overdef) => Lat::Overdef,
                _ => Lat::Undef,
            }
        }
        InstKind::UnOp { op, arg } => match values.get(arg).cloned().unwrap_or(Lat::Undef) {
            Lat::Const(a) => fold_un_lat(op, &a),
            Lat::Overdef => Lat::Overdef,
            Lat::Undef => Lat::Undef,
        },
        InstKind::Phi { incomings } => {
            let mut acc = Lat::Undef;
            for (_, v) in incomings {
                let lv = values.get(v).cloned().unwrap_or(Lat::Undef);
                acc = meet(&acc, &lv);
            }
            acc
        }
        InstKind::Dup { value } => values.get(value).cloned().unwrap_or(Lat::Undef),
        // Effectful / memory — overdef
        InstKind::Call { .. }
        | InstKind::Load { .. }
        | InstKind::GetGlobal { .. }
        | InstKind::GetProperty { .. }
        | InstKind::GetIndex { .. }
        | InstKind::Await { .. }
        | InstKind::GetUpvalue { .. }
        | InstKind::NewObject { .. }
        | InstKind::NewArray { .. }
        | InstKind::MakeClosure { .. } => Lat::Overdef,
        _ => Lat::Overdef,
    }
}

fn meet(a: &Lat, b: &Lat) -> Lat {
    match (a, b) {
        (Lat::Undef, x) | (x, Lat::Undef) => x.clone(),
        (Lat::Overdef, _) | (_, Lat::Overdef) => Lat::Overdef,
        (Lat::Const(x), Lat::Const(y)) if const_eq(x, y) => Lat::Const(x.clone()),
        (Lat::Const(_), Lat::Const(_)) => Lat::Overdef,
    }
}

fn meet_changed(old: &Lat, new: &Lat) -> bool {
    meet(old, new) != *old
}

fn const_eq(a: &ConstValue, b: &ConstValue) -> bool {
    match (a, b) {
        (ConstValue::Null, ConstValue::Null) => true,
        (ConstValue::Bool(x), ConstValue::Bool(y)) => x == y,
        (ConstValue::Int(x), ConstValue::Int(y)) => x == y,
        (ConstValue::Float(x), ConstValue::Float(y)) => x == y,
        (ConstValue::String(x), ConstValue::String(y)) => x == y,
        _ => false,
    }
}

fn fold_bin_lat(op: &BinOpKind, l: &ConstValue, r: &ConstValue) -> Lat {
    match (op, l, r) {
        (BinOpKind::Add, ConstValue::Int(a), ConstValue::Int(b)) => {
            Lat::Const(ConstValue::Int(a.wrapping_add(*b)))
        }
        (BinOpKind::Sub, ConstValue::Int(a), ConstValue::Int(b)) => {
            Lat::Const(ConstValue::Int(a.wrapping_sub(*b)))
        }
        (BinOpKind::Mul, ConstValue::Int(a), ConstValue::Int(b)) => {
            Lat::Const(ConstValue::Int(a.wrapping_mul(*b)))
        }
        (BinOpKind::Eq, ConstValue::Int(a), ConstValue::Int(b)) => {
            Lat::Const(ConstValue::Bool(a == b))
        }
        (BinOpKind::Lt, ConstValue::Int(a), ConstValue::Int(b)) => {
            Lat::Const(ConstValue::Bool(a < b))
        }
        (BinOpKind::And, ConstValue::Bool(a), ConstValue::Bool(b)) => {
            Lat::Const(ConstValue::Bool(*a && *b))
        }
        (BinOpKind::Or, ConstValue::Bool(a), ConstValue::Bool(b)) => {
            Lat::Const(ConstValue::Bool(*a || *b))
        }
        _ => Lat::Overdef,
    }
}

fn fold_un_lat(op: &UnOpKind, a: &ConstValue) -> Lat {
    match (op, a) {
        (UnOpKind::Not, ConstValue::Bool(b)) => Lat::Const(ConstValue::Bool(!*b)),
        (UnOpKind::Neg, ConstValue::Int(i)) => Lat::Const(ConstValue::Int(-*i)),
        _ => Lat::Overdef,
    }
}
