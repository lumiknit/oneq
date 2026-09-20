//! Shared value operations for path bytecode and scalar builtins.
use super::JqError;
use crate::{data::Value, strs};
use std::rc::Rc;

pub(crate) fn error(message: impl Into<String>) -> JqError {
    JqError::Runtime(Value::String(message.into().into()))
}
pub(crate) fn index(base: &Value, key: &Value) -> Result<Value, JqError> {
    match (base, key) {
        // `.[{start, end}]` is jq's slice-by-object shorthand, usable anywhere `.[expr]` is.
        (_, Value::Object(bounds))
            if matches!(base, Value::Null | Value::Array(_) | Value::String(_)) =>
        {
            slice(
                base,
                bounds.get(&strs::keyword_start()).unwrap_or(&Value::Null),
                bounds.get(&strs::keyword_end()).unwrap_or(&Value::Null),
            )
        }
        // `.[b]` on two arrays finds every offset where `b` occurs as a subsequence of `.`.
        (Value::Array(a), Value::Array(b)) => Ok(Value::Array(Rc::new(if b.is_empty() {
            vec![]
        } else {
            a.windows(b.len())
                .enumerate()
                .filter_map(|(i, w)| (w == b.as_slice()).then_some(Value::int(i as i64)))
                .collect()
        }))),
        (Value::Null, Value::String(_) | Value::Decimal(_) | Value::Float(_)) => Ok(Value::Null),
        (Value::Object(map), Value::String(key)) => {
            Ok(map.get(&strs::intern(key)).cloned().unwrap_or(Value::Null))
        }
        (Value::Array(array), Value::Decimal(_) | Value::Float(_)) => {
            let n = key
                .as_number()
                .ok_or_else(|| error("invalid array index"))?;
            if !n.is_finite() {
                return Ok(Value::Null);
            }
            let mut n = n.floor() as i64;
            if n < 0 {
                n = n.saturating_add(array.len() as i64);
            }
            Ok(usize::try_from(n)
                .ok()
                .and_then(|n| array.get(n))
                .cloned()
                .unwrap_or(Value::Null))
        }
        _ => Err(error(format!(
            "Cannot index {} with {} ({})",
            base.type_name(),
            key.type_name(),
            truncated_repr(key)
        ))),
    }
}

/// Renders a value for an error message the way jq does: full JSON for
/// values whose rendering is at most 14 characters, truncated to the first
/// 11 characters plus a trailing "..." otherwise - jq leaves the quote or
/// bracket unclosed either way.
/// jq truncates long values in error messages to a fixed *total* rendered
/// length of 29 bytes: below that, the value prints in full; above it, the
/// delimited content (inside `"..."`/`[...]`/`{...}`, or the bare digits for
/// a number) is cut to whatever fits alongside `...` and the closing
/// delimiter (if any) within that same 29-byte budget - e.g. a 30+ char
/// string truncates to 24 kept characters (`1 open + 24 + 3 dots + 1 close`)
/// while a 30+ digit number keeps 26 (`26 + 3 dots`, no delimiters at all).
/// Verified empirically against real jq by bisecting the exact
/// truncate/don't-truncate boundary for both shapes.
pub(crate) fn truncated_repr(value: &Value) -> String {
    const MAX_TOTAL: usize = 29;
    let rendered = value.to_compact_json();
    if rendered.len() <= MAX_TOTAL {
        return rendered;
    }
    let (open, close, content): (&str, &str, &str) = match rendered.as_bytes().first() {
        Some(b'"') => ("\"", "\"", &rendered[1..rendered.len() - 1]),
        Some(b'[') => ("[", "]", &rendered[1..rendered.len() - 1]),
        Some(b'{') => ("{", "}", &rendered[1..rendered.len() - 1]),
        _ => ("", "", rendered.as_str()),
    };
    let keep = MAX_TOTAL.saturating_sub(open.len() + close.len() + 3);
    let mut end = keep.min(content.len());
    while !content.is_char_boundary(end) {
        end -= 1;
    }
    format!("{open}{}...{close}", &content[..end])
}
pub(crate) fn slice(base: &Value, start: &Value, end: &Value) -> Result<Value, JqError> {
    let length = match base {
        Value::Null => return Ok(Value::Null),
        Value::Array(a) => a.len(),
        Value::String(s) => s.chars().count(),
        _ => return Err(error(format!("Cannot slice {}", base.type_name()))),
    };
    // jq rounds the start bound down and the end bound up (so
    // `.[1.2:3.5]` keeps indices 1..4, i.e. 3 elements) - and treats `nan`
    // exactly like an omitted bound (`null`) rather than as a number.
    let bound = |v: &Value, default: f64| -> Result<f64, JqError> {
        if matches!(v, Value::Null) {
            return Ok(default);
        }
        let n = v
            .as_number()
            .ok_or_else(|| error("slice bounds must be numbers"))?;
        if n.is_nan() {
            return Ok(default);
        }
        let n = if n < 0.0 { n + length as f64 } else { n };
        Ok(n.clamp(0.0, length as f64))
    };
    let start = bound(start, 0.0)?.floor() as usize;
    let end = (bound(end, length as f64)?.ceil() as usize).clamp(start, length);
    Ok(match base {
        Value::Array(a) => Value::Array(Rc::new(a[start..end].to_vec())),
        Value::String(s) => Value::String(
            s.chars()
                .skip(start)
                .take(end - start)
                .collect::<String>()
                .into(),
        ),
        _ => unreachable!(),
    })
}

