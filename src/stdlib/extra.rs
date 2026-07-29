//! Extended Math, String, List, Convert, Env native functions.

use crate::error::RuntimeResult;
use crate::value::Value;
use std::collections::HashSet;

// ── helpers ───────────────────────────────────────────────────────────────────

fn as_f64(v: &Value) -> f64 {
    match v {
        Value::Float(f) => *f,
        Value::Int(i) => *i as f64,
        Value::UInt(u) => *u as f64,
        _ => 0.0,
    }
}

fn get_str(v: &Value) -> String {
    match v {
        Value::String(s) => s.as_ref().to_string(),
        other => other.as_string().to_string(),
    }
}

fn get_array(v: &Value) -> Option<&crate::gc::GcArray> {
    if let Value::Array(a) = v { Some(a.as_ref()) } else { None }
}

fn skip_module(args: &[Value]) -> &[Value] {
    if matches!(args.first(), Some(Value::TypeModule(_))) {
        &args[1..]
    } else {
        args
    }
}

fn mk_str(s: String) -> Value {
    Value::String(s.into())
}

fn mk_arr(v: Vec<Value>) -> Value {
    crate::gc::alloc_array(v)
}

fn as_usize(v: &Value) -> usize {
    match v {
        Value::Int(i) => (*i).max(0) as usize,
        Value::UInt(u) => *u as usize,
        Value::Float(f) => (*f).max(0.0) as usize,
        _ => 0,
    }
}

// ── Math extended ─────────────────────────────────────────────────────────────

pub fn math_clamp(args: &[Value]) -> RuntimeResult<Value> {
    let args = skip_module(args);
    let (v, lo, hi) = (as_f64(&args[0]), as_f64(&args[1]), as_f64(&args[2]));
    Ok(Value::Float(v.clamp(lo, hi)))
}
pub fn math_log2(args: &[Value]) -> RuntimeResult<Value> { let args = skip_module(args); Ok(Value::Float(as_f64(&args[0]).log2())) }
pub fn math_log10(args: &[Value]) -> RuntimeResult<Value> { let args = skip_module(args); Ok(Value::Float(as_f64(&args[0]).log10())) }
pub fn math_atan2(args: &[Value]) -> RuntimeResult<Value> { let args = skip_module(args); Ok(Value::Float(as_f64(&args[0]).atan2(as_f64(&args[1])))) }
pub fn math_sign(args: &[Value]) -> RuntimeResult<Value> {
    let args = skip_module(args);
    let v = as_f64(&args[0]);
    Ok(Value::Int(if v > 0.0 { 1 } else if v < 0.0 { -1 } else { 0 }))
}
pub fn math_truncate(args: &[Value]) -> RuntimeResult<Value> {
    let args = skip_module(args);
    let t = as_f64(&args[0]).trunc();
    Ok(Value::Int(t as i64))
}
pub fn math_is_nan(args: &[Value]) -> RuntimeResult<Value> { let args = skip_module(args); Ok(Value::Bool(as_f64(&args[0]).is_nan())) }
pub fn math_is_inf(args: &[Value]) -> RuntimeResult<Value> { let args = skip_module(args); Ok(Value::Bool(as_f64(&args[0]).is_infinite())) }
pub fn math_lerp(args: &[Value]) -> RuntimeResult<Value> {
    let args = skip_module(args);
    let (a, b, t) = (as_f64(&args[0]), as_f64(&args[1]), as_f64(&args[2]));
    Ok(Value::Float(a + (b - a) * t))
}
pub fn math_asin(args: &[Value]) -> RuntimeResult<Value> { let args = skip_module(args); Ok(Value::Float(as_f64(&args[0]).asin())) }
pub fn math_acos(args: &[Value]) -> RuntimeResult<Value> { let args = skip_module(args); Ok(Value::Float(as_f64(&args[0]).acos())) }
pub fn math_atan(args: &[Value]) -> RuntimeResult<Value> { let args = skip_module(args); Ok(Value::Float(as_f64(&args[0]).atan())) }
pub fn math_sinh(args: &[Value]) -> RuntimeResult<Value> { let args = skip_module(args); Ok(Value::Float(as_f64(&args[0]).sinh())) }
pub fn math_cosh(args: &[Value]) -> RuntimeResult<Value> { let args = skip_module(args); Ok(Value::Float(as_f64(&args[0]).cosh())) }
pub fn math_tanh(args: &[Value]) -> RuntimeResult<Value> { let args = skip_module(args); Ok(Value::Float(as_f64(&args[0]).tanh())) }
pub fn math_cbrt(args: &[Value]) -> RuntimeResult<Value> { let args = skip_module(args); Ok(Value::Float(as_f64(&args[0]).cbrt())) }
pub fn math_hypot(args: &[Value]) -> RuntimeResult<Value> {
    let args = skip_module(args);
    Ok(Value::Float(as_f64(&args[0]).hypot(as_f64(&args[1]))))
}

