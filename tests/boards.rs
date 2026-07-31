//! Board BSP kits: UART/GPIO examples under `examples/boards/`.

use raytask::{compile_file, BuildOptions, Optimize, Target};
use std::fs;
use std::path::PathBuf;

fn embedded_opts() -> BuildOptions {
    BuildOptions {
        target: Target::Embedded,
        optimize: Optimize::None,
        gc: false,
        ..BuildOptions::default()
    }
}

#[test]
fn stm32_embedded_copies_board_link_ld() {
    let out = compile_file(
        "examples/boards/stm32f103_bluepill/blink.rt",
        &embedded_opts(),
    )
    .expect("embedded build");
    let c = PathBuf::from(&out);
    assert!(c.exists(), "missing {out}");
    let dir = c.parent().unwrap();
    let link = dir.join("link.ld");
    let startup = dir.join("startup.c");
    let body = fs::read_to_string(&link).unwrap();
    assert!(
        body.contains("0x08000000") && body.contains("Reset_Handler"),
        "expected STM32 board link.ld, got:\n{body}"
    );
    assert!(startup.is_file(), "startup.c should be copied from board");
    let csrc = fs::read_to_string(&c).unwrap();
    assert!(csrc.contains("MmioWrite32"), "HAL helpers missing");
    assert!(csrc.contains("LedInit"), "BSP LedInit missing");
    assert!(
        csrc.contains("0x40011000") || csrc.contains("1073811456"),
        "GPIOC base missing"
    );
    assert!(csrc.contains("memset") && csrc.contains("MmioRead32"));
}

#[test]
fn mps2_embedded_uses_qemu_memory_map() {
    let out = compile_file(
        "examples/boards/mps2_an385/uart_hello.rt",
        &embedded_opts(),
    )
    .expect("embedded build");
    let dir = PathBuf::from(&out).parent().unwrap().to_path_buf();
    let body = fs::read_to_string(dir.join("link.ld")).unwrap();
    assert!(
        body.contains("ORIGIN = 0x00000000"),
        "expected MPS2 flash at 0x0, got:\n{body}"
    );
    assert!(dir.join("startup.c").is_file());
    let csrc = fs::read_to_string(&out).unwrap();
    assert!(csrc.contains("UartInit") || csrc.contains("UartWriteHello"));
    // UART0 base 0x40004000 as hex define or decimal const
    assert!(
        csrc.contains("0x40004000")
            || csrc.contains("1073758208")
            || csrc.contains("Uart0Regs_ADDR")
    );
}
