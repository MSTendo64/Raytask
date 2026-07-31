//! Remove unreachable blocks and fold constant / trivial branches.

use crate::ssa::cfg::{rebuild_edges, reachable};
use crate::ssa::ir::{ConstValue, InstKind, SsaModule, Terminator};
use crate::ssa::pass::Pass;

pub struct CfgSimplify;

impl Pass for CfgSimplify {
    fn name(&self) -> &'static str {
        "cfg-simplify"
    }

    fn run(&mut self, module: &mut SsaModule) -> bool {
        let mut changed = false;
        for func in &mut module.functions {
            rebuild_edges(func);
            // Fold CondBr on constant bool
            let bids: Vec<_> = func.blocks.keys().copied().collect();
            for bid in bids {
                let term = func.block(bid).term.clone();
                if let Terminator::CondBr {
                    cond,
                    then_bb,
                    else_bb,
                } = term
                {
                    if let Some(inst) = func.find_def(cond) {
                        if let InstKind::Const(ConstValue::Bool(b)) = &inst.kind {
                            let target = if *b { then_bb } else { else_bb };
                            func.set_term(bid, Terminator::Br(target));
                            changed = true;
                        }
                    }
                }
            }
            rebuild_edges(func);
            let live = reachable(func);
            let dead: Vec<_> = func
                .blocks
                .keys()
                .copied()
                .filter(|b| !live.contains(b))
                .collect();
            for d in dead {
                func.blocks.remove(&d);
                changed = true;
            }
            rebuild_edges(func);
        }
        changed
    }
}