// ── String extended ────────────────────────────────────────────────────────────

pub fn str_pad_left(args: &[Value]) -> RuntimeResult<Value> {
    let s = get_str(&args[0]);
    let width = as_usize(&args[1]);
    let pad_ch = if args.len() > 2 { get_str(&args[2]).chars().next().unwrap_or(' ') } else { ' ' };
    let chars: Vec<char> = s.chars().collect();
    if chars.len() >= width { return Ok(mk_str(s)); }
    let mut out: String = std::iter::repeat(pad_ch).take(width - chars.len()).collect();
    out.push_str(&s);
    Ok(mk_str(out))
}

pub fn str_pad_right(args: &[Value]) -> RuntimeResult<Value> {
    let s = get_str(&args[0]);
    let width = as_usize(&args[1]);
    let pad_ch = if args.len() > 2 { get_str(&args[2]).chars().next().unwrap_or(' ') } else { ' ' };
    let chars: Vec<char> = s.chars().collect();
    if chars.len() >= width { return Ok(mk_str(s)); }
    let mut out = s;
    while out.chars().count() < width { out.push(pad_ch); }
    Ok(mk_str(out))
}

pub fn str_repeat(args: &[Value]) -> RuntimeResult<Value> {
    let s = get_str(&args[0]);
    let n = as_usize(&args[1]);
    Ok(mk_str(s.repeat(n)))
}

pub fn str_reverse(args: &[Value]) -> RuntimeResult<Value> {
    Ok(mk_str(get_str(&args[0]).chars().rev().collect()))
}

pub fn str_chars(args: &[Value]) -> RuntimeResult<Value> {
    let list: Vec<Value> = get_str(&args[0]).chars().map(Value::Char).collect();
    Ok(mk_arr(list))
}

pub fn str_lines(args: &[Value]) -> RuntimeResult<Value> {
    let list: Vec<Value> = get_str(&args[0]).lines().map(|l| mk_str(l.to_string())).collect();
    Ok(mk_arr(list))
}

pub fn str_parse_int(args: &[Value]) -> RuntimeResult<Value> {
    Ok(match get_str(&args[0]).trim().parse::<i64>() {
        Ok(i) => Value::Int(i),
        Err(_) => Value::Null,
    })
}

pub fn str_parse_float(args: &[Value]) -> RuntimeResult<Value> {
    Ok(match get_str(&args[0]).trim().parse::<f64>() {
        Ok(f) => Value::Float(f),
        Err(_) => Value::Null,
    })
}

pub fn str_is_empty(args: &[Value]) -> RuntimeResult<Value> {
    Ok(Value::Bool(match &args[0] {
        Value::Null => true,
        v => get_str(v).is_empty(),
    }))
}

pub fn str_is_whitespace(args: &[Value]) -> RuntimeResult<Value> {
    Ok(Value::Bool(match &args[0] {
        Value::Null => true,
        v => get_str(v).trim().is_empty(),
    }))
}

pub fn str_join(args: &[Value]) -> RuntimeResult<Value> {
    let args = skip_module(args);
    // String.Join(sep, list)
    let sep = get_str(&args[0]);
    let list = match &args[1] {
        Value::Array(a) => a.borrow().iter().map(|v| get_str(v)).collect::<Vec<_>>(),
        _ => return Ok(mk_str(String::new())),
    };
    Ok(mk_str(list.join(&sep)))
}

pub fn str_format(args: &[Value]) -> RuntimeResult<Value> {
    let args = skip_module(args);
    // String.Format("{0} and {1}", a, b)
    let mut s = get_str(&args[0]);
    for (i, arg) in args[1..].iter().enumerate() {
        s = s.replace(&format!("{{{i}}}"), &arg.as_string());
    }
    Ok(mk_str(s))
}

pub fn str_count(args: &[Value]) -> RuntimeResult<Value> {
    let s = get_str(&args[0]);
    let needle = get_str(&args[1]);
    if needle.is_empty() { return Ok(Value::Int(0)); }
    Ok(Value::Int(s.matches(needle.as_str()).count() as i64))
}

pub fn str_remove(args: &[Value]) -> RuntimeResult<Value> {
    let s = get_str(&args[0]);
    let start = as_usize(&args[1]);
    let chars: Vec<char> = s.chars().collect();
    let count = match args.get(2) { Some(v) => as_usize(v), _ => chars.len().saturating_sub(start) };
    let end = (start + count).min(chars.len());
    let mut out: String = chars[..start].iter().collect();
    out.extend(&chars[end..]);
    Ok(mk_str(out))
}

