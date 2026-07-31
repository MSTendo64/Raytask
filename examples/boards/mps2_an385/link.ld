/* ARM MPS2 AN385 (Cortex-M3) — matches QEMU -M mps2-an385 */
MEMORY
{
  FLASH (rx) : ORIGIN = 0x00000000, LENGTH = 4M
  RAM (rwx)  : ORIGIN = 0x20000000, LENGTH = 4M
}

ENTRY(Reset_Handler)

_estack = ORIGIN(RAM) + LENGTH(RAM);

SECTIONS
{
  .isr_vector : {
    KEEP(*(.isr_vector))
  } > FLASH

  .text : {
    *(.text*)
    *(.text.isr.*)
    *(.rodata*)
  } > FLASH

  _sidata = LOADADDR(.data);

  .data : {
    _sdata = .;
    *(.data*)
    _edata = .;
  } > RAM AT> FLASH

  .bss : {
    _sbss = .;
    *(.bss*)
    *(COMMON)
    _ebss = .;
  } > RAM
}
