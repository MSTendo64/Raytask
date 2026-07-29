//! bstd.logging natives.

use crate::error::RuntimeResult;
use crate::value::Value;

fn skip_recv(args: &[Value]) -> &[Value] {
    match args.first() {
        Some(Value::TypeModule(_)) | Some(Value::Object(_)) => &args[1..],
        _ => args,
    }
}

pub fn log(level: &str, args: &[Value]) -> RuntimeResult<Value> {
    let args = skip_recv(args);
    let msg = args
        .iter()
        .map(|a| a.as_string())
        .collect::<Vec<_>>()
        .join(" ");
    crate::debug_io::write_stderr(&format!("[{}] {}", level, msg));
    Ok(Value::Null)
}
 