pub fn str_insert(args: &[Value]) -> RuntimeResult<Value> {
    let s = get_str(&args[0]);
    let idx = as_usize(&args[1]);
    let ins = get_str(&args[2]);
    let chars: Vec<char> = s.chars().collect();
    let idx = idx.min(chars.len());
    let mut out: String = chars[..idx].iter().collect();
    out.push_str(&ins);
    out.extend(&chars[idx..]);
    Ok(mk_str(out))
}

// ── List/Array extended ────────────────────────────────────────────────────────

fn cmp_vals(a: &Value, b: &Value) -> std::cmp::Ordering {
    match (a, b) {
        (Value::Int(x), Value::Int(y)) => x.cmp(y),
        (Value::Float(x), Value::Float(y)) => x.partial_cmp(y).unwrap_or(std::cmp::Ordering::Equal),
        (Value::Int(x), Value::Float(y)) => (*x as f64).partial_cmp(y).unwrap_or(std::cmp::Ordering::Equal),
        (Value::Float(x), Value::Int(y)) => x.partial_cmp(&(*y as f64)).unwrap_or(std::cmp::Ordering::Equal),
        (Value::String(x), Value::String(y)) => x.cmp(y),
        _ => std::cmp::Ordering::Equal,
    }
}

pub fn list_sort(args: &[Value]) -> RuntimeResult<Value> {
    if let Some(a) = get_array(&args[0]) {
        let mut v = a.borrow().clone();
        v.sort_by(cmp_vals);
        return Ok(mk_arr(v));
    }
    Ok(args[0].clone())
}

pub fn list_sort_desc(args: &[Value]) -> RuntimeResult<Value> {
    if let Some(a) = get_array(&args[0]) {
        let mut v = a.borrow().clone();
        v.sort_by(|x, y| cmp_vals(y, x));
        return Ok(mk_arr(v));
    }
    Ok(args[0].clone())
}

pub fn list_reverse(args: &[Value]) -> RuntimeResult<Value> {
    if let Some(a) = get_array(&args[0]) {
        let mut v = a.borrow().clone();
        v.reverse();
        return Ok(mk_arr(v));
    }
    Ok(args[0].clone())
}

pub fn list_distinct(args: &[Value]) -> RuntimeResult<Value> {
    if let Some(a) = get_array(&args[0]) {
        let v = a.borrow();
        let mut seen = HashSet::new();
        let out: Vec<Value> = v.iter().filter(|x| seen.insert(x.as_string().to_string())).cloned().collect();
        return Ok(mk_arr(out));
    }
    Ok(args[0].clone())
}

pub fn list_count(args: &[Value]) -> RuntimeResult<Value> {
    if let Some(a) = get_array(&args[0]) {
        return Ok(Value::Int(a.borrow().len() as i64));
    }
    Ok(Value::Int(0))
}

pub fn list_take(args: &[Value]) -> RuntimeResult<Value> {
    if let Some(a) = get_array(&args[0]) {
        let n = as_usize(&args[1]);
        let v: Vec<Value> = a.borrow().iter().take(n).cloned().collect();
        return Ok(mk_arr(v));
    }
    Ok(args[0].clone())
}

pub fn list_skip(args: &[Value]) -> RuntimeResult<Value> {
    if let Some(a) = get_array(&args[0]) {
        let n = as_usize(&args[1]);
        let v: Vec<Value> = a.borrow().iter().skip(n).cloned().collect();
        return Ok(mk_arr(v));
    }
    Ok(args[0].clone())
}

pub fn list_flatten(args: &[Value]) -> RuntimeResult<Value> {
    if let Some(a) = get_array(&args[0]) {
        let mut out = Vec::new();
        for item in a.borrow().iter() {
            if let Some(inner) = get_array(item) {
                out.extend(inner.borrow().iter().cloned());
            } else {
                out.push(item.clone());
            }
        }
        return Ok(mk_arr(out));
    }
    Ok(args[0].clone())
}

pub fn list_zip(args: &[Value]) -> RuntimeResult<Value> {
    let a_arr = match get_array(&args[0]) { Some(a) => a.borrow().clone(), None => return Ok(Value::Null) };
    let b_arr = match get_array(&args[1]) { Some(a) => a.borrow().clone(), None => return Ok(Value::Null) };
    let out: Vec<Value> = a_arr.iter().zip(b_arr.iter()).map(|(x, y)| {
        mk_arr(vec![x.clone(), y.clone()])
    }).collect();
    Ok(mk_arr(out))
}

