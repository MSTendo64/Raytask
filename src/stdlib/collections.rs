//! bstd.collections natives.

use crate::error::{RuntimeError, RuntimeResult};
use crate::value::Value;

fn recv_array(args: &[Value]) -> RuntimeResult<std::rc::Rc<crate::gc::GcArray>> {
    match args.first() {
        Some(Value::Array(a)) => Ok(a.clone()),
        _ => Err(RuntimeError::TypeError("expected List/array receiver".into())),
    }
}

fn recv_obj_items(
    args: &[Value],
) -> RuntimeResult<(Value, std::rc::Rc<crate::gc::GcArray>)> {
    match args.first() {
        Some(obj @ Value::Object(o)) => {
            let items = o
                .borrow()
                .fields
                .get("items")
                .cloned()
                .ok_or_else(|| RuntimeError::TypeError("collection missing items".into()))?;
            match items {
                Value::Array(a) => Ok((obj.clone(), a)),
                _ => Err(RuntimeError::TypeError("items is not array".into())),
            }
        }
        _ => Err(RuntimeError::TypeError("expected collection receiver".into())),
    }
}

pub fn list_add(args: &[Value]) -> RuntimeResult<Value> {
    let a = recv_array(args)?;
    let item = args.get(1).cloned().unwrap_or(Value::Null);
    a.borrow_mut().push(item);
    Ok(Value::Null)
}

pub fn list_get(args: &[Value]) -> RuntimeResult<Value> {
    let a = recv_array(args)?;
    let i = args.get(1).map(|v| v.as_int()).transpose()?.unwrap_or(0) as usize;
    let result = a.borrow().get(i).cloned().ok_or(RuntimeError::IndexOutOfRange)?;
    Ok(result)
}

pub fn list_remove_at(args: &[Value]) -> RuntimeResult<Value> {
    let a = recv_array(args)?;
    let i = args.get(1).map(|v| v.as_int()).transpose()?.unwrap_or(0) as usize;
    let mut arr = a.borrow_mut();
    if i >= arr.len() {
        return Err(RuntimeError::IndexOutOfRange);
    }
    arr.remove(i);
    Ok(Value::Null)
}

pub fn list_contains(args: &[Value]) -> RuntimeResult<Value> {
    let a = recv_array(args)?;
    let item = args.get(1).cloned().unwrap_or(Value::Null);
    let found = a.borrow().iter().any(|v| v.equals(&item));
    Ok(Value::Bool(found))
}

pub fn list_clear(args: &[Value]) -> RuntimeResult<Value> {
    recv_array(args)?.borrow_mut().clear();
    Ok(Value::Null)
}

pub fn list_sum(args: &[Value]) -> RuntimeResult<Value> {
    let a = recv_array(args)?;
    let mut sum = 0.0;
    let mut all_int = true;
    for v in a.borrow().iter() {
        if !matches!(v, Value::Int(_) | Value::UInt(_)) {
            all_int = false;
        }
        sum += v.as_float()?;
    }
    if all_int {
        Ok(Value::Int(sum as i64))
    } else {
        Ok(Value::Float(sum))
    }
}

pub fn list_average(args: &[Value]) -> RuntimeResult<Value> {
    let a = recv_array(args)?;
    let n = a.borrow().len();
    if n == 0 {
        return Ok(Value::Float(0.0));
    }
    let sum = list_sum(args)?;
    Ok(Value::Float(sum.as_float()? / n as f64))
}

pub fn list_max(args: &[Value]) -> RuntimeResult<Value> {
    let a = recv_array(args)?;
    let arr = a.borrow();
    let first = arr.first().cloned().ok_or(RuntimeError::IndexOutOfRange)?;
    let mut best = first.as_float()?;
    let mut best_v = first;
    for v in arr.iter().skip(1) {
        let f = v.as_float()?;
        if f > best {
            best = f;
            best_v = v.clone();
        }
    }
    drop(arr);
    Ok(best_v)
}

pub fn list_min(args: &[Value]) -> RuntimeResult<Value> {
    let a = recv_array(args)?;
    let arr = a.borrow();
    let first = arr.first().cloned().ok_or(RuntimeError::IndexOutOfRange)?;
    let mut best = first.as_float()?;
    let mut best_v = first;
    for v in arr.iter().skip(1) {
        let f = v.as_float()?;
        if f < best {
            best = f;
            best_v = v.clone();
        }
    }
    drop(arr);
    Ok(best_v)
}

pub fn list_first(args: &[Value]) -> RuntimeResult<Value> {
    let a = recv_array(args)?;
    let result = a.borrow().first().cloned().ok_or(RuntimeError::IndexOutOfRange)?;
    Ok(result)
}

pub fn list_last(args: &[Value]) -> RuntimeResult<Value> {
    let a = recv_array(args)?;
    let result = a.borrow().last().cloned().ok_or(RuntimeError::IndexOutOfRange)?;
    Ok(result)
}

