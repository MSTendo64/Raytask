//! C header → FFI binding tests.

use raytask::c_header::{parse_header_source, prototypes_to_raytask};
use raytask::ffi::FfiType;
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
fn parse_macro_prefixed_prototypes_and_structs() {
    let src = r#"
        #define BGFX_C_API
        typedef struct bgfx_init_s {
            uint16_t vendorId;
            void* nwh;
        } bgfx_init_t;
        BGFX_C_API bool bgfx_init(const bgfx_init_t* _init);
        BGFX_C_API void bgfx_shutdown(void);
    "#;
    let h = parse_header_source(src, Path::new(".")).unwrap();
    assert!(
        h.structs.iter().any(|s| s.name == "bgfx_init_t"),
        "structs={:?}",
        h.structs.iter().map(|s| &s.name).collect::<Vec<_>>()
    );
    let names: Vec<_> = h.prototypes.iter().map(|p| p.name.as_str()).collect();
    assert!(names.contains(&"bgfx_init"), "{:?}", names);
    assert!(names.contains(&"bgfx_shutdown"), "{:?}", names);
    let init = h.prototypes.iter().find(|p| p.name == "bgfx_init").unwrap();
    assert!(
        matches!(init.params.first(), Some(FfiType::StructPtr(s)) if s.name == "bgfx_init_t"),
        "param={:?}",
        init.params
    );
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
fn parse_fn_ptr_fields_with_pointer_return() {
    let src = r#"
        typedef struct bgfx_vertex_layout_s { uint16_t stride; } bgfx_vertex_layout_t;
        typedef struct bgfx_interface_vtbl {
            bgfx_vertex_layout_t* (*vertex_layout_begin)(bgfx_vertex_layout_t* _this);
            uint16_t (*vertex_layout_get_offset)(const bgfx_vertex_layout_t* _this);
            uint16_t (*vertex_layout_get_stride)(const bgfx_vertex_layout_t* _this);
        } bgfx_interface_vtbl_t;
    "#;
    let h = parse_header_source(src, Path::new(".")).unwrap();
    let s = h
        .structs
        .iter()
        .find(|s| s.name == "bgfx_interface_vtbl_t")
        .expect("vtbl");
    let names: Vec<_> = s
        .members
        .iter()
        .filter_map(|m| match m {
            raytask::ast::Member::Field(f) => Some(f.name.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(
        names,
        [
            "vertex_layout_begin",
            "vertex_layout_get_offset",
            "vertex_layout_get_stride"
        ]
    );
}

#[test]
fn parse_real_bgfx_header_smoke() {
    let path = Path::new(r"C:\Users\mstendo\clfw\include\bgfx\c99\bgfx.h");
    if !path.is_file() {
        return;
    }
    let h = raytask::c_header::parse_header_file(path).unwrap();
    assert!(h.prototypes.iter().any(|p| p.name == "bgfx_init"));
    assert!(h.structs.iter().any(|s| s.name == "bgfx_init_t"));
    if let Some(s) = h.structs.iter().find(|s| s.name.contains("interface_vtbl")) {
        let mut names = std::collections::HashMap::new();
        for m in &s.members {
            if let raytask::ast::Member::Field(f) = m {
                *names.entry(f.name.clone()).or_insert(0usize) += 1;
            }
        }
        let dups: Vec<_> = names.into_iter().filter(|(_, c)| *c > 1).collect();
        assert!(
            dups.is_empty(),
            "duplicate fields in {}: {:?}",
            s.name,
            dups
        );
    }
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
