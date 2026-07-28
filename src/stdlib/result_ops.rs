//! bstd.result natives.

use crate::error::RuntimeResult;
use crate::value::{ObjectInstance, Value};
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

pub fn ok(args: &[Value]) -> RuntimeResult<Value> {
    let mut fields = HashMap::new();
    fields.insert("ok".into(), Value::Bool(true));
    fields.insert(
        "value".into(),
        args.first().cloned().unwrap_or(Value::Null),
    );
    fields.insert("error".into(), Value::Null);
    Ok(crate::gc::alloc_object(ObjectInstance {
        class_name: "Result".into(),
        fields,
        class_index: None,
        finalized: false,
    }))
}

pub fn err(args: &[Value]) -> RuntimeResult<Value> {
    let mut fields = HashMap::new();
    fields.insert("ok".into(), Value::Bool(false));
    fields.insert("value".into(), Value::Null);
    fields.insert(
        "error".into(),
        args.first().cloned().unwrap_or(Value::String("error".into())),
    );
    Ok(crate::gc::alloc_object(ObjectInstance {
        class_name: "Result".into(),
        fields,
        class_index: None,
        finalized: false,
    }))
}
