//! RayTask compiler library.

pub mod abi;
pub mod app_build;
pub mod ast;
pub mod async_rt;
pub mod bytecode;
pub mod bytecode_format;
pub mod c_header;
pub mod codegen_c;
pub mod compiler;
pub mod dap;
pub mod debug_io;
pub mod debug_symbols;
pub mod error;
pub mod ffi;
pub mod ffi_bind;
pub mod gc;
pub mod lexer;
pub mod link;
pub mod linker;
pub mod migrate;
pub mod mono;
pub mod native_codegen;
pub mod native_triple;
pub mod parser;
pub mod preprocess;
pub mod project;
pub mod registry;
pub mod resolve;
pub mod sema;
pub mod span;
pub mod ssa;
pub mod stdlib;
pub mod stdlib_types;
pub mod targets;
pub mod tcc;
pub mod token;
pub mod types;
pub mod value;
pub mod vm;
pub mod web_runtime;

use crate::app_build::{build_app, Platform};
use crate::bytecode_format::{deserialize_module, serialize_module};
use crate::codegen_c::{CCodegen, CodegenOptions};
use crate::error::CompileResult;
use crate::mono::monomorphize;
use crate::bytecode::Module;
use crate::sema::{typecheck_or_err, TypeCheckReport};
use crate::vm::Vm;
use std::fmt::Write as _;
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target {
    /// Emit .rtbc bytecode file
    Bytecode,
    /// Transpile to C (optionally compile with gcc/clang)
    Native,
    /// Standalone app: runtime stub + embedded bytecode
    App,
    /// WebAssembly (+ HTML shell)
    Wasm,
    /// Web app bundle (WASM host + bytecode payload)
    Web,
    /// Android + iOS scaffolds with embedded bytecode
    Mobile,
    /// Freestanding embedded C
    Embedded,
    /// Kernel / freestanding (no GC)
    Kernel,
    /// NativeCodeGen + Linker → PE/ELF/Mach-O (platform selects OS)
    NativeBin,
    /// UEFI PE32+ application (.efi)
    Efi,
    /// Flat raw binary (sections concatenated)
    Raw,
}

