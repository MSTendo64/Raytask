//! bstd.compress — gz (flate2) and zstd compression/decompression.

use crate::error::{RuntimeError, RuntimeResult};
use crate::value::Value;
use std::io::{Read, Write};

fn bytes_from_val(v: &Value) -> Vec<u8> {
    match v {
        Value::Array(a) => a
            .borrow()
            .iter()
            .filter_map(|x| x.as_int().ok().map(|n| n as u8))
            .collect(),
        _ => v.as_string().into_bytes(),
    }
}

fn bytes_to_val(b: Vec<u8>) -> Value {
    let vals: Vec<Value> = b.into_iter().map(|x| Value::Int(x as i64)).collect();
    crate::gc::alloc_array(vals)
}

fn data_arg(args: &[Value]) -> &Value {
    let offset = if matches!(args.first(), Some(Value::TypeModule(_))) { 1 } else { 0 };
    args.get(offset).unwrap_or(&Value::Null)
}

fn path_arg(args: &[Value], idx: usize) -> String {
    let offset = if matches!(args.first(), Some(Value::TypeModule(_))) { 1 } else { 0 };
    args.get(idx + offset).map(|v| v.as_string()).unwrap_or_default()
}

fn int_arg(args: &[Value], idx: usize) -> i64 {
    let offset = if matches!(args.first(), Some(Value::TypeModule(_))) { 1 } else { 0 };
    args.get(idx + offset)
        .and_then(|v| v.as_int().ok())
        .unwrap_or(6)
}

// ---------- gz ----------

pub fn gz_compress(args: &[Value]) -> RuntimeResult<Value> {
    use flate2::{write::GzEncoder, Compression};
    let data = bytes_from_val(data_arg(args));
    let level = int_arg(args, 1).clamp(0, 9) as u32;
    let mut enc = GzEncoder::new(Vec::new(), Compression::new(level));
    enc.write_all(&data)
        .map_err(|e| RuntimeError::Message(e.to_string()))?;
    let out = enc.finish().map_err(|e| RuntimeError::Message(e.to_string()))?;
    Ok(bytes_to_val(out))
}

pub fn gz_decompress(args: &[Value]) -> RuntimeResult<Value> {
    use flate2::read::GzDecoder;
    let data = bytes_from_val(data_arg(args));
    let mut dec = GzDecoder::new(data.as_slice());
    let mut out = Vec::new();
    dec.read_to_end(&mut out)
        .map_err(|e| RuntimeError::Message(e.to_string()))?;
    Ok(bytes_to_val(out))
}

pub fn gz_compress_file(args: &[Value]) -> RuntimeResult<Value> {
    use flate2::{write::GzEncoder, Compression};
    let src = path_arg(args, 0);
    let dst = path_arg(args, 1);
    let level = int_arg(args, 2).clamp(0, 9) as u32;
    let data = std::fs::read(&src).map_err(|e| RuntimeError::Message(format!("{src}: {e}")))?;
    let f = std::fs::File::create(&dst)
        .map_err(|e| RuntimeError::Message(format!("{dst}: {e}")))?;
    let mut enc = GzEncoder::new(f, Compression::new(level));
    enc.write_all(&data)
        .map_err(|e| RuntimeError::Message(e.to_string()))?;
    enc.finish().map_err(|e| RuntimeError::Message(e.to_string()))?;
    Ok(Value::Null)
}

pub fn gz_decompress_file(args: &[Value]) -> RuntimeResult<Value> {
    use flate2::read::GzDecoder;
    let src = path_arg(args, 0);
    let dst = path_arg(args, 1);
    let data = std::fs::read(&src).map_err(|e| RuntimeError::Message(format!("{src}: {e}")))?;
    let mut dec = GzDecoder::new(data.as_slice());
    let mut out = Vec::new();
    dec.read_to_end(&mut out)
        .map_err(|e| RuntimeError::Message(e.to_string()))?;
    std::fs::write(&dst, out).map_err(|e| RuntimeError::Message(format!("{dst}: {e}")))?;
    Ok(Value::Null)
}

// ---------- zstd ----------

pub fn zstd_compress(args: &[Value]) -> RuntimeResult<Value> {
    let data = bytes_from_val(data_arg(args));
    let level = int_arg(args, 1).clamp(1, 22) as i32;
    let out = zstd::bulk::compress(&data, level)
        .map_err(|e| RuntimeError::Message(e.to_string()))?;
    Ok(bytes_to_val(out))
}

pub fn zstd_decompress(args: &[Value]) -> RuntimeResult<Value> {
    let data = bytes_from_val(data_arg(args));
    let out = zstd::bulk::decompress(&data, 64 * 1024 * 1024)
        .map_err(|e| RuntimeError::Message(e.to_string()))?;
    Ok(bytes_to_val(out))
}

pub fn zstd_compress_file(args: &[Value]) -> RuntimeResult<Value> {
    let src = path_arg(args, 0);
    let dst = path_arg(args, 1);
    let level = int_arg(args, 2).clamp(1, 22) as i32;
    let data = std::fs::read(&src).map_err(|e| RuntimeError::Message(format!("{src}: {e}")))?;
    let out = zstd::bulk::compress(&data, level)
        .map_err(|e| RuntimeError::Message(e.to_string()))?;
    std::fs::write(&dst, out).map_err(|e| RuntimeError::Message(format!("{dst}: {e}")))?;
    Ok(Value::Null)
}

pub fn zstd_decompress_file(args: &[Value]) -> RuntimeResult<Value> {
    let src = path_arg(args, 0);
    let dst = path_arg(args, 1);
    let data = std::fs::read(&src).map_err(|e| RuntimeError::Message(format!("{src}: {e}")))?;
    let out = zstd::bulk::decompress(&data, 64 * 1024 * 1024)
        .map_err(|e| RuntimeError::Message(e.to_string()))?;
    std::fs::write(&dst, out).map_err(|e| RuntimeError::Message(format!("{dst}: {e}")))?;
    Ok(Value::Null)
}
