//! bstd.json natives.

use crate::error::{RuntimeError, RuntimeResult};
use crate::value::Value;
use serde_json::Value as JsonValue;
use std::collections::HashMap;

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
    let j: JsonValue = serde_json::from_str(&s)
        .map_err(|e| RuntimeError::Message(format!("JSON parse error: {}", e)))?;
    Ok(from_json(&j))
}

pub fn stringify(args: &[Value]) -> RuntimeResult<Value> {
    let args = skip_module(args);
    let v = args.first().cloned().unwrap_or(Value::Null);
    let pretty = args
        .get(1)
        .map(|x| x.is_truthy())
        .unwrap_or(false);
    let j = to_json(&v);
    let s = if pretty {
        serde_json::to_string_pretty(&j)
    } else {
        serde_json::to_string(&j)
    }
    .map_err(|e| RuntimeError::Message(e.to_string()))?;
    Ok(Value::String(s.into()))
}

/// Serialize a Value to a JSON string (for threads cross-boundary use).
pub fn stringify_raw(v: &Value) -> String {
    serde_json::to_string(&to_json(v)).unwrap_or_else(|_| "null".into())
}

/// Deserialize a serde_json::Value to a RayTask Value.
pub fn json_to_value(j: serde_json::Value) -> Value {
    from_json(&j)
}

fn from_json(j: &JsonValue) -> Value {
    match j {
        JsonValue::Null => Value::Null,
        JsonValue::Bool(b) => Value::Bool(*b),
        JsonValue::Number(n) => {
            if let Some(i) = n.as_i64() {
                Value::Int(i)
            } else if let Some(f) = n.as_f64() {
                Value::Float(f)
            } else {
                Value::Null
            }
        }
        JsonValue::String(s) => Value::String(s.clone().into()),
        JsonValue::Array(a) => {
            let items: Vec<Value> = a.iter().map(from_json).collect();
            crate::gc::alloc_array(items)
        }
        JsonValue::Object(o) => {
            let mut map = HashMap::new();
            for (k, v) in o {
                map.insert(k.clone(), from_json(v));
            }
            crate::gc::alloc_dict(map)
        }
    }
}

fn to_json(v: &Value) -> JsonValue {
    match v {
        Value::Null => JsonValue::Null,
        Value::Bool(b) => JsonValue::Bool(*b),
        Value::Int(n) => JsonValue::Number((*n).into()),
        Value::UInt(n) => JsonValue::Number((*n).into()),
        Value::Float(n) => serde_json::Number::from_f64(*n)
            .map(JsonValue::Number)
            .unwrap_or(JsonValue::Null),
        Value::Char(c) => JsonValue::String(c.to_string()),
        Value::String(s) => JsonValue::String(s.to_string()),
        Value::Array(a) => JsonValue::Array(a.borrow().iter().map(to_json).collect()),
        Value::Dict(d) => {
            let mut map = serde_json::Map::new();
            for (k, v) in d.borrow().iter() {
                map.insert(k.clone(), to_json(v));
            }
            JsonValue::Object(map)
        }
        Value::Object(o) => {
            let o = o.borrow();
            let mut map = serde_json::Map::new();
            for (k, v) in o.fields.iter() {
                map.insert(k.clone(), to_json(v));
            }
            JsonValue::Object(map)
        }
        other => JsonValue::String(other.as_string()),
    }
}
