//! bstd.time — DateTime and TimeSpan natives.

use crate::error::{RuntimeError, RuntimeResult};
use crate::value::{ObjectInstance, Value};
use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};

// ---------- internal helpers ----------

fn unix_ms_now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64
}

/// Build a DateTime object from Unix-ms ticks.
fn dt_from_ms(ms: i64, utc: bool) -> Value {
    use std::time::{Duration, UNIX_EPOCH};
    let sys = UNIX_EPOCH + Duration::from_millis(ms.max(0) as u64);
    // Convert to calendar fields
    let total_secs = ms / 1000;
    let (year, month, day, hour, minute, second) = unix_secs_to_fields(total_secs);
    let mut fields = HashMap::new();
    fields.insert("Ticks".into(), Value::Int(ms));
    fields.insert("Utc".into(), Value::Bool(utc));
    fields.insert("Year".into(), Value::Int(year));
    fields.insert("Month".into(), Value::Int(month));
    fields.insert("Day".into(), Value::Int(day));
    fields.insert("Hour".into(), Value::Int(hour));
    fields.insert("Minute".into(), Value::Int(minute));
    fields.insert("Second".into(), Value::Int(second));
    let _ = sys;
    crate::gc::alloc_object(ObjectInstance {
        class_name: "DateTime".into(),
        fields,
        class_index: None,
        finalized: false,
    })
}

