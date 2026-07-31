# RayTask User Guide

Practical guide for building and running RayTask programs.

## Project layout

```bash
raytask new myapp
```

Creates:

```
myapp/
  project.rtp
  src/main.rt
  README.md
```

Run without naming a file (uses `project.rtp` entry):

```bash
cd myapp
raytask run
```

### `project.rtp`

```
project "myapp" {
    version = "0.1.0"
    entry = "src/main.rt"

    dependencies {
    }

    build {
        optimize = "speed"
        target = "bytecode"
        gc = true
    }
}
```

## Build and run

```bash
raytask check src/main.rt          # parse + typecheck
raytask run src/main.rt            # typecheck + VM
raytask build src/main.rt          # write .rtbc
raytask build src/main.rt --optimize speed
raytask build src/main.rt --target native
raytask tcc examples/tcc/hello.c -o hello.exe
raytask tcc -run examples/tcc/hello.c -- demo
```

### Optimization (`--optimize`)

Bytecode-producing targets run through a mid-level SSA IR:

| Level | Effect |
|-------|--------|
| `none` (default) | AST → bytecode → SSA lift (validation) → keep original bytecode |
| `speed` | Full pipeline: CFG simplify, mem2reg, SCCP, const fold, copy prop, GVN/CSE, DCE, LICM, strength reduction, aggressive inlining, cleanup |
| `size` | Same core passes with conservative inlining; skips code-growing strength reduction |

```bash
raytask build src/main.rt --optimize speed -o out.rtbc
```

`project.rtp` may set `build.optimize = "speed"`; an explicit CLI `--optimize` overrides the project default.

AST→C for host `transpile_c` still starts from the AST; **`embedded` / `kernel` / `native` / `native-bin` use SSA → C**. Host AOT links with TCC/gcc into a real binary with **no RTBC interpreter** (`--target app` keeps stub+bytecode).

## Systems / bare metal

RayTask can target firmware and kernels without abandoning managed apps:

```bash
raytask build examples/systems/blink.rt --target embedded --no-gc
raytask build examples/boards/mps2_an385/uart_hello.rt --target embedded --no-gc
raytask build kernel.rt --target kernel
```

Board kits (`examples/boards/`) ship real `link.ld` + `startup.c` + GPIO/UART BSPs (STM32 Blue Pill, MPS2 AN385 / QEMU). Sibling board assets are copied into `dist/*_embedded/` automatically.

`--target embedded|kernel` lowers **optimized SSA** to C (`--optimize speed|size` recommended). Types / `[address:]` / consts still come from the AST; function bodies are SSA block CFGs (`goto bbN`).

Language extras for C-like work: `union`, `[packed]`, `[align: N]`, `[repr: "C"]`, `volatile T`, `sizeof(T)`, `offsetof(T, field)`, `unsafe { asm("nop"); }`.

See `docs/spec/05-systems.md` and `docs/ROADMAP.md`.

GC controls:

```bash
raytask run --gc              # default
raytask run --no-gc
raytask run --gc-stress       # collect on every allocation
```

## Language essentials

### Imports and entry

```
import bstd.io;
import bstd.collections;

void Main() {
    print("hi");
}
```

`import foo.bar;` loads a sibling `.rt` file (`foo/bar.rt` or `foo.bar.rt` relative to the entry file). Stdlib `bstd.*` is built into the VM.

Multi-file example: [`examples/modules/`](../examples/modules/) — `lib.rt` defines classes/functions, `main.rt` imports them and owns `Main`:

```
# lib.rt
export class Greeter { … }
export int Double(x: int) { return x * 2; }

# main.rt
import lib;
void Main() {
    print(new Greeter("RayTask").Hello());
    print(Double(21));
}
```

```bash
raytask run examples/modules/main.rt
```

### Types and variables

```
int n = 1;
var inferred = 2;
dyn anything = "ok";
string? maybe = null;
```

### Classes and structs

```
export class Point {
    int x;
    int y;

    new(x: int, y: int) {
        this.x = x;
        this.y = y;
    }

    int Manhattan(Point other) {
        return Abs(this.x - other.x) + Abs(this.y - other.y);
    }
}
```

Visibility: members are private by default; use `export` for public API.

### Async

```
import bstd.async;

async void Work() {
    await Task.Delay(100);
    print("done");
}

void Main() {
    Work();
}
```

### Memory

- Default heap objects are GC-managed in the VM (and in native C when GC is enabled).
- `owned` locals dispose on scope exit when a `Dispose` method exists.
- `using (...) { ... }` calls `Dispose`.
- `unsafe` enables pointer operations (`ptr<T>`, `*`, `&`).

### Preprocessor

