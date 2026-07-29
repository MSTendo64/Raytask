//! bstd.fs file streams — sequential read/write with buffering.
//!
//! A Stream object holds an OS file descriptor stashed as a raw pointer
//! inside a string field so it survives GC moves. Streams must be
//! explicitly closed to flush and release the fd.

use crate::error::{RuntimeError, RuntimeResult};
use crate::gc::GcObject;
use crate::value::{ObjectInstance, Value};
use std::collections::HashMap;
use std::fs;
use std::io::{BufRead, BufReader, BufWriter, Read, Seek, SeekFrom, Write};
use std::rc::Rc;
use std::sync::Mutex;

// We store the stream as a heap-allocated Box<Mutex<StreamInner>> whose
// address is kept as a decimal string in the object field "__fd".

enum StreamInner {
    Read(BufReader<fs::File>),
    Write(BufWriter<fs::File>),
}

fn get_stream(obj: &Rc<GcObject>) -> RuntimeResult<*mut Mutex<StreamInner>> {
    let o = obj.borrow();
    match o.fields.get("__fd") {
        Some(Value::String(s)) => {
            let addr: usize = s
                .parse()
                .map_err(|_| RuntimeError::Message("corrupt Stream object".into()))?;
            Ok(addr as *mut Mutex<StreamInner>)
        }
        _ => Err(RuntimeError::Message("invalid Stream object".into())),
    }
}

fn alloc_stream(inner: StreamInner, mode: &str) -> Value {
    let boxed: Box<Mutex<StreamInner>> = Box::new(Mutex::new(inner));
    let addr = Box::into_raw(boxed) as usize;
    let mut fields = HashMap::new();
    fields.insert("__fd".into(), Value::String(addr.to_string().into()));
    fields.insert("Mode".into(), Value::String(mode.into()));
    crate::gc::alloc_object(ObjectInstance {
        class_name: "Stream".into(),
        fields,
        class_index: None,
        finalized: false,
    })
}

fn path_arg(args: &[Value]) -> String {
    let offset = if matches!(args.first(), Some(Value::TypeModule(_))) { 1 } else { 0 };
    args.get(offset).map(|v| v.as_string()).unwrap_or_default()
}

fn stream_arg(args: &[Value]) -> Option<&Rc<GcObject>> {
    for v in args {
        if let Value::Object(o) = v {
            if o.borrow().class_name == "Stream" {
                return Some(o);
            }
        }
    }
    None
}

// ---------- open ----------

pub fn open_read(args: &[Value]) -> RuntimeResult<Value> {
    let p = path_arg(args);
    let f = fs::File::open(&p).map_err(|e| RuntimeError::Message(format!("{p}: {e}")))?;
    Ok(alloc_stream(StreamInner::Read(BufReader::new(f)), "r"))
}

pub fn open_write(args: &[Value]) -> RuntimeResult<Value> {
    let p = path_arg(args);
    let f = fs::OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(&p)
        .map_err(|e| RuntimeError::Message(format!("{p}: {e}")))?;
    Ok(alloc_stream(StreamInner::Write(BufWriter::new(f)), "w"))
}

// ---------- read ----------

pub fn stream_read(args: &[Value]) -> RuntimeResult<Value> {
    let obj = stream_arg(args)
        .ok_or_else(|| RuntimeError::TypeError("Stream.Read: expected Stream".into()))?;
    let ptr = get_stream(obj)?;
    let count = args
        .iter()
        .filter_map(|v| v.as_int().ok())
        .next()
        .unwrap_or(4096) as usize;
    // SAFETY: pointer valid as long as object alive
    let guard = unsafe { &*ptr }
        .lock()
        .map_err(|_| RuntimeError::Message("Stream poisoned".into()))?;
    let inner = &*guard;
    match inner {
        StreamInner::Read(_) => {}
        StreamInner::Write(_) => {
            return Err(RuntimeError::Message(
                "Stream.Read called on write-mode stream".into(),
            ))
        }
    }
    drop(guard);
    let mut mg = unsafe { &*ptr }
        .lock()
        .map_err(|_| RuntimeError::Message("Stream poisoned".into()))?;
    if let StreamInner::Read(ref mut br) = *mg {
        let mut buf = vec![0u8; count];
        let n = br
            .read(&mut buf)
            .map_err(|e| RuntimeError::Message(e.to_string()))?;
        buf.truncate(n);
        let vals: Vec<Value> = buf.into_iter().map(|b| Value::Int(b as i64)).collect();
        Ok(crate::gc::alloc_array(vals))
    } else {
        unreachable!()
    }
}

pub fn stream_read_line(args: &[Value]) -> RuntimeResult<Value> {
    let obj = stream_arg(args)
        .ok_or_else(|| RuntimeError::TypeError("Stream.ReadLine: expected Stream".into()))?;
    let ptr = get_stream(obj)?;
    let mut mg = unsafe { &*ptr }
        .lock()
        .map_err(|_| RuntimeError::Message("Stream poisoned".into()))?;
    if let StreamInner::Read(ref mut br) = *mg {
        let mut line = String::new();
        let n = br
            .read_line(&mut line)
            .map_err(|e| RuntimeError::Message(e.to_string()))?;
        if n == 0 {
            return Ok(Value::Null); // EOF
        }
        if line.ends_with('\n') {
            line.pop();
            if line.ends_with('\r') {
                line.pop();
            }
        }
        Ok(Value::String(line.into()))
    } else {
        Err(RuntimeError::Message(
            "Stream.ReadLine called on write-mode stream".into(),
        ))
    }
}

