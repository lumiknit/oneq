//! jq's date/time builtins, delegating the actual calendar math to
//! `crate::data::core::time` (already shared with the calc engine).
use super::{scalar::number, strings::string};
use crate::{
    data::{Value, core::time},
    jq::vm::{JqError, value::error},
};
use std::rc::Rc;

fn broken_down(fields: (f64, f64, f64, f64, f64, f64, f64, f64)) -> Value {
    let (y, mo, d, h, mi, s, wd, yd) = fields;
    Value::Array(Rc::new(
        [y, mo, d, h, mi, s, wd, yd]
            .into_iter()
            .map(Value::Float)
            .collect(),
    ))
}
/// A broken-down time array shorter than 6 elements is fine (trailing
/// fields default to 0, e.g. `[2024,2,15]` means midnight) - jq only
/// rejects it if it isn't an array of numbers at all, with an error naming
/// the calling builtin (`err_name` is e.g. `"strftime/1"` or `"mktime"`).
fn time_fields(input: &Value, err_name: &str) -> Result<[f64; 6], JqError> {
    let Value::Array(a) = input else {
        return Err(error(format!("{err_name} requires parsed datetime inputs")));
    };
    let mut fields = [1900.0, 0.0, 0.0, 0.0, 0.0, 0.0];
    for (i, dst) in fields.iter_mut().enumerate() {
        if let Some(value) = a.get(i) {
            *dst = value
                .as_number()
                .ok_or_else(|| error(format!("{err_name} requires parsed datetime inputs")))?;
        }
    }
    Ok(fields)
}
pub fn now(_: &Value, _: &[Value]) -> Result<Value, JqError> {
    Ok(Value::Float(time::now()))
}
pub fn gmtime(input: &Value, _: &[Value]) -> Result<Value, JqError> {
    let secs = number(input)?;
    let fields = time::gmtime(secs).ok_or_else(|| error("gmtime: epoch time out of range"))?;
    Ok(broken_down(fields))
}
pub fn localtime(input: &Value, _: &[Value]) -> Result<Value, JqError> {
    let secs = number(input)?;
    let fields =
        time::localtime(secs).ok_or_else(|| error("localtime: epoch time out of range"))?;
    Ok(broken_down(fields))
}
pub fn mktime(input: &Value, _: &[Value]) -> Result<Value, JqError> {
    let [y, mo, d, h, mi, s] = time_fields(input, "mktime")?;
    time::mktime(y as i64, mo as i64, d as i64, h as i64, mi as i64, s)
        .map(Value::Float)
        .ok_or_else(|| error("mktime: invalid time array"))
}
fn format_string<'a>(args: &'a [Value], err_name: &str) -> Result<&'a str, JqError> {
    match &args[0] {
        Value::String(s) => Ok(s),
        _ => Err(error(format!("{err_name} requires a string format"))),
    }
}
fn strftime_impl(
    input: &Value,
    args: &[Value],
    err_name: &str,
    to_broken: impl Fn(&Value) -> Result<Value, JqError>,
) -> Result<Value, JqError> {
    let fmt = format_string(args, err_name)?;
    // jq auto-converts a plain epoch number via gmtime/localtime before
    // formatting.
    let converted = matches!(input, Value::Float(_) | Value::Decimal(_));
    let broken = if converted {
        to_broken(input)?
    } else {
        input.clone()
    };
    let [y, mo, d, h, mi, s] = time_fields(&broken, err_name)?;
    time::strftime(
        y as i64,
        mo as i64 + 1,
        d as i64,
        h as i64,
        mi as i64,
        s as i64,
        fmt,
    )
    .map(|s| Value::String(s.into()))
    .or_else(|| {
        // jq accepts incomplete or non-calendar broken-down arrays here.
        // Keep the common ISO format permissive instead of requiring chrono
        // to construct a real calendar date (e.g. [2] -> 0002-01-00).
        (fmt == "%Y-%m-%dT%H:%M:%SZ").then(|| {
            Value::String(
                format!(
                    "{y:04}-{month:02}-{d:02}T{h:02}:{mi:02}:{s:02}Z",
                    month = mo as i64 + 1
                )
                .into(),
            )
        })
    })
    .ok_or_else(|| error("strftime: invalid time array"))
}
pub fn strftime(input: &Value, args: &[Value]) -> Result<Value, JqError> {
    strftime_impl(input, args, "strftime/1", |v| gmtime(v, &[]))
}
pub fn strflocaltime(input: &Value, args: &[Value]) -> Result<Value, JqError> {
    strftime_impl(input, args, "strflocaltime/1", |v| localtime(v, &[]))
}
pub fn strptime(input: &Value, args: &[Value]) -> Result<Value, JqError> {
    let s = string(input)?;
    let fmt = string(&args[0])?;
    let (y, mo, d, h, mi, sec) = time::strptime(s, fmt)
        .ok_or_else(|| error("date \"".to_string() + s + "\" does not match format"))?;
    let (wd, yd) = time::weekday_and_yday(y, mo, d);
    Ok(broken_down((
        y as f64,
        (mo - 1) as f64,
        d as f64,
        h as f64,
        mi as f64,
        sec as f64,
        wd as f64,
        yd as f64,
    )))
}
