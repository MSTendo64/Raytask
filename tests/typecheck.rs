//! Typechecker integration tests.

use raytask::check_source;

#[test]
fn accepts_hello() {
    let src = std::fs::read_to_string("examples/hello.rt").unwrap();
    let report = check_source(&src).unwrap();
    assert!(report.ok(), "{}", report.format_all());
}

#[test]
fn accepts_point_and_oop() {
    for path in ["examples/point.rt", "examples/oop.rt"] {
        let src = std::fs::read_to_string(path).unwrap();
        let report = check_source(&src).unwrap();
        assert!(report.ok(), "{}: {}", path, report.format_all());
    }
}

#[test]
fn rejects_bad_types() {
    let src = std::fs::read_to_string("examples/bad_types.rt").unwrap();
    let report = check_source(&src).unwrap();
    assert!(!report.ok());
    assert!(report.errors.len() >= 4);
}

#[test]
fn catches_return_mismatch() {
    let src = r#"
        int Foo() {
            return "nope";
        }
        void Main() {}
    "#;
    let report = check_source(src).unwrap();
    assert!(!report.ok());
    assert!(report.format_all().contains("return"));
}

#[test]
fn catches_undefined_name() {
    let src = r#"
        void Main() {
            print(missing);
        }
    "#;
    let report = check_source(src).unwrap();
    assert!(!report.ok());
    assert!(report.format_all().contains("undefined"));
}

#[test]
fn dyn_escapes_checking() {
    let src = r#"
        void Main() {
            dyn x = 1;
            x = "ok";
            x = true;
        }
    "#;
    let report = check_source(src).unwrap();
    assert!(report.ok(), "{}", report.format_all());
}
