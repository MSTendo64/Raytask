# RayTask Product Roadmap — C-capable & Multifunctional

RayTask's north star: **one language** that covers managed apps *and* systems software, across platforms.

## Already strong

- Managed VM + `.rtbc`, async, OOP, generics, packages, web/registry, FFI, SSA optimizer
- Multi-target scaffolding: native C, wasm, web, mobile, embedded, kernel, efi, native-bin

## This milestone (landed)

- Language: `union`, `[packed]`/`[align]`/`[repr:"C"]`, `volatile`, `sizeof`/`offsetof`, `asm`
- Freestanding bump allocator (64 KiB arena) instead of null `malloc`
- Opaque struct/union tags in C header FFI
- **By-value C ABI** for `[repr: "C"]` structs in FFI (register if size 1/2/4/8, else pointer-to-copy / sret)
- **Board kits** — STM32F103 Blue Pill + MPS2 AN385 (QEMU) with real `link.ld`, `startup.c`, GPIO/UART BSPs
- **SSA → C** for `--target embedded|kernel` (shared pass manager; `--optimize speed|size`)
- **True AOT** — `--target native` / `native-bin` = SSA → C → TCC/gcc/clang (**no RTBC interpreter**); `--target app` keeps stub+bytecode
- **Built-in linker** — ELF64/COFF ingest, symbol resolve, relocs, PE/ELF emit; `raytask link *.o`
- **Multi-arch** — `--arch x86_64|aarch64|arm|i686` + `--platform`; cross via clang/zig `-target`
 - Domain stubs: `bstd.hal`, `bstd.bots`, `bstd.game`
- Spec chapter: `docs/spec/05-systems.md`
- Example: `examples/systems/blink.rt`, `examples/boards/`

## Next milestones

1. **C ABI v2** — full header field layouts / nested typedef structs from `.h`
2. **Richer SSA→C** — objects, floats, async parity on the Host AOT path
3. **Linker v2** — Mach-O emit, more reloc kinds, CRT/import libs for Host `--link-builtin`
4. **Product runtimes** — mobile host beyond scaffolds; game backend package

Managed defaults stay; systems features are opt-in via attributes and freestanding targets.
