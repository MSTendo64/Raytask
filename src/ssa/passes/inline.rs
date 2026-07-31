//! Function inlining with size/speed heuristics.

use crate::ssa::ir::*;
use crate::ssa::pass::Pass;
use std::collections::HashMap;

pub struct Inline {
    max_callee_insts: usize,
    allow_multi_callsite: bool,
}

impl Inline {
    pub fn aggressive() -> Self {
        Self {
            max_callee_insts: 40,
            allow_multi_callsite: true,
        }
    }

    pub fn conservative() -> Self {
        Self {
            max_callee_insts: 8,
            allow_multi_callsite: false,
        }
    }
}

impl Pass for Inline {
    fn name(&self) -> &'static str {
        "inline"
    }

    fn run(&mut self, module: &mut SsaModule) -> bool {
        // Count callsites per FuncId
        let mut callsites: HashMap<FuncId, usize> = HashMap::new();
        for f in &module.functions {
            for (_, inst) in f.values() {
                if let InstKind::Call { callee, .. } = &inst.kind {
                    if let Some(fid) = resolve_func(module, f, *callee) {
                        *callsites.entry(fid).or_default() += 1;
                    }
                }
            }
        }

        let mut changed = false;
        // Collect inline candidates: (caller_idx, block, inst_id, callee_fid)
        let mut candidates = Vec::new();
        for (ci, f) in module.functions.iter().enumerate() {
            for (bid, inst) in f.values() {
                if let InstKind::Call {
                    callee,
                    args,
                    effectful: _,
                } = &inst.kind
                {
                    if let Some(fid) = resolve_func(module, f, *callee) {
                        if fid.0 as usize == ci {
                            continue; // recursion
                        }
                        let callee_f = &module.functions[fid.0 as usize];
                        if callee_f.is_async {
                            continue;
                        }
                        // No exotic closures
                        let has_closure = callee_f.values().any(|(_, i)| {
                            matches!(i.kind, InstKind::MakeClosure { .. } | InstKind::GetUpvalue { .. })
                        });
                        if has_closure {
                            continue;
                        }
                        let inst_count: usize =
                            callee_f.blocks.values().map(|b| b.insts.len()).sum();
                        if inst_count > self.max_callee_insts {
                            continue;
                        }
                        let sites = callsites.get(&fid).copied().unwrap_or(0);
                        if !self.allow_multi_callsite && sites > 1 {
                            continue;
                        }
                        candidates.push((ci, *bid, inst.id, fid, args.clone()));
                    }
                }
            }
        }

        for (ci, bid, call_id, fid, args) in candidates {
            if inline_call(module, ci, bid, call_id, fid, &args) {
                changed = true;
            }
        }
        changed
    }
}

fn resolve_func(_module: &SsaModule, caller: &SsaFunction, callee: ValueId) -> Option<FuncId> {
    let inst = caller.find_def(callee)?;
    match &inst.kind {
        InstKind::Const(ConstValue::FuncRef(f)) => Some(*f),
        InstKind::Const(ConstValue::Native(_)) => None,
        _ => {
            // GetGlobal of function — try match by looking at const after load
            None
        }
    }
}

fn inline_call(
    module: &mut SsaModule,
    caller_idx: usize,
    call_bb: BlockId,
    call_id: ValueId,
    callee_id: FuncId,
    args: &[ValueId],
) -> bool {
    // Simplified inlining: if callee is a single-block function that returns a value
    // computed from params/consts, splice the computation into the caller.
    let callee = module.functions[callee_id.0 as usize].clone();
    if callee.blocks.len() != 1 {
        return false;
    }
    let entry = callee.entry;
    let cal_block = callee.block(entry);
    let ret_val = match &cal_block.term {
        Terminator::Return(v) => *v,
        _ => return false,
    };

    // Map callee params to args
    let mut val_map: HashMap<ValueId, ValueId> = HashMap::new();
    for inst in &cal_block.insts {
        if let InstKind::Param { index } = &inst.kind {
            if let Some(&a) = args.get(*index as usize) {
                val_map.insert(inst.id, a);
            }
        }
    }

    let caller = &mut module.functions[caller_idx];
    // Clone pure instructions from callee into caller before the call
    let mut new_insts = Vec::new();
    for inst in &cal_block.insts {
        match &inst.kind {
            InstKind::Alloca { .. } | InstKind::Param { .. } | InstKind::Store { .. } => {
                // Skip memory plumbing; params mapped
                continue;
            }
            InstKind::Load { ptr } => {
                // If load of param alloca — already mapped via store of param; skip
                let _ = ptr;
                continue;
            }
            _ => {
                if inst.effectful && !matches!(inst.kind, InstKind::Print { .. }) {
                    // Only allow print as effect when inlining tiny helpers
                    if !matches!(inst.kind, InstKind::Print { .. }) {
                        return false;
                    }
                }
                let new_id = caller.alloc_value();
                val_map.insert(inst.id, new_id);
                let mut kind = inst.kind.clone();
                remap_kind(&mut kind, &val_map);
                new_insts.push(Inst {
                    id: new_id,
                    kind,
                    ty: inst.ty,
                    line: inst.line,
                    effectful: inst.effectful,
                });
            }
        }
    }

    let mapped_ret = ret_val.map(|v| *val_map.get(&v).unwrap_or(&v));

    // Insert before call and replace call uses
    let block = caller.block_mut(call_bb);
    let pos = block.insts.iter().position(|i| i.id == call_id);
    let Some(pos) = pos else {
        return false;
    };
    for (i, ni) in new_insts.into_iter().enumerate() {
        block.insts.insert(pos + i, ni);
    }
    // Remove call
    block.insts.retain(|i| i.id != call_id);
    if let Some(rv) = mapped_ret {
        caller.replace_uses(call_id, rv);
    } else {
        // void — replace with null const
        let n = caller.alloc_value();
        caller.block_mut(call_bb).insts.insert(
            pos,
            Inst {
                id: n,
                kind: InstKind::Const(ConstValue::Null),
                ty: SsaTy::Dyn,
                line: 0,
                effectful: false,
            },
        );
        caller.replace_uses(call_id, n);
    }
    true
}

fn remap_kind(kind: &mut InstKind, map: &HashMap<ValueId, ValueId>) {
    let mapv = |v: &mut ValueId| {
        if let Some(n) = map.get(v) {
            *v = *n;
        }
    };
    match kind {
        InstKind::BinOp { lhs, rhs, .. } => {
            mapv(lhs);
            mapv(rhs);
        }
        InstKind::UnOp { arg, .. } => mapv(arg),
        InstKind::Call { callee, args, .. } => {
            mapv(callee);
            for a in args {
                mapv(a);
            }
        }
        InstKind::Print { value } => mapv(value),
        InstKind::Dup { value } => mapv(value),
        InstKind::Const(_) => {}
        _ => {}
    }
}
