//! Binary bytecode format (.rtbc) — serialize/deserialize Module.

use crate::bytecode::{Chunk, ClassInfo, Module};
use crate::error::{CompileError, CompileResult};
use crate::value::{FunctionRef, Value};
use std::rc::Rc;

pub const RTBC_MAGIC: &[u8; 4] = b"RTBC";
pub const RTBC_VERSION: u16 = 6;

/// Trailer magic for standalone apps: [...bytecode...][u64 len][APP_MAGIC]
pub const APP_MAGIC: &[u8; 8] = b"RTBCAP\x01\0";

pub fn serialize_module(module: &Module) -> Vec<u8> {
    let mut w = Writer::new();
    w.bytes(RTBC_MAGIC);
    w.u16(RTBC_VERSION);
    w.u32(module.main_chunk as u32);

    w.u32(module.globals.len() as u32);
    for g in &module.globals {
        w.str(g);
    }

    w.u32(module.classes.len() as u32);
    for c in &module.classes {
        w.str(&c.name);
        w.u32(c.fields.len() as u32);
        for f in &c.fields {
            w.str(f);
        }
        w.u32(c.methods.len() as u32);
        for (name, idx) in &c.methods {
            w.str(name);
            w.u32(*idx as u32);
        }
        match c.constructor {
            Some(i) => {
                w.u8(1);
                w.u32(i as u32);
            }
            None => w.u8(0),
        }
        match c.base {
            Some(i) => {
                w.u8(1);
                w.u32(i as u32);
            }
            None => w.u8(0),
        }
        match c.destructor {
            Some(i) => {
                w.u8(1);
                w.u32(i as u32);
            }
            None => w.u8(0),
        }
    }

    w.u32(module.chunks.len() as u32);
    for chunk in &module.chunks {
        w.str(&chunk.name);
        w.u32(chunk.arity as u32);
        w.u32(chunk.local_count as u32);
        w.u8(if chunk.is_async { 1 } else { 0 });
        w.u32(chunk.code.len() as u32);
        w.raw(&chunk.code);
        w.u32(chunk.lines.len() as u32);
        for line in &chunk.lines {
            w.u32(*line as u32);
        }
        w.u32(chunk.constants.len() as u32);
        for c in &chunk.constants {
            write_value(&mut w, c);
        }
    }

    // FFI metadata
    w.u32(module.ffi.includes.len() as u32);
    for s in &module.ffi.includes {
        w.str(s);
    }
    w.u32(module.ffi.links.len() as u32);
    for s in &module.ffi.links {
        w.str(s);
    }
    w.u32(module.ffi.embeds.len() as u32);
    for e in &module.ffi.embeds {
        w.str(&e.lib_name);
        w.str(&e.source);
    }

    w.into_inner()
}

pub fn deserialize_module(data: &[u8]) -> CompileResult<Module> {
    let mut r = Reader::new(data);
    let magic = r.bytes(4)?;
    if magic != RTBC_MAGIC {
        return Err(CompileError::Io {
            message: "invalid .rtbc magic".into(),
        });
    }
    let version = r.u16()?;
    if version != RTBC_VERSION {
        return Err(CompileError::Io {
            message: format!(
                "unsupported .rtbc version {} (runtime expects {}); rebuild the app: raytask build … --target native-bin (updates raytask-stub)",
                version, RTBC_VERSION
            ),
        });
    }
    let main_chunk = r.u32()? as usize;

    let n_globals = r.u32()? as usize;
    let mut globals = Vec::with_capacity(n_globals);
    for _ in 0..n_globals {
        globals.push(r.str()?);
    }

    let n_classes = r.u32()? as usize;
    let mut classes = Vec::with_capacity(n_classes);
    for _ in 0..n_classes {
        let name = r.str()?;
        let n_fields = r.u32()? as usize;
        let mut fields = Vec::with_capacity(n_fields);
        for _ in 0..n_fields {
            fields.push(r.str()?);
        }
        let n_methods = r.u32()? as usize;
        let mut methods = Vec::with_capacity(n_methods);
        for _ in 0..n_methods {
            let mname = r.str()?;
            let idx = r.u32()? as usize;
            methods.push((mname, idx));
        }
        let constructor = if r.u8()? == 1 {
            Some(r.u32()? as usize)
        } else {
            None
        };
        let base = if r.u8()? == 1 {
            Some(r.u32()? as usize)
        } else {
            None
        };
        let destructor = if r.u8()? == 1 {
            Some(r.u32()? as usize)
        } else {
            None
        };
        classes.push(ClassInfo {
            name,
            fields,
            methods,
            constructor,
            base,
            destructor,
        });
    }

    let n_chunks = r.u32()? as usize;
    let mut chunks = Vec::with_capacity(n_chunks);
    for _ in 0..n_chunks {
        let name = r.str()?;
        let arity = r.u32()? as usize;
        let local_count = r.u32()? as usize;
        let is_async = r.u8()? != 0;
        let code_len = r.u32()? as usize;
        let code = r.bytes(code_len)?.to_vec();
        let n_lines = r.u32()? as usize;
        let mut lines = Vec::with_capacity(n_lines);
        for _ in 0..n_lines {
            lines.push(r.u32()? as usize);
        }
        let n_consts = r.u32()? as usize;
        let mut constants = Vec::with_capacity(n_consts);
        for _ in 0..n_consts {
            constants.push(read_value(&mut r)?);
        }
        chunks.push(Chunk {
            name,
            code,
            constants,
            lines,
            arity,
            local_count,
            is_async,
        });
    }

    let mut ffi = crate::ffi::FfiModuleInfo::default();
    let n_inc = r.u32()? as usize;
    for _ in 0..n_inc {
        ffi.includes.push(r.str()?);
    }
    let n_link = r.u32()? as usize;
    for _ in 0..n_link {
        ffi.links.push(r.str()?);
    }
    let n_emb = r.u32()? as usize;
    for _ in 0..n_emb {
        let lib_name = r.str()?;
        let source = r.str()?;
        ffi.embeds.push(crate::ffi::FfiEmbed { source, lib_name });
    }

    Ok(Module {
        chunks,
        main_chunk,
        globals,
        classes,
        ffi,
    })
}

