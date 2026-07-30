//! RayTask CLI — build, run, test, new.

use clap::{Parser, Subcommand, ValueEnum};
use colored::Colorize;
use raytask::app_build::Platform;
use raytask::{
    compile_file, inspect_bytecode, parse_file, run_file_with, run_source, transpile_c,
    BuildOptions, Optimize, RunOptions, Target,
};
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

#[derive(Parser)]
#[command(name = "raytask")]
#[command(version = "0.1.0")]
#[command(about = "RayTask — cross-platform programming language", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Compile a RayTask source file or project
    Build {
        /// Source file (.rt)
        file: Option<PathBuf>,
        /// Target: bytecode | native | app | wasm | web | mobile | embedded | kernel | native-bin | efi | raw
        #[arg(long, default_value = "bytecode")]
        target: TargetArg,
        /// Target OS for --target app / native-bin: current | windows | linux | macos | uefi
        #[arg(long, default_value = "current")]
        platform: String,
        /// Output path (for --target app)
        #[arg(short, long)]
        output: Option<PathBuf>,
        /// Optimization: none, speed, size
        #[arg(long, default_value = "none")]
        optimize: OptArg,
        /// Disable GC (embedded)
        #[arg(long)]
        no_gc: bool,
        /// Enable GC
        #[arg(long)]
        gc: bool,
        /// Include debug info / emit `.rtdbg` symbols (-g)
        #[arg(short = 'g', long)]
        debug: bool,
        /// Disable built-in bstd.* imports, types, and runtime globals
        #[arg(long)]
        no_stdlib: bool,
        /// Skip typechecker
        #[arg(long)]
        no_typecheck: bool,
    },
    /// Compile and run a RayTask program
    Run {
        /// Source file (.rt); omit to use project.rtp entry
        file: Option<PathBuf>,
        /// Enable GC (default)
        #[arg(long, conflicts_with = "no_gc")]
        gc: bool,
        /// Disable GC (embedded / manual memory)
        #[arg(long)]
        no_gc: bool,
        /// Collect on every allocation (debug)
        #[arg(long)]
        gc_stress: bool,
        /// Disable built-in bstd.* imports, types, and runtime globals
        #[arg(long)]
        no_stdlib: bool,
        /// Skip typechecker
        #[arg(long)]
        no_typecheck: bool,
        /// Extra args passed to the program
        #[arg(trailing_var_arg = true)]
        args: Vec<String>,
    },
    /// Parse/check a file without running
    Check {
        file: PathBuf,
        /// Disable built-in bstd.* imports, types, and runtime globals
        #[arg(long)]
        no_stdlib: bool,
    },
    /// Run [test] attributed functions
    Test {
        #[arg(default_value = ".")]
        path: PathBuf,
    },
    /// Create a new RayTask project
    New {
        name: String,
    },
    /// Transpile to C
    EmitC {
        file: PathBuf,
        #[arg(short, long)]
        output: Option<PathBuf>,
    },
    /// Show tokens (debug)
    Lex {
        file: PathBuf,
    },
    /// Show AST (debug)
    Ast {
        file: PathBuf,
    },
    /// Install a package from a configured repository
    Install {
        /// Package name, optionally with version: `MyLib` or `MyLib@1.2.0`
        package: String,
        /// Show package description and instructions before installing
        #[arg(long)]
        info: bool,
    },
    /// Uninstall a package from external/
    Uninstall {
        package: String,
    },
    /// Update packages declared in project.rtp
    Update,
    /// Search packages across configured repositories
    Search {
        query: String,
    },
    /// List installed packages in external/
    List,
    /// Publish a package to local or remote registry
    Publish {
        #[arg(default_value = ".")]
        path: PathBuf,
    },
    /// Import a .csproj project into a RayTask tree
    Migrate {
        csproj: PathBuf,
        #[arg(short, long)]
        output: Option<PathBuf>,
    },
    /// Convert a .cs source file to .rt
    Convert {
        file: PathBuf,
        #[arg(long = "to-rt", default_value_t = true)]
        to_rt: bool,
        #[arg(short, long)]
        output: Option<PathBuf>,
    },
    /// Analyze a .csproj for conversion notes
    Analyze {
        csproj: PathBuf,
    },
    /// Debug Adapter Protocol server (stdio) for VS Code / editors
    Dap,
    /// Generate debug symbols (`.rtdbg`) for a source file
    Symbols {
        file: PathBuf,
        #[arg(short, long)]
        output: Option<PathBuf>,
    },
    /// Generate RayTask FFI decls from a C header (no gcc)
    Bind {
        /// C header file (.h)
        header: PathBuf,
        /// Shared library name (.dll / .so / .dylib)
        #[arg(long)]
        lib: String,
        #[arg(short, long)]
        output: Option<PathBuf>,
    },
    /// Link a .rtbc bytecode file into a native binary
    Link {
        /// Input .rtbc file
        file: PathBuf,
        /// windows | linux | macos | uefi | raw | current
        #[arg(long, default_value = "current")]
        platform: String,
        #[arg(short, long)]
        output: Option<PathBuf>,
    },
    /// Generate markdown docs from `///` comments
    Doc {
        #[arg(default_value = ".")]
        path: PathBuf,
    },
    /// Invoke vendored TinyCC through RayTask
    Tcc {
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
    },
    /// Inspect a binary .rtbc file in human-readable form
    InspectRtbc {
        file: PathBuf,
        /// Include disassembled chunk bytecode
        #[arg(long)]
        disassemble: bool,
    },
}

