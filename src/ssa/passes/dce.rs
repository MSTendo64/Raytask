//! Effect-aware dead code elimination.

use crate::ssa::ir::{InstKind, SsaModule, Terminator, ValueId};
use crate::ssa::pass::Pass;
use std::collections::{HashMap, HashSet};

pub struct Dce;

impl Pass for Dce {
    fn name(&self) -> &'static str {
        "dce"
    }

    fn run(&mut self, module: &mut SsaModule) -> bool {
        let mut changed = false;
        for func in &mut module.functions {
            let mut live: HashSet<ValueId> = HashSet::new();
            // Roots: effectful insts + terminator uses
            for b in func.blocks.values() {
                for inst in &b.insts {
                    if inst.effectful {
                        live.insert(inst.id);
                        mark_uses(&inst.kind, &mut live);
                    }
                }
                match &b.term {
                    Terminator::CondBr { cond, .. } => {
                        live.insert(*cond);
                    }
                    Terminator::Return(Some(v)) | Terminator::Throw(v) => {
                        live.insert(*v);
                    }
                    _ => {}
                }
            }
            // Propagate liveness backward through defs
            let mut work: Vec<ValueId> = live.iter().copied().collect();
            let def_of: HashMap<ValueId, InstKind> = func
                .values()
                .map(|(_, i)| (i.id, i.kind.clone()))
                .collect();
            while let Some(v) = work.pop() {
                if let Some(kind) = def_of.get(&v) {
                    let mut deps = HashSet::new();
                    mark_uses(kind, &mut deps);
                    for d in deps {
                        if live.insert(d) {
                            work.push(d);
                        }
                    }
                }
            }
            let bids: Vec<_> = func.blocks.keys().copied().collect();
            for bid in bids {
                let before = func.block(bid).insts.len();
                func.block_mut(bid).insts.retain(|i| {
                    i.effectful || live.contains(&i.id) || matches!(i.kind, InstKind::Param { .. })
                });
                if func.block(bid).insts.len() != before {
                    changed = true;
                }
            }
        }
        changed
    }
}

fn mark_uses(kind: &InstKind, live: &mut HashSet<ValueId>) {
    match kind {
        InstKind::Load { ptr } => {
            live.insert(*ptr);
        }
        InstKind::Store { ptr, value } => {
            live.insert(*ptr);
            live.insert(*value);
        }
        InstKind::BinOp { lhs, rhs, .. } => {
            live.insert(*lhs);
            live.insert(*rhs);
        }
        InstKind::UnOp { arg, .. } => {
            live.insert(*arg);
        }
        InstKind::Call { callee, args, .. } => {
            live.insert(*callee);
            for a in args {
                live.insert(*a);
            }
        }
        InstKind::SetGlobal { value, .. } | InstKind::DefineGlobal { value, .. } => {
            live.insert(*value);
        }
        InstKind::GetProperty { object, name } => {
            live.insert(*object);
            live.insert(*name);
        }
        InstKind::SetProperty {
            object,
            name,
            value,
        } => {
            live.insert(*object);
            live.insert(*name);
            live.insert(*value);
        }
        InstKind::GetIndex { object, index } => {
            live.insert(*object);
            live.insert(*index);
        }
        InstKind::SetIndex {
            object,
            index,
            value,
        } => {
            live.insert(*object);
            live.insert(*index);
            live.insert(*value);
        }
        InstKind::NewObject { name: Some(n), .. } => {
            live.insert(*n);
        }
        InstKind::NewArray { elems } => {
            for e in elems {
                live.insert(*e);
            }
        }
        InstKind::Print { value }
        | InstKind::Dup { value }
        | InstKind::Await { value }
        | InstKind::SetUpvalue { value, .. } => {
            live.insert(*value);
        }
        InstKind::MakeClosure { proto, .. } => {
            live.insert(*proto);
        }
        InstKind::Phi { incomings } => {
            for (_, v) in incomings {
                live.insert(*v);
            }
        }
        _ => {}
    }
}