pub(crate) fn getpath(root: &Value, path: &[Value]) -> Result<Value, JqError> {
    let mut value = root.clone();
    for key in path {
        value = index(&value, key)?;
    }
    Ok(value)
}
pub(crate) fn setpath(root: &Value, path: &[Value], replacement: &Value) -> Result<Value, JqError> {
    let Some((key, rest)) = path.split_first() else {
        return Ok(replacement.clone());
    };
    match key {
        Value::String(key) => {
            let mut map = match root {
                Value::Null => indexmap::IndexMap::new(),
                Value::Object(map) => map.as_ref().clone(),
                _ => return Err(error("Cannot set object key on non-object")),
            };
            let key = strs::intern(key);
            let old = map.get(&key).unwrap_or(&Value::Null);
            let value = setpath(old, rest, replacement)?;
            map.insert(key, value);
            Ok(Value::Object(Rc::new(map)))
        }
        Value::Decimal(_) | Value::Float(_) => {
            let mut array = match root {
                Value::Null => Vec::new(),
                Value::Array(array) => array.as_ref().clone(),
                _ => {
                    return Err(error(format!(
                        "Cannot index {} with {} ({})",
                        root.type_name(),
                        key.type_name(),
                        truncated_repr(key)
                    )));
                }
            };
            let n = key
                .as_number()
                .ok_or_else(|| error("invalid array index"))?;
            if n.is_nan() {
                return Err(error("Cannot set array element at NaN index"));
            }
            if !n.is_finite() {
                return Err(error("invalid array index"));
            }
            let n = n.floor();
            let n = if n < 0.0 { n + array.len() as f64 } else { n };
            if n < 0.0 {
                return Err(error("Out of bounds negative array index"));
            }
            // Matches jq's own array size cap (1 << 29 elements) - without
            // this, an index like `999999999` would allocate a ~1B-element
            // array of nulls instead of erroring.
            if n >= (1u64 << 29) as f64 {
                return Err(error("Array index too large"));
            }
            let n = n as usize;
            if n >= array.len() {
                array
                    .try_reserve(n + 1 - array.len())
                    .map_err(|_| error("array too large"))?;
                array.resize(n + 1, Value::Null);
            }
            array[n] = setpath(&array[n], rest, replacement)?;
            Ok(Value::Array(Rc::new(array)))
        }
        Value::Object(bounds) => {
            if matches!(root, Value::String(_)) {
                return Err(error("Cannot update string slices"));
            }
            let Value::Array(array) = root else {
                return Err(error("slice assignment requires an array"));
            };
            let start = bounds.get(&strs::keyword_start()).unwrap_or(&Value::Null);
            let end = bounds.get(&strs::keyword_end()).unwrap_or(&Value::Null);
            let bound = |value: &Value, default: f64| -> Result<f64, JqError> {
                if matches!(value, Value::Null) {
                    return Ok(default);
                }
                let n = value
                    .as_number()
                    .ok_or_else(|| error("invalid slice bound"))?;
                if n.is_nan() {
                    return Ok(default);
                }
                Ok((if n < 0.0 { n + array.len() as f64 } else { n })
                    .clamp(0.0, array.len() as f64))
            };
            let start = bound(start, 0.0)?.floor() as usize;
            let end = (bound(end, array.len() as f64)?.ceil() as usize).clamp(start, array.len());
            let old = Value::Array(Rc::new(array[start..end].to_vec()));
            let Value::Array(new) = setpath(&old, rest, replacement)? else {
                return Err(error("slice replacement must be an array"));
            };
            let mut result = array.as_ref().clone();
            result.splice(start..end, new.iter().cloned());
            Ok(Value::Array(Rc::new(result)))
        }
        Value::Array(_) if matches!(root, Value::Array(_)) => {
            Err(error("Cannot update field at array index of array"))
        }
        _ => Err(error(format!(
            "Cannot index {} with {} ({})",
            root.type_name(),
            key.type_name(),
            truncated_repr(key)
        ))),
    }
}
fn normalize_slice_bounds(
    current: &Value,
    bounds: &indexmap::IndexMap<strs::Symbol, Value>,
) -> Result<Value, JqError> {
    let length = match current {
        Value::Array(a) => a.len(),
        Value::String(s) => s.chars().count(),
        _ => 0,
    };
    let bound = |v: Option<&Value>, default: usize| -> Result<usize, JqError> {
        match v {
            None | Some(Value::Null) => Ok(default),
            Some(v) => {
                let n = v
                    .as_number()
                    .ok_or_else(|| error("slice bounds must be numbers"))?;
                let n = if n < 0.0 { n + length as f64 } else { n };
                Ok(n.clamp(0.0, length as f64).trunc() as usize)
            }
        }
    };
    let start = bound(bounds.get(&strs::keyword_start()), 0)?;
    let end = bound(bounds.get(&strs::keyword_end()), length)?.max(start);
    let mut map = indexmap::IndexMap::new();
    map.insert(strs::keyword_start(), Value::int(start as i64));
    map.insert(strs::keyword_end(), Value::int(end as i64));
    Ok(Value::Object(Rc::new(map)))
}
fn delete(root: &Value, path: &[Value]) -> Result<Value, JqError> {
    let Some((key, rest)) = path.split_first() else {
        return Ok(Value::Null);
    };
    match (root, key) {
        (Value::Null, _) => Ok(Value::Null),
        (Value::Object(map), Value::String(key)) => {
            let mut map = map.as_ref().clone();
            let key = strs::intern(key);
            if rest.is_empty() {
                map.shift_remove(&key);
            } else if let Some(old) = map.get(&key) {
                let value = delete(old, rest)?;
                map.insert(key, value);
            }
            Ok(Value::Object(Rc::new(map)))
        }
        (Value::Array(array), Value::Decimal(_) | Value::Float(_)) => {
            let mut array = array.as_ref().clone();
            let n = key
                .as_number()
                .ok_or_else(|| error("invalid array index"))?
                .floor();
            let n = if n < 0.0 { n + array.len() as f64 } else { n };
            if n >= 0.0 && n < array.len() as f64 {
                let n = n as usize;
                if rest.is_empty() {
                    array.remove(n);
                } else {
                    array[n] = delete(&array[n], rest)?;
                }
            }
            Ok(Value::Array(Rc::new(array)))
        }
        (Value::Array(array), Value::Object(bounds)) => {
            let mut array = array.as_ref().clone();
            let start = bounds
                .get(&strs::keyword_start())
                .and_then(Value::as_number)
                .unwrap_or(0.0) as usize;
            let end = (bounds
                .get(&strs::keyword_end())
                .and_then(Value::as_number)
                .unwrap_or(array.len() as f64) as usize)
                .max(start)
                .min(array.len());
            let start = start.min(end);
            if rest.is_empty() {
                array.drain(start..end);
            } else {
                let old = Value::Array(Rc::new(array[start..end].to_vec()));
                let Value::Array(new) = delete(&old, rest)? else {
                    return Err(error("invalid deletion path"));
                };
                array.splice(start..end, new.iter().cloned());
            }
            Ok(Value::Array(Rc::new(array)))
        }
        _ => Err(error("invalid deletion path")),
    }
}
pub(crate) fn delpaths(root: &Value, paths: &[Value]) -> Result<Value, JqError> {
    let mut normalized = Vec::new();
    for path in paths {
        let Value::Array(path) = path else {
            return Err(error(format!(
                "Path must be specified as array, not {}",
                path.type_name()
            )));
        };
        let mut current = root.clone();
        let mut normalized_path = Vec::new();
        for key in path.iter() {
            let key = match (&current, key) {
                (Value::Array(a), _) if key.as_number().is_some_and(|n| n < 0.0) => {
                    Value::Float(key.as_number().unwrap().floor() + a.len() as f64)
                }
                // Resolve `.[{start,end}]` slice paths to concrete, clamped
                // bounds up front so descending-order sorting below (needed
                // to delete right-to-left without invalidating earlier
                // indices) compares actual positions rather than raw,
                // possibly-negative/null bounds.
                (Value::Array(_) | Value::String(_) | Value::Null, Value::Object(bounds)) => {
                    normalize_slice_bounds(&current, bounds)?
                }
                _ => key.clone(),
            };
            current = index(&current, &key)?;
            normalized_path.push(key);
        }
        normalized.push(normalized_path);
    }
    normalized.sort_by(|a, b| b.partial_cmp(a).unwrap_or(std::cmp::Ordering::Equal));
    normalized.dedup();
    let mut result = root.clone();
    for path in normalized {
        result = delete(&result, &path)?;
    }
    Ok(result)
}