#[derive(Clone, Copy, ValueEnum)]
enum TargetArg {
    Bytecode,
    Native,
    /// Standalone application (runtime + bytecode embedded)
    App,
    Wasm,
    Web,
    Mobile,
    Embedded,
    Kernel,
    /// NativeCodeGen + Linker (PE/ELF/Mach-O)
    #[value(name = "native-bin")]
    NativeBin,
    Efi,
    Raw,
}

#[derive(Clone, Copy, ValueEnum)]
enum OptArg {
    None,
    Speed,
    Size,
}

fn main() {
    let cli = Cli::parse();
    if let Err(e) = dispatch(cli) {
        eprintln!("{} {}", "error:".red().bold(), e);
        std::process::exit(1);
    }
}

fn dispatch(cli: Cli) -> Result<(), Box<dyn std::error::Error>> {
    match cli.command {
        Commands::Build {
            file,
            target,
            platform,
            output,
            optimize,
            no_gc,
            gc: _,
            debug,
            no_stdlib,
            no_typecheck,
        } => {
            let cli_target = target;
            let (file, mut options) = resolve_build_context(
                file,
                target,
                &platform,
                output,
                optimize,
                no_gc,
                debug,
                no_stdlib,
                no_typecheck,
            )?;
            // Ensure declared dependencies are present
            if Path::new("project.rtp").exists() {
                if let Ok(proj) = raytask::project::load_project(Path::new(".")) {
                    let _ = raytask::project::update_packages(&proj);
                    println!(
                        "{} project {} v{}",
                        "Building".cyan().bold(),
                        proj.name,
                        proj.version
                    );
                    options.optimize = proj.build.optimize;
                    if matches!(cli_target, TargetArg::Bytecode) {
                        // Only apply project target when user left default bytecode
                        // (clap default) — still override with project settings.
                        options.target = proj.build.target;
                    }
                    if !no_gc {
                        options.gc = proj.build.gc;
                    }
                    options.debug = options.debug || proj.build.debug;
                }
            }
            let out = compile_file(file.to_str().unwrap(), &options)?;
            println!("{} {}", "Built".green().bold(), out);
            if options.debug {
                println!(
                    "{} debug symbols (.rtdbg) written alongside output",
                    "note:".cyan()
                );
            }
            if !options.gc {
                println!("{} GC disabled (--no-gc)", "note:".yellow());
            }
        }
        Commands::Run {
            file,
            gc: _,
            no_gc,
            gc_stress,
            no_stdlib,
            no_typecheck,
            args: _,
        } => {
            let file = resolve_entry(file)?;
            println!("{} {}", "Running".cyan().bold(), file.display());
            let mut gc = !no_gc;
            if Path::new("project.rtp").exists() && !no_gc {
                if let Ok(proj) = raytask::project::load_project(Path::new(".")) {
                    let _ = raytask::project::update_packages(&proj);
                    gc = proj.build.gc;
                }
            }
            let opts = RunOptions {
                gc,
                gc_stress,
                no_typecheck,
                no_stdlib,
            };
            if !opts.gc {
                println!("{} GC disabled (--no-gc)", "note:".yellow());
            }
            run_file_with(&file, &opts)?;
        }
        Commands::Check { file, no_stdlib } => {
            let program = raytask::parse_file_with_stdlib(&file, !no_stdlib)?;
            let report = raytask::sema::typecheck_with_stdlib(&program, !no_stdlib);
            if report.ok() {
                println!(
                    "{} {} — typecheck passed ({} top-level items)",
                    "OK".green().bold(),
                    file.display(),
                    program.items.len()
                );
                if !report.warnings.is_empty() {
                    eprint!("{}", report.format_all());
                }
            } else {
                eprint!("{}", report.format_all());
                println!(
                    "{} {} error(s), {} warning(s)",
                    "FAILED".red().bold(),
                    report.errors.len(),
                    report.warnings.len()
                );
                std::process::exit(1);
            }
        }
        Commands::Test { path } => {
            run_tests(&path)?;
        }
        Commands::New { name } => {
            create_project(&name)?;
        }
        Commands::EmitC { file, output } => {
            let source = std::fs::read_to_string(&file)?;
            let c = transpile_c(&source)?;
            let out = output.unwrap_or_else(|| file.with_extension("c"));
            std::fs::write(&out, c)?;
            println!("{} {}", "Wrote".green().bold(), out.display());
        }
        Commands::Lex { file } => {
            let source = std::fs::read_to_string(&file)?;
            let tokens = raytask::lexer::Lexer::new(&source).tokenize()?;
            for t in tokens {
                println!("{:>4}:{:<3} {:?} {:?}", t.span.line, t.span.column, t.kind, t.lexeme);
            }
        }
        Commands::Ast { file } => {
            let program = parse_file(&file)?;
            println!("{:#?}", program);
        }
        Commands::Install { package, info } => {
            let (name, ver) = if let Some((n, v)) = package.split_once('@') {
                (n, Some(v))
            } else {
                (package.as_str(), None)
            };
            let path = raytask::registry::install(raytask::registry::InstallOptions {
                name,
                version: ver,
                show_info: info,
            })
            .map_err(|e| raytask::error::CompileError::Io { message: e })?;
            println!(
                "{} {} → {}",
                "Installed".green().bold(),
                name,
                path.display()
            );
            if Path::new("project.rtp").exists() {
                append_dependency_to_project(name, ver.unwrap_or("*"))?;
            }
        }
        Commands::Uninstall { package } => {
            if raytask::registry::uninstall(&package)
                .map_err(|e| raytask::error::CompileError::Io { message: e })?
            {
                println!("{} {}", "Removed".green().bold(), package);
            } else {
                println!("{} package not found", "warning:".yellow());
            }
        }
        Commands::Update => {
            if Path::new("project.rtp").exists() {
                let proj = raytask::project::load_project(Path::new("."))?;
                let updated = raytask::project::update_packages(&proj)?;
                if updated.is_empty() {
                    println!("{}", "No dependencies to update.".green());
                } else {
                    for u in updated {
                        println!("{} {}", "Updated".green().bold(), u);
                    }
                }
            } else {
                println!("{}", "No project.rtp — nothing to update.".yellow());
            }
        }
        Commands::Search { query } => {
            let results = raytask::registry::search(&query);
            if results.is_empty() {
                println!("{}", "No packages matched.".yellow());
            } else {
                for r in results {
                    println!(
                        "{} @ {}  [{}]  {}",
                        r.entry.name.green().bold(),
                        r.entry.version,
                        r.repo_name.cyan(),
                        r.entry.description.as_deref().unwrap_or("")
                    );
                }
            }
        }
        Commands::List => {
            let pkgs = raytask::registry::list_installed();
            if pkgs.is_empty() {
                println!("{}", "No packages installed in external/.".yellow());
            } else {
                println!("{:<24} {:<12} {}", "Package", "Version", "Repository");
                println!("{}", "─".repeat(56));
                for p in pkgs {
                    println!("{:<24} {:<12} {}", p.name.green(), p.version, p.repo.cyan());
                }
            }
        }
        Commands::Publish { path } => {
            let msg = raytask::project::publish_package(&path)?;
            println!("{} {}", "Published".green().bold(), msg);
        }
        Commands::Migrate { csproj, output } => {
            let dest = raytask::migrate::migrate_csproj(&csproj, output.as_deref())?;
            println!("{} {}", "Migrated".green().bold(), dest.display());
        }
        Commands::Convert {
            file,
            to_rt: _,
            output,
        } => {
            let out = raytask::migrate::convert_file(&file, output.as_deref())?;
            println!("{} {}", "Converted".green().bold(), out.display());
        }
        Commands::Analyze { csproj } => {
            let report = raytask::migrate::analyze_csproj(&csproj)?;
            for n in &report.notes {
                println!("{} {}", "note:".cyan(), n);
            }
            for i in &report.issues {
                println!("{} {}", "review:".yellow(), i);
            }
            println!(
                "{} {} file(s), {} issue(s)",
                "Analyze".green().bold(),
                report.files.len(),
                report.issues.len()
            );
        }
        Commands::Link {
            file,
            platform,
            output,
        } => {
            let bytes = std::fs::read(&file)?;
            let link_target = raytask::linker::link_target_from_platform(&platform)
                .ok_or_else(|| {
                    format!(
                        "unknown platform '{platform}' (use: current, windows, linux, macos, uefi, raw)"
                    )
                })?;
            // raw platform alias
            let link_target = if platform.eq_ignore_ascii_case("raw")
                || platform.eq_ignore_ascii_case("bin")
            {
                raytask::native_codegen::LinkTarget::RawX64
            } else {
                link_target
            };
            let out = output.unwrap_or_else(|| {
                file.with_extension(link_target.default_ext())
            });
            let result = raytask::linker::link_rtbc(&bytes, link_target, &out, "linked")?;
            for n in &result.notes {
                println!("{} {}", "note:".cyan(), n);
            }
            println!("{} {}", "Linked".green().bold(), result.output.display());
        }
        Commands::Bind {
            header,
            lib,
            output,
        } => {
            let parsed = raytask::c_header::parse_header_file(&header)?;
            let text = raytask::c_header::prototypes_to_raytask(&lib, &parsed.prototypes);
            if let Some(out) = output {
                std::fs::write(&out, &text)?;
                println!(
                    "{} {} — {} prototype(s) → {}",
                    "Bound".green().bold(),
                    header.display(),
                    parsed.prototypes.len(),
                    out.display()
                );
            } else {
                print!("{}", text);
                eprintln!(
                    "{} {} prototype(s) from {}",
                    "note:".cyan(),
                    parsed.prototypes.len(),
                    header.display()
                );
            }
        }
        Commands::Dap => {
            raytask::dap::run_dap()?;
        }
        Commands::Tcc { args } => {
            raytask::tcc::run_tcc_cli(&args)
                .map_err(|e| -> Box<dyn std::error::Error> { e.into() })?;
        }
        Commands::InspectRtbc { file, disassemble } => {
            let bytes = std::fs::read(&file)?;
            let text = inspect_bytecode(&bytes, disassemble)?;
            print!("{}", text);
        }
        Commands::Symbols { file, output } => {
            let program = parse_file(&file)?;
            let report = raytask::sema::typecheck(&program);
            if !report.ok() {
                eprint!("{}", report.format_all());
                std::process::exit(1);
            }
            let program = raytask::mono::monomorphize(program);
            let mut module = raytask::compiler::Compiler::new()
                .with_source(file.display().to_string())
                .compile(&program)?;
            raytask::debug_symbols::stamp_source(&mut module, &file);
            let out = output.unwrap_or_else(|| raytask::debug_symbols::sidecar_path(&file));
            let sym = raytask::debug_symbols::DebugSymbols::from_module(&module, &file, None);
            sym.write_file(&out)?;
            println!("{} {}", "Symbols".green().bold(), out.display());
            println!(
                "  {} chunk(s), {} global(s)",
                sym.chunks.len(),
                sym.globals.len()
            );
        }
        Commands::Doc { path } => {
            generate_docs(&path)?;
        }
    }
    Ok(())
}

