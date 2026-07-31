//! AST → SSA lowering.

use super::ir::SsaModule;
use super::lift::lift_module;
use crate::ast::Program;
use crate::compiler::Compiler;
use crate::error::CompileResult;

/// Lower a monomorphized program to SSA.
///
/// Implementation: reuse the battle-tested AST→bytecode compiler, then lift each
/// chunk into SSA form (allocas for locals). This preserves full language coverage
/// while presenting a true mid-level CFG/SSA IR to the optimizer.
pub fn lower_program(program: &Program, stdlib_enabled: bool) -> CompileResult<SsaModule> {
    lower_program_with_source(program, stdlib_enabled, None)
}

pub fn lower_program_with_source(
    program: &Program,
    stdlib_enabled: bool,
    source: Option<&str>,
) -> CompileResult<SsaModule> {
    let mut c = Compiler::new().with_stdlib(stdlib_enabled);
    if let Some(p) = source {
        c = c.with_source(p);
    }
    let module = c.compile(program)?;
    Ok(lift_module(&module))
}
