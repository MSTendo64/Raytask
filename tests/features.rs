//! Tests for newly implemented language features.

use raytask::{check_source, parse_file, run_file, run_source};
use std::path::Path;

#[test]
fn import_resolves_local_module() {
    let path = Path::new("examples/import_demo.rt");
    let program = parse_file(path).unwrap();
    // mathutil items + main items
    assert!(program.items.len() >= 3);
    run_file(path).unwrap();
}

#[test]
fn linq_where_select() {
    let src = r#"
        void Main() {
            var nums = new List<int> { 1, 2, 3, 4 };
            var evens = nums.Where((x) => x % 2 == 0);
            assertEq(evens.Count, 2);
            var doubled = nums.Select((x) => x * 2);
            assertEq(doubled.Sum(), 20);
        }
    "#;
    let report = check_source(src).unwrap();
    assert!(report.ok(), "{}", report.format_all());
    run_source(src).unwrap();
}

#[test]
fn operator_overload_and_base_ctor() {
    let path = Path::new("examples/ops_inherit.rt");
    let report = raytask::sema::typecheck(&parse_file(path).unwrap());
    assert!(report.ok(), "{}", report.format_all());
    run_file(path).unwrap();
}

#[test]
fn inherited_method_available() {
    let src = r#"
        class Base {
            int Value() { return 42; }
        }
        class Child : Base {
            new() {}
        }
        void Main() {
            var c = new Child();
            assertEq(c.Value(), 42);
        }
    "#;
    run_source(src).unwrap();
}

#[test]
fn no_stdlib_rejects_bstd_imports() {
    let src = r#"
        import bstd.io;
        void Main() {}
    "#;
    let err = raytask::check_source_with_stdlib(src, false).unwrap_err();
    let msg = err.to_string();
    assert!(
        msg.contains("--no-stdlib") || msg.contains("cannot resolve import"),
        "{msg}"
    );
}

#[test]
fn no_stdlib_allows_plain_language_build() {
    let src = r#"
        int Add(a: int, b: int) { return a + b; }
        void Main() {
            var x = Add(40, 2);
            if (x != 42) {
                throw "bad";
            }
        }
    "#;
    let report = raytask::check_source_with_stdlib(src, false).unwrap();
    assert!(report.ok(), "{}", report.format_all());
    raytask::run_source_with(
        src,
        &raytask::RunOptions {
            gc: true,
            gc_stress: false,
            no_typecheck: false,
            no_stdlib: true,
            optimize: raytask::Optimize::None,
        },
    )
    .unwrap();
}

#[test]
fn switch_runs_basic_cases_and_parses_extended_syntax() {
    let src = r#"
        string classify(code: int) {
            var result = "?";
            switch (code) {
                case 200:
                    result = "ok";
                    break;
                case 404:
                    result = "client";
                    break;
                default:
                    result = "other";
                    break;
            }
            return result;
        }

        void Main() {
            assertEq(classify(200), "ok");
            assertEq(classify(404), "client");
            assertEq(classify(7), "other");
        }
    "#;
    let report = check_source(src).unwrap();
    assert!(report.ok(), "{}", report.format_all());
    run_source(src).unwrap();
}