pub fn list_chunk(args: &[Value]) -> RuntimeResult<Value> {
    if let Some(a) = get_array(&args[0]) {
        let size = as_usize(&args[1]).max(1);
        let v = a.borrow();
        let out: Vec<Value> = v.chunks(size).map(|c| mk_arr(c.to_vec())).collect();
        return Ok(mk_arr(out));
    }
    Ok(args[0].clone())
}

pub fn list_index_of(args: &[Value]) -> RuntimeResult<Value> {
    if let Some(a) = get_array(&args[0]) {
        let needle = args[1].as_string().to_string();
        for (i, item) in a.borrow().iter().enumerate() {
            if item.as_string() == needle.as_str() {
                return Ok(Value::Int(i as i64));
            }
        }
        return Ok(Value::Int(-1));
    }
    Ok(Value::Int(-1))
}

pub fn list_fill(args: &[Value]) -> RuntimeResult<Value> {
    let args = skip_module(args);
    let val = args[0].clone();
    let n = as_usize(&args[1]);
    Ok(mk_arr(vec![val; n]))
}

pub fn list_range_static(args: &[Value]) -> RuntimeResult<Value> {
    let args = skip_module(args);
    let (start, count) = if args.len() >= 2 {
        (match &args[0] { Value::Int(i) => *i, _ => 0 },
         match &args[1] { Value::Int(i) => *i, _ => 0 })
    } else {
        (0, match &args[0] { Value::Int(i) => *i, _ => 0 })
    };
    let v: Vec<Value> = (start..start + count).map(Value::Int).collect();
    Ok(mk_arr(v))
}

pub fn list_copy(args: &[Value]) -> RuntimeResult<Value> {
    if let Some(a) = get_array(&args[0]) {
        return Ok(mk_arr(a.borrow().clone()));
    }
    Ok(args[0].clone())
}

// ── Convert static ─────────────────────────────────────────────────────────────

pub fn conv_to_int(args: &[Value]) -> RuntimeResult<Value> {
    let args = skip_module(args);
    Ok(match &args[0] {
        Value::Int(i) => Value::Int(*i),
        Value::Float(f) => Value::Int(*f as i64),
        Value::Bool(b) => Value::Int(*b as i64),
        Value::String(s) => s.trim().parse::<i64>().map(Value::Int).unwrap_or(Value::Null),
        Value::Char(c) => Value::Int(*c as i64),
        _ => Value::Null,
    })
}

pub fn conv_to_float(args: &[Value]) -> RuntimeResult<Value> {
    let args = skip_module(args);
    Ok(match &args[0] {
        Value::Float(f) => Value::Float(*f),
        Value::Int(i) => Value::Float(*i as f64),
        Value::Bool(b) => Value::Float(*b as u8 as f64),
        Value::String(s) => s.trim().parse::<f64>().map(Value::Float).unwrap_or(Value::Null),
        _ => Value::Null,
    })
}

pub fn conv_to_bool(args: &[Value]) -> RuntimeResult<Value> {
    let args = skip_module(args);
    Ok(Value::Bool(match &args[0] {
        Value::Bool(b) => *b,
        Value::Int(i) => *i != 0,
        Value::Float(f) => *f != 0.0,
        Value::Null => false,
        Value::String(s) => !s.is_empty() && s.as_ref() != "false" && s.as_ref() != "0",
        _ => true,
    }))
}

pub fn conv_to_string(args: &[Value]) -> RuntimeResult<Value> {
    let args = skip_module(args);
    Ok(mk_str(args[0].as_string().to_string()))
}

pub fn conv_to_hex(args: &[Value]) -> RuntimeResult<Value> {
    let args = skip_module(args);
    let n = match &args[0] { Value::Int(i) => *i as u64, Value::UInt(u) => *u, _ => 0 };
    Ok(mk_str(format!("{n:X}")))
}

pub fn conv_from_hex(args: &[Value]) -> RuntimeResult<Value> {
    let args = skip_module(args);
    let raw = get_str(&args[0]);
    let s = raw.trim_start_matches("0x").trim_start_matches("0X");
    Ok(i64::from_str_radix(s, 16).map(Value::Int).unwrap_or(Value::Null))
}

pub fn conv_to_bytes(args: &[Value]) -> RuntimeResult<Value> {
    let args = skip_module(args);
    let bytes = match &args[0] {
        Value::String(s) => s.as_bytes().to_vec(),
        _ => return Ok(Value::Null),
    };
    let list: Vec<Value> = bytes.into_iter().map(|b| Value::Int(b as i64)).collect();
    Ok(mk_arr(list))
}

