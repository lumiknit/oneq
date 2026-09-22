//! Scalar semantics shared by VM dispatch and future calc recipes.
use crate::{data::Value, jq::vm::JqError};
use std::io::Write;

pub fn type_name(input: &Value, _: &[Value]) -> Result<Value, JqError> {
    Ok(Value::String(input.type_name().to_string().into()))
}
/// jq's `debug`: prints `["DEBUG:", .]` as JSON to stderr, and passes the input through.
pub fn debug(input: &Value, _: &[Value]) -> Result<Value, JqError> {
    writeln!(
        crate::io::stderr(),
        "[\"DEBUG:\",{}]",
        input.to_compact_json()
    )
    .map_err(|e| JqError::Runtime(Value::String(e.to_string().into())))?;
    Ok(input.clone())
}
/// jq's `stderr`: prints `.` in raw-and-compact mode to stderr with no
/// decoration, no trailing newline. "Raw" here means a string prints
/// unquoted, unlike `debug`; every other type still prints as compact JSON.
pub fn stderr(input: &Value, _: &[Value]) -> Result<Value, JqError> {
    let result = match input {
        Value::String(s) => write!(crate::io::stderr(), "{s}"),
        other => write!(crate::io::stderr(), "{}", other.to_compact_json()),
    };
    result.map_err(|e| JqError::Runtime(Value::String(e.to_string().into())))?;
    Ok(input.clone())
}
pub fn length(input: &Value, _: &[Value]) -> Result<Value, JqError> {
    let length = match input {
        Value::Null => 0,
        Value::String(s) => s.chars().count(),
        Value::Array(a) => a.len(),
        Value::Object(o) => o.len(),
        Value::Decimal(n) => return Ok(Value::decimal(n.abs())),
        Value::Float(n) => return Ok(Value::Float(n.abs())),
        _ => {
            return Err(JqError::Runtime(Value::String(
                format!(
                    "{} ({}) has no length",
                    input.type_name(),
                    crate::jq::vm::value::truncated_repr(input)
                )
                .into(),
            )));
        }
    };
    // A counted length is a plain double in jq, unlike the `abs` of a number
    // literal above which keeps its exact decimal value. Only the double
    // carries IEEE's signed zero, so `[] | length | -.` is `-0`.
    Ok(Value::Float(length as f64))
}

use crate::{jq::vm::value::error, strs};
use std::rc::Rc;

