//! Runtime values for the RayTask VM.

use crate::bytecode::ClassKind;
use crate::error::{RuntimeError, RuntimeResult};
use crate::gc::{GcArray, GcDict, GcObject};
use std::cell::RefCell;
use std::collections::HashMap;
use std::fmt;
use std::rc::Rc;

/// Shared mutable cell for a closed-over local (capture-by-value at closure creation,
/// shared across nested closures that chain the same upvalue).
pub type UpvalueCell = Rc<RefCell<Value>>;

/// Runtime reflection handle produced by `typeof` / `Type.Of`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TypeHandle {
    pub name: String,
    /// `"class" | "struct" | "union" | "primitive" | "array" | …`
    pub kind: String,
    pub class_index: Option<usize>,
    pub fields: Vec<String>,
    pub field_types: Vec<String>,
    pub methods: Vec<String>,
}

impl TypeHandle {
    pub fn primitive(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            kind: "primitive".into(),
            class_index: None,
            fields: Vec::new(),
            field_types: Vec::new(),
            methods: Vec::new(),
        }
    }

    pub fn from_class_info(info: &crate::bytecode::ClassInfo, class_index: usize) -> Self {
        Self {
            name: info.name.clone(),
            kind: info.kind.as_str().into(),
            class_index: Some(class_index),
            fields: info.fields.clone(),
            field_types: info.field_types.clone(),
            methods: info.methods.iter().map(|(n, _)| n.clone()).collect(),
        }
    }

    pub fn from_class(name: impl Into<String>, kind: ClassKind, class_index: usize) -> Self {
        Self {
            name: name.into(),
            kind: kind.as_str().into(),
            class_index: Some(class_index),
            fields: Vec::new(),
            field_types: Vec::new(),
            methods: Vec::new(),
        }
    }
}

#[derive(Debug, Clone)]
pub enum Value {
    Null,
    Bool(bool),
    Int(i64),
    UInt(u64),
    Float(f64),
    Char(char),
    String(Rc<str>),
    Array(Rc<GcArray>),
    Dict(Rc<GcDict>),
    Object(Rc<GcObject>),
    Function(FunctionRef),
    Native(usize),
    TypeModule(Rc<str>),
    /// Reflection type handle (`typeof(T)`, `Type.Of(x)`).
    Type(Rc<TypeHandle>),
    Task(crate::async_rt::TaskHandle),
    Ffi(crate::ffi::FfiFunction),
    Ptr(usize),
}

#[derive(Debug, Clone)]
pub struct ObjectInstance {
    pub class_name: String,
    pub fields: HashMap<String, Value>,
    pub class_index: Option<usize>,
    pub finalized: bool,
}

#[derive(Debug, Clone)]
pub struct FunctionRef {
    pub name: String,
    pub chunk_index: usize,
    pub arity: usize,
    pub defaults: Vec<Value>,
    pub is_async: bool,
    /// Closed-over values; empty for ordinary functions.
    pub upvalues: Vec<UpvalueCell>,
}

impl FunctionRef {
    pub fn plain(name: impl Into<String>, chunk_index: usize, arity: usize) -> Self {
        Self {
            name: name.into(),
            chunk_index,
            arity,
            defaults: vec![],
            is_async: false,
            upvalues: vec![],
        }
    }
}

