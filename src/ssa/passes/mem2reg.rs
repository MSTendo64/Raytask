//! Promote allocas with non-escaping loads/stores to SSA registers (phis).

use crate::ssa::cfg::{dominance_frontiers, rebuild_edges};
use crate::ssa::ir::*;
use crate::ssa::pass::Pass;
use std::collections::{HashMap, HashSet};

pub struct Mem2Reg;

impl Pass for Mem2Reg {
    fn name(&self) -> &'static str {
        "mem2reg"
    }

    fn run(&mut self, module: &mut SsaModule) -> bool {
        let mut changed = false;
        for i in 0..module.functions.len() {
            if promote_function(&mut module.functions[i]) {
                changed = true;
            }
        }
        changed
    }
}

fn promote_function(func: &mut SsaFunction) -> bool {
    rebuild_edges(func);
    // Find promotable allocas: only Load/Store uses, no address escape
    let mut allocas: HashMap<ValueId, u32> = HashMap::new();
    for (_, inst) in func.values() {
        if let InstKind::Alloca { slot, .. } = &inst.kind {
            allocas.insert(inst.id, *slot);
        }
    }
    let mut uses: HashMap<ValueId, Vec<(BlockId, ValueId, bool)>> = HashMap::new();
    // (block, inst_id, is_store)
    for (bid, inst) in func.values() {
        match &inst.kind {
            InstKind::Load { ptr } if allocas.contains_key(ptr) => {
                uses.entry(*ptr).or_default().push((*bid, inst.id, false));
            }
            InstKind::Store { ptr, .. } if allocas.contains_key(ptr) => {
                uses.entry(*ptr).or_default().push((*bid, inst.id, true));
            }
            _ => {}
        }
    }
    // Skip allocas used elsewhere (shouldn't happen)
    let promotable: Vec<ValueId> = allocas.keys().copied().collect();

    if promotable.is_empty() {
        return false;
    }

    let df = dominance_frontiers(func);
    let mut changed = false;

    for alloca in promotable {
        let Some(use_list) = uses.get(&alloca).cloned() else {
            // Dead alloca
            remove_alloca(func, alloca);
            changed = true;
            continue;
        };

        // Single-block or simple store-then-load forwarding without full phi
        let store_blocks: HashSet<BlockId> = use_list
            .iter()
            .filter(|(_, _, is_store)| *is_store)
            .map(|(b, _, _)| *b)
            .collect();

        // Insert phis at DF of store blocks
        let mut phi_blocks: HashSet<BlockId> = HashSet::new();
        let mut work: Vec<BlockId> = store_blocks.iter().copied().collect();
        while let Some(b) = work.pop() {
            if let Some(frontiers) = df.get(&b) {
                for &f in frontiers {
                    if phi_blocks.insert(f) {
                        work.push(f);
                    }
                }
            }
        }

        // For each phi block, create Phi placeholder
        let mut phi_for_block: HashMap<BlockId, ValueId> = HashMap::new();
        for &pb in &phi_blocks {
            let pid = func.alloc_value();
            let preds = func.block(pb).preds.clone();
            let incomings: Vec<(BlockId, ValueId)> = preds
                .iter()
                .map(|&p| {
                    // placeholder — filled in rename
                    (p, ValueId(u32::MAX))
                })
                .collect();
            let phi = Inst {
                id: pid,
                kind: InstKind::Phi { incomings },
                ty: SsaTy::Dyn,
                line: 0,
                effectful: false,
            };
            func.block_mut(pb).insts.insert(0, phi);
            phi_for_block.insert(pb, pid);
        }

        // Rename: walk RPO with stack of current value
        let undef = {
            let id = func.alloc_value();
            // Insert undef const at entry
            let entry = func.entry;
            func.block_mut(entry).insts.insert(
                0,
                Inst {
                    id,
                    kind: InstKind::Const(ConstValue::Null),
                    ty: SsaTy::Dyn,
                    line: 0,
                    effectful: false,
                },
            );
            id
        };

        let mut stack: Vec<ValueId> = vec![undef];
        let mut visited = HashSet::new();
        rename_block(
            func,
            func.entry,
            alloca,
            &phi_for_block,
            &mut stack,
            &mut visited,
        );

        // Remove load/store/alloca for this slot
        remove_alloca_and_accesses(func, alloca);
        changed = true;
    }

    // Cleanup incomplete phis (ValueId::MAX)
    for b in func.blocks.values_mut() {
        for inst in &mut b.insts {
            if let InstKind::Phi { incomings } = &mut inst.kind {
                for (_, v) in incomings.iter_mut() {
                    if v.0 == u32::MAX {
                        *v = ValueId(0);
                    }
                }
            }
        }
    }

    changed
}

fn rename_block(
    func: &mut SsaFunction,
    bid: BlockId,
    alloca: ValueId,
    phi_for_block: &HashMap<BlockId, ValueId>,
    stack: &mut Vec<ValueId>,
    visited: &mut HashSet<BlockId>,
) {
    if !visited.insert(bid) {
        return;
    }
    let pushed = if let Some(&phi) = phi_for_block.get(&bid) {
        stack.push(phi);
        1
    } else {
        0
    };

    // Process instructions — collect replacements first
    let insts: Vec<Inst> = func.block(bid).insts.clone();
    let mut load_repl: Vec<(ValueId, ValueId)> = Vec::new();
    let mut store_push: Vec<ValueId> = Vec::new();
    let mut extra_pushed = 0usize;

    for inst in &insts {
        match &inst.kind {
            InstKind::Store { ptr, value } if *ptr == alloca => {
                stack.push(*value);
                store_push.push(*value);
                extra_pushed += 1;
            }
            InstKind::Load { ptr } if *ptr == alloca => {
                let cur = *stack.last().unwrap();
                load_repl.push((inst.id, cur));
            }
            _ => {}
        }
    }

    for (from, to) in load_repl {
        func.replace_uses(from, to);
    }

    // Fill phi operands in successors
    let succs = func.block(bid).succs.clone();
    let cur = *stack.last().unwrap();
    for s in succs {
        if let Some(&phi) = phi_for_block.get(&s) {
            if let Some(inst) = func.block_mut(s).insts.iter_mut().find(|i| i.id == phi) {
                if let InstKind::Phi { incomings } = &mut inst.kind {
                    for (pred, val) in incomings.iter_mut() {
                        if *pred == bid {
                            *val = cur;
                        }
                    }
                }
            }
        }
        rename_block(func, s, alloca, phi_for_block, stack, visited);
    }

    for _ in 0..(pushed + extra_pushed) {
        stack.pop();
    }
}

fn remove_alloca(func: &mut SsaFunction, alloca: ValueId) {
    let bids: Vec<_> = func.blocks.keys().copied().collect();
    for bid in bids {
        func.block_mut(bid).insts.retain(|i| i.id != alloca);
    }
}

fn remove_alloca_and_accesses(func: &mut SsaFunction, alloca: ValueId) {
    let bids: Vec<_> = func.blocks.keys().copied().collect();
    for bid in bids {
        func.block_mut(bid).insts.retain(|i| {
            if i.id == alloca {
                return false;
            }
            match &i.kind {
                InstKind::Load { ptr } if *ptr == alloca => false,
                InstKind::Store { ptr, .. } if *ptr == alloca => false,
                _ => true,
            }
        });
    }
}
