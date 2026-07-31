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
fn embed_c_struct_by_value_when_compiler_available() {
    let src = r#"
        [repr: "C"]
        struct Point {
            int x;
            int y;
            new(x: int, y: int) {
                this.x = x;
                this.y = y;
            }
        }

        [c: "
        typedef struct { int x; int y; } Point;
        Point ray_add_points(Point a, Point b) {
            Point r;
            r.x = a.x + b.x;
            r.y = a.y + b.y;
            return r;
        }
        "]
        [link: "raytask_embed_1"]
        Point ray_add_points(a: Point, b: Point);

        void Main() {
            var a = new Point(1, 2);
            var b = new Point(3, 4);
            var r = ray_add_points(a, b);
            assertEq(r.x, 4);
            assertEq(r.y, 6);
        }
    "#;
    match run_source(src) {
        Ok(()) => {}
        Err(e) => {
            let msg = format!("{e}");
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

#[test]
fn transpile_repr_c_ffi_by_value() {
    let src = r#"
        [repr: "C"]
        struct Point {
            int x;
            int y;
        }
        [DllImport: "m"]
        Point make_point(x: int, y: int);
        void Main() {}
    "#;
    let c = raytask::transpile_c(src).unwrap();
    assert!(
        c.contains("extern Point make_point("),
        "expected by-value Point in extern: {c}"
    );
    assert!(
        !c.contains("extern Point* make_point"),
        "must not pass Point as pointer in FFI signature"
    );
}
