# AGENTS.md

Guidance for AI coding agents working in this repository.

## Project overview

**RayTask** is a cross-platform programming language with one syntax for web, desktop, mobile, embedded, and systems code. This repository contains the full toolchain, written in **Rust** (edition 2021, requires Rust 1.70+):

- Compiler pipeline: lexer → parser → AST → typechecker → bytecode → bytecode VM
- True AOT path: SSA IR → C → vendored TinyCC (or gcc/clang/cl) → host binary, with a built-in ELF/COFF linker
- CLI (`raytask`): `build`, `run`, `check`, `test`, `new`, `doc`, `symbols`, `bind`, `link`, `tcc`, `dap`, and package commands (`install`, `uninstall`, `search`, `list`, `update`, `publish`)
- Standard library `bstd.*` (RayTask sources in `stdlib/bstd/*.rt` backed by Rust natives in `src/stdlib/*.rs`)
- Multi-repository package manager with a registry server app written in RayTask itself

The language supports classes, structs, interfaces, generics, `var`/`dyn`, nullable `T?`, properties, indexers, extension methods, operator overloading, async/await, closures, LINQ-style queries, reflection (`typeof`/`nameof`/`is`), FFI to C, GC (default in the VM), `stack`/`owned`/`unsafe` memory modes, inline `asm`, and systems attributes (`[packed]`, `[align]`, `[repr:"C"]`, `volatile`, `sizeof`/`offsetof`).

### Crate layout

- Library crate `raytask` at `src/lib.rs` — all compiler/VM modules are public.
- Binary `raytask` at `src/main.rs` — the CLI (clap derive).
- Binary `raytask-stub` at `src/bin/raytask_stub.rs` — runtime stub embedded into `--target app` executables (stub + `.rtbc` payload).

## Build and test commands

```bash
cargo build                 # debug build (build.rs compiles vendored TinyCC — needs a host C compiler)
cargo build --release       # LTO + codegen-units=1; binary at target/release/raytask[.exe]
cargo install --path .

cargo test                                # full integration suite
cargo test --test features                # one suite
cargo test --test product_targets --test spec_gaps
cargo run -- run examples/hello.rt        # smoke-run a RayTask program
cargo run -- check examples/bad_types.rt  # typechecker errors expected here
```

Notes:

- `build.rs` compiles the vendored TinyCC tree (`tcc/`) into a static `libtcc` via the `cc` crate, stages runtime headers/objects, and bootstraps `libtcc1.a` using a freshly built host `tcc`. A host C compiler is therefore required to build this crate. It also exports `RAYTASK_VENDORED_TCC_ROOT` and `RAYTASK_TCC_RUNTIME` as `rustc-env`.
- Some tests (AOT/linking) degrade gracefully when no C toolchain or stub binary is available — they skip rather than fail.
- There is no lint config beyond rustc defaults; keep `cargo build` warning-clean.

## Code organization

### `src/` — Rust compiler/runtime (each file = one module, re-exported from `lib.rs`)

- Front end: `lexer.rs`, `token.rs`, `parser.rs`, `ast.rs`, `span.rs`, `error.rs`, `preprocess.rs` (`#if`), `resolve.rs` (imports/modules), `sema.rs` (typechecker), `mono.rs` (monomorphization), `types.rs`, `stdlib_types.rs`
- Bytecode backend: `compiler.rs` (AST → bytecode), `bytecode.rs`, `bytecode_format.rs` (`.rtbc`), `vm.rs`, `value.rs`, `gc.rs`, `async_rt.rs`
- Native/AOT backend: `ssa/` (IR, builder, CFG, passes, `emit_c.rs` SSA→C), `codegen_c.rs` (AST→C transpile), `native_codegen.rs`, `native_triple.rs`, `targets.rs`, `app_build.rs`, `abi.rs` (C ABI for FFI), `native_rt/` (runtime C templates)
- Linking/TCC: `tcc.rs` (embedded libtcc bindings), `linker.rs` + `link/` (built-in ELF/COFF linker: `elf.rs`, `coff.rs`, `object.rs`, `resolve.rs`)
- FFI: `ffi.rs`, `ffi_bind.rs`, `c_header.rs` (C header parser, `raytask bind`)
- Tooling: `dap.rs` (Debug Adapter Protocol), `debug_symbols.rs` / `debug_io.rs` (`.rtdbg`), `migrate.rs`, `web_runtime.rs` (HttpServer/Web/Template/Sqlite primitives)
- Packages: `project.rs` (`project.rtp`), `registry.rs` (multi-repo client, `rtp.repos.yml`)
- Stdlib natives: `stdlib/` — one Rust file per `bstd` module (`io.rs`, `fs.rs`, `json.rs`, `net.rs`, `threads.rs`, `crypto.rs`, `compress.rs`, `time.rs`, `reflect.rs`, …). `src/registry.rs` uses `ureq` (TLS) + `serde_yaml`; SQLite via bundled `rusqlite`.