// ---------- write ----------

pub fn stream_write(args: &[Value]) -> RuntimeResult<Value> {
    let obj = stream_arg(args)
        .ok_or_else(|| RuntimeError::TypeError("Stream.Write: expected Stream".into()))?;
    let ptr = get_stream(obj)?;
    // collect data: bytes array or string
    let data: Vec<u8> = args
        .iter()
        .find(|v| !matches!(v, Value::Object(_)))
        .map(|v| match v {
            Value::Array(a) => a
                .borrow()
                .iter()
                .filter_map(|x| x.as_int().ok().map(|n| n as u8))
                .collect(),
            _ => v.as_string().into_bytes(),
        })
        .unwrap_or_default();

    let mut mg = unsafe { &*ptr }
        .lock()
        .map_err(|_| RuntimeError::Message("Stream poisoned".into()))?;
    if let StreamInner::Write(ref mut bw) = *mg {
        bw.write_all(&data)
            .map_err(|e| RuntimeError::Message(e.to_string()))?;
        Ok(Value::Int(data.len() as i64))
    } else {
        Err(RuntimeError::Message(
            "Stream.Write called on read-mode stream".into(),
        ))
    }
}

pub fn stream_write_line(args: &[Value]) -> RuntimeResult<Value> {
    let obj = stream_arg(args)
        .ok_or_else(|| RuntimeError::TypeError("Stream.WriteLine: expected Stream".into()))?;
    let ptr = get_stream(obj)?;
    let text = args
        .iter()
        .find(|v| !matches!(v, Value::Object(_)))
        .map(|v| v.as_string())
        .unwrap_or_default();
    let mut mg = unsafe { &*ptr }
        .lock()
        .map_err(|_| RuntimeError::Message("Stream poisoned".into()))?;
    if let StreamInner::Write(ref mut bw) = *mg {
        writeln!(bw, "{text}").map_err(|e| RuntimeError::Message(e.to_string()))?;
        Ok(Value::Null)
    } else {
        Err(RuntimeError::Message(
            "Stream.WriteLine called on read-mode stream".into(),
        ))
    }
}

// ---------- seek ----------

pub fn stream_seek(args: &[Value]) -> RuntimeResult<Value> {
    let obj = stream_arg(args)
        .ok_or_else(|| RuntimeError::TypeError("Stream.Seek: expected Stream".into()))?;
    let ptr = get_stream(obj)?;
    let pos = args
        .iter()
        .filter_map(|v| v.as_int().ok())
        .next()
        .unwrap_or(0) as u64;
    let mut mg = unsafe { &*ptr }
        .lock()
        .map_err(|_| RuntimeError::Message("Stream poisoned".into()))?;
    let new_pos = match *mg {
        StreamInner::Read(ref mut br) => br
            .seek(SeekFrom::Start(pos))
            .map_err(|e| RuntimeError::Message(e.to_string()))?,
        StreamInner::Write(ref mut bw) => bw
            .seek(SeekFrom::Start(pos))
            .map_err(|e| RuntimeError::Message(e.to_string()))?,
    };
    Ok(Value::Int(new_pos as i64))
}

// ---------- flush / close ----------

pub fn stream_flush(args: &[Value]) -> RuntimeResult<Value> {
    let obj = stream_arg(args)
        .ok_or_else(|| RuntimeError::TypeError("Stream.Flush: expected Stream".into()))?;
    let ptr = get_stream(obj)?;
    let mut mg = unsafe { &*ptr }
        .lock()
        .map_err(|_| RuntimeError::Message("Stream poisoned".into()))?;
    if let StreamInner::Write(ref mut bw) = *mg {
        bw.flush().map_err(|e| RuntimeError::Message(e.to_string()))?;
    }
    Ok(Value::Null)
}

pub fn stream_close(args: &[Value]) -> RuntimeResult<Value> {
    let obj = stream_arg(args)
        .ok_or_else(|| RuntimeError::TypeError("Stream.Close: expected Stream".into()))?;
    let ptr = get_stream(obj)?;
    // Flush write streams before dropping
    {
        let mut mg = unsafe { &*ptr }
            .lock()
            .map_err(|_| RuntimeError::Message("Stream poisoned".into()))?;
        if let StreamInner::Write(ref mut bw) = *mg {
            let _ = bw.flush();
        }
    }
    // Drop the Box, releasing fd
    unsafe { drop(Box::from_raw(ptr)) };
    // Null out the field so double-close is a no-op
    if let Value::Object(o) = args.iter().find(|v| matches!(v, Value::Object(_))).unwrap() {
        o.borrow_mut().fields.insert("__fd".into(), Value::String("0".into()));
    }
    Ok(Value::Null)
}
