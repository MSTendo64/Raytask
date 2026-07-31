# STM32F103 Blue Pill BSP

Cortex-M3 MCU kit with **GPIO (PC13 LED)** and **USART1 (PA9/PA10)**.

| Item | Value |
|------|--------|
| Flash / RAM | `0x08000000` 64K / `0x20000000` 20K |
| LED | PC13, active-low |
| UART | USART1 @ 115200 8N1 (HSI 8 MHz) |
| ST-Link | `openocd.cfg` included |

## Build

```bash
raytask build examples/boards/stm32f103_bluepill/blink.rt --target embedded --no-gc
cd examples/boards/stm32f103_bluepill/dist/blink_embedded
sh build.sh   # requires arm-none-eabi-gcc
```

Flash (example):

```bash
openocd -f openocd.cfg -c "program firmware.elf verify reset exit"
```

## Examples

- `blink.rt` — toggle LED
- `uart_hello.rt` — print banner, echo RX→TX
- `bsp.rt` — RCC / GPIO / USART drivers (`MmioRead32` / `MmioWrite32`)
