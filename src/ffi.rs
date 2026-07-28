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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum FfiType {
    Void = 0,
    Bool = 1,
    I8 = 2,
    I16 = 3,
    I32 = 4,
    I64 = 5,
    U8 = 6,
    U16 = 7,
    U32 = 8,
    U64 = 9,
    F32 = 10,
    F64 = 11,
    Ptr = 12,
    CString = 13,
}

impl FfiType {
    pub fn from_type_ref(tr: &TypeRef) -> Self {
        if tr.name == "ptr" || tr.name == "pointer" {
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

    pub fn is_float(self) -> bool {
        matches!(self, FfiType::F32 | FfiType::F64)
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
            _ => FfiType::CString,
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
            .map(|p| FfiType::from_type_ref(&p.ty))
            .collect(),
        ret: FfiType::from_type_ref(&f.return_type),
    })
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
    PathBuf::from(name)
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
            "failed to compile embedded C for '{}': no working C compiler (gcc/clang/cl)",
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

    // Note: do not use WSL gcc on Windows — it produces Linux ELF, not a LoadLibrary-compatible DLL.

    Ok(false)
}

pub fn prepare_module_ffi(info: &FfiModuleInfo, base_dir: Option<&Path>) -> RuntimeResult<()> {
    for embed in &info.embeds {
        let _ = compile_embed(embed, base_dir)?;
    }
    for link in &info.links {
        let path = Path::new(link);
        if path.extension().and_then(|e| e.to_str()) == Some("c") {
            let stem = path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("ffi_link")
                .to_string();
            let source = std::fs::read_to_string(path).map_err(|e| {
                RuntimeError::Message(format!("cannot read '{}': {}", link, e))
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

fn value_to_slot(v: &Value, ty: FfiType, strings: &mut Vec<CString>) -> RuntimeResult<ArgSlot> {
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

    let mut strings = Vec::new();
    let mut slots = Vec::with_capacity(args.len());
    for (a, ty) in args.iter().zip(func.params.iter()) {
        slots.push(value_to_slot(a, *ty, &mut strings)?);
    }

    let has_float = func.params.iter().any(|t| t.is_float()) || func.ret.is_float();
    let result = if has_float {
        call_float(func, &slots)?
    } else {
        call_int(func, &slots)?
    };
    drop(strings);
    Ok(result)
}

fn call_int(func: &FfiFunction, slots: &[ArgSlot]) -> RuntimeResult<Value> {
    let lib_name = func.library.clone();
    let sym = func.symbol.clone();
    let ret = func.ret;
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

    let raw: u64 = unsafe {
        match n {
            0 => {
                let f = get_sym!(unsafe extern "C" fn() -> u64);
                f()
            }
            1 => {
                let f = get_sym!(unsafe extern "C" fn(u64) -> u64);
                f(a[0])
            }
            2 => {
                let f = get_sym!(unsafe extern "C" fn(u64, u64) -> u64);
                f(a[0], a[1])
            }
            3 => {
                let f = get_sym!(unsafe extern "C" fn(u64, u64, u64) -> u64);
                f(a[0], a[1], a[2])
            }
            4 => {
                let f = get_sym!(unsafe extern "C" fn(u64, u64, u64, u64) -> u64);
                f(a[0], a[1], a[2], a[3])
            }
            5 => {
                let f = get_sym!(unsafe extern "C" fn(u64, u64, u64, u64, u64) -> u64);
                f(a[0], a[1], a[2], a[3], a[4])
            }
            6 => {
                let f = get_sym!(unsafe extern "C" fn(u64, u64, u64, u64, u64, u64) -> u64);
                f(a[0], a[1], a[2], a[3], a[4], a[5])
            }
            _ => {
                return Err(RuntimeError::Message(format!(
                    "FFI '{}': at most 6 integer/pointer arguments supported",
                    func.name
                )));
            }
        }
    };

    decode_return(ret, raw)
}

fn call_float(func: &FfiFunction, slots: &[ArgSlot]) -> RuntimeResult<Value> {
    let lib_name = func.library.clone();
    let sym = func.symbol.clone();
    let ret = func.ret;

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

    Err(RuntimeError::Message(format!(
        "FFI '{}': unsupported float signature",
        func.name
    )))
}

fn decode_return(ret: FfiType, raw: u64) -> RuntimeResult<Value> {
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
    })
}

pub fn c_type_name(ty: FfiType) -> &'static str {
    match ty {
        FfiType::Void => "void",
        FfiType::Bool => "bool",
        FfiType::I8 => "int8_t",
        FfiType::I16 => "int16_t",
        FfiType::I32 => "int",
        FfiType::I64 => "int64_t",
        FfiType::U8 => "uint8_t",
        FfiType::U16 => "uint16_t",
        FfiType::U32 => "unsigned",
        FfiType::U64 => "uint64_t",
        FfiType::F32 => "float",
        FfiType::F64 => "double",
        FfiType::Ptr => "void*",
        FfiType::CString => "const char*",
    }
}
