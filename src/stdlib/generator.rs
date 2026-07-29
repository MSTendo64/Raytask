//! bstd generator/coroutine support.
//!
//! Generator in RayTask is an object wrapping a precomputed or lazily-evaluated sequence.
//! The `yield`-style API is provided via a builder pattern:
//!
//!   var gen = Generator.From([1, 2, 3]);
//!   while (gen.HasNext()) { print(gen.Next()); }
//!
//! Or via a range:
//!   var r = Generator.Range(0, 10);      // 0..9
//!   var r2 = Generator.Range(0, 10, 2);  // 0, 2, 4, 6, 8
//!
//! Or infinite (manual advance):
//!   var counter = Generator.Repeat(1);   // 1, 1, 1, ...
//!
//! The generator state is stored in object fields so it survives GC.

use crate::error::{RuntimeError, RuntimeResult};
use crate::value::{ObjectInstance, Value};
use std::collections::HashMap;

fn make_gen(items: Vec<Value>) -> Value {
    let arr = crate::gc::alloc_array(items);
    let mut fields = HashMap::new();
    fields.insert("__items".into(), arr);
    fields.insert("__idx".into(), Value::Int(0));
    fields.insert("__infinite".into(), Value::Bool(false));
    fields.insert("__step".into(), Value::Int(1));
    crate::gc::alloc_object(ObjectInstance {
        class_name: "Generator".into(),
        fields,
        class_index: None,
        finalized: false,
    })
}

fn gen_obj(args: &[Value]) -> Option<&crate::gc::GcObject> {
    for v in args {
        if let Value::Object(o) = v {
            if o.borrow().class_name == "Generator" {
                // We return a raw ref for a borrow-friendly API
                // SAFETY: we only call during the native call, arg lives on the stack
                return Some(unsafe { &*(&**o as *const _) });
            }
        }
    }
    None
}

fn gen_rc(args: &[Value]) -> Option<std::rc::Rc<crate::gc::GcObject>> {
    for v in args {
        if let Value::Object(o) = v {
            if o.borrow().class_name == "Generator" {
                return Some(o.clone());
            }
        }
    }
    None
}

// ---------- factories ----------

/// Generator.From(list) — wraps an existing array
pub fn gen_from(args: &[Value]) -> RuntimeResult<Value> {
    let offset = if matches!(args.first(), Some(Value::TypeModule(_))) { 1 } else { 0 };
    let arr = match args.get(offset) {
        Some(Value::Array(a)) => a.borrow().clone(),
        Some(v) => vec![v.clone()],
        None => vec![],
    };
    Ok(make_gen(arr))
}

/// Generator.Range(start, end [, step]) — integer range
pub fn gen_range(args: &[Value]) -> RuntimeResult<Value> {
    let offset = if matches!(args.first(), Some(Value::TypeModule(_))) { 1 } else { 0 };
    let start = args.get(offset).and_then(|v| v.as_int().ok()).unwrap_or(0);
    let end = args.get(offset + 1).and_then(|v| v.as_int().ok()).unwrap_or(0);
    let step = args.get(offset + 2).and_then(|v| v.as_int().ok()).unwrap_or(1).max(1);
    let mut items = Vec::new();
    let mut i = start;
    while i < end {
        items.push(Value::Int(i));
        i += step;
    }
    Ok(make_gen(items))
}

/// Generator.Repeat(value [, count]) — repeat a value N times (or infinite with count=-1)
pub fn gen_repeat(args: &[Value]) -> RuntimeResult<Value> {
    let offset = if matches!(args.first(), Some(Value::TypeModule(_))) { 1 } else { 0 };
    let val = args.get(offset).cloned().unwrap_or(Value::Null);
    let count = args.get(offset + 1).and_then(|v| v.as_int().ok()).unwrap_or(-1);
    if count < 0 {
        // Infinite generator: store single item, set __infinite = true
        let mut fields = HashMap::new();
        fields.insert("__items".into(), crate::gc::alloc_array(vec![val]));
        fields.insert("__idx".into(), Value::Int(0));
        fields.insert("__infinite".into(), Value::Bool(true));
        fields.insert("__step".into(), Value::Int(1));
        return Ok(crate::gc::alloc_object(ObjectInstance {
            class_name: "Generator".into(),
            fields,
            class_index: None,
            finalized: false,
        }));
    }
    let items = vec![val; count as usize];
    Ok(make_gen(items))
}

