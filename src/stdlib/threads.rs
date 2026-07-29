//! bstd.threads — Mutex and Channel natives.
//!
//! Mutex wraps an Arc<std::sync::Mutex<Value>> stored as an opaque Object field.
//! Channel wraps Arc<(Mutex<VecDeque<Value>>, Condvar)> the same way.
//! Both survive being passed across thread boundaries because we serialize Value
//! to/from JSON on the boundary (RayTask Values are not Send).

use crate::error::{RuntimeError, RuntimeResult};
use crate::gc::GcObject;
use crate::value::{ObjectInstance, Value};
use std::collections::HashMap;
use std::collections::VecDeque;
use std::rc::Rc;
use std::sync::{Arc, Condvar, Mutex};

// ---------- helpers ----------

fn get_arc_mutex(obj: &Rc<GcObject>) -> Option<Arc<Mutex<serde_json::Value>>> {
    let o = obj.borrow();
    match o.fields.get("__arc")? {
        Value::String(s) => {
            // We stash a pointer address as a decimal string
            let addr: usize = s.parse().ok()?;
            // SAFETY: the Arc lives as long as the object, kept alive by cloning into the field
            let arc = unsafe { Arc::from_raw(addr as *const Mutex<serde_json::Value>) };
            let clone = arc.clone();
            std::mem::forget(arc); // don't drop the original
            Some(clone)
        }
        _ => None,
    }
}

fn get_arc_channel(
    obj: &Rc<GcObject>,
) -> Option<Arc<(Mutex<(VecDeque<serde_json::Value>, bool)>, Condvar)>> {
    let o = obj.borrow();
    match o.fields.get("__arc")? {
        Value::String(s) => {
            let addr: usize = s.parse().ok()?;
            let arc = unsafe {
                Arc::from_raw(
                    addr as *const (Mutex<(VecDeque<serde_json::Value>, bool)>, Condvar),
                )
            };
            let clone = arc.clone();
            std::mem::forget(arc);
            Some(clone)
        }
        _ => None,
    }
}

fn val_to_json(v: &Value) -> serde_json::Value {
    serde_json::from_str(&crate::stdlib::json::stringify_raw(v))
        .unwrap_or(serde_json::Value::Null)
}

fn json_to_val(j: serde_json::Value) -> Value {
    // Reuse the JSON parser for round-trip
    crate::stdlib::json::json_to_value(j)
}

// ---------- Mutex ----------

pub fn mutex_new(_args: &[Value]) -> RuntimeResult<Value> {
    let inner: Arc<Mutex<serde_json::Value>> = Arc::new(Mutex::new(serde_json::Value::Null));
    let addr = Arc::into_raw(inner) as usize;
    let mut fields = HashMap::new();
    fields.insert("__arc".into(), Value::String(addr.to_string().into()));
    fields.insert("__kind".into(), Value::String("Mutex".into()));
    Ok(crate::gc::alloc_object(ObjectInstance {
        class_name: "Mutex".into(),
        fields,
        class_index: None,
        finalized: false,
    }))
}

pub fn mutex_lock(args: &[Value]) -> RuntimeResult<Value> {
    match args.first() {
        Some(Value::Object(o)) => {
            let arc = get_arc_mutex(o)
                .ok_or_else(|| RuntimeError::Message("invalid Mutex object".into()))?;
            let guard = arc
                .lock()
                .map_err(|_| RuntimeError::Message("Mutex poisoned".into()))?;
            Ok(json_to_val(guard.clone()))
        }
        _ => Err(RuntimeError::TypeError("Mutex.Lock: expected Mutex".into())),
    }
}

pub fn mutex_unlock(args: &[Value]) -> RuntimeResult<Value> {
    // Stores a value under the lock
    match (args.first(), args.get(1)) {
        (Some(Value::Object(o)), Some(v)) => {
            let arc = get_arc_mutex(o)
                .ok_or_else(|| RuntimeError::Message("invalid Mutex object".into()))?;
            let mut guard = arc
                .lock()
                .map_err(|_| RuntimeError::Message("Mutex poisoned".into()))?;
            *guard = val_to_json(v);
            Ok(Value::Null)
        }
        _ => Err(RuntimeError::TypeError("Mutex.Unlock: expected (Mutex, value)".into())),
    }
}

