//! Runtime reflection natives (`Type` / `bstd.reflect`).

use crate::error::{RuntimeError, RuntimeResult};
use crate::value::{TypeHandle, Value};
use std::rc::Rc;

/// `Type.Foo(...)` is compiled as an instance-style call with `TypeModule`/`Type` as `this`.
pub fn strip_type_receiver_pub(args: &[Value]) -> &[Value] {
    strip_type_receiver(args)
}

fn strip_type_receiver(args: &[Value]) -> &[Value] {
    match args.first() {
        Some(Value::TypeModule(n)) if n.as_ref() == "Type" || n.as_ref() == "Reflect" => {
            &args[1..]
        }
        Some(Value::Type(_)) => &args[1..],
        _ => args,
    }
}

pub fn type_of(args: &[Value]) -> RuntimeResult<Value> {
    let args = strip_type_receiver(args);
    let v = args.first().cloned().unwrap_or(Value::Null);
    Ok(Value::Type(Rc::new(type_handle_of(&v))))
}

pub fn type_handle_of(v: &Value) -> TypeHandle {
    match v {
        Value::Null => TypeHandle::primitive("null"),
        Value::Bool(_) => TypeHandle::primitive("bool"),
        Value::Int(_) => TypeHandle::primitive("int"),
        Value::UInt(_) => TypeHandle::primitive("uint"),
        Value::Float(_) => TypeHandle::primitive("double"),
        Value::Char(_) => TypeHandle::primitive("char"),
        Value::String(_) => TypeHandle::primitive("string"),
        Value::Array(_) => TypeHandle {
            name: "array".into(),
            kind: "array".into(),
            class_index: None,
            fields: Vec::new(),
            field_types: Vec::new(),
            methods: Vec::new(),
        },
        Value::Dict(_) => TypeHandle {
            name: "dictionary".into(),
            kind: "class".into(),
            class_index: None,
            fields: Vec::new(),
            field_types: Vec::new(),
            methods: Vec::new(),
        },
        Value::Object(o) => {
            let o = o.borrow();
            TypeHandle {
                name: o.class_name.clone(),
                kind: "class".into(),
                class_index: o.class_index,
                fields: o.fields.keys().cloned().collect(),
                field_types: Vec::new(),
                methods: o
                    .fields
                    .iter()
                    .filter(|(_, v)| matches!(v, Value::Function(_)))
                    .map(|(k, _)| k.clone())
                    .collect(),
            }
        }
        Value::Function(_) => TypeHandle {
            name: "function".into(),
            kind: "primitive".into(),
            class_index: None,
            fields: Vec::new(),
            field_types: Vec::new(),
            methods: Vec::new(),
        },
        Value::Native(_) => TypeHandle::primitive("native"),
        Value::TypeModule(n) => TypeHandle {
            name: n.to_string(),
            kind: "type".into(),
            class_index: None,
            fields: Vec::new(),
            field_types: Vec::new(),
            methods: Vec::new(),
        },
        Value::Type(t) => (**t).clone(),
        Value::Task(_) => TypeHandle {
            name: "Task".into(),
            kind: "class".into(),
            class_index: None,
            fields: Vec::new(),
            field_types: Vec::new(),
            methods: Vec::new(),
        },
        Value::Ffi(_) => TypeHandle::primitive("ffi"),
        Value::Ptr(_) => TypeHandle::primitive("ptr"),
    }
}

pub fn get_field(args: &[Value]) -> RuntimeResult<Value> {
    let args = strip_type_receiver(args);
    let obj = args
        .first()
        .ok_or_else(|| RuntimeError::Message("Type.GetField(obj, name)".into()))?;
    let name = args
        .get(1)
        .map(|v| v.as_string())
        .ok_or_else(|| RuntimeError::Message("Type.GetField requires a field name".into()))?;
    match obj {
        Value::Object(o) => Ok(o
            .borrow()
            .fields
            .get(&name)
            .cloned()
            .unwrap_or(Value::Null)),
        Value::Dict(d) => Ok(d.borrow().get(&name).cloned().unwrap_or(Value::Null)),
        Value::Type(t) => match name.as_str() {
            "Name" => Ok(Value::String(t.name.clone().into())),
            "Kind" => Ok(Value::String(t.kind.clone().into())),
            _ => Ok(Value::Null),
        },
        _ => Err(RuntimeError::TypeError(
            "Type.GetField expects an object".into(),
        )),
    }
}

pub fn set_field(args: &[Value]) -> RuntimeResult<Value> {
    let args = strip_type_receiver(args);
    let obj = args
        .first()
        .ok_or_else(|| RuntimeError::Message("Type.SetField(obj, name, value)".into()))?;
    let name = args
        .get(1)
        .map(|v| v.as_string())
        .ok_or_else(|| RuntimeError::Message("Type.SetField requires a field name".into()))?;
    let value = args.get(2).cloned().unwrap_or(Value::Null);
    match obj {
        Value::Object(o) => {
            o.borrow_mut().fields.insert(name, value);
            Ok(Value::Null)
        }
        Value::Dict(d) => {
            d.borrow_mut().insert(name, value);
            Ok(Value::Null)
        }
        _ => Err(RuntimeError::TypeError(
            "Type.SetField expects an object".into(),
        )),
    }
}

pub fn is_instance(args: &[Value]) -> RuntimeResult<Value> {
    let args = strip_type_receiver(args);
    // Type.IsInstance(obj, type) — type may be Type handle or string name.
    let obj = args.first().cloned().unwrap_or(Value::Null);
    let ty = args.get(1).cloned().unwrap_or(Value::Null);
    let ok = match (&obj, &ty) {
        (Value::Object(o), Value::Type(t)) => {
            let o = o.borrow();
            o.class_name == t.name
                || t.class_index.is_some_and(|i| o.class_index == Some(i))
        }
        (Value::Object(o), Value::String(s)) => o.borrow().class_name == s.as_ref(),
        (v, Value::Type(t)) => v.type_name() == t.name,
        (v, Value::String(s)) => v.type_name() == s.as_ref(),
        _ => false,
    };
    Ok(Value::Bool(ok))
}

pub fn fields_list(t: &TypeHandle) -> Value {
    let items: Vec<Value> = t
        .fields
        .iter()
        .enumerate()
        .map(|(i, name)| {
            let ty = t
                .field_types
                .get(i)
                .cloned()
                .unwrap_or_else(|| "dyn".into());
            // Return field name string for simple List; richer later.
            let _ = ty;
            Value::String(name.clone().into())
        })
        .collect();
    crate::gc::alloc_array(items)
}

pub fn methods_list(t: &TypeHandle) -> Value {
    let items: Vec<Value> = t
        .methods
        .iter()
        .map(|n| Value::String(n.clone().into()))
        .collect();
    crate::gc::alloc_array(items)
}

/// Resolve a method Function from an object instance (installed on fields).
pub fn find_method(obj: &Value, name: &str) -> RuntimeResult<crate::value::FunctionRef> {
    match obj {
        Value::Object(o) => {
            let o = o.borrow();
            match o.fields.get(name) {
                Some(Value::Function(f)) => Ok(f.clone()),
                _ => Err(RuntimeError::Message(format!(
                    "method '{}' not found on {}",
                    name, o.class_name
                ))),
            }
        }
        _ => Err(RuntimeError::TypeError(
            "Type.Invoke expects an object receiver".into(),
        )),
    }
}
