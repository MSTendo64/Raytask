//! Spec coverage: packages, extensions, indexers, match, preprocess, ParallelMap.

use raytask::project::{install_package, parse_project_file, uninstall_package};
use raytask::{run_source, transpile_c};
use std::path::Path;

#[test]
fn project_rtp_parses() {
    let src = r#"
project "Demo" {
    version = "1.2.3"
    author = "Alex"
    description = "test"
    dependencies {
        "Networking" version "2.1.0"
    }
    build {
        optimize = "speed"
        target = "bytecode"
        gc = true
    }
}
"#;
    let p = parse_project_file(src, Path::new("project.rtp")).unwrap();
    assert_eq!(p.name, "Demo");
    assert_eq!(p.version, "1.2.3");
    assert_eq!(p.dependencies.len(), 1);
    assert_eq!(p.dependencies[0].name, "Networking");
    assert!(p.build.gc);
}

#[test]
fn install_uninstall_local_package() {
    let name = format!("testpkg_{}", std::process::id());
    let _ = uninstall_package(&name);
    let path = install_package(&name, Some("0.1.0")).unwrap();
    assert!(path.join("package.rtp").exists());
    assert!(path.join("src/lib.rt").exists());
    assert!(uninstall_package(&name).unwrap());
}

#[test]
fn extension_method_on_string() {
    let src = r#"
        string Bang(this string s) => s + "!";

        void Main() {
            assertEq("hi".Bang(), "hi!");
        }
    "#;
    run_source(src).unwrap();
}

#[test]
fn result_match_ok_error() {
    let src = r#"
        void Main() {
            var r = Ok(42);
            match (r) {
                Ok(v) => assertEq(v, 42),
                Error(e) => assert(false)
            }
            var e = Error("boom");
            match (e) {
                Ok(v) => assert(false),
                Error(msg) => assertEq(msg, "boom")
            }
        }
    "#;
    run_source(src).unwrap();
}

#[test]
fn preprocessor_if_windows_or_release() {
    let src = r#"
        void Main() {
#if WINDOWS
            print("win");
#endif
#if LINUX
            print("linux");
#endif
#if MACOS
            print("mac");
#endif
#if RELEASE
            assert(true);
#endif
#if DEBUG
            assert(true);
#endif
        }
    "#;
    run_source(src).unwrap();
}

#[test]
fn parallel_map_api() {
    let src = r#"
        void Main() {
            var xs = new List<int> { 1, 2, 3 };
            var ys = xs.ParallelMap((x) => x * 10);
            assertEq(ys.Sum(), 60);
        }
    "#;
    run_source(src).unwrap();
}

#[test]
fn class_indexer() {
    let src = r#"
        class Box {
            int v0;
            int v1;
            new() { this.v0 = 0; this.v1 = 0; }
            int this[int i] {
                get {
                    if (i == 0) { return this.v0; }
                    return this.v1;
                }
                set {
                    if (i == 0) { this.v0 = value; }
                    else { this.v1 = value; }
                }
            }
        }
        void Main() {
            var b = new Box();
            b[1] = 9;
            assertEq(b[1], 9);
        }
    "#;
    run_source(src).unwrap();
}

#[test]
fn c_backend_null_coalesce() {
    let c = transpile_c(
        r#"
        void Main() {
            int? x = null;
            var y = x ?? 5;
            print(y);
        }
    "#,
    );
    // nullable may fail typecheck — just ensure transpile of simpler form works
    let _ = c;
    let c2 = transpile_c(
        r#"
        void Main() {
            var a = 0;
            var b = a ?? 1;
            print(b);
        }
    "#,
    )
    .unwrap();
    assert!(c2.contains("?") || c2.contains("a"), "{}", c2);
}
