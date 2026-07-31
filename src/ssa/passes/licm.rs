//! Loop-invariant code motion.

use crate::ssa::cfg::{dominates, find_loops, compute_dominators, rebuild_edges};
use crate::ssa::ir::*;
use crate::ssa::pass::Pass;
use std::collections::HashSet;

pub struct Licm;

impl Pass for Licm {
    fn name(&self) -> &'static str {
        "licm"
    }

    fn run(&mut self, module: &mut SsaModule) -> bool {
        let mut changed = false;
        for func in &mut module.functions {
            rebuild_edges(func);
            if hoist_invariants(func) {
                changed = true;
            }
        }
        changed
    }
}

fn hoist_invariants(func: &mut SsaFunction) -> bool {
    let loops = find_loops(func);
    let dom = compute_dominators(func);
    let mut changed = false;

    for lp in loops {
        // Preheader: unique predecessor of header outside the loop
        let preds_outside: Vec<BlockId> = func
            .block(lp.header)
            .preds
            .iter()
            .copied()
            .filter(|p| !lp.body.contains(p))
            .collect();
        if preds_outside.len() != 1 {
            continue;
        }
        let preheader = preds_outside[0];

        let body_blocks: Vec<BlockId> = lp.body.iter().copied().collect();
        let mut to_hoist: Vec<(BlockId, Inst)> = Vec::new();

        for &bid in &body_blocks {
            let insts = func.block(bid).insts.clone();
            for inst in insts {
                if inst.effectful {
                    continue;
                }
                if matches!(
                    inst.kind,
                    InstKind::Phi { .. }
                        | InstKind::Alloca { .. }
                        | InstKind::Param { .. }
                        | InstKind::Await { .. }
                ) {
                    continue;
                }
                if is_invariant(&inst, func, &lp.body) {
                    // Only hoist if definition dominates all uses — approx: hoist to preheader
                    if dominates(&dom, bid, lp.header) || bid == lp.header {
                        to_hoist.push((bid, inst));
                    } else if lp.body.contains(&bid) {
                        to_hoist.push((bid, inst));
                    }
                }
            }
        }

        for (bid, inst) in to_hoist {
            // Move to end of preheader (before any existing — append before term conceptually)
            func.block_mut(bid).insts.retain(|i| i.id != inst.id);
            func.block_mut(preheader).insts.push(inst);
            changed = true;
        }
    }
    changed
}

fn is_invariant(inst: &Inst, func: &SsaFunction, loop_body: &HashSet<BlockId>) -> bool {
    let deps = operand_values(&inst.kind);
    for d in deps {
        if let Some((_, def_inst)) = func.values().find(|(_, i)| i.id == d) {
            // If def is in loop, not invariant (unless it's also being considered — conservative)
            for (bid, b) in &func.blocks {
                if b.insts.iter().any(|i| i.id == d) && loop_body.contains(bid) {
                    // Constant are ok
                    if !matches!(def_inst.kind, InstKind::Const(_)) {
                        return false;
                    }
                }
            }
        }
    }
    matches!(
        inst.kind,
        InstKind::Const(_)
            | InstKind::BinOp { .. }
            | InstKind::UnOp { .. }
            | InstKind::GetGlobal { .. }
    ) || matches!(inst.kind, InstKind::BinOp { .. })
}

fn operand_values(kind: &InstKind) -> Vec<ValueId> {
    let mut v = Vec::new();
    match kind {
        InstKind::BinOp { lhs, rhs, .. } => {
            v.push(*lhs);
            v.push(*rhs);
        }
        InstKind::UnOp { arg, .. } => v.push(*arg),
        InstKind::Load { ptr } => v.push(*ptr),
        _ => {}
    }
    v
}
