//! bstd.test natives.

use crate::error::{RuntimeError, RuntimeResult};
use crate::value::Value;

pub fn assert_true(args: &[Value]) -> RuntimeResult<Value> {
    let ok = args.first().map(|v| v.is_truthy()).unwrap_or(false);
    if !ok {
        let msg = args
            .get(1)
            .map(|v| v.as_string())
            .unwrap_or_else(|| "assertion failed".into());
        return Err(RuntimeError::Message(msg));
    }
    Ok(Value::Null)
}

pub fn assert_eq(args: &[Value]) -> RuntimeResult<Value> {
    let a = args.first().cloned().unwrap_or(Value::Null);
    let b = args.get(1).cloned().unwrap_or(Value::Null);
    if !a.equals(&b) {
        return Err(RuntimeError::Message(format!(
            "assertEq failed: {} != {}",
            a.as_string(),
            b.as_string()
        )));
    }
    Ok(Value::Null)
}
