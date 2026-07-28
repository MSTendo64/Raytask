//! bstd.unsafe memory natives (arena-backed).

use crate::error::{RuntimeError, RuntimeResult};
use crate::value::Value;
use std::cell::RefCell;
use std::collections::HashMap;

thread_local! {
    static HEAP: RefCell<HashMap<usize, Vec<u8>>> = RefCell::new(HashMap::new());
    static NEXT: RefCell<usize> = RefCell::new(1);
}

pub fn malloc(args: &[Value]) -> RuntimeResult<Value> {
    let size = args.first().map(|v| v.as_int()).transpose()?.unwrap_or(0) as usize;
    let ptr = NEXT.with(|n| {
        let mut n = n.borrow_mut();
        let id = *n;
        *n += 1;
        id
    });
    HEAP.with(|h| {
        h.borrow_mut().insert(ptr, vec![0u8; size]);
    });
    Ok(Value::Ptr(ptr))
}

pub fn free(args: &[Value]) -> RuntimeResult<Value> {
    if let Some(Value::Ptr(p)) = args.first() {
        HEAP.with(|h| {
            h.borrow_mut().remove(p);
        });
    }
    Ok(Value::Null)
}

pub fn sizeof_val(args: &[Value]) -> RuntimeResult<Value> {
    let n = match args.first() {
        Some(Value::Ptr(p)) => HEAP.with(|h| h.borrow().get(p).map(|b| b.len()).unwrap_or(0)),
        Some(Value::String(s)) => s.len(),
        Some(Value::Array(a)) => a.borrow().len(),
        Some(Value::Int(_)) => 8,
        Some(Value::Float(_)) => 8,
        Some(Value::Bool(_)) => 1,
        _ => 0,
    };
    Ok(Value::Int(n as i64))
}

#[allow(dead_code)]
pub fn read_byte(ptr: usize, offset: usize) -> RuntimeResult<u8> {
    HEAP.with(|h| {
        h.borrow()
            .get(&ptr)
            .and_then(|b| b.get(offset).copied())
            .ok_or(RuntimeError::NullReference)
    })
}