```
#if DEBUG
    print("debug");
#endif

#if WINDOWS
    // ...
#elif LINUX
    // ...
#endif
```

## Product targets

| Command | Purpose |
|---------|---------|
| `--target bytecode` | VM module (`.rtbc`) |
| `--target native` | **True AOT**: SSA → C → TCC/gcc/clang (no RTBC VM); `--arch` / `--platform` for cross |
| `--target native-bin` | same True AOT as `native` (UEFI platform still uses payload packaging) |
| `--target app` | standalone app = runtime stub + embedded `.rtbc` |
| `--target wasm` | C + HTML shell (+ emcc/clang when available) |
| `--target web` | web bundle (wasm scaffold + bytecode assets) |
| `--target mobile` | Android + iOS scaffolds |
| `--target embedded` | freestanding / MCU-oriented C (SSA→C) |
| `--target kernel` | freestanding kernel C (SSA→C) |
| `--target efi` | UEFI `.efi` (mini-interp / payload packaging) |
| `--target raw` | flat `.bin` image (payload packaging) |

### True AOT (`native` / `native-bin`)

```bash
raytask build main.rt --target native --optimize speed
raytask build main.rt --target native-bin -o app.exe
raytask build main.rt --target native --platform linux --arch aarch64
raytask build main.rt --target native --link-builtin   # .o + built-in linker
```

Pipeline: `.rt` → SSA → C (Host runtime) → **TCC / gcc / clang** → PE/ELF.  
Cross: `clang`/`zig cc` `-target <triple>`. Arches: `x86_64`, `aarch64`, `arm`, `i686`.  
`--link-builtin` compiles to `.o` and links with RayTask’s ELF/COFF linker (best for freestanding).  
The binary does **not** embed an RTBC interpreter. Use `--target app` for stub + bytecode.

### Built-in linker

```bash
raytask link a.o b.o --platform linux --arch x86_64 -o app.elf --entry _start
raytask link main.rtbc --platform windows -o app.exe   # RTBC payload packager
```

The object path parses ELF64 / COFF, merges sections, resolves symbols, applies common relocs, and emits PE32+ or ELF64 executables.

### Payload / EFI packaging

```bash
raytask build main.rt --target efi
raytask build main.rt --target raw
raytask build main.rt --target app
raytask link main.rtbc --platform uefi -o main.efi
```

`efi` / `raw` / `.rtbc` link still package RTBC (or a UEFI mini-interpreter). That is separate from Host True AOT.

### Vendored TinyCC backend

RayTask vendors TinyCC in `tcc/` and compiles `libtcc` during the normal Cargo build. This gives the project an embedded C compiler/backend that can be used in two ways:

- `raytask build ... --target native` tries the embedded TCC backend before falling back to external host compilers.
- `raytask tcc ...` invokes the bundled TinyCC bridge directly for compiling or running C sources.

Example:

```bash
raytask tcc examples/tcc/hello.c -o hello.exe
raytask tcc -run examples/tcc/hello.c -- demo
```

- Host OS images package the RayTask runtime stub with embedded `.rtbc` when the stub is available.
- UEFI images include a freestanding C mini-interpreter (`dist/*_native/*_uefi.c`); `clang` can produce a full `.efi`, otherwise a PE32+ EFI shell with payload is written.

Embedded / kernel attributes:

```
[address: 0x40021000]
struct GpioRegisters {
    uint mode;
}

[interrupt: 0x80]
void SystemCallHandler() {
}

[export: "kmain"]
void KernelMain() {
}
```

## Packages

```bash
raytask install SomeLib
raytask uninstall SomeLib
raytask update
raytask search Some
raytask publish .
```

Local search paths: `.raytask/packages/`, `registry/`, `RAYTASK_REGISTRY`.  
Remote: `RAYTASK_REGISTRY_URL`.

## Server-side web stack

RayTask now ships a minimal server-side stack for registry-style applications.

### `bstd.web`

Use `HttpServer.ServeScript(host, port, script, staticDir)` to run a RayTask request handler.
Each HTTP request executes the target RayTask script inside a web context.

Available helpers:

- `Web.Method()`, `Web.Path()`, `Web.Query(name)`, `Web.Form(name)`
- `Web.Header(name)`, `Web.Cookie(name)`, `Web.Body()`
- `Web.SetStatus(code)`, `Web.SetHeader(name, value)`, `Web.SetCookie(...)`
- `Web.Text(...)`, `Web.Html(...)`, `Web.Json(...)`, `Web.File(...)`
- `Web.Render(templatePath, model)` for server-side HTML rendering

### `bstd.sqlite`

The SQLite bridge is intentionally small and works well for app-style code:

```raytask
import bstd.sqlite;

void Main() {
    var db = Sqlite.Open("app.db");
    db.Execute("CREATE TABLE IF NOT EXISTS notes (id INTEGER PRIMARY KEY, title TEXT);");
    db.Execute("INSERT INTO notes (title) VALUES ('hello');");
    var rows = db.Query("SELECT id, title FROM notes;");
    print(rows[0]["title"]);
    db.Close();
}
```

### Templates

`Template.Render(path, model)` and `Web.Render(path, model)` support:

- escaped variables: `{{title}}`
- raw variables: `{{{content}}}`
- conditionals: `{{#if notice}}...{{/if}}`
- loops: `{{#each items}}...{{/each}}`

### Example app: `apps/registry/`

The repository contains a complete RayTask web application:

- `apps/registry/main.rt` starts the HTTP server
- `apps/registry/src/app.rt` contains routes, auth/session logic, moderation workflow, and API handlers
- `packages/RTWebApp/src/lib.rt` contains the reusable web-platform helper layer written fully in RayTask
- `apps/registry/templates/` contains layout templates
- `apps/registry/static/` contains the CSS theme

`RTWebApp` now also includes:

- `RTWebContext` for request context helpers
- `RTWebRouteMatch` for prefix-based router abstraction
- reusable `RTWebRenderPage(...)`, auth/session, audit, and publish-token helpers

Start it with:

```bash
cargo run -- run apps/registry/main.rt
```

Useful env vars:

```bash
RAYTASK_REGISTRY_ADMIN_USER=admin
RAYTASK_REGISTRY_ADMIN_PASS=change-me
RAYTASK_REGISTRY_PUBLISH_TOKEN=dev-token
```

## Testing and docs

```bash
raytask test                  # runs [test] functions
raytask doc                   # markdown from /// into docs/api/
```

```
[test]
void TestAdd() {
    assertEq(1 + 1, 2);
}
```

## FFI

```
[DllImport: "mylib"]
[include: "mylib.h"]
int foreign_add(a: int, b: int);

[c: "
int add(int a, int b) { return a + b; }
"]
```

`[repr: "C"]` structs in FFI signatures are passed/returned **by value** (C ABI): small aggregates (1/2/4/8 bytes) in a register; larger via pointer-to-copy / sret. C codegen emits `extern Point f(...)` rather than `Point*`.

See `examples/ffi_demo.rt`, `examples/ffi_header/main.rt`, `examples/ffi_embed.rt`.

### C headers without gcc

RayTask can parse a **subset of C headers** and bind them to an existing shared library (`.dll` / `.so` / `.dylib`). No `gcc`/`clang` required:

```
[DllImport: "mylib.dll"]
[bind: "mylib.h"]

void Main() {
    my_api(1, 2);
}
```

```bash
raytask bind mylib.h --lib mylib.dll        # print / write RayTask decls
raytask run examples/ffi_header/main.rt     # Windows demo
```

Embedded `[c: "..."]` still needs a host C compiler if you want to *compile* C source. Prefer shipping a prebuilt library + `.h` for a pure-RayTask workflow.

## switch / case

RayTask supports C-style `switch` with several extensions.

### Basic

```rt
switch (x) {
    case 1:
        print_ln("one");
        break;
    case 2:
        print_ln("two");
        break;
    default:
        print_ln("other");
}
```

### Multiple patterns per case (`|`)

```rt
switch (code) {
    case 200 | 201 | 204:
        print_ln("Success");
        break;
    case 400 | 401 | 403 | 404:
        print_ln("Client error");
        break;
    default:
        print_ln("Unknown");
}
```

### Range patterns (`lo..hi`)

Inclusive range — `lo` and `hi` both match.

```rt
switch (score) {
    case 90..100:
        print_ln("A");
        break;
    case 80..89:
        print_ln("B");
        break;
    default:
        print_ln("Below B");
}
```

Ranges can be combined with `|`:

```rt
case 1..5 | 10 | 20..25:
    ...
```

### Guard expression (`when`)

```rt
switch (n) {
    case n when n % 15 == 0:
        print_ln("FizzBuzz");
        break;
    case n when n % 3 == 0:
        print_ln("Fizz");
        break;
    case n when n % 5 == 0:
        print_ln("Buzz");
        break;
    default:
        print_ln(n.ToString());
}
```

### Value binding

Bind the matched value to a name within the arm:

```rt
switch (getValue()) {
    case v when v > 0:
        print_ln("Positive: " + v.ToString());
        break;
    default:
        print_ln("Non-positive");
}
```

---

## Standard Library Extensions

### Math

