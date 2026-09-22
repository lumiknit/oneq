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

/// jq names both operands of the failed operation. `min`/`max` go through
/// its `_min_by_impl` pair, so there the same value is reported twice.
fn pair_error(a: &Value, b: &Value, reason: &str) -> JqError {
    error(format!(
        "{} ({}) and {} ({}) {reason}",
        a.type_name(),
        crate::jq::vm::value::truncated_repr(a),
        b.type_name(),
        crate::jq::vm::value::truncated_repr(b)
    ))
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
    let Value::Array(values) = input else {
        return Err(pair_error(input, input, "cannot be iterated over"));
    };
    Ok(values
        .iter()
        .min_by(|a, b| compare(a, b))
        .cloned()
        .unwrap_or(Value::Null))
}
pub fn max(input: &Value, _: &[Value]) -> Result<Value, JqError> {
    let Value::Array(values) = input else {
        return Err(pair_error(input, input, "cannot be iterated over"));
    };
    Ok(values
        .iter()
        .max_by(|a, b| compare(a, b))
        .cloned()
        .unwrap_or(Value::Null))
}

/// jq only defines containment between two strings, arrays or objects. Any
/// other pair is `true` when the two values are equal and `false` when both
/// are numbers; everything else is a type error, which is why
/// `1 | contains(2)` is `false` but `false | contains(true)` fails. The
/// recursion *inside* a container never type-checks, so
/// `[1] | contains(["a"])` is plain `false`.
pub fn contains(input: &Value, args: &[Value]) -> Result<Value, JqError> {
    let other = &args[0];
    match (input, other) {
        (Value::String(_), Value::String(_))
        | (Value::Array(_), Value::Array(_))
        | (Value::Object(_), Value::Object(_)) => Ok(Value::Bool(contains_value(input, other))),
        _ if input == other => Ok(Value::Bool(true)),
        _ if input.as_number().is_some() && other.as_number().is_some() => Ok(Value::Bool(false)),
        _ => Err(error(format!(
            "{} ({}) and {} ({}) cannot have their containment checked",
            input.type_name(),
            crate::jq::vm::value::truncated_repr(input),
            other.type_name(),
            crate::jq::vm::value::truncated_repr(other)
        ))),
    }
}

fn contains_value(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::String(a), Value::String(b)) => a.contains(&**b),
        (Value::Array(a), Value::Array(b)) => b
            .iter()
            .all(|b| a.iter().any(|a| contains_value(a, b))),
        (Value::Object(a), Value::Object(b)) => b
            .iter()
            .all(|(key, b)| a.get(key).is_some_and(|a| contains_value(a, b))),
        (a, b) => a == b,
    }
}

/// Zips `.` with an already-computed `keys` array (jq's `map([f])`) into the
/// `[[key], value]` pairs `sort_by_keys`/`group_sorted` expect.
fn zip_keys(input: &Value, keys: &Value) -> Result<Vec<Value>, JqError> {
    let Value::Array(items) = input else {
        return Err(pair_error(input, keys, "cannot be sorted, as they are not both arrays"));
    };
    let Value::Array(keys) = keys else {
        return Err(pair_error(input, keys, "cannot be sorted, as they are not both arrays"));
    };
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