pub fn conv_from_bytes(args: &[Value]) -> RuntimeResult<Value> {
    let args = skip_module(args);
    if let Some(a) = get_array(&args[0]) {
        let bytes: Vec<u8> = a.borrow().iter().map(|v| match v { Value::Int(i) => *i as u8, _ => 0 }).collect();
        return Ok(mk_str(String::from_utf8_lossy(&bytes).into_owned()));
    }
    Ok(Value::Null)
}

pub fn conv_to_base64(args: &[Value]) -> RuntimeResult<Value> {
    let args = skip_module(args);
    let data: Vec<u8> = match &args[0] {
        Value::String(s) => s.as_bytes().to_vec(),
        Value::Array(a) => a.borrow().iter().map(|v| match v { Value::Int(i) => *i as u8, _ => 0 }).collect(),
        _ => return Ok(Value::Null),
    };
    const CHARS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for chunk in data.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = if chunk.len() > 1 { chunk[1] as u32 } else { 0 };
        let b2 = if chunk.len() > 2 { chunk[2] as u32 } else { 0 };
        let n = (b0 << 16) | (b1 << 8) | b2;
        out.push(CHARS[((n >> 18) & 63) as usize] as char);
        out.push(CHARS[((n >> 12) & 63) as usize] as char);
        out.push(if chunk.len() > 1 { CHARS[((n >> 6) & 63) as usize] as char } else { '=' });
        out.push(if chunk.len() > 2 { CHARS[(n & 63) as usize] as char } else { '=' });
    }
    Ok(mk_str(out))
}

pub fn conv_from_base64(args: &[Value]) -> RuntimeResult<Value> {
    let args = skip_module(args);
    let s = match &args[0] { Value::String(s) => s.to_string(), _ => return Ok(Value::Null) };
    let decode_char = |c: u8| -> u8 {
        match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            _ => 0,
        }
    };
    let bytes: Vec<u8> = s.as_bytes().chunks(4).flat_map(|chunk| {
        let b: Vec<u8> = chunk.iter().map(|&c| decode_char(c)).collect();
        let n = ((b[0] as u32) << 18)
              | ((b.get(1).copied().unwrap_or(0) as u32) << 12)
              | ((b.get(2).copied().unwrap_or(0) as u32) << 6)
              | (b.get(3).copied().unwrap_or(0) as u32);
        let mut r = vec![(n >> 16) as u8];
        if chunk.get(2).copied().unwrap_or(b'=') != b'=' { r.push((n >> 8) as u8); }
        if chunk.get(3).copied().unwrap_or(b'=') != b'=' { r.push(n as u8); }
        r
    }).collect();
    Ok(mk_str(String::from_utf8_lossy(&bytes).into_owned()))
}

pub fn conv_to_binary(args: &[Value]) -> RuntimeResult<Value> {
    let args = skip_module(args);
    let n = match &args[0] { Value::Int(i) => *i as u64, Value::UInt(u) => *u, _ => 0 };
    Ok(mk_str(format!("{n:b}")))
}

// ── Env static ────────────────────────────────────────────────────────────────

pub fn env_get_var(args: &[Value]) -> RuntimeResult<Value> {
    let args = skip_module(args);
    let name = get_str(&args[0]);
    Ok(std::env::var(&name).map(mk_str).unwrap_or(Value::Null))
}

pub fn env_set_var(args: &[Value]) -> RuntimeResult<Value> {
    let args = skip_module(args);
    let name = get_str(&args[0]);
    let val = get_str(&args[1]);
    std::env::set_var(&name, &val);
    Ok(Value::Null)
}

pub fn env_has_var(args: &[Value]) -> RuntimeResult<Value> {
    let args = skip_module(args);
    Ok(Value::Bool(std::env::var(get_str(&args[0]).as_str()).is_ok()))
}

pub fn env_args(_args: &[Value]) -> RuntimeResult<Value> {
    let list: Vec<Value> = std::env::args().map(mk_str).collect();
    Ok(mk_arr(list))
}

pub fn env_current_dir(_args: &[Value]) -> RuntimeResult<Value> {
    Ok(std::env::current_dir()
        .map(|p| mk_str(p.display().to_string()))
        .unwrap_or(Value::Null))
}

pub fn env_os(_args: &[Value]) -> RuntimeResult<Value> {
    Ok(mk_str(std::env::consts::OS.to_string()))
}

pub fn env_home(_args: &[Value]) -> RuntimeResult<Value> {
    Ok(std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(|p| mk_str(p.to_string_lossy().into_owned()))
        .unwrap_or(Value::Null))
}
