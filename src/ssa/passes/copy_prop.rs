//! Copy and trivial phi propagation.

use crate::ssa::ir::{InstKind, SsaModule, ValueId};
use crate::ssa::pass::Pass;

pub struct CopyProp;

impl Pass for CopyProp {
    fn name(&self) -> &'static str {
        "copy-prop"
    }

    fn run(&mut self, module: &mut SsaModule) -> bool {
        let mut changed = false;
        for func in &mut module.functions {
            let mut repl: Vec<(ValueId, ValueId)> = Vec::new();
            for (_, inst) in func.values() {
                match &inst.kind {
                    InstKind::Dup { value } => repl.push((inst.id, *value)),
                    InstKind::Phi { incomings } => {
                        if let Some(v) = same_phi(incomings) {
                            repl.push((inst.id, v));
                        }
                    }
                    _ => {}
                }
            }
            for (from, to) in repl {
                if from != to {
                    func.replace_uses(from, to);
                    changed = true;
                }
            }
        }
        changed
    }
}

fn same_phi(incomings: &[(crate::ssa::ir::BlockId, ValueId)]) -> Option<ValueId> {
    let first = incomings.first()?.1;
    if incomings.iter().all(|(_, v)| *v == first) {
        Some(first)
    } else {
        None
    }
}
