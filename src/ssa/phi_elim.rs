//! Lower SSA phi nodes to stack slots + moves so the bytecode emitter can run.

use super::ir::*;
use std::collections::HashMap;

/// Replace each `Phi` with a dedicated local slot: preds store into it, block loads it.
pub fn eliminate_phis(func: &mut SsaFunction) {
    let bids: Vec<BlockId> = func.blocks.keys().copied().collect();
    let mut phis: Vec<(BlockId, ValueId, Vec<(BlockId, ValueId)>)> = Vec::new();
    for &bid in &bids {
        for inst in &func.block(bid).insts {
            if let InstKind::Phi { incomings } = &inst.kind {
                phis.push((bid, inst.id, incomings.clone()));
            }
        }
    }
    if phis.is_empty() {
        return;
    }

    for (bb, phi_id, incomings) in phis {
        let slot = func.local_count.min(254) as u32;
        func.local_count = (slot as usize) + 1;

        // Create alloca for the phi slot at entry
        let ptr = func.alloc_value();
        let entry = func.entry;
        func.block_mut(entry).insts.insert(
            0,
            Inst {
                id: ptr,
                kind: InstKind::Alloca {
                    ty: SsaTy::Dyn,
                    slot,
                },
                ty: SsaTy::Ref,
                line: 0,
                effectful: false,
            },
        );

        // At each predecessor, store incoming value
        for (pred, val) in &incomings {
            if val.0 == u32::MAX {
                continue;
            }
            let store_id = func.alloc_value();
            func.block_mut(*pred).insts.push(Inst {
                id: store_id,
                kind: InstKind::Store {
                    ptr,
                    value: *val,
                },
                ty: SsaTy::Void,
                line: 0,
                effectful: true,
            });
        }

        // Replace phi with load
        if let Some(inst) = func.block_mut(bb).insts.iter_mut().find(|i| i.id == phi_id) {
            inst.kind = InstKind::Load { ptr };
            inst.effectful = false;
        }
    }

    // Remove any remaining Phi kinds (safety)
    for b in func.blocks.values_mut() {
        for inst in &mut b.insts {
            if matches!(inst.kind, InstKind::Phi { .. }) {
                inst.kind = InstKind::Const(ConstValue::Null);
            }
        }
    }

    let _ = HashMap::<ValueId, u8>::new();
}