### Other top-level directories

- `stdlib/bstd/*.rt` — RayTask-side stdlib wrappers (`bstd.io`, `bstd.collections`, `bstd.sqlite`, `bstd.web`, …)
- `tests/*.rs` — integration tests (see below)
- `examples/` — `.rt` sample programs; **many tests execute these files**, so keep them working (`hello.rt`, `modules/`, `all_features/`, `systems/`, `boards/`, `tests.rt`, …)
- `docs/` — `SPEC.md` (language reference), `GUIDE.md`, `REGISTRY_PROTOCOL.md`, `ROADMAP.md`, `spec/` (chaptered spec, e.g. `05-systems.md`)
- `ru/` — Russian mirror of the docs
- `editors/vscode/` — VS Code extension (plain JavaScript: syntax highlighting, diagnostics via `raytask check`, DAP debugger client)
- `apps/registry/` — registry server application **written in RayTask** (`main.rt`), served by `web_runtime.rs`
- `packages/RTWebApp/` — reusable RayTask web layer package extracted from the registry app
- `deploy/registry-{windows,linux}-server/` — deployment copies/scripts for the registry server
- `tcc/` — vendored TinyCC source tree (treat as third-party; do not restyle)

## Build targets of the RayTask compiler itself

`raytask build main.rt --target …` supports: `bytecode` (`.rtbc`), `native` / `native-bin` (true AOT: SSA→C→TCC/gcc/clang; cross via `--platform`/`--arch` and clang `-target`), `app` (stub + embedded `.rtbc`), `wasm`, `web`, `mobile` (scaffolds), `embedded` / `kernel` (freestanding SSA→C), `efi`, `raw`. `--no-stdlib` produces freestanding programs without `bstd.*`; `-g` keeps debug info and emits `.rtdbg`. `raytask link *.o` drives the built-in linker; `raytask tcc …` exposes the bundled TinyCC.

## Code style guidelines

- Primary language of code, comments, and docs is **English**. A Russian doc mirror lives under `ru/`; update it only if the change affects documented user-facing behavior and you can match its style.
- Rust: standard `rustfmt` conventions, 4-space indent, `thiserror`/`anyhow` for errors. Modules are large single files (e.g. `vm.rs` ~2.4k lines, `parser.rs` ~2.8k lines) — extend the existing file rather than splitting, and match local naming/structure.
- Make minimal, scoped changes; do not refactor surrounding code, and do not touch `tcc/` (vendored).
- When changing user-visible CLI/language behavior, update `README.md` and the relevant doc in `docs/` (and this `AGENTS.md` if architecture/conventions change).

## Testing instructions

- Tests are Rust integration tests in `tests/`, organized by feature area (`features.rs`, `typecheck.rs`, `async.rs`, `ffi.rs`, `gc.rs`, `ssa_c.rs`, `ssa_opt.rs`, `aot.rs`, `linker_builtin.rs`, `product_targets.rs`, `spec_conformance.rs`, `spec_gaps.rs`, …).
- Tests drive the library API (`parse_file`, `run_file`, `run_source`, `check_source`) and frequently run files from `examples/`; RayTask-level assertions use builtins like `assertEq`. No tests are marked `#[ignore]`.
- Add tests in the suite matching your change; if you add a language feature, add an `examples/*.rt` demo and a test that runs it, and check conformance suites (`spec_gaps.rs`, `spec_conformance.rs`) for related expectations.
- `raytask test` runs RayTask-language tests (e.g. `examples/tests.rt`); `cargo test` is the Rust-side gate. Run both when touching the VM/compiler.

## Security considerations

- `Cargo.toml` pins `ureq` with `default-features = false, features = ["tls"]`; keep network code TLS-only. Registry configs have a `secure: true` flag that rejects non-HTTPS sources — do not weaken it.
- The package manager supports repo `token`s in `rtp.repos.yml`; never log or echo tokens. Do not add env-var substitution casually (documented as unsupported).
- `build.rs` and the CLI spawn external compilers (cc, tcc, gcc/clang) — validate/sanitize any user-controlled paths passed to them.
- FFI (`ffi.rs`, `libloading`) and `bstd.unsafe`/`unsafe` blocks are inherently memory-unsafe; changes there need extra scrutiny.
- The registry app (`apps/registry/`) handles auth and package uploads; secrets come from env vars (`RAYTASK_REGISTRY_ADMIN_USER`, `RAYTASK_REGISTRY_ADMIN_PASS`, `RAYTASK_REGISTRY_PUBLISH_TOKEN`) — never hardcode credentials.