pub fn add_owned(args: &mut [Value]) -> Result<Value, JqError> {
    let (a, b) = (std::mem::take(&mut args[0]), std::mem::take(&mut args[1]));
    Ok(match (a, b) {
        (Value::Null, b) => b,
        (a, Value::Null) => a,
        // Unlike unary negation (which preserves a `Decimal` literal's exact
        // value), binary arithmetic always goes through `f64` - matches
        // jq's own decNum mode, which round-trips *unmodified* literals
        // exactly but still truncates any arithmetic result to double
        // precision (verified against real jq: `13911860366432393 - 10` is
        // `13911860366432382`, i.e. the literal rounds to the nearest
        // double *before* subtracting, not exact-bigint-minus-10).
        (Value::String(mut a), Value::String(b)) => {
            // Reuse a unique accumulator; saved bindings and continuations
            // still force a copy through the same COW rule as arrays.
            Rc::make_mut(&mut a).push_str(&b);
            Value::String(a)
        }
        (Value::Array(mut a), Value::Array(b)) => {
            Rc::make_mut(&mut a).extend(b.iter().cloned());
            Value::Array(a)
        }
        (Value::Object(mut a), Value::Object(b)) => {
            Rc::make_mut(&mut a).extend(b.iter().map(|(k, v)| (*k, v.clone())));
            Value::Object(a)
        }
        (a, b) => match (a.as_number(), b.as_number()) {
            (Some(a), Some(b)) => Value::Float(a + b),
            _ => return Err(arith_type_error("added", &a, &b)),
        },
    })
}
pub fn number(value: &Value) -> Result<f64, JqError> {
    value.as_number().ok_or_else(|| {
        error(format!(
            "{} ({}) number required",
            value.type_name(),
            crate::jq::vm::value::truncated_repr(value)
        ))
    })
}
/// jq's `"{a} ({repr}) and {b} ({repr}) cannot be {added,subtracted,...}"`,
/// used when a binary arithmetic op's operand types don't have a defined
/// combination (e.g. a string minus a number).
fn arith_type_error(verb: &str, a: &Value, b: &Value) -> JqError {
    error(format!(
        "{} ({}) and {} ({}) cannot be {verb}",
        a.type_name(),
        crate::jq::vm::value::truncated_repr(a),
        b.type_name(),
        crate::jq::vm::value::truncated_repr(b),
    ))
}
pub fn subtract(_: &Value, args: &[Value]) -> Result<Value, JqError> {
    Ok(match (&args[0], &args[1]) {
        (Value::Array(a), Value::Array(b)) => Value::Array(Rc::new(
            a.iter().filter(|v| !b.contains(v)).cloned().collect(),
        )),
        (a, b) => match (a.as_number(), b.as_number()) {
            (Some(na), Some(nb)) => Value::Float(na - nb),
            _ => return Err(arith_type_error("subtracted", a, b)),
        },
    })
}
/// jq's `abs` is native (not the naive `if . < 0 then -. else . end`) -
/// among other things, that jq-language version doesn't clear the sign on
/// `-0` (`-0 < 0` is false, so the `else` branch returns `.` = `-0`
/// unchanged), but real jq's native `abs` does normalize `-0` to `0`.
/// jq preserves strings, arrays, and objects through native `abs`; booleans
/// and null still use the negation error path.
pub fn abs(input: &Value, _: &[Value]) -> Result<Value, JqError> {
    match input {
        Value::Decimal(n) => Ok(Value::decimal(n.abs())),
        Value::Float(f) => Ok(Value::Float(f.abs())),
        Value::String(_) | Value::Array(_) | Value::Object(_) => Ok(input.clone()),
        _ => Err(error(format!(
            "{} ({}) cannot be negated",
            input.type_name(),
            crate::jq::vm::value::truncated_repr(input)
        ))),
    }
}
pub fn negate(_: &Value, args: &[Value]) -> Result<Value, JqError> {
    Ok(match &args[0] {
        Value::Decimal(n) => Value::decimal(n.negate()),
        n @ Value::Float(_) => Value::Float(-number(n)?),
        n => {
            return Err(crate::jq::vm::value::error(format!(
                "{} ({}) cannot be negated",
                n.type_name(),
                crate::jq::vm::value::truncated_repr(n)
            )));
        }
    })
}
pub fn multiply(_: &Value, args: &[Value]) -> Result<Value, JqError> {
    Ok(match (&args[0], &args[1]) {
        (Value::String(s), n) | (n, Value::String(s)) if n.as_number().is_some() => {
            let n = number(n)?;
            if n < 0.0 || n.is_nan() {
                Value::Null
            } else if s.is_empty() {
                // Repeating an empty string is always empty, regardless of
                // `n` - short-circuit instead of looping `n` times (`n` can
                // be up to `usize::MAX`, i.e. a no-op billions-of-iterations
                // loop) doing nothing each time.
                Value::String(String::new().into())
            } else {
                if !n.is_finite() || n > usize::MAX as f64 {
                    return Err(error("Repeat string result too long"));
                }
                let n = n as usize;
                let size = s
                    .len()
                    .checked_mul(n)
                    .ok_or_else(|| error("Repeat string result too long"))?;
                // Matches jq's own cap (roughly `i32::MAX` bytes) on the
                // repeated result - without it, `"abc" * 1000000000` would
                // try to allocate/copy ~3 GiB before erroring anyway.
                if size > i32::MAX as usize {
                    return Err(error("Repeat string result too long"));
                }
                Value::String(s.repeat(n).into())
            }
        }
        (Value::Object(a), Value::Object(b)) => {
            let mut result = a.as_ref().clone();
            for (key, value) in b.iter() {
                let value = match (result.get(key), value) {
                    (Some(old @ Value::Object(_)), Value::Object(_)) => {
                        multiply(&Value::Null, &[old.clone(), value.clone()])?
                    }
                    _ => value.clone(),
                };
                result.insert(*key, value);
            }
            Value::Object(Rc::new(result))
        }
        (a, b) if a.as_number().is_some() && b.as_number().is_some() => {
            Value::Float(number(a)? * number(b)?)
        }
        (a, b) => {
            return Err(error(format!(
                "{} ({}) and {} ({}) cannot be multiplied",
                a.type_name(),
                crate::jq::vm::value::truncated_repr(a),
                b.type_name(),
                crate::jq::vm::value::truncated_repr(b),
            )));
        }
    })
}
pub fn divide(_: &Value, args: &[Value]) -> Result<Value, JqError> {
    if let (Value::String(s), Value::String(separator)) = (&args[0], &args[1]) {
        return Ok(Value::Array(Rc::new(if separator.is_empty() {
            s.chars()
                .map(|c| Value::String(c.to_string().into()))
                .collect()
        } else {
            s.split(&**separator)
                .map(|s| Value::String(s.to_string().into()))
                .collect()
        })));
    }
    // `number()` would report the one bad operand ("array ([1]) number
    // required"); jq names both for an operator, so check the pair first.
    let (Some(dividend), Some(divisor)) = (args[0].as_number(), args[1].as_number()) else {
        return Err(arith_type_error("divided", &args[0], &args[1]));
    };
    if divisor == 0.0 {
        return Err(error(format!(
            "{} ({}) and {} ({}) cannot be divided because the divisor is zero",
            args[0].type_name(),
            crate::jq::vm::value::truncated_repr(&args[0]),
            args[1].type_name(),
            crate::jq::vm::value::truncated_repr(&args[1]),
        )));
    }
    Ok(Value::Float(dividend / divisor))
}
fn remainder_by_zero_error(a: &Value, b: &Value) -> JqError {
    error(format!(
        "{} ({}) and {} ({}) cannot be divided (remainder) because the divisor is zero",
        a.type_name(),
        crate::jq::vm::value::truncated_repr(a),
        b.type_name(),
        crate::jq::vm::value::truncated_repr(b),
    ))
}
pub fn modulo(_: &Value, args: &[Value]) -> Result<Value, JqError> {
    let (Some(a_f), Some(b_f)) = (args[0].as_number(), args[1].as_number()) else {
        return Err(arith_type_error("divided (remainder)", &args[0], &args[1]));
    };
    if a_f.is_nan() || b_f.is_nan() {
        return Ok(Value::Float(f64::NAN));
    }
    // jq casts both operands to (64-bit) integers before taking the
    // remainder, saturating infinities to i64::MAX/MIN rather than
    // producing NaN the way a plain float `%` would.
    let a = a_f as i64;
    let b = b_f as i64;
    if b == 0 {
        return Err(remainder_by_zero_error(&args[0], &args[1]));
    }
    Ok(Value::Float(a.wrapping_rem(b) as f64))
}
macro_rules! compare {
    ($name:ident, $op:tt) => {
        pub fn $name(_: &Value, args: &[Value]) -> Result<Value,JqError> { Ok(Value::Bool(args[0] $op args[1])) }
    }
}
compare!(equal, ==);
compare!(unequal, !=);
compare!(less, <);
compare!(less_equal, <=);
compare!(greater, >);
compare!(greater_equal, >=);
pub const fn not(input: &Value, _: &[Value]) -> Result<Value, JqError> {
    Ok(Value::Bool(matches!(
        input,
        Value::Null | Value::Bool(false)
    )))
}
pub fn tostring(input: &Value, _: &[Value]) -> Result<Value, JqError> {
    Ok(match input {
        Value::String(_) => input.clone(),
        _ => Value::String(input.to_compact_json().into()),
    })
}
pub fn tojson(input: &Value, _: &[Value]) -> Result<Value, JqError> {
    Ok(Value::String(input.to_compact_json().into()))
}
pub fn fromjson(input: &Value, _: &[Value]) -> Result<Value, JqError> {
    let Value::String(s) = input else {
        return Err(error("fromjson requires a string"));
    };
    crate::data::parse_json_str(s).map_err(error)
}

