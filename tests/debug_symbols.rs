use raytask::bytecode_format::{deserialize_module, serialize_module};
use raytask::compiler::Compiler;
use raytask::debug_symbols::{self, DebugSymbols};
use raytask::inspect_bytecode;
use raytask::resolve::resolve_program;
use std::path::Path;

#[test]
fn emit_and_reload_rtdbg() {
    let src = r#"
void Main() {
    var answer = 42;
    print(answer);
}
"#;
    let program = resolve_program(src, None).unwrap();
    let mut module = Compiler::new()
        .with_source("demo.rt")
        .compile(&program)
        .unwrap();
    debug_symbols::stamp_source(&mut module, Path::new("demo.rt"));

    let dir = std::env::temp_dir().join(format!("raytask-rtdbg-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let rtbc = dir.join("demo.rtbc");
    let rtdbg = dir.join("demo.rtdbg");

    // Release-style bytecode (stripped) + sidecar symbols
    let mut stripped = module.clone();
    debug_symbols::strip_module_debug(&mut stripped);
    std::fs::write(&rtbc, serialize_module(&stripped)).unwrap();

    let sym = DebugSymbols::from_module(&module, Path::new("demo.rt"), Some(&rtbc));
    sym.write_file(&rtdbg).unwrap();

    let bytes = std::fs::read(&rtbc).unwrap();
    let mut loaded = deserialize_module(&bytes).unwrap();
    assert!(loaded.chunks.iter().all(|c| c.local_debug.is_empty()));

    let reloaded = DebugSymbols::read_file(&rtdbg).unwrap();
    reloaded.apply_to_module(&mut loaded);
    let main = loaded.chunks.iter().find(|c| c.name == "Main").unwrap();
    assert!(
        main.local_debug.iter().any(|l| l.name == "answer"),
        "expected answer in {:?}",
        main.local_debug
    );
    assert_eq!(main.source.as_deref(), Some("demo.rt"));

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn inspect_rtbc_prints_human_readable_summary() {
    let src = r#"
void Main() {
    print("hi");
}
"#;
    let program = resolve_program(src, None).unwrap();
    let module = Compiler::new().with_source("demo.rt").compile(&program).unwrap();
    let bytes = serialize_module(&module);
    let text = inspect_bytecode(&bytes, true).unwrap();
    assert!(text.contains("RTBC"));
    assert!(text.contains("chunks:"));
    assert!(text.contains("Main"));
    assert!(text.contains("code:"));
}
