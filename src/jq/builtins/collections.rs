use crate::{
    data::Value,
    jq::vm::{JqError, value::error},
};
use std::{cmp::Ordering, rc::Rc};

fn array(value: &Value) -> Result<&Vec<Value>, JqError> {
    match value {
        Value::Array(a) => Ok(a),
        _ => Err(error("array required")),
    }
}

pub fn compare(a: &Value, b: &Value) -> Ordering {
    a.partial_cmp(b).unwrap_or(Ordering::Equal)
}

pub fn sort(input: &Value, _: &[Value]) -> Result<Value, JqError> {
    let mut values = match input {
        Value::Array(values) => values.as_ref().clone(),
        _ => {
            return Err(error(format!(
                "{} ({}) cannot be sorted, as it is not an array",
                input.type_name(),
                crate::jq::vm::value::truncated_repr(input)
            )));
        }
    };
    values.sort_by(compare);
    Ok(Value::Array(Rc::new(values)))
}

pub fn unique(input: &Value, args: &[Value]) -> Result<Value, JqError> {
    let Value::Array(values) = sort(input, args)? else {
        unreachable!()
    };
    let mut values = values.as_ref().clone();
    values.dedup();
    Ok(Value::Array(Rc::new(values)))
}

pub fn min(input: &Value, _: &[Value]) -> Result<Value, JqError> {
    Ok(array(input)?
        .iter()
        .min_by(|a, b| compare(a, b))
        .cloned()
        .unwrap_or(Value::Null))
}
pub fn max(input: &Value, _: &[Value]) -> Result<Value, JqError> {
    Ok(array(input)?
        .iter()
        .max_by(|a, b| compare(a, b))
        .cloned()
        .unwrap_or(Value::Null))
}

pub fn contains(input: &Value, args: &[Value]) -> Result<Value, JqError> {
    Ok(Value::Bool(contains_value(input, &args[0])?))
}

fn contains_value(a: &Value, b: &Value) -> Result<bool, JqError> {
    Ok(match (a, b) {
        (Value::String(a), Value::String(b)) => a.contains(&**b),
        (Value::Array(a), Value::Array(b)) => {
            for b in b.iter() {
                let mut found = false;
                for a in a.iter() {
                    if contains_value(a, b).unwrap_or(false) {
                        found = true;
                        break;
                    }
                }
                if !found {
                    return Ok(false);
                }
            }
            true
        }
        (Value::Object(a), Value::Object(b)) => {
            for (key, b) in b.iter() {
                let Some(a) = a.get(key) else {
                    return Ok(false);
                };
                if !contains_value(a, b)? {
                    return Ok(false);
                }
            }
            true
        }
        (a, b) if a.type_name() == b.type_name() => a == b,
        _ => return Err(error("incompatible types for contains")),
    })
}

/// Zips `.` with an already-computed `keys` array (jq's `map([f])`) into the
/// `[[key], value]` pairs `sort_by_keys`/`group_sorted` expect.
fn zip_keys(input: &Value, keys: &Value) -> Result<Vec<Value>, JqError> {
    let items = array(input)?;
    let keys = array(keys)?;
    if items.len() != keys.len() {
        return Err(error("_sort_by_impl: keys length mismatch"));
    }
    Ok(items
        .iter()
        .zip(keys.iter())
        .map(|(item, key)| Value::Array(Rc::new(vec![key.clone(), item.clone()])))
        .collect())
}
/// jq's native `_sort_by_impl(keys)`.
pub fn sort_by_impl(input: &Value, args: &[Value]) -> Result<Value, JqError> {
    let pairs = Value::Array(Rc::new(zip_keys(input, &args[0])?));
    let Value::Array(sorted) = sort_by_keys(&pairs, &[])? else {
        unreachable!()
    };
    Ok(Value::Array(Rc::new(
        sorted
            .iter()
            .map(|pair| array(pair).unwrap()[1].clone())
            .collect(),
    )))
}
/// jq's native `_group_by_impl(keys)`.
pub fn group_by_impl(input: &Value, args: &[Value]) -> Result<Value, JqError> {
    let pairs = Value::Array(Rc::new(zip_keys(input, &args[0])?));
    let Value::Array(sorted) = sort_by_keys(&pairs, &[])? else {
        unreachable!()
    };
    group_sorted(&Value::Array(sorted), &[])
}
/// jq's native `_min_by_impl(keys)`.
pub fn min_by_impl(input: &Value, args: &[Value]) -> Result<Value, JqError> {
    let Value::Array(sorted) = sort_by_impl(input, args)? else {
        unreachable!()
    };
    Ok(sorted.first().cloned().unwrap_or(Value::Null))
}
/// jq's native `_max_by_impl(keys)`.
pub fn max_by_impl(input: &Value, args: &[Value]) -> Result<Value, JqError> {
    let Value::Array(sorted) = sort_by_impl(input, args)? else {
        unreachable!()
    };
    Ok(sorted.last().cloned().unwrap_or(Value::Null))
}
/// jq's native `_unique_by_impl(keys)`.
pub fn unique_by_impl(input: &Value, args: &[Value]) -> Result<Value, JqError> {
    let Value::Array(groups) = group_by_impl(input, args)? else {
        unreachable!()
    };
    Ok(Value::Array(Rc::new(
        groups
            .iter()
            .map(|group| array(group).unwrap()[0].clone())
            .collect(),
    )))
}
pub fn sort_by_keys(input: &Value, _: &[Value]) -> Result<Value, JqError> {
    let mut pairs = array(input)?.clone();
    pairs.sort_by(|a, b| match (a, b) {
        (Value::Array(a), Value::Array(b)) => compare(&a[0], &b[0]),
        _ => Ordering::Equal,
    });
    Ok(Value::Array(Rc::new(pairs)))
}

pub fn group_sorted(input: &Value, _: &[Value]) -> Result<Value, JqError> {
    let mut groups: Vec<Value> = Vec::new();
    let mut key: Option<Value> = None;
    let mut group = Vec::new();
    for pair in array(input)? {
        let pair = array(pair)?;
        if pair.len() != 2 {
            return Err(error("invalid grouping pair"));
        }
        if key.as_ref().is_some_and(|k| k != &pair[0]) {
            groups.push(Value::Array(Rc::new(std::mem::take(&mut group))));
        }
        key = Some(pair[0].clone());
        group.push(pair[1].clone());
    }
    if key.is_some() {
        groups.push(Value::Array(Rc::new(group)));
    }
    Ok(Value::Array(Rc::new(groups)))
}