pub fn tonumber(input: &Value, _: &[Value]) -> Result<Value, JqError> {
    let invalid = || {
        error(format!(
            "{} ({}) cannot be parsed as a number",
            input.type_name(),
            crate::jq::vm::value::truncated_repr(input)
        ))
    };
    match input {
        Value::Decimal(_) | Value::Float(_) => Ok(input.clone()),
        // jq's tonumber parses like a C `strtod`/`strtoll`: a leading `+` is
        // fine, but (unlike JSON's own permissive-whitespace parsing)
        // leading/trailing whitespace is not - `" 4"` and `"5 "` are both
        // rejected even though `"4"` and `"5"` parse fine.
        Value::String(s) if s.trim() == **s => {
            let stripped = s.strip_prefix('+').unwrap_or(s);
            match crate::data::parse_json_str_detailed(stripped) {
                Ok(value @ (Value::Decimal(_) | Value::Float(_))) => Ok(value),
                _ => Err(invalid()),
            }
        }
        _ => Err(invalid()),
    }
}

pub fn keys_unsorted(input: &Value, _: &[Value]) -> Result<Value, JqError> {
    Ok(Value::Array(Rc::new(match input {
        Value::Array(a) => (0..a.len()).map(|i| Value::int(i as i64)).collect(),
        Value::Object(o) => o
            .keys()
            .map(|k| Value::String(strs::resolve(*k).unwrap_or("").to_string().into()))
            .collect(),
        _ => {
            return Err(error(format!(
                "{} ({}) has no keys",
                input.type_name(),
                crate::jq::vm::value::truncated_repr(input)
            )));
        }
    })))
}
pub fn keys(input: &Value, args: &[Value]) -> Result<Value, JqError> {
    let Value::Array(a) = keys_unsorted(input, args)? else {
        unreachable!()
    };
    let mut a = a.as_ref().clone();
    a.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    Ok(Value::Array(Rc::new(a)))
}
pub fn has(input: &Value, args: &[Value]) -> Result<Value, JqError> {
    Ok(Value::Bool(match (input, &args[0]) {
        (Value::Null, _) => false,
        (Value::Object(o), Value::String(k)) => o.contains_key(&strs::intern(k)),
        (Value::Array(a), v) => {
            let n = number(v)?;
            n >= 0.0 && n.fract() == 0.0 && n < a.len() as f64
        }
        _ => return Err(error("invalid has argument")),
    }))
}

