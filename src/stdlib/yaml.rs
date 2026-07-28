//! bstd.yml natives.

use crate::error::{RuntimeError, RuntimeResult};
use crate::value::Value;
use serde_json::Value as JsonValue;

fn skip_module(args: &[Value]) -> &[Value] {
    if matches!(args.first(), Some(Value::TypeModule(_))) {
        &args[1..]
    } else {
        args
    }
}

pub fn parse(args: &[Value]) -> RuntimeResult<Value> {
    let args = skip_module(args);
    let s = args.first().map(|v| v.as_string()).unwrap_or_default();
    // Parse YAML into JSON Value then reuse json conversion via stringify path
    let j: JsonValue = serde_yaml::from_str(&s)
        .map_err(|e| RuntimeError::Message(format!("YAML parse error: {}", e)))?;
    // Convert via JSON string to reuse json::from path — call json helpers indirectly
    let tmp = serde_json::to_string(&j).map_err(|e| RuntimeError::Message(e.to_string()))?;
    crate::stdlib::json::parse(&[Value::String(tmp.into())])
}

pub fn serialize(args: &[Value]) -> RuntimeResult<Value> {
    let args = skip_module(args);
    let v = args.first().cloned().unwrap_or(Value::Null);
    // Convert Value -> JSON -> YAML
    let json_s = crate::stdlib::json::stringify(&[v])?;
    let j: JsonValue = serde_json::from_str(&json_s.as_string())
        .map_err(|e| RuntimeError::Message(e.to_string()))?;
    let y = serde_yaml::to_string(&j).map_err(|e| RuntimeError::Message(e.to_string()))?;
    Ok(Value::String(y.into()))
}
