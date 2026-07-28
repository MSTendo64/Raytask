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
raytask build src/main.rt --target native
```

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
| `--target native` | C source / host binary |
| `--target app` | single executable with embedded runtime |
| `--target wasm` | WebAssembly scaffold |
| `--target web` | browser bundle |
| `--target mobile` | Android + iOS scaffolds |
| `--target embedded` | freestanding / MCU-oriented C |
| `--target kernel` | freestanding kernel-style C (no GC) |
| `--target native-bin` | NativeCodeGen + Linker (OS binary from bytecode) |
| `--target efi` | UEFI `.efi` application |
| `--target raw` | flat `.bin` image |

### Native binaries from bytecode

```bash
raytask build main.rt --target native-bin --platform windows
raytask build main.rt --target native-bin --platform linux
raytask build main.rt --target native-bin --platform macos
raytask build main.rt --target efi
raytask build main.rt --target raw
raytask build main.rt --target bytecode
raytask link main.rtbc --platform uefi -o main.efi
```

Pipeline: `.rt` → RTBC → **NativeCodeGen** (`ObjectFile`) → **Linker** (PE / ELF / Mach-O / EFI / raw).

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

See `examples/ffi_demo.rt`, `examples/ffi_embed.rt`.

## Further reading

- [SPEC.md](SPEC.md) — language and `bstd` reference
- [stdlib/README.md](../stdlib/README.md) — module status table
- [ru/docs/GUIDE.md](../ru/docs/GUIDE.md) — this guide in Russian