/// Extract bytecode payload appended to a standalone app executable.
pub fn extract_app_payload(exe_bytes: &[u8]) -> Option<Vec<u8>> {
    if exe_bytes.len() < 16 {
        return None;
    }
    let magic = &exe_bytes[exe_bytes.len() - 8..];
    if magic != APP_MAGIC {
        return None;
    }
    let len_bytes: [u8; 8] = exe_bytes[exe_bytes.len() - 16..exe_bytes.len() - 8]
        .try_into()
        .ok()?;
    let len = u64::from_le_bytes(len_bytes) as usize;
    if exe_bytes.len() < 16 + len {
        return None;
    }
    let start = exe_bytes.len() - 16 - len;
    Some(exe_bytes[start..start + len].to_vec())
}

/// Append bytecode payload to a runtime stub, producing a standalone app.
pub fn package_app(stub_bytes: &[u8], bytecode: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(stub_bytes.len() + bytecode.len() + 16);
    out.extend_from_slice(stub_bytes);
    out.extend_from_slice(bytecode);
    out.extend_from_slice(&(bytecode.len() as u64).to_le_bytes());
    out.extend_from_slice(APP_MAGIC);
    out
}

fn write_value(w: &mut Writer, v: &Value) {
    match v {
        Value::Null => w.u8(0),
        Value::Bool(b) => {
            w.u8(1);
            w.u8(if *b { 1 } else { 0 });
        }
        Value::Int(n) => {
            w.u8(2);
            w.i64(*n);
        }
        Value::UInt(n) => {
            w.u8(3);
            w.u64(*n);
        }
        Value::Float(n) => {
            w.u8(4);
            w.f64(*n);
        }
        Value::Char(c) => {
            w.u8(5);
            w.u32(*c as u32);
        }
        Value::String(s) => {
            w.u8(6);
            w.str(s);
        }
        Value::Function(f) => {
            w.u8(7);
            w.str(&f.name);
            w.u32(f.chunk_index as u32);
            w.u32(f.arity as u32);
            w.u8(if f.is_async { 1 } else { 0 });
            w.u32(f.defaults.len() as u32);
            for d in &f.defaults {
                write_value(w, d);
            }
        }
        Value::Native(id) => {
            w.u8(8);
            w.u32(*id as u32);
        }
        Value::Ptr(p) => {
            w.u8(9);
            w.u64(*p as u64);
        }
        Value::TypeModule(name) => {
            w.u8(10);
            w.str(name);
        }
        Value::Ffi(f) => {
            w.u8(11);
            w.str(&f.name);
            w.str(&f.library);
            w.str(&f.symbol);
            w.u8(match f.abi {
                crate::ffi::FfiAbi::Cdecl => 0,
                crate::ffi::FfiAbi::Stdcall => 1,
                crate::ffi::FfiAbi::System => 2,
            });
            w.u8(f.ret as u8);
            w.u32(f.params.len() as u32);
            for p in &f.params {
                w.u8(*p as u8);
            }
        }
        // Arrays/objects/dicts/tasks are runtime-only; store as null in constants
        Value::Array(_) | Value::Dict(_) | Value::Object(_) | Value::Task(_) => w.u8(0),
    }
}