/// Generator.Empty() — empty generator
pub fn gen_empty(_args: &[Value]) -> RuntimeResult<Value> {
    Ok(make_gen(vec![]))
}

// ---------- instance methods ----------

/// gen.HasNext() → bool
pub fn gen_has_next(args: &[Value]) -> RuntimeResult<Value> {
    let o = gen_rc(args)
        .ok_or_else(|| RuntimeError::TypeError("Generator.HasNext: expected Generator".into()))?;
    let b = o.borrow();
    let infinite = b
        .fields
        .get("__infinite")
        .map(|v| v.is_truthy())
        .unwrap_or(false);
    if infinite {
        return Ok(Value::Bool(true));
    }
    let idx = b
        .fields
        .get("__idx")
        .and_then(|v| v.as_int().ok())
        .unwrap_or(0);
    let len = match b.fields.get("__items") {
        Some(Value::Array(a)) => a.borrow().len() as i64,
        _ => 0,
    };
    Ok(Value::Bool(idx < len))
}

/// gen.Next() → value or null at end
pub fn gen_next(args: &[Value]) -> RuntimeResult<Value> {
    let o = gen_rc(args)
        .ok_or_else(|| RuntimeError::TypeError("Generator.Next: expected Generator".into()))?;
    let mut b = o.borrow_mut();
    let infinite = b
        .fields
        .get("__infinite")
        .map(|v| v.is_truthy())
        .unwrap_or(false);
    let idx = b
        .fields
        .get("__idx")
        .and_then(|v| v.as_int().ok())
        .unwrap_or(0);
    let item = match b.fields.get("__items") {
        Some(Value::Array(a)) => {
            let arr = a.borrow();
            if infinite {
                arr.first().cloned().unwrap_or(Value::Null)
            } else {
                arr.get(idx as usize).cloned().unwrap_or(Value::Null)
            }
        }
        _ => Value::Null,
    };
    if !infinite {
        b.fields.insert("__idx".into(), Value::Int(idx + 1));
    }
    Ok(item)
}

/// gen.Reset() — restart from beginning
pub fn gen_reset(args: &[Value]) -> RuntimeResult<Value> {
    let o = gen_rc(args)
        .ok_or_else(|| RuntimeError::TypeError("Generator.Reset: expected Generator".into()))?;
    o.borrow_mut()
        .fields
        .insert("__idx".into(), Value::Int(0));
    Ok(Value::Null)
}

/// gen.ToList() — collect all remaining items into a List
pub fn gen_to_list(args: &[Value]) -> RuntimeResult<Value> {
    let o = gen_rc(args)
        .ok_or_else(|| RuntimeError::TypeError("Generator.ToList: expected Generator".into()))?;
    let b = o.borrow();
    let infinite = b
        .fields
        .get("__infinite")
        .map(|v| v.is_truthy())
        .unwrap_or(false);
    if infinite {
        return Err(RuntimeError::Message(
            "Cannot collect infinite Generator to List".into(),
        ));
    }
    let idx = b
        .fields
        .get("__idx")
        .and_then(|v| v.as_int().ok())
        .unwrap_or(0) as usize;
    let items = match b.fields.get("__items") {
        Some(Value::Array(a)) => a.borrow()[idx..].to_vec(),
        _ => vec![],
    };
    Ok(crate::gc::alloc_array(items))
}