| Method | Description |
|---|---|
| `Math.Clamp(v, lo, hi)` | Clamp value to [lo, hi] |
| `Math.Lerp(a, b, t)` | Linear interpolation |
| `Math.Sign(x)` | -1, 0, or 1 |
| `Math.Truncate(x)` | Integer part |
| `Math.IsNaN(x)` | Is Not-a-Number |
| `Math.IsInfinity(x)` | Is infinity |
| `Math.Log2(x)` / `Math.Log10(x)` | Logarithms |
| `Math.Asin/Acos/Atan(x)` | Inverse trig |
| `Math.Atan2(y, x)` | Angle of vector |
| `Math.Sinh/Cosh/Tanh(x)` | Hyperbolic |
| `Math.Cbrt(x)` | Cube root |
| `Math.Hypot(a, b)` | `sqrt(a²+b²)` |
| `Math.Tau` | 2π |

### String

Instance methods (called on a string value):

| Method | Description |
|---|---|
| `.Reverse()` | Reverse character order |
| `.Repeat(n)` | Repeat string n times |
| `.PadLeft(n)` / `.PadLeft(n, ch)` | Left-pad to width |
| `.PadRight(n)` / `.PadRight(n, ch)` | Right-pad to width |
| `.Chars()` | Returns `char[]` |
| `.Lines()` | Split by newlines |
| `.ParseInt()` | Parse to int (null on fail) |
| `.ParseFloat()` | Parse to float (null on fail) |
| `.IsEmpty()` | `true` if length == 0 |
| `.IsWhitespace()` | `true` if all whitespace |
| `.Count(sub)` | Count substring occurrences |
| `.Remove(idx, count)` | Remove characters |
| `.Insert(idx, str)` | Insert string at index |

Static methods:

```rt
String.Join(", ", myList)             // join list to string
String.Format("{0} = {1}", key, val)  // format template
String.IsNullOrEmpty(s)               // null-safe empty check
String.IsNullOrWhiteSpace(s)          // null-safe whitespace check
```

### List / Array

Instance methods:

| Method | Description |
|---|---|
| `.Sort()` / `.SortAsc()` | Sort ascending (returns new list) |
| `.SortDesc()` | Sort descending |
| `.Reverse()` | Reverse order |
| `.Distinct()` | Remove duplicates |
| `.Take(n)` | First n elements |
| `.Skip(n)` | Skip first n elements |
| `.Flatten()` | Flatten one level of nested lists |
| `.Zip(other)` | Pair elements with another list |
| `.Chunk(size)` | Split into chunks of `size` |
| `.IndexOf(value)` | First index of value (-1 if not found) |
| `.Count()` | Number of elements |
| `.Copy()` | Shallow copy |

Static methods:

```rt
List.Fill(value, count)   // [value, value, ...]
List.Range(count)         // [0, 1, 2, ..., count-1]
List.Range(start, count)  // [start, start+1, ...]
```

### Convert

All methods are static on `Convert`:

```rt
Convert.ToInt("42")          // 42
Convert.ToFloat("3.14")      // 3.14
Convert.ToBool(0)            // false
Convert.ToString(42)         // "42"
Convert.ToHex(255)           // "FF"
Convert.FromHex("FF")        // 255
Convert.ToBinary(10)         // "1010"
Convert.ToBytes("Hi")        // [72, 105]
Convert.FromBytes(bytes)     // "Hi"
Convert.ToBase64("Hello!")   // "SGVsbG8h"
Convert.FromBase64("SGVsbG8h") // "Hello!"
```

### Env

```rt
Env.OS               // "windows" | "linux" | "macos"
Env.CurrentDir       // current working directory path
Env.Home             // user home directory
Env.Args             // string[] of command-line arguments

Env.Get("NAME")      // get environment variable (null if not set)
Env.Set("NAME", "v") // set environment variable
Env.Has("NAME")      // bool — variable is set
```

---

## Editor support

VS Code / Cursor extension: [`editors/vscode`](../editors/vscode) — highlighting, check-on-save, snippets, and debugging via `raytask dap`.

### Debug symbols

```bash
raytask build app.rt -g        # emit app.rtbc + app.rtdbg
raytask symbols app.rt         # symbols only
```

`.rtdbg` is JSON: chunks, line maps, local live ranges, globals, classes. The debugger loads a sidecar next to `.rtbc` automatically.

## Further reading

- [SPEC.md](SPEC.md) — language and `bstd` reference
- [stdlib/README.md](../stdlib/README.md) — module status table
- [ru/docs/GUIDE.md](../ru/docs/GUIDE.md) — this guide in Russian
- [editors/vscode/README.md](../editors/vscode/README.md) — IDE extension
