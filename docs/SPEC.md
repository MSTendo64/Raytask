# RayTask Language Specification

Short reference for the language and standard library. See also the [user guide](GUIDE.md).

## Language

| Item | Detail |
|------|--------|
| File extension | `.rt` |
| Entry point | `void Main()` |
| Imports | `import bstd.io;` |
| Visibility | `export` for public API |
| Parameters | `name: type` |
| Typing | `dyn` (dynamic), `var` (inferred) |
| Memory | GC / `stack` / `owned` / `unsafe` |
| CLI GC flags | `raytask run --gc` (default) / `--no-gc` / `--gc-stress` |
| Runtime GC | `Gc.Collect()`, `Gc.Stats()`, `gc()`, finalizers `~new` |
| Closures | lambdas capture locals |
| Generics | compile-time monomorphization (`Id__int`, `Box__string`) |

## Product targets (`--target`)

| Target | Result |
|--------|--------|
| `bytecode` | `.rtbc` for the VM |
| `native` | C + gcc/clang; runtime with **async (`RtTask` / `await`) and GC** |
| `app` | standalone executable (stub + bytecode) |
| `wasm` | C + HTML/JS shell (+ emcc/clang when available) |
| `web` | web bundle: wasm scaffold + `embedded.rtbc` / `rtbc.js` |
| `mobile` | Android + iOS scaffolds with bytecode |
| `embedded` | freestanding C + `link.ld` |
| `kernel` | freestanding, GC off, `[export:"kmain"]`, `[interrupt:]` |
| `native-bin` | NativeCodeGen + Linker → PE/ELF/Mach-O (`--platform windows\|linux\|macos`) |
| `efi` | UEFI PE32+ (`.efi`) with freestanding mini-interpreter |
| `raw` | flat binary (`.text` + RTBC payload) |

Also: `raytask link program.rtbc --platform windows|linux|macos|uefi|raw -o out`.

Attributes: `[address: 0x…]` (MMIO), `[interrupt: N]`, `[no_gc]`, `[export:]`, `[target: wasm]`.

## Packages / registry

- Manifest: `project.rtp`
- Commands: `raytask install` / `uninstall` / `update` / `search` / `publish`
- Local: `registry/`, `RAYTASK_REGISTRY`
- Remote: `RAYTASK_REGISTRY_URL` → `GET /index.json`, `GET`/`POST /packages/{name}/{version}`

## Standard library (`bstd`)

| Namespace | Description |
|-----------|-------------|
| `bstd.io` | Console: `print`, `write`, `readLine`, `readKey` |
| `bstd.fs` | `File`, `Directory`, `FileInfo` |
| `bstd.net` | HTTP / TCP / UDP (sync in the current VM) |
| `bstd.async` | `Task.Delay` / `Task.Run` |
| `bstd.string` | String methods, `string.Join`, `StringBuilder` |
| `bstd.regex` | `regex(pattern)` |
| `bstd.json` | `Json.Parse` / `Stringify` |
| `bstd.yml` | `Yaml.Parse` / `Serialize` |
| `bstd.collections` | `List`, `Dictionary`, `Set`, `Queue`, `Stack` |
| `bstd.math` | `Math`, `Random` |
| `bstd.time` | `DateTime` |
| `bstd.crypto` | `Hash` (SHA256 / SHA1 / MD5) |
| `bstd.unsafe` | `malloc` / `free` / `sizeof` |
| FFI attrs | `[DllImport:]` / `[link:]` / `[include:]` / `[c:]` / `[abi:]` / `[export:]` |
| `bstd.result` | `Ok` / `Error` |
| `bstd.test` | `assert` / `assertEq` |
| `bstd.logging` | `Logger` |

API stubs: `stdlib/bstd/*.rt`. Implementations: `src/stdlib/`.
