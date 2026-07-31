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
