# RayTask

**RayTask** is a cross-platform programming language with one syntax for web, desktop, mobile, embedded, and systems code.

## Features

- Lexer → parser → AST → bytecode VM
- Transpile to C (`--target native`)
- CLI: `build`, `run`, `test`, `new`, `check`, `doc`, package commands
- Classes, structs, interfaces, generics, `var` / `dyn`, properties, async
- Memory: GC (default in the VM), `stack` / `owned` / `unsafe`
- Standard library `bstd.*` (core APIs as VM natives)

## Install

Requires [Rust](https://rustup.rs/) 1.70+:

```bash
cargo install --path .
```

Or from the repo root:

```bash
cargo build --release
# binary: target/release/raytask (or raytask.exe on Windows)
```

## Quick start

```bash
raytask new myapp
cd myapp
raytask run src/main.rt
```

Example:

```
import bstd.io;

void Main() {
    print("Hello, RayTask!");
    var x = 40;
    print($"Answer: {x + 2}");
}
```

## CLI

```bash
raytask check main.rt
raytask build main.rt --no-typecheck
raytask run main.rt
raytask build main.rt                              # → .rtbc
raytask build main.rt --target native              # → .c (+ gcc/clang), async+GC runtime
raytask build main.rt --target app --platform current
raytask build main.rt --target wasm
raytask build main.rt --target web
raytask build main.rt --target mobile
raytask build main.rt --target embedded --no-gc
raytask build main.rt --target kernel
raytask build main.rt --target native-bin --platform windows
raytask build main.rt --target efi
raytask build main.rt --target raw
raytask link main.rtbc --platform linux -o app.elf
raytask install SomeLib
raytask search http
raytask publish .
raytask test
raytask new myproject
raytask doc
```

### Product targets

| `--target` | Output |
|------------|--------|
| `bytecode` | `.rtbc` |
| `native` | C with GC + cooperative async |
| `app` | single executable (VM + bytecode) |
| `native-bin` | PE/ELF/Mach-O via NativeCodeGen + Linker |
| `efi` / `raw` | UEFI `.efi` or flat `.bin` |
| `wasm` / `web` | WASM/HTML bundle (+ bytecode payload for web) |
| `mobile` | Android / iOS scaffolds |
| `embedded` / `kernel` | freestanding C (+ ISR / MMIO attributes) |

Remote registry: set `RAYTASK_REGISTRY_URL` (`index.json` + packages).

### Typechecker

Static checks run before build and run:

- primitives, `var` / `dyn`, nullable `T?`, arrays, `ptr<T>`, generics
- functions, returns, call arguments
- classes / structs / interfaces, fields, properties, methods, `new`
- inheritance and override compatibility
- operators, assignments, control flow
- `unsafe` for pointers

```bash
raytask check examples/hello.rt
raytask check examples/bad_types.rt
```

### Standalone app (`--target app`)

Builds one executable: runtime stub (VM) + embedded `.rtbc`.

```bash
raytask build examples/hello.rt --target app --platform current
./examples/dist/hello
```

A portable Cargo project is also written under `dist/<name>_app/` for rebuilding on another host.

## Examples

```bash
cargo run -- run examples/hello.rt
cargo run -- run examples/point.rt
cargo run -- test examples/tests.rt
```

## Compiler pipeline

```
.rt source
   ↓ Lexer
 tokens
   ↓ Parser
 AST
   ↓ Compiler
 bytecode Module
   ↓ VM
 execution

AST ──→ CCodegen ──→ .c ──→ gcc/clang / emcc / freestanding ──→ native|wasm|kernel
```

## Documentation

- [Language specification](docs/SPEC.md)
- [User guide](docs/GUIDE.md)
- [Standard library](stdlib/README.md)
- Russian: [ru/README.md](ru/README.md)

## Status

Implemented (spec §§1–30 core):

- Compiler, typechecker, VM, C transpile, product targets
- Imports, OOP, LINQ-style queries, operators, properties, indexers, extension methods
- async/await (VM + native C), FFI, GC (VM + native), closures, monomorphization
- `project.rtp`, install / search / publish, local and remote registry
- Preprocessor `#if`, `raytask doc`, `match` on Result, `using` / `owned`

```bash
raytask new myapp && cd myapp && raytask run
raytask build src/main.rt --target web
cargo test --test product_targets --test spec_gaps
```

## License

MIT
