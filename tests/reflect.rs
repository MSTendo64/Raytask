//! Runtime reflection: typeof, nameof, is, Type.* APIs.

use raytask::run_source_with;
use raytask::RunOptions;

#[test]
fn typeof_and_nameof() {
    let src = r#"
        class Point {
            int X;
            int Y;
        }
        void Main() {
            Type t = typeof(Point);
            print(t.Name);
            print(t.Kind);
            print(nameof(Point));
            print(nameof(t.Name));
        }
    "#;
    run_source_with(src, &RunOptions::default()).expect("reflect typeof");
}

#[test]
fn is_respects_inheritance() {
    let src = r#"
        class A {}
        class B : A {}
        void Main() {
            B b = new B();
            assert(b is B);
            assert(b is A);
            assert(!(b is string));
        }
    "#;
    run_source_with(src, &RunOptions::default()).expect("is inheritance");
}

#[test]
fn get_set_invoke() {
    let src = r#"
        class Counter {
            int N;
            int Twice() { return this.N * 2; }
        }
        void Main() {
            Counter c = new Counter();
            c.N = 3;
            assertEq(Type.GetField(c, "N"), 3);
            Type.SetField(c, "N", 10);
            assertEq(Type.GetField(c, "N"), 10);
            assertEq(Type.Invoke(c, "Twice"), 20);
            Type t = Type.Of(c);
            assertEq(t.Name, "Counter");
        }
    "#;
    run_source_with(src, &RunOptions::default()).expect("get/set/invoke");
}

#[test]
fn fields_and_methods_lists() {
    let src = r#"
        class Box {
            int Value;
            void Clear() { this.Value = 0; }
        }
        void Main() {
            Type t = typeof(Box);
            assert(t.Fields.Count >= 1);
            assert(t.Methods.Count >= 1);
        }
    "#;
    run_source_with(src, &RunOptions::default()).expect("fields/methods");
}