pub fn getpath(input: &Value, args: &[Value]) -> Result<Value, JqError> {
    let Value::Array(path) = &args[0] else {
        return Err(error("path must be an array"));
    };
    crate::jq::vm::value::getpath(input, path)
}
pub fn setpath(input: &Value, args: &[Value]) -> Result<Value, JqError> {
    let Value::Array(path) = &args[0] else {
        return Err(error("path must be an array"));
    };
    crate::jq::vm::value::setpath(input, path, &args[1])
}
pub fn delpaths(input: &Value, args: &[Value]) -> Result<Value, JqError> {
    let Value::Array(paths) = &args[0] else {
        return Err(error("Paths must be specified as an array"));
    };
    crate::jq::vm::value::delpaths(input, paths)
}

pub fn builtins(_: &Value, _: &[Value]) -> Result<Value, JqError> {
    static NAMES: std::sync::OnceLock<Vec<String>> = std::sync::OnceLock::new();
    let names = NAMES.get_or_init(|| {
        let mut names: Vec<_> = super::registry()
            .iter()
            .map(|spec| format!("{}/{}", spec.name, spec.params.len()))
            .collect();
        for (path, source) in [
            ("<builtin.jq>", super::BUILTIN_JQ),
            ("<compat.jq>", super::COMPAT_JQ),
        ] {
            let (files, root) = crate::jq::parser::parse_pairs(path, source)
                .expect("embedded jq library must parse");
            let mut pending: Vec<_> = root.semantic_children().collect();
            while let Some(pair) = pending.pop() {
                if pair.tag == crate::jq::parser::pairs::PairTag::Def {
                    let children: Vec<_> = pair.semantic_children().collect();
                    names.push(format!(
                        "{}/{}",
                        pair.text(&files).unwrap(),
                        children.len() - 2
                    ));
                    pending.push(children[children.len() - 1]);
                }
            }
        }
        names.retain(|name| name.as_bytes().first().is_some_and(u8::is_ascii_alphabetic));
        names.sort();
        names.dedup();
        names
    });
    Ok(Value::Array(Rc::new(
        names
            .iter()
            .cloned()
            .map(|s| Value::String(s.into()))
            .collect(),
    )))
}

#[cfg(test)]
mod abs_tests {
    use super::abs;
    use crate::data::Value;
    use std::rc::Rc;

    #[test]
    fn abs_preserves_non_scalar_containers_like_jq() {
        let array = Value::Array(Rc::new(vec![Value::int(1)]));
        let object = Value::Object(Rc::new(indexmap::IndexMap::new()));
        assert_eq!(abs(&array, &[]).unwrap(), array);
        assert_eq!(abs(&object, &[]).unwrap(), object);
        assert_eq!(
            abs(&Value::String(Rc::new("x".to_owned())), &[]).unwrap(),
            Value::String(Rc::new("x".to_owned()))
        );
    }

    #[test]
    fn abs_rejects_boolean_and_null() {
        assert!(abs(&Value::Bool(true), &[]).is_err());
        assert!(abs(&Value::Null, &[]).is_err());
    }
}
