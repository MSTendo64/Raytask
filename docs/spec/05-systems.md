# Systems Programming Surface

RayTask remains a **managed** language by default (GC, classes, async), but ships a **systems profile** for C-level work on freestanding targets.

## Goals

Write firmware, drivers, kernels, and FFI-heavy code with:

- deterministic layout (`[repr: "C"]`, `[packed]`, `[align: N]`)
- `union` types (native/C overlapping storage)
- `volatile` types for MMIO
- `sizeof(T)` / `offsetof(T, field)`
- `unsafe { asm(...); }` escapes with GCC-style operands / clobbers (C backend)
- freestanding bump-heap (`--target embedded|kernel --no-gc`)

Hosted VM still runs the same sources with best-effort semantics (sizeof for primitives, asm no-op, unions as independent fields).

### Inline assembly

```raytask
unsafe {
    asm("nop");
    asm volatile ("nop");
    // outputs : inputs : clobbers  (same idea as GCC extended asm)
    asm("addl %1, %0" : "=r"(sum) : "r"(a), "0"(b) : "cc");
    // RayTask sugar: {N} → %N
    asm("addl {1}, {0}", out sum, in a);
}
```

See `examples/systems/asm_demo.rt`.

## Targets

| Domain | How |
|--------|-----|
| MCU / drivers | `--target embedded`, `bstd.hal`, `[address:]` MMIO |
| Kernel / EFI | `--target kernel` / `efi` |
| Compilers / tools | CLI + SSA + TCC |
| Web / API | `bstd.web`, `bstd.net`, registry app |
| Mobile | `--target mobile` scaffolds |
| Games | `bstd.game` stubs + platform packages |
| Bots | `bstd.bots` + `Http.*` |

## Example

See `examples/systems/blink.rt` and **board kits** in `examples/boards/`:

| Board | UART / GPIO | Notes |
|-------|-------------|--------|
| `stm32f103_bluepill` | USART1 PA9/10, LED PC13 | real `link.ld` + OpenOCD |
| `mps2_an385` | CMSDK UART0 / GPIO0 | QEMU `-M mps2-an385` |

```bash
raytask build examples/boards/mps2_an385/uart_hello.rt --target embedded --no-gc
cd examples/boards/mps2_an385/dist/uart_hello_embedded && sh build.sh
qemu-system-arm -M mps2-an385 -kernel firmware.elf -serial stdio -nographic
```

`--target embedded` copies sibling `link.ld` / `startup.c` / `build.sh` from the board directory when present.

## FFI by-value (`[repr: "C"]`)

`[repr: "C"]` (also `[packed]` / `[align]`) structs used in FFI params/returns follow a Win64-ish C ABI:

- size ∈ {1, 2, 4, 8} → pass/return in an integer register
- larger → pass as pointer to a temporary copy; return via hidden sret buffer

C codegen emits by-value types in `extern` signatures (not `T*`). Hosted VM packs/unpacks object fields to the C layout.

## Roadmap (remaining to full C parity)

1. Full C header field layouts / nested typedef structs from `.h`
2. Richer SSA→C (objects, floats, async) on the Host AOT path
3. Preserve `asm` through SSA→C function bodies (today AST→C / non-SSA bodies)
4. Direct SSA → machine code (optional; Host AOT already uses C toolchain)
5. More MCU kits (RP2040, nRF52, …)
