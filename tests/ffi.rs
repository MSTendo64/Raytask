//! FFI / DllImport tests.

use raytask::{check_source, run_source};

#[test]
#[cfg(windows)]
fn dllimport_get_tick_count() {
    let src = r#"
        [DllImport: "kernel32.dll"]
        uint GetTickCount();

        void Main() {
            var t = GetTickCount();
            assert(t > 0);
        }
    "#;
    let report = check_source(src).unwrap();
    assert!(report.ok(), "{}", report.format_all());
    run_source(src).unwrap();
}

#[test]
#[cfg(windows)]
fn link_attr_kernel32() {
    let src = r#"
        [link: "kernel32.dll"]
        uint GetTickCount();

        void Main() {
            var a = GetTickCount();
            var b = GetTickCount();
            assert(b >= a);
        }
    "#;
    run_source(src).unwrap();
}

#[test]
fn ffi_decl_typechecks() {
    let src = r#"
        [DllImport: "kernel32.dll"]
        uint GetTickCount();
        void Main() {}
    "#;
    let report = check_source(src).unwrap();
    assert!(report.ok(), "{}", report.format_all());
}

#[test]
fn embed_c_add_when_compiler_available() {
    let src = r#"
        [c: "
        int ray_add(int a, int b) { return a + b; }
        "]
        [link: "raytask_embed_1"]
        int ray_add(a: int, b: int);

        void Main() {
            assertEq(ray_add(40, 2), 42);
        }
    "#;
    match run_source(src) {
        Ok(()) => {}
        Err(e) => {
            let msg = format!("{e}");
            // Skip when no native C compiler can produce a loadable shared lib
            assert!(
                msg.contains("no working C compiler")
                    || msg.contains("failed to compile")
                    || msg.contains("failed to load library")
                    || msg.contains("LoadLibrary"),
                "unexpected error: {msg}"
            );
        }
    }
}

#[test]
fn transpile_emits_extern_and_include() {
    let src = r#"
        [include: "math.h"]
        [DllImport: "m"]
        double cos(x: double);

        void Main() {}
    "#;
    let c = raytask::transpile_c(src).unwrap();
    assert!(c.contains("#include \"math.h\"") || c.contains("#include <math.h>"));
    assert!(c.contains("extern") && c.contains("cos"));
}