impl Value {
    pub fn type_name(&self) -> &'static str {
        match self {
            Value::Null => "null",
            Value::Bool(_) => "bool",
            Value::Int(_) => "int",
            Value::UInt(_) => "uint",
            Value::Float(_) => "double",
            Value::Char(_) => "char",
            Value::String(_) => "string",
            Value::Array(_) => "array",
            Value::Dict(_) => "dictionary",
            Value::Object(_) => "object",
            Value::Function(_) => "function",
            Value::Native(_) => "native",
            Value::TypeModule(_) => "type",
            Value::Type(_) => "Type",
            Value::Task(_) => "Task",
            Value::Ffi(_) => "ffi",
            Value::Ptr(_) => "ptr",
        }
    }

    pub fn is_truthy(&self) -> bool {
        match self {
            Value::Null => false,
            Value::Bool(b) => *b,
            Value::Int(n) => *n != 0,
            Value::UInt(n) => *n != 0,
            Value::Float(n) => *n != 0.0,
            Value::String(s) => !s.is_empty(),
            Value::Array(a) => !a.borrow().is_empty(),
            _ => true,
        }
    }

    pub fn as_int(&self) -> RuntimeResult<i64> {
        match self {
            Value::Int(n) => Ok(*n),
            Value::UInt(n) => {
                i64::try_from(*n).map_err(|_| {
                    RuntimeError::TypeError(format!(
                        "cannot convert uint {} to int (overflow)",
                        n
                    ))
                })
            }
            Value::Float(n) => {
                if *n < i64::MIN as f64 || *n > i64::MAX as f64 {
                    return Err(RuntimeError::TypeError(format!(
                        "cannot convert float {} to int (overflow)",
                        n
                    )));
                }
                Ok(*n as i64)
            }
            Value::Bool(b) => Ok(if *b { 1 } else { 0 }),
            Value::Char(c) => Ok(*c as i64),
            _ => Err(RuntimeError::TypeError(format!(
                "cannot convert {} to int",
                self.type_name()
            ))),
        }
    }

    pub fn as_float(&self) -> RuntimeResult<f64> {
        match self {
            Value::Float(n) => Ok(*n),
            Value::Int(n) => Ok(*n as f64),
            Value::UInt(n) => Ok(*n as f64),
            _ => Err(RuntimeError::TypeError(format!(
                "cannot convert {} to float",
                self.type_name()
            ))),
        }
    }

    pub fn as_string(&self) -> String {
        match self {
            Value::Null => "null".into(),
            Value::Bool(b) => b.to_string(),
            Value::Int(n) => n.to_string(),
            Value::UInt(n) => n.to_string(),
            Value::Float(n) => {
                if n.fract() == 0.0 && n.abs() < 1e15 {
                    format!("{:.1}", n)
                } else {
                    n.to_string()
                }
            }
            Value::Char(c) => c.to_string(),
            Value::String(s) => s.to_string(),
            Value::Array(a) => {
                let items: Vec<_> = a.borrow().iter().map(|v| v.as_string()).collect();
                format!("[{}]", items.join(", "))
            }
            Value::Dict(d) => {
                let items: Vec<_> = d
                    .borrow()
                    .iter()
                    .map(|(k, v)| format!("{}: {}", k, v.as_string()))
                    .collect();
                format!("{{{}}}", items.join(", "))
            }
            Value::Object(o) => format!("{} {{...}}", o.borrow().class_name),
            Value::Function(f) => {
                if f.upvalues.is_empty() {
                    format!("<fn {}>", f.name)
                } else {
                    format!("<closure {} captures={}>", f.name, f.upvalues.len())
                }
            }
            Value::Native(i) => format!("<native #{}>", i),
            Value::TypeModule(n) => format!("<type {}>", n),
            Value::Type(t) => format!("<Type {}>", t.name),
            Value::Task(t) => match &t.borrow().state {
                crate::async_rt::TaskState::Pending => "<Task pending>".into(),
                crate::async_rt::TaskState::Ready(_) => "<Task ready>".into(),
                crate::async_rt::TaskState::Failed(e) => format!("<Task failed: {}>", e),
            },
            Value::Ffi(f) => format!("<ffi {} @ {}:{}>", f.name, f.library, f.symbol),
            Value::Ptr(p) => format!("ptr(0x{:x})", p),
        }
    }

    pub fn equals(&self, other: &Value) -> bool {
        match (self, other) {
            (Value::Null, Value::Null) => true,
            (Value::Bool(a), Value::Bool(b)) => a == b,
            (Value::Int(a), Value::Int(b)) => a == b,
            (Value::Int(a), Value::UInt(b)) => *a >= 0 && (*a as u64) == *b,
            (Value::UInt(a), Value::Int(b)) => *b >= 0 && *a == (*b as u64),
            (Value::UInt(a), Value::UInt(b)) => a == b,
            (Value::Float(a), Value::Float(b)) => a == b,
            (Value::Float(a), Value::Int(b)) => *a == *b as f64,
            (Value::Int(a), Value::Float(b)) => *a as f64 == *b,
            (Value::Char(a), Value::Char(b)) => a == b,
            (Value::String(a), Value::String(b)) => a == b,
            (Value::Ptr(a), Value::Ptr(b)) => a == b,
            (Value::Type(a), Value::Type(b)) => a == b,
            (Value::TypeModule(a), Value::TypeModule(b)) => a == b,
            (Value::Array(a), Value::Array(b)) => Rc::ptr_eq(a, b),
            (Value::Dict(a), Value::Dict(b)) => Rc::ptr_eq(a, b),
            (Value::Object(a), Value::Object(b)) => Rc::ptr_eq(a, b),
            (Value::Null, _) | (_, Value::Null) => false,
            _ => false,
        }
    }
}

impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_string())
    }
}