fn resolve_build_context(
    file: Option<PathBuf>,
    target: TargetArg,
    platform: &str,
    output: Option<PathBuf>,
    optimize: OptArg,
    no_gc: bool,
    debug: bool,
    no_stdlib: bool,
    no_typecheck: bool,
) -> Result<(PathBuf, BuildOptions), Box<dyn std::error::Error>> {
    let file = resolve_entry(file)?;
    let platform = Platform::parse(platform).ok_or_else(|| {
        format!("unknown platform '{platform}' (use: current, windows, linux, macos, uefi)")
    })?;
    let options = BuildOptions {
        target: match target {
            TargetArg::Bytecode => Target::Bytecode,
            TargetArg::Native => Target::Native,
            TargetArg::App => Target::App,
            TargetArg::Wasm => Target::Wasm,
            TargetArg::Web => Target::Web,
            TargetArg::Mobile => Target::Mobile,
            TargetArg::Embedded => Target::Embedded,
            TargetArg::Kernel => Target::Kernel,
            TargetArg::NativeBin => Target::NativeBin,
            TargetArg::Efi => Target::Efi,
            TargetArg::Raw => Target::Raw,
        },
        optimize: match optimize {
            OptArg::None => Optimize::None,
            OptArg::Speed => Optimize::Speed,
            OptArg::Size => Optimize::Size,
        },
        gc: !no_gc,
        gc_stress: false,
        debug,
        platform,
        output,
        no_typecheck,
        no_stdlib,
    };
    Ok((file, options))
}

