//! Systems language surface tests.

use raytask::{check_source, compile_bytecode_optimized, transpile_c_with, Optimize};
use raytask::codegen_c::{CodegenOptions, RuntimeProfile};

#[test]
fn sizeof_and_offsetof_typecheck() {
    let src = r#"
        [repr: "C"]
        struct Point {
            int x;
            int y;
        }
        void Main() {
            int a = sizeof(int);
            int b = offsetof(Point, y);
            assert(a > 0);
            assert(b >= 0);
        }
    "#;
    let report = check_source(src).unwrap();
    assert!(report.ok(), "{}", report.format_all());
    let module = compile_bytecode_optimized(src, true, Optimize::None).unwrap();
    raytask::vm::Vm::new(module).run().unwrap();
}

#[test]
fn union_and_packed_emit_c() {
    let src = r#"
        [packed]
        struct P {
            byte a;
            int b;
        }
        union U {
            int w;
            byte b0;
        }
        void Main() {
            int s = sizeof(P);
            assert(s > 0);
        }
    "#;
    let report = check_source(src).unwrap();
    assert!(report.ok(), "{}", report.format_all());
    let c = transpile_c_with(
        src,
        CodegenOptions {
            profile: RuntimeProfile::Embedded,
            gc: false,
            freestanding: true,
        },
    )
    .unwrap();
    assert!(c.contains("typedef union"), "expected C union, got:\n{c}");
    assert!(c.contains("#pragma pack"), "expected packed pragma");
    assert!(c.contains("rt_heap"), "expected freestanding arena heap");
}

#[test]
fn asm_requires_unsafe() {
    let src = r#"
        void Main() {
            asm("nop");
        }
    "#;
    let report = check_source(src).unwrap();
    assert!(!report.ok());
}

#[test]
fn asm_in_unsafe_ok() {
    let src = r#"
        void Main() {
            unsafe {
                asm("nop");
            }
        }
    "#;
    let report = check_source(src).unwrap();
    assert!(report.ok(), "{}", report.format_all());
}

#[test]
fn asm_extended_gcc_emits_operands() {
    use raytask::codegen_c::{CodegenOptions, RuntimeProfile};
    use raytask::transpile_c_with;

    let src = r#"
        void Main() {
            int a = 1;
            int b = 2;
            int sum = 0;
            unsafe {
                asm("addl %1, %0" : "=r"(sum) : "r"(a), "0"(b) : "cc");
            }
        }
    "#;
    let c = transpile_c_with(
        src,
        CodegenOptions {
            profile: RuntimeProfile::Embedded,
            gc: false,
            freestanding: true,
        },
    )
    .expect("transpile");
    assert!(
        c.contains("__asm__ volatile"),
        "expected asm volatile, got:\n{c}"
    );
    assert!(c.contains("\"=r\"(sum)") || c.contains("\"=r\"(sum)"), "missing out operand:\n{c}");
    assert!(c.contains("\"r\"(a)"), "missing in operand:\n{c}");
    assert!(c.contains("\"cc\""), "missing clobber:\n{c}");
}

#[test]
fn asm_sugar_rewrites_braces() {
    use raytask::codegen_c::{CodegenOptions, RuntimeProfile};
    use raytask::transpile_c_with;

    let src = r#"
        void Main() {
            int x = 0;
            int y = 1;
            unsafe {
                asm("movl {1}, {0}", out x, in y);
            }
        }
    "#;
    let c = transpile_c_with(
        src,
        CodegenOptions {
            profile: RuntimeProfile::Host,
            gc: true,
            freestanding: false,
        },
    )
    .expect("transpile");
    assert!(c.contains("%0") && c.contains("%1"), "expected %N placeholders:\n{c}");
    assert!(c.contains("\"=r\"(x)"), "expected out constraint:\n{c}");
    assert!(c.contains("\"r\"(y)"), "expected in constraint:\n{c}");
}
