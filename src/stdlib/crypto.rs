//! bstd.crypto natives.

use crate::error::RuntimeResult;
use crate::value::Value;
use md5::Md5;
use sha1::Sha1;
use sha2::Sha256;
use sha2::Digest;

fn skip_module(args: &[Value]) -> &[Value] {
    if matches!(args.first(), Some(Value::TypeModule(_))) {
        &args[1..]
    } else {
        args
    }
}

fn input_bytes(args: &[Value]) -> Vec<u8> {
    let args = skip_module(args);
    match args.first() {
        Some(Value::Array(a)) => a
            .borrow()
            .iter()
            .filter_map(|v| v.as_int().ok().map(|n| n as u8))
            .collect(),
        Some(v) => v.as_string().into_bytes(),
        None => Vec::new(),
    }
}

pub fn sha256(args: &[Value]) -> RuntimeResult<Value> {
    let mut hasher = Sha256::new();
    hasher.update(input_bytes(args));
    Ok(Value::String(hex::encode(hasher.finalize()).into()))
}

pub fn sha1(args: &[Value]) -> RuntimeResult<Value> {
    use sha1::Digest as Sha1Digest;
    let mut hasher = Sha1::new();
    Sha1Digest::update(&mut hasher, input_bytes(args));
    Ok(Value::String(hex::encode(Sha1Digest::finalize(hasher)).into()))
}

pub fn md5(args: &[Value]) -> RuntimeResult<Value> {
    use md5::Digest as Md5Digest;
    let mut hasher = Md5::new();
    Md5Digest::update(&mut hasher, input_bytes(args));
    Ok(Value::String(hex::encode(Md5Digest::finalize(hasher)).into()))
}
