//! bstd.time natives.

use crate::error::RuntimeResult;
use crate::value::{ObjectInstance, Value};
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::time::{SystemTime, UNIX_EPOCH};

pub fn get_time_ms() -> RuntimeResult<Value> {
    let ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis();
    Ok(Value::Int(ms as i64))
}

pub fn now(utc: bool) -> RuntimeResult<Value> {
    let ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64;
    let mut fields = HashMap::new();
    fields.insert("Ticks".into(), Value::Int(ms));
    fields.insert("Utc".into(), Value::Bool(utc));
    Ok(crate::gc::alloc_object(ObjectInstance {
        class_name: "DateTime".into(),
        fields,
        class_index: None,
        finalized: false,
    }))
}

pub fn dt_to_string(args: &[Value]) -> RuntimeResult<Value> {
    match args.first() {
        Some(Value::Object(o)) => {
            let ticks = o
                .borrow()
                .fields
                .get("Ticks")
                .map(|v| v.as_string())
                .unwrap_or_else(|| "0".into());
            Ok(Value::String(format!("DateTime({})", ticks).into()))
        }
        Some(v) => Ok(Value::String(v.as_string().into())),
        None => Ok(Value::String("DateTime".into())),
    }
}
