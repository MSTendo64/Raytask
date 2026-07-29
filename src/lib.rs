//! RayTask compiler library.

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
pub mod linker;
pub mod migrate;
pub mod mono;
pub mod native_codegen;
pub mod parser;
pub mod preprocess;
pub mod project;
pub mod registry;
pub mod resolve;
pub mod sema;
pub mod span;
pub mod stdlib;
pub mod stdlib_types;
pub mod targets;
pub mod token;
pub mod types;
pub mod value;
pub mod vm;

use crate::app_build::{build_app, Platform};
use crate::bytecode_format::{deserialize_module, serialize_module};
use crate::codegen_c::{CCodegen, CodegenOptions, RuntimeProfile};
use crate::compiler::Compiler;
use crate::error::CompileResult;
use crate::mono::monomorphize;
use crate::resolve::resolve_program;
use crate::sema::{typecheck, typecheck_or_err, TypeCheckReport};
use crate::vm::Vm;
use crate::bytecode::Module;
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
            output: None,
            no_typecheck: false,
            no_stdlib: false,
        }
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
}

impl Default for RunOptions {
    fn default() -> Self {
        Self {
            gc: true,
            gc_stress: false,
            no_typecheck: false,
            no_stdlib: false,
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
    let program = parse_source_with_stdlib(source, stdlib_enabled)?;
    crate::sema::typecheck_or_err_with_stdlib(&program, stdlib_enabled)?;
    let program = monomorphize(program);
    Compiler::new().with_stdlib(stdlib_enabled).compile(&program)
}

pub fn compile_bytecode_unchecked(source: &str) -> CompileResult<Module> {
    compile_bytecode_unchecked_with_stdlib(source, true)
}

pub fn compile_bytecode_unchecked_with_stdlib(
    source: &str,
    stdlib_enabled: bool,
) -> CompileResult<Module> {
    let program = parse_source_with_stdlib(source, stdlib_enabled)?;
    let program = monomorphize(program);
    Compiler::new().with_stdlib(stdlib_enabled).compile(&program)
}

pub fn run_source(source: &str) -> Result<(), Box<dyn std::error::Error>> {
    run_source_with(source, &RunOptions::default())
}

pub fn run_source_with(source: &str, opts: &RunOptions) -> Result<(), Box<dyn std::error::Error>> {
    let module = if opts.no_typecheck {
        compile_bytecode_unchecked_with_stdlib(source, !opts.no_stdlib)?
    } else {
        compile_bytecode_with_stdlib(source, !opts.no_stdlib)?
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
    let module = Compiler::new().with_stdlib(!opts.no_stdlib).compile(&program)?;
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

fn native_codegen_opts(options: &BuildOptions) -> CodegenOptions {
    CodegenOptions {
        profile: RuntimeProfile::Host,
        gc: options.gc,
        freestanding: false,
    }
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
    let mut module = Compiler::new()
        .with_stdlib(!options.no_stdlib)
        .with_source(path_buf.display().to_string())
        .compile(&program)?;
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
            let c = CCodegen::with_options(native_codegen_opts(options)).generate(&program)?;
            let out = path_buf.with_extension("c");
            std::fs::write(&out, &c)?;
            let exe = options.output.clone().unwrap_or_else(|| {
                path_buf.with_extension(if cfg!(windows) { "exe" } else { "" })
            });
            let exe_str = exe.display().to_string();
            let out_str = out.display().to_string();
            let link_libs = crate::codegen_c::collect_link_libs(&program);
            for cc in ["gcc", "clang", "cl"] {
                let mut cmd = std::process::Command::new(cc);
                if cc == "cl" {
                    cmd.arg(&out_str).arg(format!("/Fe:{}", exe_str));
                    if options.debug {
                        cmd.arg("/Zi").arg("/Od");
                    }
                    for lib in &link_libs {
                        if lib.ends_with(".dll") || lib.ends_with(".lib") {
                            cmd.arg(lib);
                        } else {
                            cmd.arg(format!("{}.lib", lib));
                        }
                    }
                } else {
                    cmd.arg(&out_str);
                    if options.debug {
                        cmd.arg("-g").arg("-O0");
                    } else {
                        cmd.arg("-O2");
                    }
                    cmd.arg("-o").arg(&exe_str);
                    for lib in &link_libs {
                        if lib.ends_with(".c") {
                            cmd.arg(lib);
                        } else if lib.ends_with(".so")
                            || lib.ends_with(".dylib")
                            || lib.ends_with(".a")
                            || lib.ends_with(".dll")
                        {
                            cmd.arg(lib);
                        } else if lib.starts_with("lib") {
                            cmd.arg(format!("-l{}", lib.trim_start_matches("lib")));
                        } else {
                            cmd.arg(format!("-l{}", lib.trim_end_matches(".dll")));
                        }
                    }
                }
                let status = cmd.status();
                if let Ok(st) = status {
                    if st.success() {
                        emit_symbols(Path::new(&exe_str), &module)?;
                        return Ok(exe_str);
                    }
                }
            }
            if cfg!(windows) {
                let mut cmd = std::process::Command::new("wsl");
                cmd.arg("gcc")
                    .arg(&out_str.replace('\\', "/"));
                if options.debug {
                    cmd.arg("-g").arg("-O0");
                } else {
                    cmd.arg("-O2");
                }
                let status = cmd
                    .arg("-o")
                    .arg(exe_str.replace('\\', "/").trim_end_matches(".exe"))
                    .status();
                if let Ok(st) = status {
                    if st.success() {
                        emit_symbols(Path::new(&exe_str), &module)?;
                        return Ok(exe_str);
                    }
                }
            }
            emit_symbols(&out, &module)?;
            Ok(out_str)
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
            let r = crate::targets::build_embedded(path_buf, &program, options.gc)?;
            for n in &r.notes {
                eprintln!("note: {}", n);
            }
            emit_symbols(&r.primary, &module)?;
            Ok(r.primary.display().to_string())
        }
        Target::Kernel => {
            let r = crate::targets::build_kernel(path_buf, &program)?;
            for n in &r.notes {
                eprintln!("note: {}", n);
            }
            emit_symbols(&r.primary, &module)?;
            Ok(r.primary.display().to_string())
        }
        Target::NativeBin => {
            let link_target = match options.platform {
                Platform::Windows => crate::native_codegen::LinkTarget::WindowsX64,
                Platform::Linux => crate::native_codegen::LinkTarget::LinuxX64,
                Platform::Macos => crate::native_codegen::LinkTarget::MacosX64,
                Platform::Current => crate::native_codegen::LinkTarget::host(),
                Platform::Uefi => crate::native_codegen::LinkTarget::UefiX64,
            };
            let r = crate::linker::build_native_bin(
                path_buf,
                &module,
                link_target,
                options.output.as_deref(),
            )?;
            for n in &r.notes {
                eprintln!("note: {}", n);
            }
            emit_symbols(&r.output, &module)?;
            Ok(format!(
                "{} (native-bin {}, objects={})",
                r.output.display(),
                link_target.name(),
                r.object_dir.display()
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
