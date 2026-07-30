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
raytask check main.rt --no-stdlib                 # reject bstd.* and builtin globals
raytask build main.rt --no-typecheck
raytask build main.rt -g                              # → .rtbc + .rtdbg debug symbols
raytask build main.rt --no-stdlib                    # freestanding bytecode without bstd
raytask symbols main.rt                               # → main.rtdbg only
raytask bind mylib.h --lib mylib.dll                  # C header → FFI decls (no gcc)
raytask run main.rt
raytask run main.rt --no-stdlib                      # run source without bstd or builtin globals
raytask build main.rt                              # → .rtbc
raytask build main.rt --target native              # → .c (+ vendored TCC first, then gcc/clang/cl fallback)
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
raytask tcc examples/tcc/hello.c -o hello.exe
raytask tcc -run examples/tcc/hello.c -- demo
raytask dap                                          # Debug Adapter Protocol (stdio)
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

### Vendored TCC

The repository now vendors TinyCC under `tcc/` and builds `libtcc` as part of the normal Cargo build.

- `raytask build ... --target native` now tries the embedded TCC backend first, then falls back to host `gcc` / `clang` / `cl` when needed.
- `raytask tcc ...` exposes the bundled compiler directly for C files, object files, shared libraries, and `-run` workflows.
- The bundled runtime headers and support files are resolved from the vendored `tcc/` tree, so no external GCC toolchain is required for the TCC path.

Remote registry: set `RAYTASK_REGISTRY_URL` (`index.json` + packages).

### Registry App

This repo now includes a server-side RayTask application at `apps/registry/` that implements
the **RayTask lib Registry** on top of new web/runtime primitives:

- `HttpServer` + `Web` request/response context
- `Template.Render(...)` with escaped variables and raw HTML blocks
- `Sqlite.Open(...)` for the app data layer
- moderated package versions, public catalog, login/register, maintainer dashboard
- machine endpoints compatible with the current package client:
  - `GET /index.json`
  - `GET /packages/{name}/{version}.zip`

The reusable RayTask-side web layer has been extracted into `packages/RTWebApp/`. This package is
written entirely in RayTask and can be published to the registry separately.

Deployment-oriented server copies are prepared under:

- `deploy/registry-windows-server/`
- `deploy/registry-linux-server/`

Run it from the repo root:

```bash
cargo run -- run apps/registry/main.rt
```

Optional bootstrap / automation env vars:

- `RAYTASK_REGISTRY_ADMIN_USER`
- `RAYTASK_REGISTRY_ADMIN_PASS`
- `RAYTASK_REGISTRY_PUBLISH_TOKEN`

Example local client config:

```yaml
repositories:
  - name: local-registry
    url: http://127.0.0.1:8080
    priority: 999
    secure: false
```

Then:

```bash
raytask search RegistryDemo
raytask install RegistryDemo@0.1.1
raytask publish examples/registry_pkg
```

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

Use `--no-stdlib` on `check`, `build`, or `run` when you want a freestanding program without
`bstd.*` imports, builtin globals like `print`, or stdlib-backed runtime types.

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
cargo run -- run examples/modules/main.rt   # multi-file: lib.rt + main.rt
cargo run -- test examples/tests.rt
```

### Modules (`examples/modules/`)

| File | Role |
|------|------|
| `lib.rt` | Classes (`Counter`, `Greeter`) and functions (`Double`, `Add`, …) |
| `main.rt` | `import lib;` then `Main()` uses that API |

```bash
raytask run examples/modules/main.rt
```

### Threads & Channels (`examples/threads/`)

```raytask
var mx = Mutex.New();  mx.Unlock(0);
var t  = Thread.Run(() => { mx.Unlock(mx.Lock() + 1); });
t.Wait();

var ch = Channel.New();
Thread.Run(() => { ch.Send(42); ch.Close(); });
print(ch.Recv());
```

### Generator / Enumerator (`examples/generators/`)

```raytask
// Диапазон с шагом
var gen = Generator.Range(0, 10, 2);   // 0, 2, 4, 6, 8
while (gen.HasNext()) { print(gen.Next()); }