fn append_dependency_to_project(name: &str, version: &str) -> Result<(), Box<dyn std::error::Error>> {
    let path = Path::new("project.rtp");
    let mut src = std::fs::read_to_string(path)?;
    if src.contains(&format!("\"{name}\"")) {
        return Ok(());
    }
    if let Some(idx) = src.find("dependencies {") {
        let insert_at = idx + "dependencies {".len();
        src.insert_str(
            insert_at,
            &format!("\n        \"{name}\" version \"{version}\""),
        );
        std::fs::write(path, src)?;
    }
    Ok(())
}

fn generate_docs(path: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let files: Vec<_> = if path.is_file() {
        vec![path.to_path_buf()]
    } else {
        WalkDir::new(path)
            .into_iter()
            .filter_map(|e| e.ok())
            .map(|e| e.path().to_path_buf())
            .filter(|p| p.extension().map(|e| e == "rt").unwrap_or(false))
            .collect::<Vec<_>>()
    };
    let out_dir = Path::new("docs").join("api");
    std::fs::create_dir_all(&out_dir)?;
    let mut index = String::from("# RayTask API\n\nGenerated from `///` doc comments.\n\n");
    for file in &files {
        let source = std::fs::read_to_string(file)?;
        let docs = extract_doc_comments(&source);
        if docs.is_empty() {
            continue;
        }
        let name = file
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("module");
        let mut md = format!("# {}\n\nSource: `{}`\n\n", name, file.display());
        for (item, doc) in &docs {
            md.push_str(&format!("## {}\n\n{}\n\n", item, doc));
            index.push_str(&format!("- [{}]({}.md) — {}\n", item, name, item));
        }
        std::fs::write(out_dir.join(format!("{}.md", name)), md)?;
    }
    std::fs::write(out_dir.join("README.md"), index)?;
    println!("{} {}", "Docs".green().bold(), out_dir.display());
    Ok(())
}

