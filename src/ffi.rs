//! Foreign Function Interface: DllImport / [link] / [include] / embedded C.

use crate::ast::{Attribute, Expr, FunctionDecl, TypeRef};
use crate::error::{RuntimeError, RuntimeResult};
use crate::value::Value;
use libloading::Library;
use std::collections::HashMap;
use std::ffi::{CStr, CString};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FfiAbi {
    Cdecl,
    Stdcall,
    System,
}

impl FfiAbi {
    pub fn parse(s: &str) -> Self {
        match s.to_ascii_lowercase().as_str() {
            "stdcall" => Self::Stdcall,
            "system" => Self::System,
            _ => Self::Cdecl,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FfiFieldLayout {
    pub name: String,
    pub offset: usize,
    pub ty: FfiType,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FfiStructLayout {
    pub name: String,
    pub size: usize,
    pub align: usize,
    pub fields: Vec<FfiFieldLayout>,
    pub packed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FfiType {
    Void,
    Bool,
    I8,
    I16,
    I32,
    I64,
    U8,
    U16,
    U32,
    U64,
    F32,
    F64,
    Ptr,
    CString,
    /// POD aggregate with C layout — passed/returned by value ABI.
    Struct(FfiStructLayout),
    /// Pointer to a known POD struct (`ptr<T>`). Always passed as `T*` (packed temp).
    StructPtr(FfiStructLayout),
}

impl FfiType {
    pub fn tag(&self) -> u8 {
        match self {
            FfiType::Void => 0,
            FfiType::Bool => 1,
            FfiType::I8 => 2,
            FfiType::I16 => 3,
            FfiType::I32 => 4,
            FfiType::I64 => 5,
            FfiType::U8 => 6,
            FfiType::U16 => 7,
            FfiType::U32 => 8,
            FfiType::U64 => 9,
            FfiType::F32 => 10,
            FfiType::F64 => 11,
            FfiType::Ptr => 12,
            FfiType::CString => 13,
            FfiType::Struct(_) => 14,
            FfiType::StructPtr(_) => 15,
        }
    }

    pub fn from_type_ref(tr: &TypeRef) -> Self {
        Self::from_type_ref_with(tr, &std::collections::HashMap::new())
    }

    pub fn from_type_ref_with(
        tr: &TypeRef,
        layouts: &std::collections::HashMap<String, FfiStructLayout>,
    ) -> Self {
        if let Some(layout) = layouts.get(&tr.name) {
            return FfiType::Struct(layout.clone());
        }
        if tr.name == "ptr" || tr.name == "pointer" {
            // ptr<KnownStruct> → pack object and pass pointer (C `T*`).
            // Stored as Struct so call() packs Value::Object; large POD always
            // goes by pointer under Win64/SysV.
            if let Some(inner) = tr.args.first() {
                if let Some(layout) = layouts.get(&inner.name) {
                    return FfiType::StructPtr(layout.clone());
                }
            }
            return FfiType::Ptr;
        }
        match tr.name.as_str() {
            "void" => FfiType::Void,
            "bool" => FfiType::Bool,
            "byte" | "i8" | "sbyte" => FfiType::I8,
            "short" | "i16" => FfiType::I16,
            "int" | "i32" => FfiType::I32,
            "long" | "i64" => FfiType::I64,
            "ubyte" | "u8" => FfiType::U8,
            "ushort" | "u16" => FfiType::U16,
            "uint" | "u32" => FfiType::U32,
            "ulong" | "u64" => FfiType::U64,
            "float" | "f32" => FfiType::F32,
            "double" | "f64" => FfiType::F64,
            "string" | "str" => FfiType::CString,
            "char" => FfiType::I32,
            _ if tr.name.starts_with("ptr") => FfiType::Ptr,
            _ => FfiType::Ptr,
        }
    }

    pub fn is_float(&self) -> bool {
        matches!(self, FfiType::F32 | FfiType::F64)
    }

    pub fn is_struct(&self) -> bool {
        matches!(self, FfiType::Struct(_) | FfiType::StructPtr(_))
    }

    pub fn from_u8(v: u8) -> Self {
        match v {
            0 => FfiType::Void,
            1 => FfiType::Bool,
            2 => FfiType::I8,
            3 => FfiType::I16,
            4 => FfiType::I32,
            5 => FfiType::I64,
            6 => FfiType::U8,
            7 => FfiType::U16,
            8 => FfiType::U32,
            9 => FfiType::U64,
            10 => FfiType::F32,
            11 => FfiType::F64,
            12 => FfiType::Ptr,
            13 => FfiType::CString,
            _ => FfiType::Ptr,
        }
    }
}

#[derive(Debug, Clone)]
pub struct FfiFunction {
    pub name: String,
    pub library: String,
    pub symbol: String,
    pub abi: FfiAbi,
    pub params: Vec<FfiType>,
    pub ret: FfiType,
}

#[derive(Debug, Clone, Default)]
pub struct FfiModuleInfo {
    pub includes: Vec<String>,
    pub links: Vec<String>,
    pub embeds: Vec<FfiEmbed>,
}

#[derive(Debug, Clone)]
pub struct FfiEmbed {
    pub source: String,
    pub lib_name: String,
}

fn libraries() -> &'static Mutex<HashMap<String, Library>> {
    static LIBS: OnceLock<Mutex<HashMap<String, Library>>> = OnceLock::new();
    LIBS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn embed_libs() -> &'static Mutex<HashMap<String, PathBuf>> {
    static EMBEDS: OnceLock<Mutex<HashMap<String, PathBuf>>> = OnceLock::new();
    EMBEDS.get_or_init(|| Mutex::new(HashMap::new()))
}

pub fn attr_string(attr: &Attribute) -> Option<String> {
    match &attr.value {
        Some(Expr::String(s, _)) => Some(s.clone()),
        Some(Expr::Ident(s, _)) => Some(s.clone()),
        Some(Expr::Interpolated(parts, _)) => {
            let mut out = String::new();
            for p in parts {
                match p {
                    crate::ast::InterpPart::Literal(t) => out.push_str(t),
                    crate::ast::InterpPart::Expr(_) => return None,
                }
            }
            Some(out)
        }
        _ => None,
    }
}

pub fn is_ffi_attr(name: &str) -> bool {
    matches!(
        name.to_ascii_lowercase().as_str(),
        "include"
            | "link"
            | "dllimport"
            | "abi"
            | "export"
            | "c"
            | "cembed"
            | "embed"
            | "lib"
            | "symbol"
            | "entry"
            | "name"
            | "bind"
            | "cheader"
    )
}

pub fn is_ffi_import(f: &FunctionDecl) -> bool {
    !f.is_abstract && f.body.is_none()
}

pub fn ffi_from_function(
    f: &FunctionDecl,
    default_lib: Option<&str>,
    attrs: &[Attribute],
) -> Result<FfiFunction, String> {
    ffi_from_function_with(f, default_lib, attrs, &std::collections::HashMap::new())
}

pub fn ffi_from_function_with(
    f: &FunctionDecl,
    default_lib: Option<&str>,
    attrs: &[Attribute],
    layouts: &std::collections::HashMap<String, FfiStructLayout>,
) -> Result<FfiFunction, String> {
    let mut library = default_lib.map(|s| s.to_string());
    let mut symbol = f.name.clone();
    let mut abi = FfiAbi::System;

    for a in attrs.iter().chain(f.attributes.iter()) {
        let key = a.name.to_ascii_lowercase();
        match key.as_str() {
            "dllimport" | "link" | "lib" => {
                if let Some(s) = attr_string(a) {
                    library = Some(s);
                }
            }
            "abi" => {
                if let Some(s) = attr_string(a) {
                    abi = FfiAbi::parse(&s);
                }
            }
            "export" | "entry" | "symbol" | "name" => {
                if let Some(s) = attr_string(a) {
                    symbol = s;
                }
            }
            _ => {}
        }
    }

    let library = library.ok_or_else(|| {
        format!(
            "FFI function '{}': missing [DllImport]/[link:] library",
            f.name
        )
    })?;

    Ok(FfiFunction {
        name: f.name.clone(),
        library,
        symbol,
        abi,
        params: f
            .params
            .iter()
            .map(|p| FfiType::from_type_ref_with(&p.ty, layouts))
            .collect(),
        ret: FfiType::from_type_ref_with(&f.return_type, layouts),
    })
}

fn search_dirs() -> &'static Mutex<Vec<PathBuf>> {
    static DIRS: OnceLock<Mutex<Vec<PathBuf>>> = OnceLock::new();
    DIRS.get_or_init(|| Mutex::new(Vec::new()))
}

/// Extra directories to search for shared libraries (entry file dir, etc.).
pub fn set_library_search_paths(paths: &[PathBuf]) {
    if let Ok(mut g) = search_dirs().lock() {
        *g = paths.to_vec();
    }
}

pub fn resolve_library_path(name: &str) -> PathBuf {
    if let Ok(map) = embed_libs().lock() {
        if let Some(p) = map.get(name) {
            return p.clone();
        }
        for (k, p) in map.iter() {
            if k == name || p.file_stem().and_then(|s| s.to_str()) == Some(name) {
                return p.clone();
            }
        }
    }
    let direct = PathBuf::from(name);
    if direct.is_file() {
        return direct;
    }
    if let Ok(dirs) = search_dirs().lock() {
        for dir in dirs.iter() {
            let c = dir.join(name);
            if c.is_file() {
                return c;
            }
            // bare name → libname.so / name.dll
            #[cfg(windows)]
            {
                let dll = dir.join(format!("{name}.dll"));
                if dll.is_file() {
                    return dll;
                }
            }
            #[cfg(not(windows))]
            {
                let so = dir.join(format!("lib{name}.so"));
                if so.is_file() {
                    return so;
                }
                let dylib = dir.join(format!("lib{name}.dylib"));
                if dylib.is_file() {
                    return dylib;
                }
            }
        }
    }
    direct
}

fn load_library(name: &str) -> RuntimeResult<()> {
    let mut libs = libraries()
        .lock()
        .map_err(|_| RuntimeError::Message("ffi library lock poisoned".into()))?;
    if libs.contains_key(name) {
        return Ok(());
    }
    let path = resolve_library_path(name);
    let lib = unsafe { Library::new(&path) }.map_err(|e| {
        RuntimeError::Message(format!(
            "failed to load library '{}': {}",
            path.display(),
            e
        ))
    })?;
    libs.insert(name.to_string(), lib);
    Ok(())
}

pub fn compile_embed(embed: &FfiEmbed, work_dir: Option<&Path>) -> RuntimeResult<PathBuf> {
    {
        let map = embed_libs()
            .lock()
            .map_err(|_| RuntimeError::Message("ffi embed lock poisoned".into()))?;
        if let Some(p) = map.get(&embed.lib_name) {
            if p.exists() {
                return Ok(p.clone());
            }
        }
    }

    let dir = work_dir
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| std::env::temp_dir().join("raytask_ffi"));
    std::fs::create_dir_all(&dir)
        .map_err(|e| RuntimeError::Message(format!("cannot create ffi work dir: {}", e)))?;

    let c_path = dir.join(format!("{}.c", embed.lib_name));
    std::fs::write(&c_path, &embed.source)
        .map_err(|e| RuntimeError::Message(format!("cannot write embed C: {}", e)))?;

    let lib_path = if cfg!(windows) {
        dir.join(format!("{}.dll", embed.lib_name))
    } else if cfg!(target_os = "macos") {
        dir.join(format!("lib{}.dylib", embed.lib_name))
    } else {
        dir.join(format!("lib{}.so", embed.lib_name))
    };

    if !compile_shared(&c_path, &lib_path)? {
        return Err(RuntimeError::Message(format!(
            "failed to compile embedded C for '{}': no working C compiler (tcc/gcc/clang/cl)",
            embed.lib_name
        )));
    }

    embed_libs()
        .lock()
        .map_err(|_| RuntimeError::Message("ffi embed lock poisoned".into()))?
        .insert(embed.lib_name.clone(), lib_path.clone());

    Ok(lib_path)
}

fn compile_shared(c_path: &Path, lib_path: &Path) -> RuntimeResult<bool> {
    let c = c_path.to_string_lossy().to_string();
    let out = lib_path.to_string_lossy().to_string();

    for cc in ["gcc", "clang"] {
        let mut cmd = std::process::Command::new(cc);
        cmd.arg("-shared")
            .arg("-fPIC")
            .arg("-O2")
            .arg(&c)
            .arg("-o")
            .arg(&out);
        if let Ok(st) = cmd.status() {
            if st.success() && lib_path.exists() {
                return Ok(true);
            }
        }
    }

    if cfg!(windows) {
        let obj = lib_path.with_extension("obj");
        if let Ok(st) = std::process::Command::new("cl")
            .arg("/nologo")
            .arg("/LD")
            .arg("/O2")
            .arg(&c)
            .arg(format!("/Fe:{}", out))
            .arg(format!("/Fo:{}", obj.display()))
            .status()
        {
            if st.success() && lib_path.exists() {
                return Ok(true);
            }
        }
    }

    // Vendored TinyCC — works without an external host toolchain.
    {
        let mut link_libs = Vec::new();
        // Auto-link sibling DLLs in the C file's directory and cwd (e.g. bgfx.dll).
        for dir in [c_path.parent(), std::env::current_dir().ok().as_deref()]
            .into_iter()
            .flatten()
        {
            if let Ok(rd) = std::fs::read_dir(dir) {
                for ent in rd.flatten() {
                    let p = ent.path();
                    if p.extension().and_then(|e| e.to_str()) == Some("dll") {
                        if let Some(s) = p.to_str() {
                            if !link_libs.iter().any(|x: &String| x == s) {
                                link_libs.push(s.to_string());
                            }
                        }
                    }
                }
            }
        }
        match crate::tcc::compile_c_to_path(
            c_path,
            lib_path,
            crate::tcc::OutputKind::Dll,
            false,
            &link_libs,
        ) {
            Ok(()) if lib_path.exists() => return Ok(true),
            Ok(()) => {}
            Err(e) => {
                return Err(RuntimeError::Message(format!(
                    "failed to compile embedded C with TinyCC: {}",
                    e
                )));
            }
        }
    }

    // Note: do not use WSL gcc on Windows — it produces Linux ELF, not a LoadLibrary-compatible DLL.

    Ok(false)
}

pub fn prepare_module_ffi(info: &FfiModuleInfo, base_dir: Option<&Path>) -> RuntimeResult<()> {
    let mut dirs = Vec::new();
    if let Some(d) = base_dir {
        dirs.push(d.to_path_buf());
    }
    if let Ok(cwd) = std::env::current_dir() {
        dirs.push(cwd);
    }
    set_library_search_paths(&dirs);

    for embed in &info.embeds {
        let _ = compile_embed(embed, base_dir)?;
    }
    for link in &info.links {
        let path = Path::new(link);
        let resolved = if path.is_file() {
            path.to_path_buf()
        } else if let Some(d) = base_dir {
            let c = d.join(link);
            if c.is_file() {
                c
            } else {
                path.to_path_buf()
            }
        } else {
            path.to_path_buf()
        };
        if resolved.extension().and_then(|e| e.to_str()) == Some("c") {
            let stem = resolved
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("ffi_link")
                .to_string();
            let source = std::fs::read_to_string(&resolved).map_err(|e| {
                RuntimeError::Message(format!("cannot read '{}': {}", resolved.display(), e))
            })?;
            let embed = FfiEmbed {
                source,
                lib_name: stem.clone(),
            };
            let lib = compile_embed(&embed, base_dir)?;
            let mut map = embed_libs()
                .lock()
                .map_err(|_| RuntimeError::Message("ffi embed lock poisoned".into()))?;
            map.insert(link.clone(), lib.clone());
            map.insert(stem, lib);
        }
    }
    Ok(())
}

enum ArgSlot {
    I64(i64),
    U64(u64),
    F64(f64),
    F32(f32),
}

fn write_scalar(buf: &mut [u8], offset: usize, ty: &FfiType, v: &Value) -> RuntimeResult<()> {
    // Uninitialized object fields are Null — treat as zero for POD packing.
    let v = if matches!(v, Value::Null) {
        match ty {
            FfiType::Bool
            | FfiType::I8
            | FfiType::U8
            | FfiType::I16
            | FfiType::U16
            | FfiType::I32
            | FfiType::U32
            | FfiType::I64
            | FfiType::U64 => &Value::Int(0),
            FfiType::F32 | FfiType::F64 => &Value::Float(0.0),
            FfiType::Ptr | FfiType::CString | FfiType::StructPtr(_) => &Value::Null,
            _ => v,
        }
    } else {
        v
    };
    let end = match ty {
        FfiType::Bool | FfiType::I8 | FfiType::U8 => offset + 1,
        FfiType::I16 | FfiType::U16 => offset + 2,
        FfiType::I32 | FfiType::U32 | FfiType::F32 => offset + 4,
        FfiType::I64 | FfiType::U64 | FfiType::F64 | FfiType::Ptr | FfiType::CString => offset + 8,
        FfiType::Void => return Ok(()),
        FfiType::Struct(_) => {
            return Err(RuntimeError::Message(
                "nested struct field packing requires Struct path".into(),
            ));
        }
        FfiType::StructPtr(_) => {
            // Struct pointer fields are just pointers (8 bytes)
            offset + 8
        }
    };
    if end > buf.len() {
        return Err(RuntimeError::Message("struct buffer overflow".into()));
    }
    match ty {
        FfiType::Bool => buf[offset] = if v.is_truthy() { 1 } else { 0 },
        FfiType::I8 => buf[offset] = v.as_int()? as u8,
        FfiType::U8 => buf[offset] = v.as_int()? as u8,
        FfiType::I16 => buf[offset..offset + 2].copy_from_slice(&(v.as_int()? as i16).to_ne_bytes()),
        FfiType::U16 => buf[offset..offset + 2].copy_from_slice(&(v.as_int()? as u16).to_ne_bytes()),
        FfiType::I32 => buf[offset..offset + 4].copy_from_slice(&(v.as_int()? as i32).to_ne_bytes()),
        FfiType::U32 => buf[offset..offset + 4].copy_from_slice(&(v.as_int()? as u32).to_ne_bytes()),
        FfiType::I64 => buf[offset..offset + 8].copy_from_slice(&v.as_int()?.to_ne_bytes()),
        FfiType::U64 => {
            buf[offset..offset + 8].copy_from_slice(&(v.as_int()? as u64).to_ne_bytes())
        }
        FfiType::F32 => {
            buf[offset..offset + 4].copy_from_slice(&(v.as_float()? as f32).to_ne_bytes())
        }
        FfiType::F64 => buf[offset..offset + 8].copy_from_slice(&v.as_float()?.to_ne_bytes()),
        FfiType::Ptr | FfiType::StructPtr(_) => {
            let p = match v {
                Value::Ptr(p) => *p as u64,
                Value::Null => 0,
                Value::Int(n) => *n as u64,
                _ => {
                    return Err(RuntimeError::TypeError(format!(
                        "cannot pack {} as pointer field",
                        v.type_name()
                    )));
                }
            };
            buf[offset..offset + 8].copy_from_slice(&p.to_ne_bytes());
        }
        FfiType::CString => {
            // Store pointer only if already a Ptr; strings need external lifetime — use 0
            let p = match v {
                Value::Ptr(p) => *p as u64,
                Value::Null => 0,
                _ => 0,
            };
            buf[offset..offset + 8].copy_from_slice(&p.to_ne_bytes());
        }
        _ => {}
    }
    Ok(())
}

fn read_scalar(buf: &[u8], offset: usize, ty: &FfiType) -> RuntimeResult<Value> {
    Ok(match ty {
        FfiType::Void => Value::Null,
        FfiType::Bool => Value::Bool(buf.get(offset).copied().unwrap_or(0) != 0),
        FfiType::I8 => Value::Int(buf.get(offset).copied().unwrap_or(0) as i8 as i64),
        FfiType::U8 => Value::Int(buf.get(offset).copied().unwrap_or(0) as i64),
        FfiType::I16 => {
            let mut b = [0u8; 2];
            b.copy_from_slice(buf.get(offset..offset + 2).unwrap_or(&[0; 2]));
            Value::Int(i16::from_ne_bytes(b) as i64)
        }
        FfiType::U16 => {
            let mut b = [0u8; 2];
            b.copy_from_slice(buf.get(offset..offset + 2).unwrap_or(&[0; 2]));
            Value::Int(u16::from_ne_bytes(b) as i64)
        }
        FfiType::I32 => {
            let mut b = [0u8; 4];
            b.copy_from_slice(buf.get(offset..offset + 4).unwrap_or(&[0; 4]));
            Value::Int(i32::from_ne_bytes(b) as i64)
        }
        FfiType::U32 => {
            let mut b = [0u8; 4];
            b.copy_from_slice(buf.get(offset..offset + 4).unwrap_or(&[0; 4]));
            Value::Int(u32::from_ne_bytes(b) as i64)
        }
        FfiType::I64 => {
            let mut b = [0u8; 8];
            b.copy_from_slice(buf.get(offset..offset + 8).unwrap_or(&[0; 8]));
            Value::Int(i64::from_ne_bytes(b))
        }
        FfiType::U64 => {
            let mut b = [0u8; 8];
            b.copy_from_slice(buf.get(offset..offset + 8).unwrap_or(&[0; 8]));
            Value::UInt(u64::from_ne_bytes(b))
        }
        FfiType::F32 => {
            let mut b = [0u8; 4];
            b.copy_from_slice(buf.get(offset..offset + 4).unwrap_or(&[0; 4]));
            Value::Float(f32::from_ne_bytes(b) as f64)
        }
        FfiType::F64 => {
            let mut b = [0u8; 8];
            b.copy_from_slice(buf.get(offset..offset + 8).unwrap_or(&[0; 8]));
            Value::Float(f64::from_ne_bytes(b))
        }
        FfiType::Ptr => {
            let mut b = [0u8; 8];
            b.copy_from_slice(buf.get(offset..offset + 8).unwrap_or(&[0; 8]));
            let p = u64::from_ne_bytes(b);
            if p == 0 {
                Value::Null
            } else {
                Value::Ptr(p as usize)
            }
        }
        FfiType::CString => Value::Null,
        FfiType::Struct(_) => Value::Null,
        FfiType::StructPtr(_) => {
            let mut b = [0u8; 8];
            b.copy_from_slice(buf.get(offset..offset + 8).unwrap_or(&[0; 8]));
            let p = u64::from_ne_bytes(b);
            if p == 0 {
                Value::Null
            } else {
                Value::Ptr(p as usize)
            }
        }
    })
}

fn pack_struct(v: &Value, layout: &FfiStructLayout) -> RuntimeResult<Vec<u8>> {
    let mut buf = vec![0u8; layout.size];
    match v {
        Value::Object(o) => {
            let obj = o.borrow();
            for field in &layout.fields {
                let fv = obj.fields.get(&field.name).cloned().unwrap_or(Value::Null);
                match &field.ty {
                    FfiType::Struct(nested) => {
                        let nested_buf = pack_struct(&fv, nested)?;
                        let end = field.offset + nested.size;
                        if end > buf.len() {
                            return Err(RuntimeError::Message("nested struct overflow".into()));
                        }
                        buf[field.offset..end].copy_from_slice(&nested_buf);
                    }
                    other => write_scalar(&mut buf, field.offset, other, &fv)?,
                }
            }
        }
        Value::Null => {}
        _ => {
            return Err(RuntimeError::TypeError(format!(
                "expected object for struct '{}', got {}",
                layout.name,
                v.type_name()
            )));
        }
    }
    Ok(buf)
}

fn unpack_struct(buf: &[u8], layout: &FfiStructLayout) -> RuntimeResult<Value> {
    use crate::gc::alloc_object;
    use crate::value::ObjectInstance;
    use std::collections::HashMap;
    let mut fields = HashMap::new();
    for field in &layout.fields {
        let fv = match &field.ty {
            FfiType::Struct(nested) => {
                let end = (field.offset + nested.size).min(buf.len());
                let slice = if field.offset < buf.len() {
                    &buf[field.offset..end]
                } else {
                    &[]
                };
                let mut tmp = vec![0u8; nested.size];
                let n = slice.len().min(nested.size);
                tmp[..n].copy_from_slice(&slice[..n]);
                unpack_struct(&tmp, nested)?
            }
            other => read_scalar(buf, field.offset, other)?,
        };
        fields.insert(field.name.clone(), fv);
    }
    Ok(alloc_object(ObjectInstance {
        class_name: layout.name.clone(),
        fields,
        class_index: None,
        finalized: false,
    }))
}

fn blob_as_u64(blob: &[u8]) -> u64 {
    let mut b = [0u8; 8];
    let n = blob.len().min(8);
    b[..n].copy_from_slice(&blob[..n]);
    u64::from_ne_bytes(b)
}

fn value_to_slot(
    v: &Value,
    ty: &FfiType,
    strings: &mut Vec<CString>,
    blobs: &mut Vec<Vec<u8>>,
) -> RuntimeResult<ArgSlot> {
    Ok(match ty {
        FfiType::Void => ArgSlot::I64(0),
        FfiType::Bool => ArgSlot::I64(if v.is_truthy() { 1 } else { 0 }),
        FfiType::I8 | FfiType::I16 | FfiType::I32 | FfiType::I64 => ArgSlot::I64(v.as_int()?),
        FfiType::U8 | FfiType::U16 | FfiType::U32 | FfiType::U64 => {
            ArgSlot::U64(v.as_int()? as u64)
        }
        FfiType::F32 => ArgSlot::F32(v.as_float()? as f32),
        FfiType::F64 => ArgSlot::F64(v.as_float()?),
        FfiType::Ptr => match v {
            Value::Ptr(p) => ArgSlot::U64(*p as u64),
            Value::Null => ArgSlot::U64(0),
            Value::Int(n) => ArgSlot::U64(*n as u64),
            Value::String(s) => {
                let c = CString::new(s.as_ref())
                    .map_err(|_| RuntimeError::Message("C string contains NUL".into()))?;
                let ptr = c.as_ptr() as u64;
                strings.push(c);
                ArgSlot::U64(ptr)
            }
            _ => {
                return Err(RuntimeError::TypeError(format!(
                    "cannot pass {} as pointer",
                    v.type_name()
                )));
            }
        },
        FfiType::CString => {
            let s = v.as_string();
            let c = CString::new(s)
                .map_err(|_| RuntimeError::Message("C string contains NUL".into()))?;
            let ptr = c.as_ptr() as u64;
            strings.push(c);
            ArgSlot::U64(ptr)
        }
        FfiType::Struct(layout) => {
            let blob = pack_struct(v, layout)?;
            if crate::abi::struct_fits_register(layout) {
                let u = blob_as_u64(&blob);
                blobs.push(blob);
                ArgSlot::U64(u)
            } else {
                // Win64 / SysV: large aggregates passed by pointer to a copy
                blobs.push(blob);
                let ptr = blobs.last().unwrap().as_ptr() as u64;
                ArgSlot::U64(ptr)
            }
        }
        FfiType::StructPtr(layout) => {
            let blob = pack_struct(v, layout)?;
            blobs.push(blob);
            let ptr = blobs.last().unwrap().as_ptr() as u64;
            ArgSlot::U64(ptr)
        }
    })
}

fn slot_as_i64(s: &ArgSlot) -> i64 {
    match s {
        ArgSlot::I64(n) => *n,
        ArgSlot::U64(n) => *n as i64,
        ArgSlot::F64(n) => *n as i64,
        ArgSlot::F32(n) => *n as i64,
    }
}

fn slot_as_u64(s: &ArgSlot) -> u64 {
    match s {
        ArgSlot::I64(n) => *n as u64,
        ArgSlot::U64(n) => *n,
        ArgSlot::F64(n) => *n as u64,
        ArgSlot::F32(n) => *n as u64,
    }
}

pub fn call(func: &FfiFunction, args: &[Value]) -> RuntimeResult<Value> {
    let args = if args.len() > func.params.len() {
        &args[args.len() - func.params.len()..]
    } else {
        args
    };
    if args.len() != func.params.len() {
        return Err(RuntimeError::Message(format!(
            "FFI '{}': expected {} args, got {}",
            func.name,
            func.params.len(),
            args.len()
        )));
    }

    load_library(&func.library)?;

    // Stable heap buffers so pointers stay valid for the duration of the call.
    let mut strings = Vec::new();
    let mut blob_boxes: Vec<Box<[u8]>> = Vec::new();

    let sret_idx = match &func.ret {
        FfiType::Struct(layout) if !crate::abi::struct_fits_register(layout) => {
            blob_boxes.push(vec![0u8; layout.size].into_boxed_slice());
            Some(0usize)
        }
        _ => None,
    };

    let mut pending: Vec<(usize, bool)> = Vec::new(); // (blob_idx, fits_register)
    let mut scalar_slots: Vec<ArgSlot> = Vec::new();

    // Build arg list conceptually: optional sret + params
    if sret_idx.is_some() {
        scalar_slots.push(ArgSlot::U64(0)); // patched below
    }

    for (a, ty) in args.iter().zip(func.params.iter()) {
        match ty {
            FfiType::Struct(layout) => {
                let blob = pack_struct(a, layout)?;
                let fits = crate::abi::struct_fits_register(layout);
                let idx = blob_boxes.len();
                blob_boxes.push(blob.into_boxed_slice());
                pending.push((idx, fits));
                scalar_slots.push(ArgSlot::U64(0)); // patched
            }
            FfiType::StructPtr(layout) => {
                let blob = pack_struct(a, layout)?;
                let idx = blob_boxes.len();
                blob_boxes.push(blob.into_boxed_slice());
                pending.push((idx, false)); // always pointer
                scalar_slots.push(ArgSlot::U64(0));
            }
            other => {
                let mut tmp_blobs = Vec::new();
                scalar_slots.push(value_to_slot(a, other, &mut strings, &mut tmp_blobs)?);
            }
        }
    }

    // Patch struct / sret pointers now that blob_boxes is final
    let mut slot_i = 0usize;
    if let Some(si) = sret_idx {
        scalar_slots[slot_i] = ArgSlot::U64(blob_boxes[si].as_ptr() as u64);
        slot_i += 1;
    }
    let mut pend_i = 0usize;
    for ty in &func.params {
        if matches!(ty, FfiType::Struct(_) | FfiType::StructPtr(_)) {
            let (bi, fits) = pending[pend_i];
            pend_i += 1;
            if fits {
                scalar_slots[slot_i] = ArgSlot::U64(blob_as_u64(&blob_boxes[bi]));
            } else {
                scalar_slots[slot_i] = ArgSlot::U64(blob_boxes[bi].as_ptr() as u64);
            }
        }
        slot_i += 1;
    }

    let has_float = func.params.iter().any(|t| t.is_float()) || func.ret.is_float();
    let result = if has_float && sret_idx.is_none() && !func.params.iter().any(|t| t.is_struct()) {
        call_float(func, &scalar_slots)?
    } else {
        call_int(func, &scalar_slots)?
    };

    let result = if let Some(si) = sret_idx {
        match &func.ret {
            FfiType::Struct(layout) => unpack_struct(&blob_boxes[si], layout)?,
            _ => result,
        }
    } else {
        result
    };

    drop(strings);
    drop(blob_boxes);
    Ok(result)
}

fn call_int(func: &FfiFunction, slots: &[ArgSlot]) -> RuntimeResult<Value> {
    let lib_name = func.library.clone();
    let sym = func.symbol.clone();
    let ret = func.ret.clone();
    let n = slots.len();
    let a: Vec<u64> = slots.iter().map(slot_as_u64).collect();

    load_library(&lib_name)?;
    let libs = libraries()
        .lock()
        .map_err(|_| RuntimeError::Message("ffi library lock poisoned".into()))?;
    let lib = libs
        .get(&lib_name)
        .ok_or_else(|| RuntimeError::Message(format!("library '{}' not loaded", lib_name)))?;

    macro_rules! get_sym {
        ($t:ty) => {{
            let r: Result<libloading::Symbol<$t>, _> = unsafe { lib.get(sym.as_bytes()) };
            match r {
                Ok(s) => s,
                Err(e) => {
                    return Err(RuntimeError::Message(format!(
                        "symbol '{}' not found in '{}': {}",
                        sym, lib_name, e
                    )));
                }
            }
        }};
    }

    let _ = func.abi;

    let raw: u64 = match n {
        0 => {
            let f = get_sym!(unsafe extern "C" fn() -> u64);
            unsafe { f() }
        }
        1 => {
            let f = get_sym!(unsafe extern "C" fn(u64) -> u64);
            unsafe { f(a[0]) }
        }
        2 => {
            let f = get_sym!(unsafe extern "C" fn(u64, u64) -> u64);
            unsafe { f(a[0], a[1]) }
        }
        3 => {
            let f = get_sym!(unsafe extern "C" fn(u64, u64, u64) -> u64);
            unsafe { f(a[0], a[1], a[2]) }
        }
        4 => {
            let f = get_sym!(unsafe extern "C" fn(u64, u64, u64, u64) -> u64);
            unsafe { f(a[0], a[1], a[2], a[3]) }
        }
        5 => {
            let f = get_sym!(unsafe extern "C" fn(u64, u64, u64, u64, u64) -> u64);
            unsafe { f(a[0], a[1], a[2], a[3], a[4]) }
        }
        6 => {
            let f = get_sym!(unsafe extern "C" fn(u64, u64, u64, u64, u64, u64) -> u64);
            unsafe { f(a[0], a[1], a[2], a[3], a[4], a[5]) }
        }
        _ => {
            return Err(RuntimeError::Message(format!(
                "FFI '{}': at most 6 integer/pointer arguments supported",
                func.name
            )));
        }
    };

    decode_return(&ret, raw)
}

fn call_float(func: &FfiFunction, slots: &[ArgSlot]) -> RuntimeResult<Value> {
    let lib_name = func.library.clone();
    let sym = func.symbol.clone();
    let ret = func.ret.clone();

    load_library(&lib_name)?;
    let libs = libraries()
        .lock()
        .map_err(|_| RuntimeError::Message("ffi library lock poisoned".into()))?;
    let lib = libs
        .get(&lib_name)
        .ok_or_else(|| RuntimeError::Message(format!("library '{}' not loaded", lib_name)))?;

    macro_rules! get_sym {
        ($t:ty) => {{
            let r: Result<libloading::Symbol<$t>, _> = unsafe { lib.get(sym.as_bytes()) };
            match r {
                Ok(s) => s,
                Err(e) => {
                    return Err(RuntimeError::Message(format!(
                        "symbol '{}' not found in '{}': {}",
                        sym, lib_name, e
                    )));
                }
            }
        }};
    }

    if func.params.is_empty() && matches!(ret, FfiType::F64) {
        let f = get_sym!(unsafe extern "C" fn() -> f64);
        let r = unsafe { f() };
        return Ok(Value::Float(r));
    }
    if func.params.is_empty() && matches!(ret, FfiType::F32) {
        let f = get_sym!(unsafe extern "C" fn() -> f32);
        let r = unsafe { f() };
        return Ok(Value::Float(r as f64));
    }
    if func.params.len() == 1
        && matches!(func.params[0], FfiType::F64)
        && matches!(ret, FfiType::Void)
    {
        let x = match &slots[0] {
            ArgSlot::F64(v) => *v,
            _ => slot_as_i64(&slots[0]) as f64,
        };
        let f = get_sym!(unsafe extern "C" fn(f64));
        unsafe { f(x) };
        return Ok(Value::Null);
    }
    if func.params.len() == 1
        && matches!(func.params[0], FfiType::F64)
        && matches!(ret, FfiType::F64)
    {
        let x = match &slots[0] {
            ArgSlot::F64(v) => *v,
            _ => slot_as_i64(&slots[0]) as f64,
        };
        let f = get_sym!(unsafe extern "C" fn(f64) -> f64);
        let r = unsafe { f(x) };
        return Ok(Value::Float(r));
    }
    if func.params.len() == 2
        && matches!(func.params[0], FfiType::F64)
        && matches!(func.params[1], FfiType::F64)
        && matches!(ret, FfiType::F64)
    {
        let x = match &slots[0] {
            ArgSlot::F64(v) => *v,
            _ => slot_as_i64(&slots[0]) as f64,
        };
        let y = match &slots[1] {
            ArgSlot::F64(v) => *v,
            _ => slot_as_i64(&slots[1]) as f64,
        };
        let f = get_sym!(unsafe extern "C" fn(f64, f64) -> f64);
        let r = unsafe { f(x, y) };
        return Ok(Value::Float(r));
    }
    if func.params.len() == 1
        && matches!(func.params[0], FfiType::F32)
        && matches!(ret, FfiType::F32)
    {
        let x = match &slots[0] {
            ArgSlot::F32(v) => *v,
            ArgSlot::F64(v) => *v as f32,
            _ => slot_as_i64(&slots[0]) as f32,
        };
        let f = get_sym!(unsafe extern "C" fn(f32) -> f32);
        let r = unsafe { f(x) };
        return Ok(Value::Float(r as f64));
    }

    // void(f32,f32,f32,f32,u32) — e.g. platformer_draw_rect
    if matches!(ret, FfiType::Void)
        && func.params.len() == 5
        && matches!(func.params[0], FfiType::F32)
        && matches!(func.params[1], FfiType::F32)
        && matches!(func.params[2], FfiType::F32)
        && matches!(func.params[3], FfiType::F32)
        && matches!(func.params[4], FfiType::U32 | FfiType::I32 | FfiType::U64 | FfiType::I64)
    {
        let f32_at = |i: usize| -> f32 {
            match &slots[i] {
                ArgSlot::F32(v) => *v,
                ArgSlot::F64(v) => *v as f32,
                _ => slot_as_i64(&slots[i]) as f32,
            }
        };
        let u = slot_as_u64(&slots[4]) as u32;
        let f = get_sym!(unsafe extern "C" fn(f32, f32, f32, f32, u32));
        unsafe { f(f32_at(0), f32_at(1), f32_at(2), f32_at(3), u) };
        return Ok(Value::Null);
    }

    // void(u16, u16, u32, f32, u8) — bgfx_set_view_clear
    if matches!(ret, FfiType::Void)
        && func.params.len() == 5
        && is_intish(&func.params[0])
        && is_intish(&func.params[1])
        && is_intish(&func.params[2])
        && matches!(func.params[3], FfiType::F32)
        && is_intish(&func.params[4])
    {
        let a0 = slot_as_u64(&slots[0]) as u16;
        let a1 = slot_as_u64(&slots[1]) as u16;
        let a2 = slot_as_u64(&slots[2]) as u32;
        let depth = match &slots[3] {
            ArgSlot::F32(v) => *v,
            ArgSlot::F64(v) => *v as f32,
            _ => slot_as_i64(&slots[3]) as f32,
        };
        let a4 = slot_as_u64(&slots[4]) as u8;
        let f = get_sym!(unsafe extern "C" fn(u16, u16, u32, f32, u8));
        unsafe { f(a0, a1, a2, depth, a4) };
        return Ok(Value::Null);
    }

    // Generic: void with ≤4 integer args then one f32 (common mixed Win64 pattern).
    if matches!(ret, FfiType::Void) {
        if let Some(fi) = func.params.iter().position(|t| matches!(t, FfiType::F32)) {
            let all_rest_int = func
                .params
                .iter()
                .enumerate()
                .all(|(i, t)| i == fi || is_intish(t));
            if all_rest_int && func.params.iter().filter(|t| t.is_float()).count() == 1 {
                let f32_at = |i: usize| -> f32 {
                    match &slots[i] {
                        ArgSlot::F32(v) => *v,
                        ArgSlot::F64(v) => *v as f32,
                        _ => slot_as_i64(&slots[i]) as f32,
                    }
                };
                match (func.params.len(), fi) {
                    (1, 0) => {
                        let f = get_sym!(unsafe extern "C" fn(f32));
                        unsafe { f(f32_at(0)) };
                        return Ok(Value::Null);
                    }
                    (2, 0) => {
                        let f = get_sym!(unsafe extern "C" fn(f32, u64));
                        unsafe { f(f32_at(0), slot_as_u64(&slots[1])) };
                        return Ok(Value::Null);
                    }
                    (2, 1) => {
                        let f = get_sym!(unsafe extern "C" fn(u64, f32));
                        unsafe { f(slot_as_u64(&slots[0]), f32_at(1)) };
                        return Ok(Value::Null);
                    }
                    (3, 0) => {
                        let f = get_sym!(unsafe extern "C" fn(f32, u64, u64));
                        unsafe { f(f32_at(0), slot_as_u64(&slots[1]), slot_as_u64(&slots[2])) };
                        return Ok(Value::Null);
                    }
                    (3, 1) => {
                        let f = get_sym!(unsafe extern "C" fn(u64, f32, u64));
                        unsafe { f(slot_as_u64(&slots[0]), f32_at(1), slot_as_u64(&slots[2])) };
                        return Ok(Value::Null);
                    }
                    (3, 2) => {
                        let f = get_sym!(unsafe extern "C" fn(u64, u64, f32));
                        unsafe { f(slot_as_u64(&slots[0]), slot_as_u64(&slots[1]), f32_at(2)) };
                        return Ok(Value::Null);
                    }
                    (4, 3) => {
                        let f = get_sym!(unsafe extern "C" fn(u64, u64, u64, f32));
                        unsafe {
                            f(
                                slot_as_u64(&slots[0]),
                                slot_as_u64(&slots[1]),
                                slot_as_u64(&slots[2]),
                                f32_at(3),
                            )
                        };
                        return Ok(Value::Null);
                    }
                    (5, 3) => {
                        // already handled above for set_view_clear; keep u64 form as fallback
                        let f = get_sym!(unsafe extern "C" fn(u64, u64, u64, f32, u64));
                        unsafe {
                            f(
                                slot_as_u64(&slots[0]),
                                slot_as_u64(&slots[1]),
                                slot_as_u64(&slots[2]),
                                f32_at(3),
                                slot_as_u64(&slots[4]),
                            )
                        };
                        return Ok(Value::Null);
                    }
                    _ => {}
                }
            }
        }
    }

    Err(RuntimeError::Message(format!(
        "FFI '{}': unsupported float signature",
        func.name
    )))
}

fn is_intish(t: &FfiType) -> bool {
    matches!(
        t,
        FfiType::Bool
            | FfiType::I8
            | FfiType::U8
            | FfiType::I16
            | FfiType::U16
            | FfiType::I32
            | FfiType::U32
            | FfiType::I64
            | FfiType::U64
            | FfiType::Ptr
            | FfiType::CString
    )
}

fn decode_return(ret: &FfiType, raw: u64) -> RuntimeResult<Value> {
    Ok(match ret {
        FfiType::Void => Value::Null,
        FfiType::Bool => Value::Bool(raw != 0),
        FfiType::I8 => Value::Int(raw as i8 as i64),
        FfiType::I16 => Value::Int(raw as i16 as i64),
        FfiType::I32 => Value::Int(raw as i32 as i64),
        FfiType::I64 => Value::Int(raw as i64),
        FfiType::U8 => Value::Int((raw as u8) as i64),
        FfiType::U16 => Value::Int((raw as u16) as i64),
        FfiType::U32 => Value::Int((raw as u32) as i64),
        FfiType::U64 => Value::UInt(raw),
        FfiType::F32 | FfiType::F64 => Value::Float(f64::from_bits(raw)),
        FfiType::Ptr => {
            if raw == 0 {
                Value::Null
            } else {
                Value::Ptr(raw as usize)
            }
        }
        FfiType::CString => {
            if raw == 0 {
                Value::Null
            } else {
                let s = unsafe { CStr::from_ptr(raw as *const i8) };
                Value::String(s.to_string_lossy().into_owned().into())
            }
        }
        FfiType::Struct(layout) => {
            // Small struct returned in register(s)
            let n = layout.size.min(8);
            let mut buf = vec![0u8; layout.size];
            let bytes = raw.to_ne_bytes();
            buf[..n].copy_from_slice(&bytes[..n]);
            unpack_struct(&buf, layout)?
        }
        FfiType::StructPtr(_) => {
            if raw == 0 {
                Value::Null
            } else {
                Value::Ptr(raw as usize)
            }
        }
    })
}

pub fn c_type_name(ty: &FfiType) -> String {
    match ty {
        FfiType::Void => "void".into(),
        FfiType::Bool => "bool".into(),
        FfiType::I8 => "int8_t".into(),
        FfiType::I16 => "int16_t".into(),
        FfiType::I32 => "int".into(),
        FfiType::I64 => "int64_t".into(),
        FfiType::U8 => "uint8_t".into(),
        FfiType::U16 => "uint16_t".into(),
        FfiType::U32 => "unsigned".into(),
        FfiType::U64 => "uint64_t".into(),
        FfiType::F32 => "float".into(),
        FfiType::F64 => "double".into(),
        FfiType::Ptr => "void*".into(),
        FfiType::CString => "const char*".into(),
        FfiType::Struct(s) => s.name.clone(),
        FfiType::StructPtr(s) => format!("{}*", s.name),
    }
}
