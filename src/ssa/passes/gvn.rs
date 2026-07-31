//! Global value numbering / CSE for pure operations.

use crate::ssa::cfg::block_rpo;
use crate::ssa::ir::*;
use crate::ssa::pass::Pass;
use std::collections::HashMap;

pub struct Gvn;

impl Pass for Gvn {
    fn name(&self) -> &'static str {
        "gvn"
    }

    fn run(&mut self, module: &mut SsaModule) -> bool {
        let mut changed = false;
        for func in &mut module.functions {
            let mut table: HashMap<String, ValueId> = HashMap::new();
            let order = block_rpo(func);
            let mut repl = Vec::new();
            for bid in order {
                let insts = func.block(bid).insts.clone();
                for inst in insts {
                    if inst.effectful {
                        continue;
                    }
                    if let Some(key) = expr_key(&inst.kind) {
                        if let Some(&existing) = table.get(&key) {
                            if existing != inst.id {
                                repl.push((inst.id, existing));
                            }
                        } else {
                            table.insert(key, inst.id);
                        }
                    }
                }
            }
            for (from, to) in repl {
                func.replace_uses(from, to);
                changed = true;
            }
        }
        changed
    }
}

fn expr_key(kind: &InstKind) -> Option<String> {
    match kind {
        InstKind::BinOp { op, lhs, rhs } => Some(format!("bin:{op:?}:{}:{}", lhs.0, rhs.0)),
        InstKind::UnOp { op, arg } => Some(format!("un:{op:?}:{}", arg.0)),
        InstKind::Const(c) => Some(format!("c:{c:?}")),
        InstKind::Load { ptr } => Some(format!("load:{}", ptr.0)),
        InstKind::GetGlobal { index } => Some(format!("gg:{index}")),
        InstKind::GetProperty { object, name } => {
            Some(format!("gp:{}:{}", object.0, name.0))
        }
        InstKind::GetIndex { object, index } => {
            Some(format!("gi:{}:{}", object.0, index.0))
        }
        _ => None,
    }
}