impl Target {
    pub fn parse(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().as_str() {
            "bytecode" | "vm" | "rtbc" => Some(Self::Bytecode),
            "native" | "c" => Some(Self::Native),
            "app" | "standalone" => Some(Self::App),
            "wasm" | "webassembly" => Some(Self::Wasm),
            "web" | "browser" => Some(Self::Web),
            "mobile" | "android" | "ios" => Some(Self::Mobile),
            "embedded" | "mcu" | "baremetal" => Some(Self::Embedded),
            "kernel" | "os" => Some(Self::Kernel),
            "native-bin" | "nativebin" | "bin-native" => Some(Self::NativeBin),
            "efi" | "uefi" => Some(Self::Efi),
            "raw" | "bin" | "flat" => Some(Self::Raw),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Bytecode => "bytecode",
            Self::Native => "native",
            Self::App => "app",
            Self::Wasm => "wasm",
            Self::Web => "web",
            Self::Mobile => "mobile",
            Self::Embedded => "embedded",
            Self::Kernel => "kernel",
            Self::NativeBin => "native-bin",
            Self::Efi => "efi",
            Self::Raw => "raw",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Optimize {
    None,
    Speed,
    Size,
}

#[derive(Debug, Clone)]
pub struct BuildOptions {
    pub target: Target,
    pub optimize: Optimize,
    pub gc: bool,
    /// Collect on every allocation (stress / tests).
    pub gc_stress: bool,
    pub debug: bool,
    pub platform: Platform,
    /// CPU architecture for native / native-bin / link (default: host).
    pub arch: crate::native_triple::Arch,
    /// Prefer the built-in object linker after compiling to `.o` (freestanding / no CRT).
    pub link_builtin: bool,
    pub output: Option<std::path::PathBuf>,
    /// Skip typechecker (not recommended)
    pub no_typecheck: bool,
    /// Disable built-in bstd.* imports, types, and runtime globals.
    pub no_stdlib: bool,
}

impl Default for BuildOptions {
    fn default() -> Self {
        Self {
            target: Target::Bytecode,
            optimize: Optimize::None,
            gc: true,
            gc_stress: false,
            debug: false,
            platform: Platform::Current,
            arch: crate::native_triple::Arch::host(),
            link_builtin: false,
            output: None,
            no_typecheck: false,
            no_stdlib: false,
        }
    }
}

impl BuildOptions {
    /// Resolved OS × arch triple for AOT / native-bin.
    pub fn native_triple(&self) -> crate::native_triple::NativeTriple {
        use crate::native_triple::{NativeTriple, OsKind};
        let os = match self.platform {
            Platform::Windows => OsKind::Windows,
            Platform::Linux => OsKind::Linux,
            Platform::Macos => OsKind::Macos,
            Platform::Uefi => OsKind::Uefi,
            Platform::Current => OsKind::host(),
        };
        NativeTriple::new(os, self.arch)
    }
}

/// Runtime options for `run` / `run_file`.
#[derive(Debug, Clone)]
pub struct RunOptions {
    pub gc: bool,
    pub gc_stress: bool,
    pub no_typecheck: bool,
    /// Disable built-in bstd.* imports, types, and runtime globals.
    pub no_stdlib: bool,
    /// Optimization level when compiling from source.
    pub optimize: Optimize,
}

impl Default for RunOptions {
    fn default() -> Self {
        Self {
            gc: true,
            gc_stress: false,
            no_typecheck: false,
            no_stdlib: false,
            optimize: Optimize::None,
        }
    }
}

pub fn parse_source(source: &str) -> CompileResult<ast::Program> {
    parse_source_with_stdlib(source, true)
}

pub fn parse_source_with_stdlib(source: &str, stdlib_enabled: bool) -> CompileResult<ast::Program> {
    let defs = crate::preprocess::default_defs(cfg!(debug_assertions));
    let source = crate::preprocess::preprocess(source, &defs);
    let mut program = crate::resolve::resolve_program_with_stdlib(&source, None, stdlib_enabled)?;
    crate::ffi_bind::expand_c_header_binds(&mut program, None)?;
    Ok(program)
}

pub fn parse_file(path: &Path) -> CompileResult<ast::Program> {
    parse_file_with_stdlib(path, true)
}

pub fn parse_file_with_stdlib(path: &Path, stdlib_enabled: bool) -> CompileResult<ast::Program> {
    let source = std::fs::read_to_string(path).map_err(|e| {
        crate::error::CompileError::Io {
            message: format!("{}: {}", path.display(), e),
        }
    })?;
    let defs = crate::preprocess::default_defs(cfg!(debug_assertions));
    let source = crate::preprocess::preprocess(&source, &defs);
    let mut program =
        crate::resolve::resolve_program_with_stdlib(&source, Some(path), stdlib_enabled)?;
    crate::ffi_bind::expand_c_header_binds(&mut program, Some(path))?;
    Ok(program)
}

pub fn check_source(source: &str) -> CompileResult<TypeCheckReport> {
    check_source_with_stdlib(source, true)
}

pub fn check_source_with_stdlib(
    source: &str,
    stdlib_enabled: bool,
) -> CompileResult<TypeCheckReport> {
    let program = parse_source_with_stdlib(source, stdlib_enabled)?;
    Ok(crate::sema::typecheck_with_stdlib(&program, stdlib_enabled))
}

pub fn compile_bytecode(source: &str) -> CompileResult<Module> {
    compile_bytecode_with_stdlib(source, true)
}

pub fn compile_bytecode_with_stdlib(source: &str, stdlib_enabled: bool) -> CompileResult<Module> {
    compile_bytecode_optimized(source, stdlib_enabled, Optimize::None)
}

/// Compile source to a Module through the SSA pipeline at the given optimize level.
pub fn compile_bytecode_optimized(
    source: &str,
    stdlib_enabled: bool,
    optimize: Optimize,
) -> CompileResult<Module> {
    let program = parse_source_with_stdlib(source, stdlib_enabled)?;
    crate::sema::typecheck_or_err_with_stdlib(&program, stdlib_enabled)?;
    let program = monomorphize(program);
    crate::ssa::compile_via_ssa(&program, optimize, stdlib_enabled)
}

pub fn compile_bytecode_unchecked(source: &str) -> CompileResult<Module> {
    compile_bytecode_unchecked_with_stdlib(source, true)
}

pub fn compile_bytecode_unchecked_with_stdlib(
    source: &str,
    stdlib_enabled: bool,
) -> CompileResult<Module> {
    compile_bytecode_unchecked_optimized(source, stdlib_enabled, Optimize::None)
}

pub fn compile_bytecode_unchecked_optimized(
    source: &str,
    stdlib_enabled: bool,
    optimize: Optimize,
) -> CompileResult<Module> {
    let program = parse_source_with_stdlib(source, stdlib_enabled)?;
    let program = monomorphize(program);
    crate::ssa::compile_via_ssa(&program, optimize, stdlib_enabled)
}

pub fn run_source(source: &str) -> Result<(), Box<dyn std::error::Error>> {
    run_source_with(source, &RunOptions::default())
}

pub fn run_source_with(source: &str, opts: &RunOptions) -> Result<(), Box<dyn std::error::Error>> {
    let module = if opts.no_typecheck {
        compile_bytecode_unchecked_optimized(source, !opts.no_stdlib, opts.optimize)?
    } else {
        compile_bytecode_optimized(source, !opts.no_stdlib, opts.optimize)?
    };
    crate::ffi::prepare_module_ffi(&module.ffi, None)?;
    let mut vm = Vm::with_gc(
        module,
        crate::gc::GcConfig {
            enabled: opts.gc,
            threshold_bytes: 256 * 1024,
            stress: opts.gc_stress,
        },
    );
    vm.run()?;
    Ok(())
}

pub fn run_file(path: &Path) -> Result<(), Box<dyn std::error::Error>> {
    run_file_with(path, &RunOptions::default())
}

pub fn run_file_with(path: &Path, opts: &RunOptions) -> Result<(), Box<dyn std::error::Error>> {
    let program = parse_file_with_stdlib(path, !opts.no_stdlib)?;
    if !opts.no_typecheck {
        crate::sema::typecheck_or_err_with_stdlib(&program, !opts.no_stdlib)?;
    }
    let program = monomorphize(program);
    let module =
        crate::ssa::compile_via_ssa_with_source(&program, opts.optimize, !opts.no_stdlib, Some(&path.display().to_string()))?;
    let base = path.parent();
    crate::ffi::prepare_module_ffi(&module.ffi, base)?;
    let mut vm = Vm::with_gc(
        module,
        crate::gc::GcConfig {
            enabled: opts.gc,
            threshold_bytes: 256 * 1024,
            stress: opts.gc_stress,
        },
    );
    vm.run()?;
    Ok(())
}

pub fn run_bytecode(bytes: &[u8]) -> Result<(), Box<dyn std::error::Error>> {
    let module = deserialize_module(bytes)?;
    crate::ffi::prepare_module_ffi(&module.ffi, None)?;
    let mut vm = Vm::new(module);
    vm.run()?;
    Ok(())
}

pub fn inspect_bytecode(
    bytes: &[u8],
    disassemble: bool,
) -> Result<String, Box<dyn std::error::Error>> {
    let module = deserialize_module(bytes)?;
    let version = if bytes.len() >= 6 {
        u16::from_le_bytes([bytes[4], bytes[5]])
    } else {
        0
    };
    let mut out = String::new();
    writeln!(&mut out, "RTBC").ok();
    writeln!(&mut out, "  version: {}", version).ok();
    writeln!(&mut out, "  size: {} bytes", bytes.len()).ok();
    writeln!(&mut out, "  main_chunk: {}", module.main_chunk).ok();
    writeln!(&mut out, "  stdlib_enabled: {}", module.stdlib_enabled).ok();
    writeln!(&mut out, "  globals: {}", module.globals.len()).ok();
    for (i, g) in module.globals.iter().enumerate() {
        writeln!(&mut out, "    [{}] {}", i, g).ok();
    }
    writeln!(&mut out, "  classes: {}", module.classes.len()).ok();
    for (i, class) in module.classes.iter().enumerate() {
        writeln!(
            &mut out,
            "    [{}] {} fields={} methods={} ctor={:?} base={:?} dtor={:?}",
            i,
            class.name,
            class.fields.len(),
            class.methods.len(),
            class.constructor,
            class.base,
            class.destructor
        )
        .ok();
    }
    writeln!(&mut out, "  chunks: {}", module.chunks.len()).ok();
    for (i, chunk) in module.chunks.iter().enumerate() {
        writeln!(
            &mut out,
            "    [{}] {} arity={} locals={} async={} code={} consts={} source={}",
            i,
            chunk.name,
            chunk.arity,
            chunk.local_count,
            chunk.is_async,
            chunk.code.len(),
            chunk.constants.len(),
            chunk.source.as_deref().unwrap_or("-")
        )
        .ok();
        if !chunk.constants.is_empty() {
            for (ci, constant) in chunk.constants.iter().enumerate() {
                writeln!(&mut out, "      const[{}] = {:?}", ci, constant).ok();
            }
        }
        if disassemble {
            disassemble_chunk(&mut out, chunk);
        }
    }
    Ok(out)
}

fn disassemble_chunk(out: &mut String, chunk: &crate::bytecode::Chunk) {
    use crate::bytecode::Op;

    writeln!(out, "      code:").ok();
    let mut ip = 0usize;
    while ip < chunk.code.len() {
        let line = chunk.lines.get(ip).copied().unwrap_or(0);
        let byte = chunk.code[ip];
        let Some(op) = Op::from_byte(byte) else {
            writeln!(out, "        {:04} L{:04}  <unknown {}>", ip, line, byte).ok();
            ip += 1;
            continue;
        };
        match op {
            Op::Constant
            | Op::GetLocal
            | Op::SetLocal
            | Op::GetGlobal
            | Op::SetGlobal
            | Op::DefineGlobal
            | Op::Call
            | Op::NewObject
            | Op::NewArray
            | Op::IncLocal
            | Op::DecLocal
            | Op::GetUpvalue
            | Op::SetUpvalue => {
                let arg = chunk.code.get(ip + 1).copied().unwrap_or(0);
                writeln!(out, "        {:04} L{:04}  {:<14} {}", ip, line, op.name(), arg).ok();
                ip += 2;
            }
            Op::Jump | Op::JumpIfFalse | Op::JumpIfTrue | Op::Loop | Op::TryBegin => {
                let hi = chunk.code.get(ip + 1).copied().unwrap_or(0) as u16;
                let lo = chunk.code.get(ip + 2).copied().unwrap_or(0) as u16;
                let arg = (hi << 8) | lo;
                writeln!(out, "        {:04} L{:04}  {:<14} {}", ip, line, op.name(), arg).ok();
                ip += 3;
            }
            Op::MakeClosure => {
                let captures = chunk.code.get(ip + 1).copied().unwrap_or(0) as usize;
                writeln!(
                    out,
                    "        {:04} L{:04}  {:<14} captures={}",
                    ip,
                    line,
                    op.name(),
                    captures
                )
                .ok();
                ip += 2;
                for cap in 0..captures {
                    let is_local = chunk.code.get(ip).copied().unwrap_or(0);
                    let index = chunk.code.get(ip + 1).copied().unwrap_or(0);
                    writeln!(
                        out,
                        "                     capture[{}] {} {}",
                        cap,
                        if is_local == 1 { "local" } else { "upvalue" },
                        index
                    )
                    .ok();
                    ip += 2;
                }
            }
            _ => {
                writeln!(out, "        {:04} L{:04}  {}", ip, line, op.name()).ok();
                ip += 1;
            }
        }
    }
}

pub fn transpile_c(source: &str) -> CompileResult<String> {
    let program = parse_source(source)?;
    typecheck_or_err(&program)?;
    let program = monomorphize(program);
    CCodegen::new().generate(&program)
}

pub fn transpile_c_with(source: &str, opts: CodegenOptions) -> CompileResult<String> {
    let program = parse_source(source)?;
    typecheck_or_err(&program)?;
    let program = monomorphize(program);
    CCodegen::with_options(opts).generate(&program)
}

pub fn compile_file(path: &str, options: &BuildOptions) -> Result<String, Box<dyn std::error::Error>> {
    let path_buf = Path::new(path);
    let program = parse_file_with_stdlib(path_buf, !options.no_stdlib)?;
    if !options.no_typecheck {
        let report = crate::sema::typecheck_with_stdlib(&program, !options.no_stdlib);
        if !report.ok() {
            eprint!("{}", report.format_all());
            return Err(report.errors.into_iter().next().unwrap().into());
        }
    }
    let program = monomorphize(program);
    let mut module = crate::ssa::compile_via_ssa_with_source(
        &program,
        options.optimize,
        !options.no_stdlib,
        Some(&path_buf.display().to_string()),
    )?;
    crate::debug_symbols::stamp_source(&mut module, path_buf);

    let emit_symbols = |artifact: &Path, module: &Module| -> Result<(), Box<dyn std::error::Error>> {
        if !options.debug {
            return Ok(());
        }
        let sym_path = crate::debug_symbols::sidecar_path(artifact);
        let sym = crate::debug_symbols::DebugSymbols::from_module(module, path_buf, Some(artifact));
        sym.write_file(&sym_path)?;
        eprintln!("debug symbols: {}", sym_path.display());
        Ok(())
    };

    match options.target {
        Target::Bytecode => {
            let mut emit = module.clone();
            if !options.debug {
                crate::debug_symbols::strip_module_debug(&mut emit);
            }
            let bytes = serialize_module(&emit);
            let out = options
                .output
                .clone()
                .unwrap_or_else(|| path_buf.with_extension("rtbc"));
            std::fs::write(&out, bytes)?;
            emit_symbols(&out, &module)?;
            Ok(out.display().to_string())
        }
        Target::Native => {
            // True AOT: SSA → C → TCC/gcc/clang (no RTBC interpreter).
            let r = crate::targets::build_aot_native(
                path_buf,
                &program,
                options.optimize,
                options.gc,
                options.debug,
                options.output.as_deref(),
                options.native_triple(),
                options.link_builtin,
            )?;
            for n in &r.notes {
                eprintln!("note: {}", n);
            }
            emit_symbols(&r.exe, &module)?;
            Ok(r.exe.display().to_string())
        }
        Target::App => {
            let result = build_app(
                path_buf,
                &module,
                options.platform,
                options.output.as_deref(),
            )?;
            emit_symbols(&result.output, &module)?;
            Ok(format!(
                "{} (platform={}, bytecode={})",
                result.output.display(),
                result.platform.name(),
                result.bytecode_path.display()
            ))
        }
        Target::Wasm => {
            let r = crate::targets::build_wasm(path_buf, &program, options.gc)?;
            for n in &r.notes {
                eprintln!("note: {}", n);
            }
            emit_symbols(&r.primary, &module)?;
            Ok(r.primary.display().to_string())
        }
        Target::Web => {
            let r = crate::targets::build_web(path_buf, &program, &module, options.gc)?;
            for n in &r.notes {
                eprintln!("note: {}", n);
            }
            emit_symbols(&r.primary, &module)?;
            Ok(r.primary.display().to_string())
        }
        Target::Mobile => {
            let r = crate::targets::build_mobile(path_buf, &module)?;
            for n in &r.notes {
                eprintln!("note: {}", n);
            }
            emit_symbols(&r.primary, &module)?;
            Ok(r.primary.display().to_string())
        }
        Target::Embedded => {
            let r = crate::targets::build_embedded(
                path_buf,
                &program,
                options.gc,
                options.optimize,
            )?;
            for n in &r.notes {
                eprintln!("note: {}", n);
            }
            emit_symbols(&r.primary, &module)?;
            Ok(r.primary.display().to_string())
        }
        Target::Kernel => {
            let r =
                crate::targets::build_kernel(path_buf, &program, options.optimize)?;
            for n in &r.notes {
                eprintln!("note: {}", n);
            }
            emit_symbols(&r.primary, &module)?;
            Ok(r.primary.display().to_string())
        }
        Target::NativeBin => {
            // Host platforms: true AOT (SSA→C→native). UEFI keeps payload packaging.
            if matches!(options.platform, Platform::Uefi) {
                let r = crate::linker::build_native_bin(
                    path_buf,
                    &module,
                    crate::native_codegen::LinkTarget::UefiX64,
                    options.output.as_deref(),
                )?;
                for n in &r.notes {
                    eprintln!("note: {}", n);
                }
                emit_symbols(&r.output, &module)?;
                return Ok(format!(
                    "{} (native-bin uefi/payload, objects={})",
                    r.output.display(),
                    r.object_dir.display()
                ));
            }
            let r = crate::targets::build_aot_native(
                path_buf,
                &program,
                options.optimize,
                options.gc,
                options.debug,
                options.output.as_deref(),
                options.native_triple(),
                options.link_builtin,
            )?;
            for n in &r.notes {
                eprintln!("note: {}", n);
            }
            emit_symbols(&r.exe, &module)?;
            Ok(format!(
                "{} (native-bin AOT {}, c={})",
                r.exe.display(),
                options.native_triple(),
                r.c_path.display()
            ))
        }
        Target::Efi => {
            let r = crate::linker::build_native_bin(
                path_buf,
                &module,
                crate::native_codegen::LinkTarget::UefiX64,
                options.output.as_deref(),
            )?;
            for n in &r.notes {
                eprintln!("note: {}", n);
            }
            emit_symbols(&r.output, &module)?;
            Ok(format!(
                "{} (efi, objects={})",
                r.output.display(),
                r.object_dir.display()
            ))
        }
        Target::Raw => {
            let r = crate::linker::build_native_bin(
                path_buf,
                &module,
                crate::native_codegen::LinkTarget::RawX64,
                options.output.as_deref(),
            )?;
            for n in &r.notes {
                eprintln!("note: {}", n);
            }
            emit_symbols(&r.output, &module)?;
            Ok(format!(
                "{} (raw, objects={})",
                r.output.display(),
                r.object_dir.display()
            ))
        }
    }
}