pub fn mutex_try_lock(args: &[Value]) -> RuntimeResult<Value> {
    match args.first() {
        Some(Value::Object(o)) => {
            let arc = get_arc_mutex(o)
                .ok_or_else(|| RuntimeError::Message("invalid Mutex object".into()))?;
            let result = match arc.try_lock() {
                Ok(g) => json_to_val(g.clone()),
                Err(_) => Value::Null,
            };
            Ok(result)
        }
        _ => Err(RuntimeError::TypeError("Mutex.TryLock: expected Mutex".into())),
    }
}

// ---------- Channel ----------

pub fn channel_new(_args: &[Value]) -> RuntimeResult<Value> {
    let inner: Arc<(Mutex<(VecDeque<serde_json::Value>, bool)>, Condvar)> =
        Arc::new((Mutex::new((VecDeque::new(), false)), Condvar::new()));
    let addr = Arc::into_raw(inner) as usize;
    let mut fields = HashMap::new();
    fields.insert("__arc".into(), Value::String(addr.to_string().into()));
    fields.insert("__kind".into(), Value::String("Channel".into()));
    Ok(crate::gc::alloc_object(ObjectInstance {
        class_name: "Channel".into(),
        fields,
        class_index: None,
        finalized: false,
    }))
}

pub fn channel_send(args: &[Value]) -> RuntimeResult<Value> {
    match (args.first(), args.get(1)) {
        (Some(Value::Object(o)), Some(v)) => {
            let arc = get_arc_channel(o)
                .ok_or_else(|| RuntimeError::Message("invalid Channel".into()))?;
            let (lock, cvar) = &*arc;
            {
                let mut g = lock
                    .lock()
                    .map_err(|_| RuntimeError::Message("Channel poisoned".into()))?;
                if g.1 {
                    return Err(RuntimeError::Message("Channel is closed".into()));
                }
                g.0.push_back(val_to_json(v));
            }
            cvar.notify_one();
            Ok(Value::Null)
        }
        _ => Err(RuntimeError::TypeError("Channel.Send: expected (Channel, value)".into())),
    }
}

pub fn channel_recv(args: &[Value]) -> RuntimeResult<Value> {
    match args.first() {
        Some(Value::Object(o)) => {
            let arc = get_arc_channel(o)
                .ok_or_else(|| RuntimeError::Message("invalid Channel".into()))?;
            let (lock, cvar) = &*arc;
            let mut g = lock
                .lock()
                .map_err(|_| RuntimeError::Message("Channel poisoned".into()))?;
            loop {
                if let Some(v) = g.0.pop_front() {
                    return Ok(json_to_val(v));
                }
                if g.1 {
                    return Ok(Value::Null); // closed
                }
                g = cvar
                    .wait(g)
                    .map_err(|_| RuntimeError::Message("Channel wait failed".into()))?;
            }
        }
        _ => Err(RuntimeError::TypeError("Channel.Recv: expected Channel".into())),
    }
}

pub fn channel_try_recv(args: &[Value]) -> RuntimeResult<Value> {
    match args.first() {
        Some(Value::Object(o)) => {
            let arc = get_arc_channel(o)
                .ok_or_else(|| RuntimeError::Message("invalid Channel".into()))?;
            let (lock, _) = &*arc;
            let mut g = lock
                .lock()
                .map_err(|_| RuntimeError::Message("Channel poisoned".into()))?;
            Ok(g.0.pop_front().map(json_to_val).unwrap_or(Value::Null))
        }
        _ => Err(RuntimeError::TypeError("Channel.TryRecv: expected Channel".into())),
    }
}

pub fn channel_close(args: &[Value]) -> RuntimeResult<Value> {
    match args.first() {
        Some(Value::Object(o)) => {
            let arc = get_arc_channel(o)
                .ok_or_else(|| RuntimeError::Message("invalid Channel".into()))?;
            let (lock, cvar) = &*arc;
            {
                let mut g = lock
                    .lock()
                    .map_err(|_| RuntimeError::Message("Channel poisoned".into()))?;
                g.1 = true;
            }
            cvar.notify_all();
            Ok(Value::Null)
        }
        _ => Err(RuntimeError::TypeError("Channel.Close: expected Channel".into())),
    }
}