fn extract_doc_comments(source: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut pending = String::new();
    for line in source.lines() {
        let t = line.trim();
        if let Some(rest) = t.strip_prefix("///") {
            if !pending.is_empty() {
                pending.push('\n');
            }
            pending.push_str(rest.trim());
        } else if !pending.is_empty() {
            let item = t
                .trim_start_matches("export ")
                .trim_start_matches("async ")
                .split(['(', '{', '<', ':'])
                .next()
                .unwrap_or(t)
                .trim()
                .to_string();
            if !item.is_empty() && !item.starts_with("//") {
                out.push((item, std::mem::take(&mut pending)));
            } else {
                pending.clear();
            }
        }
    }
    out
}

fn resolve_entry(file: Option<PathBuf>) -> Result<PathBuf, Box<dyn std::error::Error>> {
    if let Some(f) = file {
        return Ok(f);
    }
    if Path::new("project.rtp").exists() {
        let proj = raytask::project::load_project(Path::new("."))?;
        return Ok(raytask::project::entry_path(&proj)?);
    }
    if Path::new("main.rt").exists() {
        return Ok(PathBuf::from("main.rt"));
    }
    if Path::new("src/main.rt").exists() {
        return Ok(PathBuf::from("src/main.rt"));
    }
    Err("no input file; pass a .rt file or create project.rtp / main.rt".into())
}

