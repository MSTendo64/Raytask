use raytask::codegen_c::{CodegenOptions, RuntimeProfile};
use raytask::{check_source, run_source, transpile_c_with};

#[test]
fn chapter1_language_model_conformance() {
    let src = r#"
        class Counter {
            static int Seed = 7;
        }

        struct Point {
            int X;
        }

        void Main() {
            Counter c = null;
            assert(IsNull(c));
            assertEq(Counter.Seed, 7);
        }
    "#;
    let report = check_source(src).unwrap();
    assert!(report.ok(), "{}", report.format_all());
    run_source(src).unwrap();
}

#[test]
fn chapter2_type_system_conformance() {
    let src = r#"
        interface IFoo {
            int Get();
        }

        class Box<T> where T: IFoo {
            T Value;
        }

        class Foo : IFoo {
            int Get() { return 1; }
        }

        void Main() {
            Box<Foo> ok = new Box<Foo>();
            assert(ok != null);
        }
    "#;
    let report = check_source(src).unwrap();
    assert!(report.ok(), "{}", report.format_all());
}

#[test]
fn chapter3_async_surface_conformance() {
    let src = r#"
        async int Slow() {
            await Task.Delay(1);
            return 5;
        }

        async void Main() {
            var g = TaskGroup.New();
            g.Run(Slow);
            var any = await g.WhenAny();
            assertEq(any, 5);
        }
    "#;
    let report = check_source(src).unwrap();
    assert!(report.ok(), "{}", report.format_all());
    run_source(src).unwrap();

    let native = transpile_c_with(
        src,
        CodegenOptions {
            profile: RuntimeProfile::Host,
            gc: true,
            freestanding: false,
        },
    )
    .unwrap();
    assert!(native.contains("TaskGroup_New"));
    assert!(native.contains("Task_WhenAny"));
    assert!(native.contains("CancellationTokenSource_New"));
}
