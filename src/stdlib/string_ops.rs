//! bstd.string natives.

use crate::error::{RuntimeError, RuntimeResult};
use crate::value::Value;
use std::cell::RefCell;
use std::rc::Rc;

fn recv_str(args: &[Value]) -> RuntimeResult<String> {
    match args.first() {
        Some(Value::String(s)) => Ok(s.to_string()),
        Some(v) => Ok(v.as_string()),
        None => Err(RuntimeError::TypeError("expected string receiver".into())),
    }
}

pub fn to_upper(args: &[Value]) -> RuntimeResult<Value> {
    Ok(Value::String(recv_str(args)?.to_uppercase().into()))
}

pub fn to_lower(args: &[Value]) -> RuntimeResult<Value> {
    Ok(Value::String(recv_str(args)?.to_lowercase().into()))
}

pub fn trim(args: &[Value]) -> RuntimeResult<Value> {
    Ok(Value::String(recv_str(args)?.trim().into()))
}

pub fn trim_start(args: &[Value]) -> RuntimeResult<Value> {
    Ok(Value::String(recv_str(args)?.trim_start().into()))
}

pub fn trim_end(args: &[Value]) -> RuntimeResult<Value> {
    Ok(Value::String(recv_str(args)?.trim_end().into()))
}

pub fn contains(args: &[Value]) -> RuntimeResult<Value> {
    let s = recv_str(args)?;
    let needle = args.get(1).map(|v| v.as_string()).unwrap_or_default();
    Ok(Value::Bool(s.contains(&needle)))
}

pub fn starts_with(args: &[Value]) -> RuntimeResult<Value> {
    let s = recv_str(args)?;
    let p = args.get(1).map(|v| v.as_string()).unwrap_or_default();
    Ok(Value::Bool(s.starts_with(&p)))
}

pub fn ends_with(args: &[Value]) -> RuntimeResult<Value> {
    let s = recv_str(args)?;
    let p = args.get(1).map(|v| v.as_string()).unwrap_or_default();
    Ok(Value::Bool(s.ends_with(&p)))
}

pub fn index_of(args: &[Value]) -> RuntimeResult<Value> {
    let s = recv_str(args)?;
    let needle = args.get(1).map(|v| v.as_string()).unwrap_or_default();
    Ok(Value::Int(
        s.find(&needle).map(|i| i as i64).unwrap_or(-1),
    ))
}

pub fn replace(args: &[Value]) -> RuntimeResult<Value> {
    let s = recv_str(args)?;
    let a = args.get(1).map(|v| v.as_string()).unwrap_or_default();
    let b = args.get(2).map(|v| v.as_string()).unwrap_or_default();
    Ok(Value::String(s.replace(&a, &b).into()))
}

pub fn substring(args: &[Value]) -> RuntimeResult<Value> {
    let s = recv_str(args)?;
    let start = args.get(1).map(|v| v.as_int()).transpose()?.unwrap_or(0) as usize;
    let chars: Vec<char> = s.chars().collect();
    if start > chars.len() {
        return Ok(Value::String("".into()));
    }
    let result = if let Some(len) = args.get(2) {
        let len = len.as_int()? as usize;
        chars[start..].iter().take(len).collect::<String>()
    } else {
        chars[start..].iter().collect::<String>()
    };
    Ok(Value::String(result.into()))
}

pub fn split(args: &[Value]) -> RuntimeResult<Value> {
    let s = recv_str(args)?;
    let sep = args
        .get(1)
        .map(|v| v.as_string())
        .unwrap_or_else(|| ",".into());
    let parts: Vec<Value> = if sep.is_empty() {
        s.chars()
            .map(|c| Value::String(c.to_string().into()))
            .collect()
    } else {
        s.split(&sep).map(|p| Value::String(p.into())).collect()
    };
    Ok(crate::gc::alloc_array(parts))
}

pub fn join(args: &[Value]) -> RuntimeResult<Value> {
    let (sep, arr) = if args.len() >= 3 {
        (
            args[1].as_string(),
            args.get(2).cloned().unwrap_or(Value::Null),
        )
    } else if args.len() >= 2 {
        (
            args[0].as_string(),
            args.get(1).cloned().unwrap_or(Value::Null),
        )
    } else {
        return Ok(Value::String("".into()));
    };
    let parts = match arr {
        Value::Array(a) => a
            .borrow()
            .iter()
            .map(|v| v.as_string())
            .collect::<Vec<_>>(),
        other => vec![other.as_string()],
    };
    Ok(Value::String(parts.join(&sep).into()))
}

pub fn sb_append(args: &[Value]) -> RuntimeResult<Value> {
    match args.first() {
        Some(Value::Object(o)) => {
            let add = args.get(1).map(|v| v.as_string()).unwrap_or_default();
            let mut obj = o.borrow_mut();
            let cur = obj
                .fields
                .get("buf")
                .map(|v| v.as_string())
                .unwrap_or_default();
            obj.fields
                .insert("buf".into(), Value::String(format!("{}{}", cur, add).into()));
            Ok(Value::Null)
        }
        _ => Err(RuntimeError::TypeError("expected StringBuilder".into())),
    }
}

pub fn sb_to_string(args: &[Value]) -> RuntimeResult<Value> {
    match args.first() {
        Some(Value::Object(o)) => Ok(o
            .borrow()
            .fields
            .get("buf")
            .cloned()
            .unwrap_or(Value::String("".into()))),
        _ => Err(RuntimeError::TypeError("expected StringBuilder".into())),
    }
}

pub fn sb_clear(args: &[Value]) -> RuntimeResult<Value> {
    if let Some(Value::Object(o)) = args.first() {
        o.borrow_mut()
            .fields
            .insert("buf".into(), Value::String("".into()));
    }
    Ok(Value::Null)
}
