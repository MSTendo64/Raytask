//! C header → FFI binding tests.

use raytask::c_header::{parse_header_source, prototypes_to_raytask};
use std::path::Path;

#[test]
fn parse_simple_prototypes() {
    let src = r#"
        typedef unsigned int DWORD;
        DWORD GetTickCount(void);
        int add(int a, int b);
        const char* name(void);
        void* alloc(size_t n);
    "#;
    let h = parse_header_source(src, Path::new(".")).unwrap();
    let names: Vec<_> = h.prototypes.iter().map(|p| p.name.as_str()).collect();
    assert!(names.contains(&"GetTickCount"), "{:?}", names);
    assert!(names.contains(&"add"), "{:?}", names);
    assert!(names.contains(&"name"), "{:?}", names);
    assert!(names.contains(&"alloc"), "{:?}", names);

    let add = h.prototypes.iter().find(|p| p.name == "add").unwrap();
    assert_eq!(add.params.len(), 2);
}

#[test]
fn emit_raytask_decls() {
    let src = "int foo(int x);\n";
    let h = parse_header_source(src, Path::new(".")).unwrap();
    let text = prototypes_to_raytask("mylib.dll", &h.prototypes);
    assert!(text.contains("[DllImport: \"mylib.dll\"]"));
    assert!(text.contains("int foo("));
}

#[test]
#[cfg(windows)]
fn run_header_bind_example() {
    use raytask::run_file_with;
    use raytask::RunOptions;
    let path = Path::new("examples/ffi_header/main.rt");
    if !path.is_file() {
        return;
    }
    run_file_with(path, &RunOptions::default()).unwrap();
}
