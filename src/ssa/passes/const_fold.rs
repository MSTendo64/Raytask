//! Local constant folding and algebraic simplification.

use crate::ssa::ir::*;
use crate::ssa::pass::Pass;

pub struct ConstFold;

impl Pass for ConstFold {
    fn name(&self) -> &'static str {
        "const-fold"
    }

    fn run(&mut self, module: &mut SsaModule) -> bool {
        let mut changed = false;
        for func in &mut module.functions {
            let bids: Vec<_> = func.blocks.keys().copied().collect();
            for bid in bids {
                let insts = func.block(bid).insts.clone();
                for inst in insts {
                    if let Some(folded) = try_fold(func, &inst) {
                        let id = inst.id;
                        // Replace inst with const
                        if let Some(slot) = func
                            .block_mut(bid)
                            .insts
                            .iter_mut()
                            .find(|i| i.id == id)
                        {
                            slot.kind = InstKind::Const(folded);
                            slot.effectful = false;
                            changed = true;
                        }
                    }
                }
            }
        }
        changed
    }
}

fn const_of(func: &SsaFunction, v: ValueId) -> Option<ConstValue> {
    match &func.find_def(v)?.kind {
        InstKind::Const(c) => Some(c.clone()),
        _ => None,
    }
}

fn try_fold(func: &SsaFunction, inst: &Inst) -> Option<ConstValue> {
    match &inst.kind {
        InstKind::BinOp { op, lhs, rhs } => {
            if let (Some(l), Some(r)) = (const_of(func, *lhs), const_of(func, *rhs)) {
                if let Some(v) = fold_bin(op, &l, &r) {
                    return Some(v);
                }
            }
            // Algebraic: x + 0, 0 + x, x * 1, x * 0
            match op {
                BinOpKind::Add => {
                    if matches!(const_of(func, *rhs), Some(ConstValue::Int(0))) {
                        return const_of(func, *lhs);
                    }
                    if matches!(const_of(func, *lhs), Some(ConstValue::Int(0))) {
                        return const_of(func, *rhs);
                    }
                }
                BinOpKind::Mul => {
                    if matches!(const_of(func, *rhs), Some(ConstValue::Int(1))) {
                        return const_of(func, *lhs);
                    }
                    if matches!(const_of(func, *lhs), Some(ConstValue::Int(1))) {
                        return const_of(func, *rhs);
                    }
                    if matches!(const_of(func, *rhs), Some(ConstValue::Int(0)))
                        || matches!(const_of(func, *lhs), Some(ConstValue::Int(0)))
                    {
                        return Some(ConstValue::Int(0));
                    }
                }
                _ => {}
            }
            None
        }
        InstKind::UnOp { op, arg } => {
            let a = const_of(func, *arg)?;
            fold_un(op, &a)
        }
        _ => None,
    }
}

fn fold_bin(op: &BinOpKind, l: &ConstValue, r: &ConstValue) -> Option<ConstValue> {
    match (op, l, r) {
        (BinOpKind::Add, ConstValue::Int(a), ConstValue::Int(b)) => {
            Some(ConstValue::Int(a.wrapping_add(*b)))
        }
        (BinOpKind::Sub, ConstValue::Int(a), ConstValue::Int(b)) => {
            Some(ConstValue::Int(a.wrapping_sub(*b)))
        }
        (BinOpKind::Mul, ConstValue::Int(a), ConstValue::Int(b)) => {
            Some(ConstValue::Int(a.wrapping_mul(*b)))
        }
        (BinOpKind::Div, ConstValue::Int(a), ConstValue::Int(b)) if *b != 0 => {
            Some(ConstValue::Int(a / b))
        }
        (BinOpKind::Mod, ConstValue::Int(a), ConstValue::Int(b)) if *b != 0 => {
            Some(ConstValue::Int(a % b))
        }
        (BinOpKind::Eq, ConstValue::Int(a), ConstValue::Int(b)) => {
            Some(ConstValue::Bool(a == b))
        }
        (BinOpKind::Ne, ConstValue::Int(a), ConstValue::Int(b)) => {
            Some(ConstValue::Bool(a != b))
        }
        (BinOpKind::Lt, ConstValue::Int(a), ConstValue::Int(b)) => {
            Some(ConstValue::Bool(a < b))
        }
        (BinOpKind::Le, ConstValue::Int(a), ConstValue::Int(b)) => {
            Some(ConstValue::Bool(a <= b))
        }
        (BinOpKind::Gt, ConstValue::Int(a), ConstValue::Int(b)) => {
            Some(ConstValue::Bool(a > b))
        }
        (BinOpKind::Ge, ConstValue::Int(a), ConstValue::Int(b)) => {
            Some(ConstValue::Bool(a >= b))
        }
        (BinOpKind::And, ConstValue::Bool(a), ConstValue::Bool(b)) => {
            Some(ConstValue::Bool(*a && *b))
        }
        (BinOpKind::Or, ConstValue::Bool(a), ConstValue::Bool(b)) => {
            Some(ConstValue::Bool(*a || *b))
        }
        (BinOpKind::BitAnd, ConstValue::Int(a), ConstValue::Int(b)) => {
            Some(ConstValue::Int(a & b))
        }
        (BinOpKind::BitOr, ConstValue::Int(a), ConstValue::Int(b)) => {
            Some(ConstValue::Int(a | b))
        }
        (BinOpKind::BitXor, ConstValue::Int(a), ConstValue::Int(b)) => {
            Some(ConstValue::Int(a ^ b))
        }
        (BinOpKind::Shl, ConstValue::Int(a), ConstValue::Int(b)) => {
            Some(ConstValue::Int(a.wrapping_shl(*b as u32)))
        }
        (BinOpKind::Shr, ConstValue::Int(a), ConstValue::Int(b)) => {
            Some(ConstValue::Int(a.wrapping_shr(*b as u32)))
        }
        (BinOpKind::Add, ConstValue::Float(a), ConstValue::Float(b)) => {
            Some(ConstValue::Float(a + b))
        }
        (BinOpKind::Sub, ConstValue::Float(a), ConstValue::Float(b)) => {
            Some(ConstValue::Float(a - b))
        }
        (BinOpKind::Mul, ConstValue::Float(a), ConstValue::Float(b)) => {
            Some(ConstValue::Float(a * b))
        }
        (BinOpKind::Div, ConstValue::Float(a), ConstValue::Float(b)) if *b != 0.0 => {
            Some(ConstValue::Float(a / b))
        }
        _ => None,
    }
}

fn fold_un(op: &UnOpKind, a: &ConstValue) -> Option<ConstValue> {
    match (op, a) {
        (UnOpKind::Neg, ConstValue::Int(i)) => Some(ConstValue::Int(-*i)),
        (UnOpKind::Neg, ConstValue::Float(f)) => Some(ConstValue::Float(-*f)),
        (UnOpKind::Not, ConstValue::Bool(b)) => Some(ConstValue::Bool(!*b)),
        (UnOpKind::BitNot, ConstValue::Int(i)) => Some(ConstValue::Int(!*i)),
        (UnOpKind::IsNull, ConstValue::Null) => Some(ConstValue::Bool(true)),
        (UnOpKind::IsNull, _) => Some(ConstValue::Bool(false)),
        _ => None,
    }
}
