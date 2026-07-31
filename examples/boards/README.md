# RayTask board kits (BSP)

Real MCU board support packages with:

- accurate MMIO register maps (`[repr: "C"]` + `[address:]`)
- GPIO + UART drivers in RayTask
- board-specific `link.ld` + Cortex-M `startup.c`
- blink and UART hello examples

| Board | MCU | Run on hardware | Run in QEMU |
|-------|-----|-----------------|-------------|
| [`stm32f103_bluepill`](stm32f103_bluepill/) | STM32F103C8 | ST-Link / openocd | — |
| [`mps2_an385`](mps2_an385/) | Cortex-M3 (MPS2) | FPGA kit | `qemu-system-arm -M mps2-an385` |

## Build flow

```bash
# from a board example
raytask build examples/boards/stm32f103_bluepill/blink.rt --target embedded --no-gc

# artifacts land next to the source:
#   examples/boards/stm32f103_bluepill/dist/blink_embedded/
#     blink.c  link.ld  startup.c  build.sh

cd examples/boards/stm32f103_bluepill/dist/blink_embedded
sh build.sh   # needs arm-none-eabi-gcc
```

Hosted `raytask run` still works for typecheck/VM smoke tests (MMIO is a no-op on the VM).
