//! Pass manager for SSA optimizations.

use super::ir::SsaModule;
use crate::Optimize;

pub trait Pass {
    fn name(&self) -> &'static str;
    fn run(&mut self, module: &mut SsaModule) -> bool;
}

pub struct PassManager {
    passes: Vec<Box<dyn Pass>>,
}

impl PassManager {
    pub fn new() -> Self {
        Self { passes: Vec::new() }
    }

    pub fn add<P: Pass + 'static>(&mut self, pass: P) {
        self.passes.push(Box::new(pass));
    }

    pub fn run(&mut self, module: &mut SsaModule) {
        for pass in &mut self.passes {
            let _ = pass.run(module);
        }
    }

    pub fn run_fixed_point(&mut self, module: &mut SsaModule, max_iters: usize) {
        for _ in 0..max_iters {
            let mut changed = false;
            for pass in &mut self.passes {
                if pass.run(module) {
                    changed = true;
                }
            }
            if !changed {
                break;
            }
        }
    }
}

impl Default for PassManager {
    fn default() -> Self {
        Self::new()
    }
}

pub fn pipeline_for(opt: Optimize) -> PassManager {
    use super::passes::*;
    let mut pm = PassManager::new();
    match opt {
        Optimize::None => {}
        Optimize::Speed => {
            pm.add(CfgSimplify);
            pm.add(Mem2Reg);
            pm.add(Sccp);
            pm.add(ConstFold);
            pm.add(CopyProp);
            pm.add(Gvn);
            pm.add(Dce);
            pm.add(Licm);
            pm.add(StrengthReduce);
            pm.add(Inline::aggressive());
            // cleanup
            pm.add(CfgSimplify);
            pm.add(CopyProp);
            pm.add(Dce);
        }
        Optimize::Size => {
            pm.add(CfgSimplify);
            pm.add(Mem2Reg);
            pm.add(Sccp);
            pm.add(ConstFold);
            pm.add(CopyProp);
            pm.add(Gvn);
            pm.add(Dce);
            pm.add(Licm);
            // skip strength reduction (may grow)
            pm.add(Inline::conservative());
            pm.add(CfgSimplify);
            pm.add(Dce);
        }
    }
    pm
}