fn create_project(name: &str) -> Result<(), Box<dyn std::error::Error>> {
    let root = Path::new(name);
    if root.exists() {
        return Err(format!("directory '{}' already exists", name).into());
    }
    std::fs::create_dir_all(root.join("src"))?;
    std::fs::write(
        root.join("project.rtp"),
        format!(
            r#"project "{name}" {{
    version = "0.1.0"
    author = ""
    description = "A RayTask application"

    dependencies {{
    }}

    build {{
        optimize = "speed"
        target = "bytecode"
        gc = true
    }}
}}
"#
        ),
    )?;
    std::fs::write(
        root.join("src/main.rt"),
        r#"import bstd.io;

void Main() {
    print("Hello, RayTask!");
}
"#,
    )?;
    std::fs::write(
        root.join("README.md"),
        format!("# {name}\n\nBuilt with [RayTask](https://github.com/raytask).\n\n```bash\nraytask run src/main.rt\n```\n"),
    )?;
    println!("{} project {}", "Created".green().bold(), name);
    println!("  cd {}", name);
    println!("  raytask run src/main.rt");
    Ok(())
}

fn run_tests(path: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let mut passed = 0;
    let mut failed = 0;
    let files: Vec<_> = if path.is_file() {
        vec![path.to_path_buf()]
    } else {
        WalkDir::new(path)
            .into_iter()
            .filter_map(|e| e.ok())
            .map(|e| e.path().to_path_buf())
            .filter(|p| p.extension().map(|e| e == "rt").unwrap_or(false))
            .collect::<Vec<_>>()
    };

    for file in files {
        let source = std::fs::read_to_string(&file)?;
        // Find [test] functions and wrap a harness
        if !source.contains("[test]") {
            continue;
        }
        println!("{} {}", "test".cyan(), file.display());
        // Extract test function names via simple scan
        let test_fns = find_test_functions(&source);
        for name in test_fns {
            let harness = format!(
                "{}\n\nvoid __raytask_test_main() {{\n    {}();\n    print(\"  PASS {}\");\n}}\n\nvoid Main() {{ __raytask_test_main(); }}\n",
                strip_main(&source),
                name,
                name
            );
            match run_source(&harness) {
                Ok(()) => {
                    println!("  {} {}", "ok".green(), name);
                    passed += 1;
                }
                Err(e) => {
                    println!("  {} {} — {}", "FAIL".red(), name, e);
                    failed += 1;
                }
            }
        }
    }

    println!();
    if failed == 0 {
        println!("{} {} passed", "OK".green().bold(), passed);
    } else {
        println!("{} {} passed, {} failed", "FAILED".red().bold(), passed, failed);
        std::process::exit(1);
    }
    Ok(())
}

fn find_test_functions(source: &str) -> Vec<String> {
    let mut result = Vec::new();
    let lines: Vec<&str> = source.lines().collect();
    for (i, line) in lines.iter().enumerate() {
        if line.trim().starts_with("[test]") {
            // next non-empty line should be function
            for j in (i + 1)..lines.len() {
                let l = lines[j].trim();
                if l.is_empty() || l.starts_with("//") || l.starts_with('[') {
                    continue;
                }
                // void Name( or Type Name(
                if let Some(paren) = l.find('(') {
                    let before = l[..paren].trim();
                    if let Some(name) = before.split_whitespace().last() {
                        result.push(name.to_string());
                    }
                }
                break;
            }
        }
    }
    result
}

fn strip_main(source: &str) -> String {
    // Remove void Main() { ... } roughly so harness can redefine
    if let Some(start) = source.find("void Main(") {
        let mut depth = 0;
        let bytes = source.as_bytes();
        let mut i = start;
        let mut started = false;
        while i < bytes.len() {
            match bytes[i] as char {
                '{' => {
                    depth += 1;
                    started = true;
                }
                '}' => {
                    depth -= 1;
                    if started && depth == 0 {
                        i += 1;
                        break;
                    }
                }
                _ => {}
            }
            i += 1;
        }
        let mut out = String::new();
        out.push_str(&source[..start]);
        out.push_str(&source[i..]);
        out
    } else {
        source.to_string()
    }
}
