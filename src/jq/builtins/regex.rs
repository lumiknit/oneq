//! Rust regex is the intentional backend. Unsupported Oniguruma syntax errors
//! explicitly; no fallback to a second regex engine or external jq.
use super::strings::string;
use crate::{
    data::Value,
    jq::vm::{JqError, value::error},
    strs,
};
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

type RegexCache = HashMap<String, HashMap<String, Rc<::regex::Regex>>>;
thread_local! {
    /// Reuse both the compiled pattern and its search scratch pool. Cloning
    /// Regex creates a fresh pool; cloning Rc keeps that pool warm across rows.
    /// Nested maps also allow allocation-free lookups using borrowed strings.
    static REGEX_CACHE: RefCell<RegexCache> =
        RefCell::new(HashMap::new());
}

fn compile(args: &[Value]) -> Result<(Rc<::regex::Regex>, bool, bool), JqError> {
    let pattern = string(&args[0])?;
    let flags = match args.get(1) {
        None | Some(Value::Null) => "",
        Some(value) => string(value)?,
    };
    for flag in flags.chars() {
        if !"gimnpsx".contains(flag) {
            return Err(error(format!("unsupported Rust regex flag: {flag}")));
        }
    }
    let regex = REGEX_CACHE.with(|cache| -> Result<Rc<::regex::Regex>, JqError> {
        let mut cache = cache.borrow_mut();
        if let Some(regex) = cache.get(pattern).and_then(|patterns| patterns.get(flags)) {
            return Ok(regex.clone());
        }
        let regex = ::regex::RegexBuilder::new(pattern)
            .case_insensitive(flags.contains('i'))
            .multi_line(!flags.contains('s') && !flags.contains('p'))
            .dot_matches_new_line(flags.contains('m') || flags.contains('p'))
            .ignore_whitespace(flags.contains('x'))
            .build()
            .map_err(|e| error(format!("Rust regex: {e}")))?;
        let regex = Rc::new(regex);
        cache
            .entry(pattern.to_owned())
            .or_default()
            .insert(flags.to_owned(), regex.clone());
        Ok(regex)
    })?;
    Ok((regex, flags.contains('g'), flags.contains('n')))
}
fn object(fields: impl IntoIterator<Item = (&'static str, Value)>) -> Value {
    Value::Object(Rc::new(
        fields
            .into_iter()
            .map(|(k, v)| (strs::intern(k), v))
            .collect(),
    ))
}
fn record(text: &str, found: Option<::regex::Match<'_>>, name: Option<&str>) -> Value {
    let (offset, length, value) = match found {
        Some(m) => (
            Value::int(text[..m.start()].chars().count() as i64),
            Value::int(m.as_str().chars().count() as i64),
            Value::String(m.as_str().to_string().into()),
        ),
        None => (Value::int(-1), Value::int(0), Value::Null),
    };
    let mut fields = vec![("offset", offset), ("length", length), ("string", value)];
    fields.push((
        "name",
        name.map_or(Value::Null, |n| Value::String(n.to_string().into())),
    ));
    object(fields)
}
fn matches(input: &Value, args: &[Value], capture_only: bool) -> Result<Vec<Value>, JqError> {
    let text = string(input)?;
    let (regex, global, no_empty) = compile(args)?;
    let names: Vec<_> = regex.capture_names().collect();
    let mut result = Vec::new();
    for captures in regex.captures_iter(text) {
        let matched = captures.get(0).unwrap();
        if no_empty && matched.is_empty() {
            continue;
        }
        if capture_only {
            // capture() only needs named substrings, not match records,
            // character offsets, lengths, or a jq reduction over those records.
            let fields = names
                .iter()
                .enumerate()
                .filter_map(|(i, name)| {
                    name.map(|name| {
                        (
                            strs::intern(name),
                            captures.get(i).map_or(Value::Null, |m| {
                                Value::String(m.as_str().to_owned().into())
                            }),
                        )
                    })
                })
                .collect();
            result.push(Value::Object(Rc::new(fields)));
            if !global {
                break;
            }
            continue;
        }
        let items = (1..captures.len())
            .map(|i| record(text, captures.get(i), names[i]))
            .collect();
        result.push(object([
            (
                "offset",
                Value::int(text[..matched.start()].chars().count() as i64),
            ),
            (
                "length",
                Value::int(matched.as_str().chars().count() as i64),
            ),
            ("string", Value::String(matched.as_str().to_string().into())),
            ("captures", Value::Array(Rc::new(items))),
        ]));
        if !global {
            break;
        }
    }
    Ok(result)
}
/// jq's native `_match_impl(re; mode; testmode)`: `testmode` selects between a
/// boolean (as `test` wants) and the full match-record array (as `match` wants).
pub fn match_impl(input: &Value, args: &[Value]) -> Result<Value, JqError> {
    let testmode = matches!(args.get(2), Some(Value::Bool(true)));
    if testmode {
        let text = string(input)?;
        let (regex, _, no_empty) = compile(&args[..2])?;
        Ok(Value::Bool(if no_empty {
            regex.find_iter(text).any(|m| !m.is_empty())
        } else {
            regex.is_match(text)
        }))
    } else {
        Ok(Value::Array(Rc::new(matches(input, &args[..2], false)?)))
    }
}

pub fn capture_impl(input: &Value, args: &[Value]) -> Result<Value, JqError> {
    Ok(Value::Array(Rc::new(matches(input, args, true)?)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cache_hits_share_the_search_pool_and_keep_flags_distinct() {
        let pattern = Value::String("cache_probe".to_string().into());
        let plain = [pattern.clone(), Value::Null];
        let first = compile(&plain).unwrap().0;
        assert!(first.is_match("cache_probe"));
        assert!(Rc::ptr_eq(&first, &compile(&plain).unwrap().0));
        let insensitive = compile(&[pattern, Value::String("i".to_string().into())])
            .unwrap()
            .0;
        assert!(!Rc::ptr_eq(&first, &insensitive));
        assert!(!first.is_match("CACHE_PROBE"));
        assert!(insensitive.is_match("CACHE_PROBE"));
    }
}
