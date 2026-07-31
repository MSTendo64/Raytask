//! Mid-level SSA IR and optimization pipeline.
//!
//! Pipeline: AST → bytecode → lift SSA → opt passes → bytecode Module
//! Embedded/kernel also: optimized SSA → freestanding C (`emit_c`).

pub mod builder;
pub mod cfg;
pub mod emit;
pub mod emit_c;
pub mod ir;
pub mod lift;
pub mod pass;
pub mod passes;
pub mod phi_elim;

use crate::ast::Program;
use crate::bytecode::Module;
use crate::error::CompileResult;
use crate::Optimize;

use emit::emit_module;
use ir::SsaModule;
use pass::pipeline_for;
use phi_elim::eliminate_phis;

/// Compile a monomorphized program through SSA with the given optimize level.
pub fn compile_via_ssa(
    program: &Program,
    optimize: Optimize,
    stdlib_enabled: bool,
) -> CompileResult<Module> {
    compile_via_ssa_with_source(program, optimize, stdlib_enabled, None)
}

pub fn compile_via_ssa_with_source(
    program: &Program,
    optimize: Optimize,
    stdlib_enabled: bool,
    source: Option<&str>,
) -> CompileResult<Module> {
    let (original, mut ssa) = lower_to_ssa(program, stdlib_enabled, source)?;
    if optimize == Optimize::None {
        return Ok(original);
    }
    optimize_ssa(&mut ssa, optimize);
    Ok(emit_module(&ssa))
}

/// AST → bytecode → SSA lift (no optimize). Returns original Module + SSA.
pub fn lower_to_ssa(
    program: &Program,
    stdlib_enabled: bool,
    source: Option<&str>,
) -> CompileResult<(Module, SsaModule)> {
    let mut c = crate::compiler::Compiler::new().with_stdlib(stdlib_enabled);
    if let Some(p) = source {
        c = c.with_source(p);
    }
    let original = c.compile(program)?;
    let ssa = lift::lift_module(&original);
    Ok((original, ssa))
}

/// Run the pass manager and phi elimination for C / bytecode emit.
pub fn optimize_ssa(ssa: &mut SsaModule, optimize: Optimize) {
    if optimize == Optimize::None {
        // Still eliminate phis so emitters see Load/Store form.
        for f in &mut ssa.functions {
            eliminate_phis(f);
        }
        return;
    }
    let mut pm = pipeline_for(optimize);
    pm.run(ssa);
    for f in &mut ssa.functions {
        eliminate_phis(f);
    }
}

/// Build an SSA module ready for C emission (optimized + phi-elim).
pub fn build_ssa_for_c(
    program: &Program,
    optimize: Optimize,
    stdlib_enabled: bool,
    source: Option<&str>,
) -> CompileResult<SsaModule> {
    let (_orig, mut ssa) = lower_to_ssa(program, stdlib_enabled, source)?;
    optimize_ssa(&mut ssa, optimize);
    Ok(ssa)
}

/// Optimize an existing SSA module in place (passes only; no phi elim).
pub fn optimize_module(ssa: &mut ir::SsaModule, optimize: Optimize) {
    let mut pm = pipeline_for(optimize);
    pm.run(ssa);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vm::Vm;

    #[test]
    fn roundtrip_none() {
        let src = r#"
            void Main() {
                print("hi");
            }
        "#;
        let program = crate::parse_source_with_stdlib(src, false).unwrap();
        let program = crate::mono::monomorphize(program);
        let module = compile_via_ssa(&program, Optimize::None, false).unwrap();
        assert!(!module.chunks.is_empty());
    }

    #[test]
    fn speed_pipeline_runs() {
        let src = r#"
            void Main() {
                print(1 + 2);
            }
        "#;
        let program = crate::parse_source_with_stdlib(src, false).unwrap();
        let program = crate::mono::monomorphize(program);
        let module = compile_via_ssa(&program, Optimize::Speed, false).unwrap();
        assert!(!module.chunks.is_empty());
        Vm::new(module).run().unwrap();
    }

    #[test]
    fn speed_branch_fold() {
        let src = r#"
            void Main() {
                if (true) {
                    print(42);
                } else {
                    print(0);
                }
            }
        "#;
        let program = crate::parse_source_with_stdlib(src, false).unwrap();
        let program = crate::mono::monomorphize(program);
        let module = compile_via_ssa(&program, Optimize::Speed, false).unwrap();
        Vm::new(module).run().unwrap();
    }
}
