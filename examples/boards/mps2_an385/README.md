# ARM MPS2 AN385 BSP (QEMU)

Cortex-M3 FPGA / QEMU machine with **CMSDK UART0** and **GPIO0**.

| Item | Value |
|------|--------|
| Code / RAM | `0x00000000` / `0x20000000` |
| UART0 | `0x40004000` (TX/RX via QEMU `-serial stdio`) |
| GPIO0 | `0x40010000` (pin 0 as LED) |
| QEMU | `qemu-system-arm -M mps2-an385` |

## Build & run

```bash
raytask build examples/boards/mps2_an385/uart_hello.rt --target embedded --no-gc
cd examples/boards/mps2_an385/dist/uart_hello_embedded
sh build.sh
qemu-system-arm -M mps2-an385 -kernel firmware.elf -serial stdio -nographic
```

## Examples

- `blink.rt` — toggle GPIO0 bit 0
- `uart_hello.rt` — banner + echo
- `bsp.rt` — CMSDK UART/GPIO via `MmioRead32` / `MmioWrite32`
