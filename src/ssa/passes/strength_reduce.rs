//! Induction-variable recognition and strength reduction.

use crate::ssa::cfg::{find_loops, rebuild_edges};
use crate::ssa::ir::*;
use crate::ssa::pass::Pass;
use std::collections::HashSet;

pub struct StrengthReduce;

impl Pass for StrengthReduce {
    fn name(&self) -> &'static str {
        "strength-reduce"
    }

    fn run(&mut self, module: &mut SsaModule) -> bool {
        let mut changed = false;
        for func in &mut module.functions {
            rebuild_edges(func);
            if reduce_muls_in_loops(func) {
                changed = true;
            }
        }
        changed
    }
}

/// Rewrite `iv * C` inside a loop where `iv` is a basic induction variable
/// (`iv = phi(init, iv+step)`) into an add-recurrence.
fn reduce_muls_in_loops(func: &mut SsaFunction) -> bool {
    let loops = find_loops(func);
    let mut changed = false;

    for lp in &loops {
        // Find basic IVs: phi at header with incomings (preheader: init, latch: iv+step)
        let header_insts = func.block(lp.header).insts.clone();
        for phi_inst in &header_insts {
            let InstKind::Phi { incomings } = &phi_inst.kind else {
                continue;
            };
            if incomings.len() != 2 {
                continue;
            }
            // Identify which incoming is the add-recurrence
            let mut init = None;
            let mut step_const = None;
            for &(pred, v) in incomings {
                if lp.body.contains(&pred) {
                    // Should be iv + step
                    if let Some(inst) = func.find_def(v) {
                        if let InstKind::BinOp {
                            op: BinOpKind::Add,
                            lhs,
                            rhs,
                        } = &inst.kind
                        {
                            let other = if *lhs == phi_inst.id {
                                *rhs
                            } else if *rhs == phi_inst.id {
                                *lhs
                            } else {
                                continue;
                            };
                            if let Some(Inst {
                                kind: InstKind::Const(ConstValue::Int(s)),
                                ..
                            }) = func.find_def(other)
                            {
                                step_const = Some(*s);
                            }
                        }
                    }
                } else {
                    init = Some(v);
                }
            }
            let (Some(_init_v), Some(step)) = (init, step_const) else {
                continue;
            };

            // Find iv * C in loop body
            let body: Vec<BlockId> = lp.body.iter().copied().collect();
            for &bid in &body {
                let insts = func.block(bid).insts.clone();
                for inst in insts {
                    if let InstKind::BinOp {
                        op: BinOpKind::Mul,
                        lhs,
                        rhs,
                    } = &inst.kind
                    {
                        let (iv, cval) = if *lhs == phi_inst.id {
                            (*lhs, *rhs)
                        } else if *rhs == phi_inst.id {
                            (*rhs, *lhs)
                        } else {
                            continue;
                        };
                        let _ = iv;
                        let Some(Inst {
                            kind: InstKind::Const(ConstValue::Int(c)),
                            ..
                        }) = func.find_def(cval)
                        else {
                            continue;
                        };
                        let c_mul = *c;
                        let stride = step.wrapping_mul(c_mul);
                        let Some(&(pre, init_incoming)) = incomings
                            .iter()
                            .find(|(p, _)| !lp.body.contains(p))
                        else {
                            continue;
                        };
                        let init_mul = if let Some(iv0) = match func.find_def(init_incoming) {
                            Some(Inst {
                                kind: InstKind::Const(ConstValue::Int(iv0)),
                                ..
                            }) => Some(*iv0),
                            _ => None,
                        } {
                            let folded = func.alloc_value();
                            func.block_mut(pre).insts.push(Inst {
                                id: folded,
                                kind: InstKind::Const(ConstValue::Int(iv0.wrapping_mul(c_mul))),
                                ty: SsaTy::Int,
                                line: 0,
                                effectful: false,
                            });
                            folded
                        } else {
                            let init_c = func.alloc_value();
                            func.block_mut(pre).insts.push(Inst {
                                id: init_c,
                                kind: InstKind::Const(ConstValue::Int(0)),
                                ty: SsaTy::Int,
                                line: 0,
                                effectful: false,
                            });
                            init_c
                        };

                        let derived_phi = func.alloc_value();
                        let stride_v = func.alloc_value();
                        // Add stride const + add in latch — put next value in header's latch pred
                        let latch = lp.latches.first().copied();
                        let Some(latch) = latch else {
                            continue;
                        };

                        func.block_mut(latch).insts.push(Inst {
                            id: stride_v,
                            kind: InstKind::Const(ConstValue::Int(stride)),
                            ty: SsaTy::Int,
                            line: 0,
                            effectful: false,
                        });
                        let next = func.alloc_value();
                        func.block_mut(latch).insts.push(Inst {
                            id: next,
                            kind: InstKind::BinOp {
                                op: BinOpKind::Add,
                                lhs: derived_phi,
                                rhs: stride_v,
                            },
                            ty: SsaTy::Dyn,
                            line: 0,
                            effectful: false,
                        });

                        let phi = Inst {
                            id: derived_phi,
                            kind: InstKind::Phi {
                                incomings: vec![(pre, init_mul), (latch, next)],
                            },
                            ty: SsaTy::Dyn,
                            line: 0,
                            effectful: false,
                        };
                        func.block_mut(lp.header).insts.insert(0, phi);
                        func.replace_uses(inst.id, derived_phi);
                        // Remove mul
                        func.block_mut(bid).insts.retain(|i| i.id != inst.id);
                        changed = true;
                    }
                }
            }
        }
    }
    let _ = HashSet::<BlockId>::new();
    changed
}