impl PartialEq for Value {
    fn eq(&self, other: &Self) -> bool {
        self.equals(other)
    }
}

pub fn binary_op(op: &str, left: &Value, right: &Value) -> RuntimeResult<Value> {
    match op {
        "+" => match (left, right) {
            (Value::String(a), b) => Ok(Value::String(format!("{}{}", a, b.as_string()).into())),
            (a, Value::String(b)) => Ok(Value::String(format!("{}{}", a.as_string(), b).into())),
            (Value::Float(_), _) | (_, Value::Float(_)) => {
                Ok(Value::Float(left.as_float()? + right.as_float()?))
            }
            _ => Ok(Value::Int(left.as_int()? + right.as_int()?)),
        },
        "-" => {
            if matches!(left, Value::Float(_)) || matches!(right, Value::Float(_)) {
                Ok(Value::Float(left.as_float()? - right.as_float()?))
            } else {
                Ok(Value::Int(left.as_int()? - right.as_int()?))
            }
        }
        "*" => {
            if matches!(left, Value::Float(_)) || matches!(right, Value::Float(_)) {
                Ok(Value::Float(left.as_float()? * right.as_float()?))
            } else {
                Ok(Value::Int(left.as_int()? * right.as_int()?))
            }
        }
        "/" => {
            if matches!(left, Value::Float(_)) || matches!(right, Value::Float(_)) {
                let lf = left.as_float()?;
                let rf = right.as_float()?;
                // IEEE 754: x/0.0 = ±Inf, 0.0/0.0 = NaN — let the CPU handle it
                Ok(Value::Float(lf / rf))
            } else {
                let ri = right.as_int()?;
                if ri == 0 {
                    return Err(RuntimeError::DivisionByZero);
                }
                Ok(Value::Int(left.as_int()? / ri))
            }
        }
        "%" => {
            if matches!(left, Value::UInt(_)) || matches!(right, Value::UInt(_)) {
                let r = right.as_int()?;
                if r == 0 {
                    return Err(RuntimeError::DivisionByZero);
                }
                let l = left.as_int()?;
                // Use wrapping_rem for unsigned-safe modulus
                if l >= 0 && r > 0 {
                    Ok(Value::Int(l % r))
                } else {
                    Ok(Value::Int(((l % r) + r) % r))
                }
            } else {
                let ri = right.as_int()?;
                if ri == 0 {
                    return Err(RuntimeError::DivisionByZero);
                }
                Ok(Value::Int(left.as_int()? % ri))
            }
        }
        "==" => Ok(Value::Bool(left.equals(right))),
        "!=" => Ok(Value::Bool(!left.equals(right))),
        "<" => {
            if matches!(left, Value::Float(_)) || matches!(right, Value::Float(_)) {
                Ok(Value::Bool(left.as_float()? < right.as_float()?))
            } else {
                Ok(Value::Bool(left.as_int()? < right.as_int()?))
            }
        }
        "<=" => {
            if matches!(left, Value::Float(_)) || matches!(right, Value::Float(_)) {
                Ok(Value::Bool(left.as_float()? <= right.as_float()?))
            } else {
                Ok(Value::Bool(left.as_int()? <= right.as_int()?))
            }
        }
        ">" => {
            if matches!(left, Value::Float(_)) || matches!(right, Value::Float(_)) {
                Ok(Value::Bool(left.as_float()? > right.as_float()?))
            } else {
                Ok(Value::Bool(left.as_int()? > right.as_int()?))
            }
        }
        ">=" => {
            if matches!(left, Value::Float(_)) || matches!(right, Value::Float(_)) {
                Ok(Value::Bool(left.as_float()? >= right.as_float()?))
            } else {
                Ok(Value::Bool(left.as_int()? >= right.as_int()?))
            }
        }
        "&&" => Ok(Value::Bool(left.is_truthy() && right.is_truthy())),
        "||" => Ok(Value::Bool(left.is_truthy() || right.is_truthy())),
        "&" => Ok(Value::Int(left.as_int()? & right.as_int()?)),
        "|" => Ok(Value::Int(left.as_int()? | right.as_int()?)),
        "^" => Ok(Value::Int(left.as_int()? ^ right.as_int()?)),
        "<<" => Ok(Value::Int(left.as_int()? << right.as_int()?)),
        ">>" => Ok(Value::Int(left.as_int()? >> right.as_int()?)),
        "??" => {
            if matches!(left, Value::Null) {
                Ok(right.clone())
            } else {
                Ok(left.clone())
            }
        }
        _ => Err(RuntimeError::Message(format!("unknown operator {}", op))),
    }
}