// Из массива
var g2 = Generator.From([10, 20, 30]);
print(g2.ToList());    // [10, 20, 30]

// Бесконечный — Repeat без счётчика
var ticks = Generator.Repeat(GetTime());   // всегда даёт текущее время при создании
```

### DateTime & TimeSpan (`examples/datetime/`)

```raytask
var now      = DateTime.Now;
var deadline = now.Add(TimeSpan.FromHours(48));
print(deadline.Format("yyyy-MM-dd HH:mm"));

var diff = now.Diff(DateTime.Parse("2020-01-01"));
print($"{diff.Days} days since 2020");
```

### File Streams (`examples/streams/`)

```raytask
var ws = Stream.OpenWrite("out.txt");
ws.WriteLine("hello");
ws.Close();

var rs = Stream.OpenRead("out.txt");
var line = rs.ReadLine();    // "hello"
rs.Close();
```

### Compression — gz / zstd (`examples/compress/`)

```raytask
Compress.GzCompressFile("big.txt", "big.txt.gz");
Compress.GzDecompressFile("big.txt.gz", "big2.txt");

Compress.ZstdCompressFile("data.bin", "data.bin.zst", 6);
Compress.ZstdDecompressFile("data.bin.zst", "data2.bin");
```

Run all at once:

```bash
raytask run examples/all_features/main.rt
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

## VS Code / Cursor

Language support and debugging live in [`editors/vscode`](editors/vscode):

- Syntax highlighting, snippets, diagnostics (`raytask check`), completions
- Debugger via `raytask dap` (breakpoints, step, locals/globals)

```bash
cargo install --path .
cd editors/vscode
npx @vscode/vsce package --no-dependencies -o raytask-0.1.0.vsix
code --install-extension ./raytask-0.1.0.vsix
```

Set `raytask.path` if the CLI is not on `PATH`. See [editors/vscode/README.md](editors/vscode/README.md).

Debugger supports breakpoints (including conditions and logpoints), step over/in/out, pause, named locals, expandable objects, and Debug Console output from `print`.

### Debug symbols (`.rtdbg`)

```bash
raytask build src/main.rt -g          # bytecode keeps locals + writes main.rtdbg
raytask symbols src/main.rt -o out.rtdbg
```

Without `-g`, release `.rtbc` strips local names/source paths (smaller). A sidecar `.rtdbg` can still be loaded by the DAP when debugging a `.rtbc` file.

## Package Manager

RayTask has a built-in multi-repository package manager. Configure repositories in `rtp.repos.yml`:

```yaml
repositories:
  - name: official
    url: https://registry.raytask.dev
    priority: 100

  - name: company
    url: https://packages.example.com/raytask
    priority: 50
    token: s3cr3t

install_dir: external   # packages land here (default)
```

### CLI commands

```bash
raytask install HttpClient          # install latest version
raytask install HttpClient@1.2.0    # install specific version
raytask install HttpClient --info   # show description + instructions, confirm before install
raytask uninstall HttpClient
raytask search http
raytask list                        # list installed packages in external/
raytask update                      # reinstall all deps from project.rtp
```

### Importing installed packages

```rt
import "external/HttpClient/src/lib"

func main() {
    let c = Http.NewClient("https://api.example.com");
    print_ln(c.Get("/users"));
}
```

### Version resolution

- If the same package version is found in multiple repos, the **highest-priority** repo wins.
- If no version is specified, the **newest** version across all repos is chosen.
- Installed packages are tracked in `external/<Name>/rtp.lock.yml`.

See [docs/REGISTRY_PROTOCOL.md](docs/REGISTRY_PROTOCOL.md) for the full registry server specification.

## Documentation

- [Language specification](docs/SPEC.md)
- [User guide](docs/GUIDE.md)
- [Registry protocol](docs/REGISTRY_PROTOCOL.md)
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
