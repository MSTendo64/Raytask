//! bstd.regex natives.

use crate::error::{RuntimeError, RuntimeResult};
use crate::value::{ObjectInstance, Value};
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

pub fn regex_new(args: &[Value]) -> RuntimeResult<Value> {
    let pat = args.first().map(|v| v.as_string()).unwrap_or_default();
    // Validate
    regex::Regex::new(&pat).map_err(|e| RuntimeError::Message(format!("invalid regex: {}", e)))?;
    let mut fields = HashMap::new();
    fields.insert("pattern".into(), Value::String(pat.into()));
    Ok(crate::gc::alloc_object(ObjectInstance {
        class_name: "Regex".into(),
        fields,
        class_index: None,
        finalized: false,
    }))
}

fn pattern_of(args: &[Value]) -> RuntimeResult<String> {
    match args.first() {
        Some(Value::Object(o)) => Ok(o
            .borrow()
            .fields
            .get("pattern")
            .map(|v| v.as_string())
            .unwrap_or_default()),
        Some(Value::String(s)) => Ok(s.to_string()),
        _ => Err(RuntimeError::TypeError("expected Regex".into())),
    }
}

pub fn find_all(args: &[Value]) -> RuntimeResult<Value> {
    let pat = pattern_of(args)?;
    let text = args.get(1).map(|v| v.as_string()).unwrap_or_default();
    let re = regex::Regex::new(&pat).map_err(|e| RuntimeError::Message(e.to_string()))?;
    let matches: Vec<Value> = re
        .find_iter(&text)
        .map(|m| Value::String(m.as_str().into()))
        .collect();
    Ok(crate::gc::alloc_array(matches))
}

pub fn is_match(args: &[Value]) -> RuntimeResult<Value> {
    let pat = pattern_of(args)?;
    let text = args.get(1).map(|v| v.as_string()).unwrap_or_default();
    let re = regex::Regex::new(&pat).map_err(|e| RuntimeError::Message(e.to_string()))?;
    Ok(Value::Bool(re.is_match(&text)))
}

pub fn replace(args: &[Value]) -> RuntimeResult<Value> {
    let pat = pattern_of(args)?;
    let text = args.get(1).map(|v| v.as_string()).unwrap_or_default();
    let rep = args.get(2).map(|v| v.as_string()).unwrap_or_default();
    let re = regex::Regex::new(&pat).map_err(|e| RuntimeError::Message(e.to_string()))?;
    Ok(Value::String(re.replace_all(&text, rep.as_str()).into_owned().into()))
}
