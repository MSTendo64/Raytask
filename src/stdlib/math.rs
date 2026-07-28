//! bstd.math natives.

use crate::error::RuntimeResult;
use crate::value::Value;

fn skip_module(args: &[Value]) -> &[Value] {
    if matches!(args.first(), Some(Value::TypeModule(_))) {
        &args[1..]
    } else {
        args
    }
}

fn f0(args: &[Value]) -> RuntimeResult<f64> {
    let args = skip_module(args);
    args.first()
        .map(|v| v.as_float())
        .transpose()
        .map(|o| o.unwrap_or(0.0))
}

fn f1(args: &[Value]) -> RuntimeResult<f64> {
    let args = skip_module(args);
    args.get(1)
        .map(|v| v.as_float())
        .transpose()
        .map(|o| o.unwrap_or(0.0))
}

pub fn abs(args: &[Value]) -> RuntimeResult<Value> {
    let args = skip_module(args);
    let v = args.first().cloned().unwrap_or(Value::Int(0));
    match v {
        Value::Int(n) => Ok(Value::Int(n.abs())),
        Value::Float(n) => Ok(Value::Float(n.abs())),
        other => Ok(Value::Float(other.as_float()?.abs())),
    }
}

pub fn sqrt(args: &[Value]) -> RuntimeResult<Value> {
    Ok(Value::Float(f0(args)?.sqrt()))
}

pub fn pow(args: &[Value]) -> RuntimeResult<Value> {
    Ok(Value::Float(f0(args)?.powf(f1(args)?)))
}

pub fn floor(args: &[Value]) -> RuntimeResult<Value> {
    Ok(Value::Float(f0(args)?.floor()))
}

pub fn ceil(args: &[Value]) -> RuntimeResult<Value> {
    Ok(Value::Float(f0(args)?.ceil()))
}

pub fn round(args: &[Value]) -> RuntimeResult<Value> {
    Ok(Value::Float(f0(args)?.round()))
}

pub fn min(args: &[Value]) -> RuntimeResult<Value> {
    Ok(Value::Float(f0(args)?.min(f1(args)?)))
}

pub fn max(args: &[Value]) -> RuntimeResult<Value> {
    Ok(Value::Float(f0(args)?.max(f1(args)?)))
}

pub fn sin(args: &[Value]) -> RuntimeResult<Value> {
    Ok(Value::Float(f0(args)?.sin()))
}

pub fn cos(args: &[Value]) -> RuntimeResult<Value> {
    Ok(Value::Float(f0(args)?.cos()))
}

pub fn tan(args: &[Value]) -> RuntimeResult<Value> {
    Ok(Value::Float(f0(args)?.tan()))
}

pub fn log(args: &[Value]) -> RuntimeResult<Value> {
    Ok(Value::Float(f0(args)?.ln()))
}

pub fn exp(args: &[Value]) -> RuntimeResult<Value> {
    Ok(Value::Float(f0(args)?.exp()))
}

pub fn random_int(args: &[Value]) -> RuntimeResult<Value> {
    let args = skip_module(args);
    let min = args.first().map(|v| v.as_int()).transpose()?.unwrap_or(0);
    let max = args.get(1).map(|v| v.as_int()).transpose()?.unwrap_or(100);
    let n = if max > min {
        min + (rand_simple().abs() % (max - min + 1))
    } else {
        min
    };
    Ok(Value::Int(n))
}

pub fn random_next(args: &[Value]) -> RuntimeResult<Value> {
    let args = skip_module(args);
    let nums: Vec<i64> = args
        .iter()
        .filter(|v| matches!(v, Value::Int(_) | Value::UInt(_)))
        .filter_map(|v| v.as_int().ok())
        .collect();
    match nums.as_slice() {
        [] => Ok(Value::Int(rand_simple().abs() % 10000)),
        [max] => Ok(Value::Int(rand_simple().abs() % (*max).max(1))),
        [min, max] => {
            let min = *min;
            let max = *max;
            Ok(Value::Int(if max > min {
                min + rand_simple().abs() % (max - min + 1)
            } else {
                min
            }))
        }
        _ => Ok(Value::Int(rand_simple().abs())),
    }
}

pub fn random_next_double(_args: &[Value]) -> RuntimeResult<Value> {
    let r = (rand_simple().abs() % 100_000) as f64 / 100_000.0;
    Ok(Value::Float(r))
}

fn rand_simple() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    let t = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos() as i64;
    (t ^ (t >> 17)).wrapping_mul(0x5bd1e995)
}
