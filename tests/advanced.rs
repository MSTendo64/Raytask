//! Closures with capture, generics monomorphization, C-backend parity.

use raytask::{compile_bytecode, run_source, transpile_c};

#[test]
fn closure_captures_local() {
    let src = r#"
        void Main() {
            int n = 10;
            var add = (x: int) => x + n;
            assertEq(add(5), 15);
        }
    "#;
    run_source(src).unwrap();
}

#[test]
fn nested_closure_captures() {
    let src = r#"
        void Main() {
            var make = (seed: int) => (x: int) => x + seed;
            var f = make(100);
            assertEq(f(7), 107);
        }
    "#;
    run_source(src).unwrap();
}

#[test]
fn linq_where_captures_threshold() {
    let src = r#"
        void Main() {
            var xs = new List<int> { 1, 2, 3, 4, 5 };
            int threshold = 2;
            var filtered = xs.Where((x) => x > threshold);
            assertEq(filtered.Count, 3);
        }
    "#;
    run_source(src).unwrap();
}

#[test]
fn monomorphize_generic_function() {
    let src = r#"
        T Id<T>(v: T) => v;

        void Main() {
            assertEq(Id<int>(42), 42);
            assertEq(Id<string>("hi"), "hi");
        }
    "#;
    let module = compile_bytecode(src).unwrap();
    let names: Vec<_> = module.globals.iter().cloned().collect();
    assert!(
        names.iter().any(|n| n.contains("Id__int")),
        "expected Id__int in globals, got {:?}",
        names
    );
    assert!(names.iter().any(|n| n.contains("Id__string")));
    run_source(src).unwrap();
}

#[test]
fn monomorphize_generic_class() {
    let src = r#"
        class Box<T> {
            T value;
            new(v: T) { this.value = v; }
            T Get() => this.value;
        }

        void Main() {
            var b = new Box<int>(7);
            assertEq(b.Get(), 7);
        }
    "#;
    let module = compile_bytecode(src).unwrap();
    assert!(
        module.classes.iter().any(|c| c.name == "Box__int"),
        "expected Box__int class"
    );
    run_source(src).unwrap();
}

#[test]
fn c_backend_emits_new_and_methods() {
    let src = r#"
        struct Point {
            int x;
            int y;
            new(x: int, y: int) {
                this.x = x;
                this.y = y;
            }
            int Manhattan(Point other) {
                var dx = this.x - other.x;
                if (dx < 0) { dx = -dx; }
                var dy = this.y - other.y;
                if (dy < 0) { dy = -dy; }
                return dx + dy;
            }
        }
        void Main() {
            var p1 = new Point(0, 0);
            var p2 = new Point(3, 4);
            var d = p1.Manhattan(p2);
            print(d);
        }
    "#;
    let c = transpile_c(src).unwrap();
    assert!(c.contains("Point_new("), "missing ctor: {}", &c[..c.len().min(500)]);
    assert!(c.contains("Point_Manhattan("), "missing method call");
    assert!(c.contains("this->x"), "expected pointer member access");
}
