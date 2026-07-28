//! bstd.io natives and global helpers.

use crate::error::{RuntimeError, RuntimeResult};
use crate::value::Value;
use std::io::{self, Write};

pub fn print_ln(args: &[Value]) -> RuntimeResult<Value> {
    let msg = args.iter().map(|a| a.as_string()).collect::<Vec<_>>().join(" ");
    println!("{}", msg);
    Ok(Value::Null)
}

pub fn write(args: &[Value]) -> RuntimeResult<Value> {
    let msg = args.iter().map(|a| a.as_string()).collect::<Vec<_>>().join(" ");
    print!("{}", msg);
    let _ = io::stdout().flush();
    Ok(Value::Null)
}

pub fn read_line() -> RuntimeResult<Value> {
    let mut line = String::new();
    io::stdin()
        .read_line(&mut line)
        .map_err(|e| RuntimeError::Message(e.to_string()))?;
    if line.ends_with('\n') {
        line.pop();
        if line.ends_with('\r') {
            line.pop();
        }
    }
    Ok(Value::String(line.into()))
}

pub fn read_key() -> RuntimeResult<Value> {
    // Portable fallback: read one line and take first char
    let mut line = String::new();
    io::stdin()
        .read_line(&mut line)
        .map_err(|e| RuntimeError::Message(e.to_string()))?;
    Ok(Value::Char(line.chars().next().unwrap_or('\0')))
}

pub fn sleep(args: &[Value]) -> RuntimeResult<Value> {
    let ms = args.first().map(|v| v.as_int()).transpose()?.unwrap_or(0);
    std::thread::sleep(std::time::Duration::from_millis(ms.max(0) as u64));
    Ok(Value::Null)
}

pub fn parse_int(args: &[Value]) -> RuntimeResult<Value> {
    let s = args.first().map(|v| v.as_string()).unwrap_or_default();
    let n: i64 = s
        .trim()
        .parse()
        .map_err(|_| RuntimeError::TypeError(format!("cannot parse '{}' as int", s)))?;
    Ok(Value::Int(n))
}

pub fn parse_float(args: &[Value]) -> RuntimeResult<Value> {
    let s = args.first().map(|v| v.as_string()).unwrap_or_default();
    let n: f64 = s
        .trim()
        .parse()
        .map_err(|_| RuntimeError::TypeError(format!("cannot parse '{}' as float", s)))?;
    Ok(Value::Float(n))
}

pub fn is_numeric(args: &[Value]) -> RuntimeResult<Value> {
    let s = args.first().map(|v| v.as_string()).unwrap_or_default();
    Ok(Value::Bool(s.trim().parse::<f64>().is_ok()))
}

pub fn is_alpha(args: &[Value]) -> RuntimeResult<Value> {
    let s = args.first().map(|v| v.as_string()).unwrap_or_default();
    Ok(Value::Bool(!s.is_empty() && s.chars().all(|c| c.is_alphabetic())))
}

pub fn is_email(args: &[Value]) -> RuntimeResult<Value> {
    let s = args.first().map(|v| v.as_string()).unwrap_or_default();
    let ok = s.contains('@')
        && s.split('@').count() == 2
        && s.split('@').nth(1).map(|d| d.contains('.')).unwrap_or(false);
    Ok(Value::Bool(ok))
}
