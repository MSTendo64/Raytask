//! Debugger metadata smoke tests.

use raytask::compiler::Compiler;
use raytask::resolve::resolve_program;

#[test]
fn local_debug_names_for_main() {
    let src = r#"
void Main() {
    var x = 10;
    var y = 32;
    print(x + y);
}
"#;
    let program = resolve_program(src, None).expect("parse");
    let module = Compiler::new()
        .with_source("test.rt")
        .compile(&program)
        .expect("compile");
    let main = module
        .chunks
        .iter()
        .find(|c| c.name == "Main")
        .expect("Main chunk");
    let names: Vec<_> = main.local_debug.iter().map(|l| l.name.as_str()).collect();
    assert!(names.contains(&"x"), "expected local x, got {:?}", names);
    assert!(names.contains(&"y"), "expected local y, got {:?}", names);
    assert!(main.source.as_deref() == Some("test.rt"));
}
