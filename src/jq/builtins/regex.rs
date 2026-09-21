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

thread_local! {
    /// Compiling a Rust regex is far costlier than running it once compiled;
    /// jq call sites like `sub`/`gsub`/`capture` recompile the same pattern on
    /// every input row, so cache by (pattern, flags) to amortize that cost.
    static REGEX_CACHE: RefCell<HashMap<(String, String), ::regex::Regex>> =
        RefCell::new(HashMap::new());
}

fn compile(args: &[Value]) -> Result<(::regex::Regex, bool, bool), JqError> {
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
    let regex = REGEX_CACHE.with(|cache| -> Result<::regex::Regex, JqError> {
        let mut cache = cache.borrow_mut();
        let key = (pattern.to_string(), flags.to_string());
        if let Some(regex) = cache.get(&key) {
            return Ok(regex.clone());
        }
        let regex = ::regex::RegexBuilder::new(pattern)
            .case_insensitive(flags.contains('i'))
            .multi_line(!flags.contains('s') && !flags.contains('p'))
            .dot_matches_new_line(flags.contains('m') || flags.contains('p'))
            .ignore_whitespace(flags.contains('x'))
            .build()
            .map_err(|e| error(format!("Rust regex: {e}")))?;
        cache.insert(key, regex.clone());
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
        name.map(|n| Value::String(n.to_string().into()))
            .unwrap_or(Value::Null),
    ));
    object(fields)
}
pub(crate) fn matches(input: &Value, args: &[Value]) -> Result<Vec<Value>, JqError> {
    let text = string(input)?;
    let (regex, global, no_empty) = compile(args)?;
    let names: Vec<_> = regex.capture_names().collect();
    let mut result = Vec::new();
    for captures in regex.captures_iter(text) {
        let matched = captures.get(0).unwrap();
        if no_empty && matched.is_empty() {
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
pub(crate) fn match_impl(input: &Value, args: &[Value]) -> Result<Value, JqError> {
    let testmode = matches!(args.get(2), Some(Value::Bool(true)));
    let found = matches(input, &args[..2])?;
    if testmode {
        Ok(Value::Bool(!found.is_empty()))
    } else {
        Ok(Value::Array(Rc::new(found)))
    }
}