pub fn list_linq_stub(id: usize, args: &[Value]) -> RuntimeResult<Value> {
    let a = recv_array(args)?;
    match id {
        crate::stdlib::ids::LIST_ANY => {
            if args.len() < 2 {
                let empty = a.borrow().is_empty();
                return Ok(Value::Bool(!empty));
            }
            Err(RuntimeError::Message(
                "List.Any(predicate) requires callable support in this runtime".into(),
            ))
        }
        crate::stdlib::ids::LIST_ALL => {
            if args.len() < 2 {
                return Ok(Value::Bool(true));
            }
            Err(RuntimeError::Message(
                "List.All(predicate) requires callable support in this runtime".into(),
            ))
        }
        _ => Err(RuntimeError::Message(
            "List.Where/Select with lambdas not yet wired in natives".into(),
        )),
    }
}

pub fn dict_contains_key(args: &[Value]) -> RuntimeResult<Value> {
    let d = match args.first() {
        Some(Value::Dict(d)) => d.clone(),
        _ => return Err(RuntimeError::TypeError("expected Dictionary".into())),
    };
    let key = args.get(1).map(|v| v.as_string()).unwrap_or_default();
    let found = d.borrow().contains_key(&key);
    Ok(Value::Bool(found))
}

pub fn dict_remove(args: &[Value]) -> RuntimeResult<Value> {
    let d = match args.first() {
        Some(Value::Dict(d)) => d.clone(),
        _ => return Err(RuntimeError::TypeError("expected Dictionary".into())),
    };
    let key = args.get(1).map(|v| v.as_string()).unwrap_or_default();
    d.borrow_mut().remove(&key);
    Ok(Value::Null)
}

pub fn dict_clear(args: &[Value]) -> RuntimeResult<Value> {
    if let Some(Value::Dict(d)) = args.first() {
        d.borrow_mut().clear();
    }
    Ok(Value::Null)
}

pub fn dict_keys(args: &[Value]) -> RuntimeResult<Value> {
    let d = match args.first() {
        Some(Value::Dict(d)) => d.clone(),
        _ => return Err(RuntimeError::TypeError("expected Dictionary".into())),
    };
    let keys: Vec<Value> = d
        .borrow()
        .keys()
        .map(|k| Value::String(k.clone().into()))
        .collect();
    Ok(crate::gc::alloc_array(keys))
}

pub fn dict_values(args: &[Value]) -> RuntimeResult<Value> {
    let d = match args.first() {
        Some(Value::Dict(d)) => d.clone(),
        _ => return Err(RuntimeError::TypeError("expected Dictionary".into())),
    };
    let vals: Vec<Value> = d.borrow().values().cloned().collect();
    Ok(crate::gc::alloc_array(vals))
}

pub fn set_add(args: &[Value]) -> RuntimeResult<Value> {
    let (_, items) = recv_obj_items(args)?;
    let item = args.get(1).cloned().unwrap_or(Value::Null);
    let exists = items.borrow().iter().any(|v| v.equals(&item));
    if !exists {
        items.borrow_mut().push(item);
    }
    Ok(Value::Null)
}

pub fn set_contains(args: &[Value]) -> RuntimeResult<Value> {
    let (_, items) = recv_obj_items(args)?;
    let item = args.get(1).cloned().unwrap_or(Value::Null);
    let found = items.borrow().iter().any(|v| v.equals(&item));
    Ok(Value::Bool(found))
}

pub fn set_remove(args: &[Value]) -> RuntimeResult<Value> {
    let (_, items) = recv_obj_items(args)?;
    let item = args.get(1).cloned().unwrap_or(Value::Null);
    items.borrow_mut().retain(|v| !v.equals(&item));
    Ok(Value::Null)
}

pub fn queue_enqueue(args: &[Value]) -> RuntimeResult<Value> {
    let (_, items) = recv_obj_items(args)?;
    items
        .borrow_mut()
        .push(args.get(1).cloned().unwrap_or(Value::Null));
    Ok(Value::Null)
}

pub fn queue_dequeue(args: &[Value]) -> RuntimeResult<Value> {
    let (_, items) = recv_obj_items(args)?;
    if items.borrow().is_empty() {
        return Err(RuntimeError::IndexOutOfRange);
    }
    let v = items.borrow_mut().remove(0);
    Ok(v)
}

pub fn queue_peek(args: &[Value]) -> RuntimeResult<Value> {
    let (_, items) = recv_obj_items(args)?;
    let result = items
        .borrow()
        .first()
        .cloned()
        .ok_or(RuntimeError::IndexOutOfRange)?;
    Ok(result)
}

pub fn stack_push(args: &[Value]) -> RuntimeResult<Value> {
    let (_, items) = recv_obj_items(args)?;
    items
        .borrow_mut()
        .push(args.get(1).cloned().unwrap_or(Value::Null));
    Ok(Value::Null)
}

pub fn stack_pop(args: &[Value]) -> RuntimeResult<Value> {
    let (_, items) = recv_obj_items(args)?;
    let v = items.borrow_mut().pop().ok_or(RuntimeError::IndexOutOfRange)?;
    Ok(v)
}

pub fn stack_peek(args: &[Value]) -> RuntimeResult<Value> {
    let (_, items) = recv_obj_items(args)?;
    let result = items
        .borrow()
        .last()
        .cloned()
        .ok_or(RuntimeError::IndexOutOfRange)?;
    Ok(result)
}