/// Very small Gregorian calendar implementation (no external crate needed).
fn unix_secs_to_fields(total_secs: i64) -> (i64, i64, i64, i64, i64, i64) {
    let secs_per_day: i64 = 86400;
    let mut days = total_secs / secs_per_day;
    let rem = total_secs % secs_per_day;
    let (hour, minute, second) = (rem / 3600, (rem % 3600) / 60, rem % 60);

    // Gregorian epoch shift
    let mut year = 1970i64;
    loop {
        let leap = is_leap(year);
        let days_in_year = if leap { 366 } else { 365 };
        if days < days_in_year {
            break;
        }
        days -= days_in_year;
        year += 1;
    }
    let months = [31, if is_leap(year) { 29 } else { 28 }, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
    let mut month = 1i64;
    for &dm in &months {
        if days < dm {
            break;
        }
        days -= dm;
        month += 1;
    }
    (year, month, days + 1, hour, minute, second)
}

fn is_leap(y: i64) -> bool {
    (y % 4 == 0 && y % 100 != 0) || y % 400 == 0
}

fn dt_ticks(args: &[Value]) -> i64 {
    match args.first() {
        Some(Value::Object(o)) => o
            .borrow()
            .fields
            .get("Ticks")
            .and_then(|v| v.as_int().ok())
            .unwrap_or(0),
        Some(v) => v.as_int().unwrap_or(0),
        None => 0,
    }
}

fn dt_utc(args: &[Value]) -> bool {
    match args.first() {
        Some(Value::Object(o)) => o
            .borrow()
            .fields
            .get("Utc")
            .map(|v| v.is_truthy())
            .unwrap_or(false),
        _ => false,
    }
}

/// Extract ms from a TimeSpan or plain int.
fn span_ms(v: &Value) -> i64 {
    match v {
        Value::Object(o) => o
            .borrow()
            .fields
            .get("Ms")
            .and_then(|x| x.as_int().ok())
            .unwrap_or(0),
        v => v.as_int().unwrap_or(0),
    }
}

fn make_timespan(ms: i64) -> Value {
    let mut fields = HashMap::new();
    let abs = ms.abs();
    fields.insert("Ms".into(), Value::Int(ms));
    fields.insert("TotalMilliseconds".into(), Value::Float(ms as f64));
    fields.insert("TotalSeconds".into(), Value::Float(ms as f64 / 1000.0));
    fields.insert("TotalMinutes".into(), Value::Float(ms as f64 / 60_000.0));
    fields.insert("TotalHours".into(), Value::Float(ms as f64 / 3_600_000.0));
    fields.insert("TotalDays".into(), Value::Float(ms as f64 / 86_400_000.0));
    fields.insert("Milliseconds".into(), Value::Int(abs % 1000));
    fields.insert("Seconds".into(), Value::Int((abs / 1000) % 60));
    fields.insert("Minutes".into(), Value::Int((abs / 60_000) % 60));
    fields.insert("Hours".into(), Value::Int((abs / 3_600_000) % 24));
    fields.insert("Days".into(), Value::Int(abs / 86_400_000));
    crate::gc::alloc_object(ObjectInstance {
        class_name: "TimeSpan".into(),
        fields,
        class_index: None,
        finalized: false,
    })
}

// ---------- public natives ----------

pub fn get_time_ms() -> RuntimeResult<Value> {
    Ok(Value::Int(unix_ms_now()))
}

pub fn now(utc: bool) -> RuntimeResult<Value> {
    Ok(dt_from_ms(unix_ms_now(), utc))
}

pub fn dt_to_string(args: &[Value]) -> RuntimeResult<Value> {
    let ms = dt_ticks(args);
    let (y, mo, d, h, mi, s) = unix_secs_to_fields(ms / 1000);
    Ok(Value::String(
        format!("{y:04}-{mo:02}-{d:02} {h:02}:{mi:02}:{s:02}").into(),
    ))
}

pub fn dt_format(args: &[Value]) -> RuntimeResult<Value> {
    let ms = dt_ticks(args);
    let fmt = args
        .get(1)
        .map(|v| v.as_string())
        .unwrap_or_else(|| "yyyy-MM-dd HH:mm:ss".into());
    let (y, mo, d, h, mi, s) = unix_secs_to_fields(ms / 1000);
    let result = fmt
        .replace("yyyy", &format!("{y:04}"))
        .replace("MM", &format!("{mo:02}"))
        .replace("dd", &format!("{d:02}"))
        .replace("HH", &format!("{h:02}"))
        .replace("mm", &format!("{mi:02}"))
        .replace("ss", &format!("{s:02}"))
        .replace("yy", &format!("{:02}", y % 100))
        .replace("M", &format!("{mo}"))
        .replace("d", &format!("{d}"))
        .replace("H", &format!("{h}"))
        .replace("m", &format!("{mi}"))
        .replace("s", &format!("{s}"));
    Ok(Value::String(result.into()))
}

pub fn dt_parse(args: &[Value]) -> RuntimeResult<Value> {
    let s = args.first().map(|v| v.as_string()).unwrap_or_default();
    // Try ISO-8601 subset: yyyy-MM-dd or yyyy-MM-ddTHH:mm:ss
    let parts: Vec<&str> = s.splitn(2, 'T').collect();
    let date_part = parts[0];
    let time_part = parts.get(1).copied().unwrap_or("00:00:00");
    let dp: Vec<i64> = date_part
        .splitn(3, '-')
        .filter_map(|x| x.parse().ok())
        .collect();
    let tp: Vec<i64> = time_part
        .splitn(3, ':')
        .filter_map(|x| x.parse().ok())
        .collect();
    if dp.len() < 3 {
        return Err(RuntimeError::Message(format!(
            "DateTime.Parse: cannot parse '{s}'"
        )));
    }
    let (y, mo, d) = (dp[0], dp[1], dp[2]);
    let (h, mi, sec) = (
        tp.first().copied().unwrap_or(0),
        tp.get(1).copied().unwrap_or(0),
        tp.get(2).copied().unwrap_or(0),
    );
    // Calculate Unix ms
    let ms = fields_to_unix_ms(y, mo, d, h, mi, sec);
    Ok(dt_from_ms(ms, false))
}

fn fields_to_unix_ms(year: i64, month: i64, day: i64, h: i64, mi: i64, s: i64) -> i64 {
    let mut days: i64 = 0;
    for y in 1970..year {
        days += if is_leap(y) { 366 } else { 365 };
    }
    let months = [31, if is_leap(year) { 29 } else { 28 }, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
    for m in 1..month {
        days += months[(m - 1) as usize];
    }
    days += day - 1;
    (days * 86400 + h * 3600 + mi * 60 + s) * 1000
}

pub fn dt_add_span(args: &[Value]) -> RuntimeResult<Value> {
    let ms = dt_ticks(args);
    let span = args.get(1).map(span_ms).unwrap_or(0);
    let utc = dt_utc(args);
    Ok(dt_from_ms(ms + span, utc))
}

pub fn dt_sub_span(args: &[Value]) -> RuntimeResult<Value> {
    let ms = dt_ticks(args);
    let span = args.get(1).map(span_ms).unwrap_or(0);
    let utc = dt_utc(args);
    Ok(dt_from_ms(ms - span, utc))
}

pub fn dt_diff(args: &[Value]) -> RuntimeResult<Value> {
    let a = dt_ticks(args);
    let b = args.get(1).map(dt_ticks_raw).unwrap_or(0);
    Ok(make_timespan(a - b))
}

fn dt_ticks_raw(v: &Value) -> i64 {
    match v {
        Value::Object(o) => o
            .borrow()
            .fields
            .get("Ticks")
            .and_then(|x| x.as_int().ok())
            .unwrap_or(0),
        v => v.as_int().unwrap_or(0),
    }
}

pub fn dt_field(args: &[Value], field: &str) -> RuntimeResult<Value> {
    match args.first() {
        Some(Value::Object(o)) => Ok(o
            .borrow()
            .fields
            .get(field)
            .cloned()
            .unwrap_or(Value::Null)),
        _ => Err(RuntimeError::TypeError(format!(
            "DateTime.{field}: expected DateTime"
        ))),
    }
}

// ---------- TimeSpan factories ----------

pub fn timespan_from_ms(args: &[Value]) -> RuntimeResult<Value> {
    let ms = args.first().and_then(|v| v.as_int().ok()).unwrap_or(0);
    Ok(make_timespan(ms))
}

pub fn timespan_from_secs(args: &[Value]) -> RuntimeResult<Value> {
    let s = args.first().and_then(|v| v.as_int().ok()).unwrap_or(0);
    Ok(make_timespan(s * 1000))
}

pub fn timespan_from_mins(args: &[Value]) -> RuntimeResult<Value> {
    let m = args.first().and_then(|v| v.as_int().ok()).unwrap_or(0);
    Ok(make_timespan(m * 60_000))
}

pub fn timespan_from_hours(args: &[Value]) -> RuntimeResult<Value> {
    let h = args.first().and_then(|v| v.as_int().ok()).unwrap_or(0);
    Ok(make_timespan(h * 3_600_000))
}

pub fn timespan_add(args: &[Value]) -> RuntimeResult<Value> {
    let a = args.first().map(|v| span_ms(v)).unwrap_or(0);
    let b = args.get(1).map(|v| span_ms(v)).unwrap_or(0);
    Ok(make_timespan(a + b))
}

pub fn timespan_sub(args: &[Value]) -> RuntimeResult<Value> {
    let a = args.first().map(|v| span_ms(v)).unwrap_or(0);
    let b = args.get(1).map(|v| span_ms(v)).unwrap_or(0);
    Ok(make_timespan(a - b))
}

pub fn timespan_total_ms(args: &[Value]) -> RuntimeResult<Value> {
    let ms = args.first().map(|v| span_ms(v)).unwrap_or(0);
    Ok(Value::Int(ms))
}

pub fn timespan_total_secs(args: &[Value]) -> RuntimeResult<Value> {
    let ms = args.first().map(|v| span_ms(v)).unwrap_or(0);
    Ok(Value::Float(ms as f64 / 1000.0))
}

pub fn timespan_to_string(args: &[Value]) -> RuntimeResult<Value> {
    let ms = args.first().map(|v| span_ms(v)).unwrap_or(0);
    let abs = ms.abs();
    let sign = if ms < 0 { "-" } else { "" };
    let d = abs / 86_400_000;
    let h = (abs / 3_600_000) % 24;
    let m = (abs / 60_000) % 60;
    let s = (abs / 1000) % 60;
    let millis = abs % 1000;
    if d > 0 {
        Ok(Value::String(
            format!("{sign}{d}.{h:02}:{m:02}:{s:02}.{millis:03}").into(),
        ))
    } else {
        Ok(Value::String(
            format!("{sign}{h:02}:{m:02}:{s:02}.{millis:03}").into(),
        ))
    }
}
