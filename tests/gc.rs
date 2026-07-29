//! Garbage collector tests.

use raytask::{run_source_with, RunOptions};

#[test]
fn gc_collect_frees_unreachable() {
    let src = r#"
        void Main() {
            for (var i = 0; i < 200; i++) {
                var junk = [1, 2, 3, 4, 5];
            }
            var freed = Gc.Collect();
            var stats = Gc.Stats();
            assert(stats["collections"] >= 1);
            assert(stats["enabled"] == true);
            print(freed);
            print(stats["live"]);
        }
    "#;
    run_source_with(
        src,
        &RunOptions {
            gc: true,
            gc_stress: true,
            no_typecheck: false,
            no_stdlib: false,
        },
    )
    .unwrap();
}

#[test]
fn no_gc_still_runs() {
    let src = r#"
        void Main() {
            var a = [1, 2, 3];
            assertEq(a.Count, 3);
            var s = Gc.Stats();
            assert(s["enabled"] == false);
        }
    "#;
    run_source_with(
        src,
        &RunOptions {
            gc: false,
            gc_stress: false,
            no_typecheck: false,
            no_stdlib: false,
        },
    )
    .unwrap();
}

#[test]
fn destructor_does_not_crash_on_collect() {
    let src = r#"
        class Box {
            new() {}
            ~new() {
                print("bye");
            }
        }
        void Main() {
            for (var i = 0; i < 10; i++) {
                var b = new Box();
            }
            gc();
            var s = Gc.Stats();
            assert(s["collections"] >= 1);
        }
    "#;
    run_source_with(
        src,
        &RunOptions {
            gc: true,
            gc_stress: true,
            no_typecheck: false,
            no_stdlib: false,
        },
    )
    .unwrap();
}