fn read_value(r: &mut Reader<'_>) -> CompileResult<Value> {
    Ok(match r.u8()? {
        0 => Value::Null,
        1 => Value::Bool(r.u8()? != 0),
        2 => Value::Int(r.i64()?),
        3 => Value::UInt(r.u64()?),
        4 => Value::Float(r.f64()?),
        5 => {
            let cp = r.u32()?;
            Value::Char(char::from_u32(cp).unwrap_or('\0'))
        }
        6 => Value::String(Rc::<str>::from(r.str()?)),
        7 => {
            let name = r.str()?;
            let chunk_index = r.u32()? as usize;
            let arity = r.u32()? as usize;
            let is_async = r.u8()? != 0;
            let n = r.u32()? as usize;
            let mut defaults = Vec::with_capacity(n);
            for _ in 0..n {
                defaults.push(read_value(r)?);
            }
            Value::Function(FunctionRef {
                name,
                chunk_index,
                arity,
                defaults,
                is_async,
                upvalues: vec![],
            })
        }
        8 => Value::Native(r.u32()? as usize),
        9 => Value::Ptr(r.u64()? as usize),
        10 => Value::TypeModule(Rc::<str>::from(r.str()?)),
        11 => {
            let name = r.str()?;
            let library = r.str()?;
            let symbol = r.str()?;
            let abi = match r.u8()? {
                1 => crate::ffi::FfiAbi::Stdcall,
                2 => crate::ffi::FfiAbi::System,
                _ => crate::ffi::FfiAbi::Cdecl,
            };
            let ret = crate::ffi::FfiType::from_u8(r.u8()?);
            let n = r.u32()? as usize;
            let mut params = Vec::with_capacity(n);
            for _ in 0..n {
                params.push(crate::ffi::FfiType::from_u8(r.u8()?));
            }
            Value::Ffi(crate::ffi::FfiFunction {
                name,
                library,
                symbol,
                abi,
                params,
                ret,
            })
        }
        other => {
            return Err(CompileError::Io {
                message: format!("unknown value tag {}", other),
            });
        }
    })
}

struct Writer {
    buf: Vec<u8>,
}

impl Writer {
    fn new() -> Self {
        Self { buf: Vec::new() }
    }
    fn into_inner(self) -> Vec<u8> {
        self.buf
    }
    fn raw(&mut self, b: &[u8]) {
        self.buf.extend_from_slice(b);
    }
    fn bytes(&mut self, b: &[u8]) {
        self.raw(b);
    }
    fn u8(&mut self, v: u8) {
        self.buf.push(v);
    }
    fn u16(&mut self, v: u16) {
        self.buf.extend_from_slice(&v.to_le_bytes());
    }
    fn u32(&mut self, v: u32) {
        self.buf.extend_from_slice(&v.to_le_bytes());
    }
    fn u64(&mut self, v: u64) {
        self.buf.extend_from_slice(&v.to_le_bytes());
    }
    fn i64(&mut self, v: i64) {
        self.buf.extend_from_slice(&v.to_le_bytes());
    }
    fn f64(&mut self, v: f64) {
        self.buf.extend_from_slice(&v.to_le_bytes());
    }
    fn str(&mut self, s: &str) {
        let b = s.as_bytes();
        self.u32(b.len() as u32);
        self.raw(b);
    }
}

struct Reader<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0 }
    }
    fn need(&self, n: usize) -> CompileResult<()> {
        if self.pos + n > self.data.len() {
            Err(CompileError::Io {
                message: "truncated .rtbc".into(),
            })
        } else {
            Ok(())
        }
    }
    fn bytes(&mut self, n: usize) -> CompileResult<&'a [u8]> {
        self.need(n)?;
        let s = &self.data[self.pos..self.pos + n];
        self.pos += n;
        Ok(s)
    }
    fn u8(&mut self) -> CompileResult<u8> {
        Ok(self.bytes(1)?[0])
    }
    fn u16(&mut self) -> CompileResult<u16> {
        let b = self.bytes(2)?;
        Ok(u16::from_le_bytes([b[0], b[1]]))
    }
    fn u32(&mut self) -> CompileResult<u32> {
        let b = self.bytes(4)?;
        Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }
    fn u64(&mut self) -> CompileResult<u64> {
        let b = self.bytes(8)?;
        Ok(u64::from_le_bytes([
            b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7],
        ]))
    }
    fn i64(&mut self) -> CompileResult<i64> {
        Ok(self.u64()? as i64)
    }
    fn f64(&mut self) -> CompileResult<f64> {
        Ok(f64::from_le_bytes(self.u64()?.to_le_bytes()))
    }
    fn str(&mut self) -> CompileResult<String> {
        let len = self.u32()? as usize;
        let b = self.bytes(len)?;
        String::from_utf8(b.to_vec()).map_err(|e| CompileError::Io {
            message: e.to_string(),
        })
    }
}
