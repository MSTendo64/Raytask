//! bstd.fs natives.

use crate::error::{RuntimeError, RuntimeResult};
use crate::value::{ObjectInstance, Value};
use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

fn path_arg(args: &[Value], idx: usize) -> String {
    // Skip TypeModule if present as receiver
    let offset = if matches!(args.first(), Some(Value::TypeModule(_))) {
        1
    } else {
        0
    };
    args.get(idx + offset)
        .map(|v| v.as_string())
        .unwrap_or_default()
}

fn arg(args: &[Value], idx: usize) -> Option<&Value> {
    let offset = if matches!(args.first(), Some(Value::TypeModule(_))) {
        1
    } else {
        0
    };
    args.get(idx + offset)
}

pub fn read_text(args: &[Value]) -> RuntimeResult<Value> {
    let p = path_arg(args, 0);
    let s = fs::read_to_string(&p).map_err(|e| RuntimeError::Message(format!("{}: {}", p, e)))?;
    Ok(Value::String(s.into()))
}

pub fn write_text(args: &[Value]) -> RuntimeResult<Value> {
    let p = path_arg(args, 0);
    let content = arg(args, 1).map(|v| v.as_string()).unwrap_or_default();
    fs::write(&p, content).map_err(|e| RuntimeError::Message(format!("{}: {}", p, e)))?;
    Ok(Value::Null)
}

pub fn append_text(args: &[Value]) -> RuntimeResult<Value> {
    use std::io::Write;
    let p = path_arg(args, 0);
    let content = arg(args, 1).map(|v| v.as_string()).unwrap_or_default();
    let mut f = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&p)
        .map_err(|e| RuntimeError::Message(format!("{}: {}", p, e)))?;
    f.write_all(content.as_bytes())
        .map_err(|e| RuntimeError::Message(e.to_string()))?;
    Ok(Value::Null)
}

pub fn read_bytes(args: &[Value]) -> RuntimeResult<Value> {
    let p = path_arg(args, 0);
    let bytes = fs::read(&p).map_err(|e| RuntimeError::Message(format!("{}: {}", p, e)))?;
    let vals: Vec<Value> = bytes.into_iter().map(|b| Value::Int(b as i64)).collect();
    Ok(crate::gc::alloc_array(vals))
}

pub fn write_bytes(args: &[Value]) -> RuntimeResult<Value> {
    let p = path_arg(args, 0);
    let bytes = match arg(args, 1) {
        Some(Value::Array(a)) => a
            .borrow()
            .iter()
            .filter_map(|v| v.as_int().ok().map(|n| n as u8))
            .collect::<Vec<_>>(),
        Some(Value::String(s)) => s.as_bytes().to_vec(),
        _ => Vec::new(),
    };
    fs::write(&p, bytes).map_err(|e| RuntimeError::Message(format!("{}: {}", p, e)))?;
    Ok(Value::Null)
}

pub fn exists(args: &[Value]) -> RuntimeResult<Value> {
    Ok(Value::Bool(Path::new(&path_arg(args, 0)).is_file()))
}

pub fn delete_file(args: &[Value]) -> RuntimeResult<Value> {
    let p = path_arg(args, 0);
    let _ = fs::remove_file(&p);
    Ok(Value::Null)
}

fn to_unix_ms(t: SystemTime) -> i64 {
    t.duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

pub fn get_info(args: &[Value]) -> RuntimeResult<Value> {
    let p = path_arg(args, 0);
    let meta = fs::metadata(&p).map_err(|e| RuntimeError::Message(format!("{}: {}", p, e)))?;
    let mut fields = HashMap::new();
    fields.insert("Path".into(), Value::String(p.into()));
    fields.insert("Size".into(), Value::Int(meta.len() as i64));
    fields.insert(
        "Created".into(),
        Value::Int(
            meta.created()
                .map(to_unix_ms)
                .unwrap_or(0),
        ),
    );
    fields.insert(
        "Modified".into(),
        Value::Int(
            meta.modified()
                .map(to_unix_ms)
                .unwrap_or(0),
        ),
    );
    Ok(crate::gc::alloc_object(ObjectInstance {
        class_name: "FileInfo".into(),
        fields,
        class_index: None,
        finalized: false,
    }))
}

pub fn get_files(args: &[Value]) -> RuntimeResult<Value> {
    let p = path_arg(args, 0);
    let mut out = Vec::new();
    if let Ok(rd) = fs::read_dir(&p) {
        for e in rd.flatten() {
            if e.path().is_file() {
                out.push(Value::String(e.path().display().to_string().into()));
            }
        }
    }
    Ok(crate::gc::alloc_array(out))
}

pub fn get_dirs(args: &[Value]) -> RuntimeResult<Value> {
    let p = path_arg(args, 0);
    let mut out = Vec::new();
    if let Ok(rd) = fs::read_dir(&p) {
        for e in rd.flatten() {
            if e.path().is_dir() {
                out.push(Value::String(e.path().display().to_string().into()));
            }
        }
    }
    Ok(crate::gc::alloc_array(out))
}

pub fn create_dir(args: &[Value]) -> RuntimeResult<Value> {
    let p = path_arg(args, 0);
    fs::create_dir_all(&p).map_err(|e| RuntimeError::Message(format!("{}: {}", p, e)))?;
    Ok(Value::Null)
}

pub fn delete_dir(args: &[Value]) -> RuntimeResult<Value> {
    let p = path_arg(args, 0);
    let _ = fs::remove_dir_all(&p);
    Ok(Value::Null)
}

pub fn dir_exists(args: &[Value]) -> RuntimeResult<Value> {
    Ok(Value::Bool(Path::new(&path_arg(args, 0)).is_dir()))
}
