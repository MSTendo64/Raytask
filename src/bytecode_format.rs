//! Binary bytecode format (.rtbc) — serialize/deserialize Module.

use crate::bytecode::{Chunk, ClassInfo, Module};
use crate::error::{CompileError, CompileResult};
use crate::value::{FunctionRef, Value};
use std::rc::Rc;

pub const RTBC_MAGIC: &[u8; 4] = b"RTBC";
pub const RTBC_VERSION: u16 = 9;

/// Trailer magic for standalone apps: [...bytecode...][u64 len][APP_MAGIC]
pub const APP_MAGIC: &[u8; 8] = b"RTBCAP\x01\0";

pub fn serialize_module(module: &Module) -> Vec<u8> {
    let mut w = Writer::new();
    w.bytes(RTBC_MAGIC);
    w.u16(RTBC_VERSION);
    w.var_u32(module.main_chunk as u32);
    w.u8(if module.stdlib_enabled { 1 } else { 0 });

    w.var_u32(module.globals.len() as u32);
    for g in &module.globals {
        w.var_str(g);
    }

    w.var_u32(module.classes.len() as u32);
    for c in &module.classes {
        w.var_str(&c.name);
        w.var_u32(c.fields.len() as u32);
        for f in &c.fields {
            w.var_str(f);
        }
        w.var_u32(c.methods.len() as u32);
        for (name, idx) in &c.methods {
            w.var_str(name);
            w.var_u32(*idx as u32);
        }
        match c.constructor {
            Some(i) => {
                w.u8(1);
                w.var_u32(i as u32);
            }
            None => w.u8(0),
        }
        match c.base {
            Some(i) => {
                w.u8(1);
                w.var_u32(i as u32);
            }
            None => w.u8(0),
        }
        match c.destructor {
            Some(i) => {
                w.u8(1);
                w.var_u32(i as u32);
            }
            None => w.u8(0),
        }
    }

    w.var_u32(module.chunks.len() as u32);
    for chunk in &module.chunks {
        w.var_str(&chunk.name);
        w.var_u32(chunk.arity as u32);
        w.var_u32(chunk.local_count as u32);
        w.u8(if chunk.is_async { 1 } else { 0 });
        w.var_u32(chunk.code.len() as u32);
        w.raw(&chunk.code);
        write_lines(&mut w, &chunk.lines);
        w.var_u32(chunk.constants.len() as u32);
        for c in &chunk.constants {
            write_value(&mut w, c);
        }
        w.var_u32(chunk.local_debug.len() as u32);
        for ld in &chunk.local_debug {
            w.var_str(&ld.name);
            w.u8(ld.slot);
            w.var_u32(ld.start_ip as u32);
            w.var_u32(if ld.end_ip == usize::MAX {
                u32::MAX
            } else {
                ld.end_ip as u32
            });
        }
        match &chunk.source {
            Some(s) => {
                w.u8(1);
                w.var_str(s);
            }
            None => w.u8(0),
        }
    }

    // FFI metadata
    w.var_u32(module.ffi.includes.len() as u32);
    for s in &module.ffi.includes {
        w.var_str(s);
    }
    w.var_u32(module.ffi.links.len() as u32);
    for s in &module.ffi.links {
        w.var_str(s);
    }
    w.var_u32(module.ffi.embeds.len() as u32);
    for e in &module.ffi.embeds {
        w.var_str(&e.lib_name);
        w.var_str(&e.source);
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
    if version != 8 && version != RTBC_VERSION {
        return Err(CompileError::Io {
            message: format!(
                "unsupported .rtbc version {} (runtime expects {}); rebuild the app: raytask build … --target native-bin (updates raytask-stub)",
                version, RTBC_VERSION
            ),
        });
    }
    if version == 8 {
        return deserialize_module_v8(&mut r);
    }
    deserialize_module_v9(&mut r)
}

fn deserialize_module_v9(r: &mut Reader<'_>) -> CompileResult<Module> {
    let main_chunk = r.var_u32()? as usize;
    let stdlib_enabled = r.u8()? != 0;

    let n_globals = r.var_u32()? as usize;
    let mut globals = Vec::with_capacity(n_globals);
    for _ in 0..n_globals {
        globals.push(r.var_str()?);
    }

    let n_classes = r.var_u32()? as usize;
    let mut classes = Vec::with_capacity(n_classes);
    for _ in 0..n_classes {
        let name = r.var_str()?;
        let n_fields = r.var_u32()? as usize;
        let mut fields = Vec::with_capacity(n_fields);
        for _ in 0..n_fields {
            fields.push(r.var_str()?);
        }
        let n_methods = r.var_u32()? as usize;
        let mut methods = Vec::with_capacity(n_methods);
        for _ in 0..n_methods {
            let mname = r.var_str()?;
            let idx = r.var_u32()? as usize;
            methods.push((mname, idx));
        }
        let constructor = if r.u8()? == 1 {
            Some(r.var_u32()? as usize)
        } else {
            None
        };
        let base = if r.u8()? == 1 {
            Some(r.var_u32()? as usize)
        } else {
            None
        };
        let destructor = if r.u8()? == 1 {
            Some(r.var_u32()? as usize)
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

    let n_chunks = r.var_u32()? as usize;
    let mut chunks = Vec::with_capacity(n_chunks);
    for _ in 0..n_chunks {
        let name = r.var_str()?;
        let arity = r.var_u32()? as usize;
        let local_count = r.var_u32()? as usize;
        let is_async = r.u8()? != 0;
        let code_len = r.var_u32()? as usize;
        let code = r.bytes(code_len)?.to_vec();
        let lines = read_lines(r)?;
        let n_consts = r.var_u32()? as usize;
        let mut constants = Vec::with_capacity(n_consts);
        for _ in 0..n_consts {
            constants.push(read_value_v9(r)?);
        }
        let n_ld = r.var_u32()? as usize;
        let mut local_debug = Vec::with_capacity(n_ld);
        for _ in 0..n_ld {
            let ld_name = r.var_str()?;
            let slot = r.u8()?;
            let start_ip = r.var_u32()? as usize;
            let end_raw = r.var_u32()?;
            let end_ip = if end_raw == u32::MAX {
                usize::MAX
            } else {
                end_raw as usize
            };
            local_debug.push(crate::bytecode::LocalDebug {
                name: ld_name,
                slot,
                start_ip,
                end_ip,
            });
        }
        let source = if r.u8()? == 1 {
            Some(r.var_str()?)
        } else {
            None
        };
        chunks.push(Chunk {
            name,
            code,
            constants,
            lines,
            arity,
            local_count,
            is_async,
            local_debug,
            source,
        });
    }

    let mut ffi = crate::ffi::FfiModuleInfo::default();
    let n_inc = r.var_u32()? as usize;
    for _ in 0..n_inc {
        ffi.includes.push(r.var_str()?);
    }
    let n_link = r.var_u32()? as usize;
    for _ in 0..n_link {
        ffi.links.push(r.var_str()?);
    }
    let n_emb = r.var_u32()? as usize;
    for _ in 0..n_emb {
        let lib_name = r.var_str()?;
        let source = r.var_str()?;
        ffi.embeds.push(crate::ffi::FfiEmbed { source, lib_name });
    }

    Ok(Module {
        chunks,
        main_chunk,
        globals,
        classes,
        ffi,
        stdlib_enabled,
    })
}

fn deserialize_module_v8(r: &mut Reader<'_>) -> CompileResult<Module> {
    let main_chunk = r.u32()? as usize;
    let stdlib_enabled = r.u8()? != 0;

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
            constants.push(read_value_v8(r)?);
        }
        let n_ld = r.u32()? as usize;
        let mut local_debug = Vec::with_capacity(n_ld);
        for _ in 0..n_ld {
            let ld_name = r.str()?;
            let slot = r.u8()?;
            let start_ip = r.u32()? as usize;
            let end_raw = r.u32()?;
            let end_ip = if end_raw == u32::MAX {
                usize::MAX
            } else {
                end_raw as usize
            };
            local_debug.push(crate::bytecode::LocalDebug {
                name: ld_name,
                slot,
                start_ip,
                end_ip,
            });
        }
        let source = if r.u8()? == 1 {
            Some(r.str()?)
        } else {
            None
        };
        chunks.push(Chunk {
            name,
            code,
            constants,
            lines,
            arity,
            local_count,
            is_async,
            local_debug,
            source,
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
        stdlib_enabled,
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
            w.var_i64(*n);
        }
        Value::UInt(n) => {
            w.u8(3);
            w.var_u64(*n);
        }
        Value::Float(n) => {
            w.u8(4);
            w.f64(*n);
        }
        Value::Char(c) => {
            w.u8(5);
            w.var_u32(*c as u32);
        }
        Value::String(s) => {
            w.u8(6);
            w.var_str(s);
        }
        Value::Function(f) => {
            w.u8(7);
            w.var_str(&f.name);
            w.var_u32(f.chunk_index as u32);
            w.var_u32(f.arity as u32);
            w.u8(if f.is_async { 1 } else { 0 });
            w.var_u32(f.defaults.len() as u32);
            for d in &f.defaults {
                write_value(w, d);
            }
        }
        Value::Native(id) => {
            w.u8(8);
            w.var_u32(*id as u32);
        }
        Value::Ptr(p) => {
            w.u8(9);
            w.var_u64(*p as u64);
        }
        Value::TypeModule(name) => {
            w.u8(10);
            w.var_str(name);
        }
        Value::Ffi(f) => {
            w.u8(11);
            w.var_str(&f.name);
            w.var_str(&f.library);
            w.var_str(&f.symbol);
            w.u8(match f.abi {
                crate::ffi::FfiAbi::Cdecl => 0,
                crate::ffi::FfiAbi::Stdcall => 1,
                crate::ffi::FfiAbi::System => 2,
            });
            w.u8(f.ret as u8);
            w.var_u32(f.params.len() as u32);
            for p in &f.params {
                w.u8(*p as u8);
            }
        }
        // Arrays/objects/dicts/tasks are runtime-only; store as null in constants
        Value::Array(_) | Value::Dict(_) | Value::Object(_) | Value::Task(_) => w.u8(0),
    }
}

fn write_lines(w: &mut Writer, lines: &[usize]) {
    w.var_u32(lines.len() as u32);
    if lines.is_empty() {
        w.var_u32(0);
        return;
    }
    let mut runs: Vec<(u32, u32)> = Vec::new();
    let mut current = lines[0] as u32;
    let mut len = 1u32;
    for &line in &lines[1..] {
        let line = line as u32;
        if line == current {
            len += 1;
        } else {
            runs.push((current, len));
            current = line;
            len = 1;
        }
    }
    runs.push((current, len));
    w.var_u32(runs.len() as u32);
    for (line, len) in runs {
        w.var_u32(line);
        w.var_u32(len);
    }
}

fn read_lines(r: &mut Reader<'_>) -> CompileResult<Vec<usize>> {
    let total = r.var_u32()? as usize;
    let n_runs = r.var_u32()? as usize;
    let mut lines = Vec::with_capacity(total);
    for _ in 0..n_runs {
        let line = r.var_u32()? as usize;
        let len = r.var_u32()? as usize;
        lines.extend(std::iter::repeat_n(line, len));
    }
    if lines.len() != total {
        return Err(CompileError::Io {
            message: "corrupt .rtbc line table".into(),
        });
    }
    Ok(lines)
}

fn read_value_v8(r: &mut Reader<'_>) -> CompileResult<Value> {
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
                defaults.push(read_value_v8(r)?);
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

fn read_value_v9(r: &mut Reader<'_>) -> CompileResult<Value> {
    Ok(match r.u8()? {
        0 => Value::Null,
        1 => Value::Bool(r.u8()? != 0),
        2 => Value::Int(r.var_i64()?),
        3 => Value::UInt(r.var_u64()?),
        4 => Value::Float(r.f64()?),
        5 => {
            let cp = r.var_u32()?;
            Value::Char(char::from_u32(cp).unwrap_or('\0'))
        }
        6 => Value::String(Rc::<str>::from(r.var_str()?)),
        7 => {
            let name = r.var_str()?;
            let chunk_index = r.var_u32()? as usize;
            let arity = r.var_u32()? as usize;
            let is_async = r.u8()? != 0;
            let n = r.var_u32()? as usize;
            let mut defaults = Vec::with_capacity(n);
            for _ in 0..n {
                defaults.push(read_value_v9(r)?);
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
        8 => Value::Native(r.var_u32()? as usize),
        9 => Value::Ptr(r.var_u64()? as usize),
        10 => Value::TypeModule(Rc::<str>::from(r.var_str()?)),
        11 => {
            let name = r.var_str()?;
            let library = r.var_str()?;
            let symbol = r.var_str()?;
            let abi = match r.u8()? {
                1 => crate::ffi::FfiAbi::Stdcall,
                2 => crate::ffi::FfiAbi::System,
                _ => crate::ffi::FfiAbi::Cdecl,
            };
            let ret = crate::ffi::FfiType::from_u8(r.u8()?);
            let n = r.var_u32()? as usize;
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
    fn f64(&mut self, v: f64) {
        self.buf.extend_from_slice(&v.to_le_bytes());
    }
    fn var_u32(&mut self, mut v: u32) {
        while v >= 0x80 {
            self.u8((v as u8) | 0x80);
            v >>= 7;
        }
        self.u8(v as u8);
    }
    fn var_u64(&mut self, mut v: u64) {
        while v >= 0x80 {
            self.u8((v as u8) | 0x80);
            v >>= 7;
        }
        self.u8(v as u8);
    }
    fn var_i64(&mut self, v: i64) {
        let zigzag = ((v << 1) ^ (v >> 63)) as u64;
        self.var_u64(zigzag);
    }
    fn var_str(&mut self, s: &str) {
        let b = s.as_bytes();
        self.var_u32(b.len() as u32);
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
    fn var_u32(&mut self) -> CompileResult<u32> {
        Ok(self.var_u64()? as u32)
    }
    fn var_u64(&mut self) -> CompileResult<u64> {
        let mut shift = 0u32;
        let mut value = 0u64;
        loop {
            let byte = self.u8()?;
            value |= ((byte & 0x7f) as u64) << shift;
            if byte & 0x80 == 0 {
                return Ok(value);
            }
            shift += 7;
            if shift > 63 {
                return Err(CompileError::Io {
                    message: "corrupt .rtbc varint".into(),
                });
            }
        }
    }
    fn var_i64(&mut self) -> CompileResult<i64> {
        let n = self.var_u64()?;
        Ok(((n >> 1) as i64) ^ (-((n & 1) as i64)))
    }
    fn var_str(&mut self) -> CompileResult<String> {
        let len = self.var_u32()? as usize;
        let b = self.bytes(len)?;
        String::from_utf8(b.to_vec()).map_err(|e| CompileError::Io {
            message: e.to_string(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rtbc_v9_roundtrip_preserves_module() {
        let mut chunk = Chunk::new("Main");
        chunk.code = vec![1, 2, 3, 4];
        chunk.lines = vec![10, 10, 10, 20];
        chunk.constants = vec![Value::Int(42), Value::String(Rc::<str>::from("hello"))];
        chunk.arity = 1;
        chunk.local_count = 2;
        chunk.is_async = true;
        chunk.local_debug.push(crate::bytecode::LocalDebug {
            name: "x".into(),
            slot: 0,
            start_ip: 0,
            end_ip: 4,
        });
        chunk.source = Some("demo.rt".into());

        let module = Module {
            chunks: vec![chunk],
            main_chunk: 0,
            globals: vec!["Main".into()],
            classes: vec![],
            ffi: crate::ffi::FfiModuleInfo::default(),
            stdlib_enabled: true,
        };

        let bytes = serialize_module(&module);
        assert_eq!(&bytes[..4], RTBC_MAGIC);
        let restored = deserialize_module(&bytes).unwrap();
        assert_eq!(restored.main_chunk, 0);
        assert_eq!(restored.globals, vec!["Main".to_string()]);
        assert_eq!(restored.chunks[0].lines, vec![10, 10, 10, 20]);
        assert_eq!(restored.chunks[0].constants.len(), 2);
        assert!(restored.chunks[0].is_async);
    }
}
