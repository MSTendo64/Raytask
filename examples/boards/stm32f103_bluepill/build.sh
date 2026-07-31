#!/bin/sh
# Build firmware ELF for STM32F103 Blue Pill (Cortex-M3).
set -e
DIR=$(CDPATH= cd -- "$(dirname "$0")" && pwd)
cd "$DIR"

APP=
for f in *.c; do
  case "$f" in
    startup.c) ;;
    *) APP=$f; break ;;
  esac
done
if [ -z "$APP" ]; then
  echo "no RayTask-generated .c found" >&2
  exit 1
fi

arm-none-eabi-gcc \
  -mcpu=cortex-m3 -mthumb \
  -ffreestanding -nostdlib -fno-builtin \
  -O2 -Wall \
  -T link.ld \
  -Wl,--gc-sections \
  -o firmware.elf \
  startup.c "$APP"

arm-none-eabi-objcopy -O binary firmware.elf firmware.bin
echo "built $DIR/firmware.elf"
