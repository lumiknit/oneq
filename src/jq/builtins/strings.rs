use crate::{
    data::Value,
    jq::vm::{JqError, value::error},
};
use std::rc::Rc;
pub(crate) fn string(value: &Value) -> Result<&str, JqError> {
    match value {
        Value::String(s) => Ok(s),
        _ => Err(error("string required")),
    }
}
pub(crate) fn utf8bytelength(input: &Value, _: &[Value]) -> Result<Value, JqError> {
    match input {
        Value::String(s) => Ok(Value::int(s.len() as i64)),
        _ => Err(error(format!(
            "{} ({}) only strings have UTF-8 byte length",
            input.type_name(),
            crate::jq::vm::value::truncated_repr(input)
        ))),
    }
}
pub(crate) fn explode(input: &Value, _: &[Value]) -> Result<Value, JqError> {
    let Value::String(input) = input else {
        return Err(error("explode input must be a string"));
    };
    Ok(Value::Array(Rc::new(
        input.chars().map(|c| Value::int(c as i64)).collect(),
    )))
}
pub(crate) fn ascii_downcase(input: &Value, _: &[Value]) -> Result<Value, JqError> {
    let Value::String(s) = input else {
        // Preserve the error from the former explode/map/implode definition.
        return Err(error("explode input must be a string"));
    };
    if !s.bytes().any(|b| b.is_ascii_uppercase()) {
        return Ok(input.clone());
    }
    Ok(Value::String(s.to_ascii_lowercase().into()))
}
pub(crate) fn ascii_upcase(input: &Value, _: &[Value]) -> Result<Value, JqError> {
    let Value::String(s) = input else {
        return Err(error("explode input must be a string"));
    };
    if !s.bytes().any(|b| b.is_ascii_lowercase()) {
        return Ok(input.clone());
    }
    Ok(Value::String(s.to_ascii_uppercase().into()))
}
pub(crate) fn implode(input: &Value, _: &[Value]) -> Result<Value, JqError> {
    let Value::Array(a) = input else {
        return Err(error("implode input must be an array"));
    };
    let mut result = String::new();
    for value in a.iter() {
        let n = value.as_number().filter(|n| !n.is_nan()).ok_or_else(|| {
            error(format!(
                "{} ({}) can't be imploded, unicode codepoint needs to be numeric",
                value.type_name(),
                value.to_compact_json()
            ))
        })?;
        // Out-of-range codepoints (negative, beyond U+10FFFF, or a UTF-16
        // surrogate) aren't an error - jq substitutes U+FFFD, same as an
        // invalid codepoint anywhere else. Non-integers round down.
        result.push(char::from_u32(n.floor() as i64 as u32).unwrap_or('\u{fffd}'));
    }
    Ok(Value::String(result.into()))
}
pub(crate) fn startswith(input: &Value, args: &[Value]) -> Result<Value, JqError> {
    let (Value::String(s), Value::String(needle)) = (input, &args[0]) else {
        return Err(error("startswith() requires string inputs"));
    };
    Ok(Value::Bool(s.starts_with(&**needle)))
}
pub(crate) fn endswith(input: &Value, args: &[Value]) -> Result<Value, JqError> {
    let (Value::String(s), Value::String(needle)) = (input, &args[0]) else {
        return Err(error("endswith() requires string inputs"));
    };
    Ok(Value::Bool(s.ends_with(&**needle)))
}
pub(crate) fn trim(input: &Value, _: &[Value]) -> Result<Value, JqError> {
    let Value::String(s) = input else {
        return Err(error("trim input must be a string"));
    };
    // jq trims full Unicode whitespace (NBSP, line/paragraph separators,
    // ideographic space, ...), not just ASCII - matches Rust's
    // `char::is_whitespace` (Unicode `White_Space` property).
    Ok(Value::String(s.trim().to_string().into()))
}
pub(crate) fn ltrim(input: &Value, _: &[Value]) -> Result<Value, JqError> {
    let Value::String(s) = input else {
        return Err(error("trim input must be a string"));
    };
    Ok(Value::String(s.trim_start().to_string().into()))
}
pub(crate) fn rtrim(input: &Value, _: &[Value]) -> Result<Value, JqError> {
    let Value::String(s) = input else {
        return Err(error("trim input must be a string"));
    };
    Ok(Value::String(s.trim_end().to_string().into()))
}
pub(crate) fn split(input: &Value, args: &[Value]) -> Result<Value, JqError> {
    let s = string(input)?;
    let separator = string(&args[0])?;
    Ok(Value::Array(Rc::new(if separator.is_empty() {
        if s.is_empty() {
            vec![Value::String(String::new().into())]
        } else {
            s.chars()
                .map(|c| Value::String(c.to_string().into()))
                .collect()
        }
    } else if s.is_empty() {
        // jq returns no fields when a non-empty separator is applied to an
        // empty string (Rust's `str::split` would otherwise yield [""]).
        Vec::new()
    } else {
        s.split(separator)
            .map(|s| Value::String(s.to_string().into()))
            .collect()
    })))
}
/// jq's native `_strindices($i)`: every character offset where `$i` occurs in `.`.
pub(crate) fn strindices(input: &Value, args: &[Value]) -> Result<Value, JqError> {
    let Value::String(s) = input else {
        return Err(error(format!(
            "{} ({}) cannot be searched, as it is not a string",
            input.type_name(),
            input.to_compact_json()
        )));
    };
    let Value::String(needle) = &args[0] else {
        return Err(error(format!(
            "{} ({}) is not a string",
            args[0].type_name(),
            &args[0].to_compact_json()
        )));
    };
    let positions: Vec<Value> = if needle.is_empty() {
        vec![]
    } else {
        s.char_indices()
            .enumerate()
            .filter_map(|(offset, (byte, _))| {
                s[byte..]
                    .starts_with(&**needle)
                    .then_some(Value::int(offset as i64))
            })
            .collect()
    };
    Ok(Value::Array(Rc::new(positions)))
}
pub(crate) fn bsearch(input: &Value, args: &[Value]) -> Result<Value, JqError> {
    let Value::Array(a) = input else {
        return Err(error(format!(
            "{} ({}) cannot be searched from",
            input.type_name(),
            input.to_compact_json()
        )));
    };
    let n = match a.binary_search_by(|v| super::collections::compare(v, &args[0])) {
        Ok(i) => i as i64,
        Err(i) => -(i as i64) - 1,
    };
    Ok(Value::int(n))
}
pub(crate) fn format(input: &Value, name: &str) -> Result<Value, JqError> {
    let text = match input {
        Value::String(s) => s.to_string(),
        v => v.to_compact_json(),
    };
    let result = match name {
        "@text" => text,
        "@json" => input.to_compact_json(),
        "@html" => text
            .replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
            .replace('\'', "&apos;")
            .replace('"', "&quot;"),
        "@htmld" => {
            let mut out = String::new();
            let mut rest = text.as_str();
            while let Some(pos) = rest.find('&') {
                out.push_str(&rest[..pos]);
                let tail = &rest[pos + 1..];
                let entity_end = tail.find(';').filter(|&i| i <= 10);
                match entity_end {
                    Some(semi) => {
                        let ent = &tail[..semi];
                        let decoded = match ent {
                            "amp" => Some('&'),
                            "lt" => Some('<'),
                            "gt" => Some('>'),
                            "apos" => Some('\''),
                            "quot" => Some('"'),
                            _ => {
                                if let Some(hex) =
                                    ent.strip_prefix("#x").or_else(|| ent.strip_prefix("#X"))
                                {
                                    u32::from_str_radix(hex, 16).ok().and_then(char::from_u32)
                                } else if let Some(dec) = ent.strip_prefix('#') {
                                    dec.parse::<u32>().ok().and_then(char::from_u32)
                                } else {
                                    None
                                }
                            }
                        };
                        match decoded {
                            Some(c) => {
                                out.push(c);
                                rest = &tail[semi + 1..];
                            }
                            None => {
                                out.push('&');
                                rest = tail;
                            }
                        }
                    }
                    None => {
                        out.push('&');
                        rest = tail;
                    }
                }
            }
            out.push_str(rest);
            out
        }
        "@uri" => text
            .bytes()
            .map(|b| {
                if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) {
                    (b as char).to_string()
                } else {
                    format!("%{b:02X}")
                }
            })
            .collect(),
        "@urid" => {
            let mut out = Vec::new();
            let mut bytes = text.bytes();
            while let Some(b) = bytes.next() {
                if b == b'%' {
                    let a = bytes
                        .next()
                        .and_then(|b| (b as char).to_digit(16))
                        .ok_or_else(|| error("invalid URI escape"))?;
                    let b = bytes
                        .next()
                        .and_then(|b| (b as char).to_digit(16))
                        .ok_or_else(|| error("invalid URI escape"))?;
                    out.push((a * 16 + b) as u8);
                } else {
                    out.push(b);
                }
            }
            String::from_utf8_lossy(&out).into_owned()
        }
        "@base64" => {
            const TABLE: &[u8] =
                b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
            let mut result = String::new();
            for bytes in text.as_bytes().chunks(3) {
                let n = ((bytes[0] as u32) << 16)
                    | ((bytes.get(1).copied().unwrap_or(0) as u32) << 8)
                    | (bytes.get(2).copied().unwrap_or(0) as u32);
                for i in 0..4 {
                    result.push(if i > bytes.len() {
                        '='
                    } else {
                        TABLE[((n >> (18 - i * 6)) & 63) as usize] as char
                    });
                }
            }
            result
        }
        "@base64d" => {
            let mut out = Vec::new();
            let mut buffer = 0u32;
            let mut bits = 0;
            for b in text.bytes().filter(|b| !b.is_ascii_whitespace()) {
                if b == b'=' {
                    break;
                }
                let n = match b {
                    b'A'..=b'Z' => b - b'A',
                    b'a'..=b'z' => b - b'a' + 26,
                    b'0'..=b'9' => b - b'0' + 52,
                    b'+' => 62,
                    b'/' => 63,
                    _ => return Err(error("invalid base64")),
                };
                buffer = (buffer << 6) | n as u32;
                bits += 6;
                if bits >= 8 {
                    bits -= 8;
                    out.push((buffer >> bits) as u8);
                }
            }
            String::from_utf8_lossy(&out).into_owned()
        }
        "@base32" => {
            const TABLE: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";
            let mut result = String::new();
            let mut buffer: u64 = 0;
            let mut bits = 0;
            for &b in text.as_bytes() {
                buffer = (buffer << 8) | b as u64;
                bits += 8;
                while bits >= 5 {
                    bits -= 5;
                    result.push(TABLE[((buffer >> bits) & 0x1f) as usize] as char);
                }
            }
            if bits > 0 {
                result.push(TABLE[((buffer << (5 - bits)) & 0x1f) as usize] as char);
            }
            while !result.len().is_multiple_of(8) {
                result.push('=');
            }
            result
        }
        "@base32d" => {
            let mut out = Vec::new();
            let mut buffer: u64 = 0;
            let mut bits = 0;
            for b in text.bytes().filter(|b| !b.is_ascii_whitespace()) {
                if b == b'=' {
                    break;
                }
                let n = match b {
                    b'A'..=b'Z' => b - b'A',
                    b'a'..=b'z' => b - b'a',
                    b'2'..=b'7' => b - b'2' + 26,
                    _ => return Err(error("invalid base32")),
                };
                buffer = (buffer << 5) | n as u64;
                bits += 5;
                if bits >= 8 {
                    bits -= 8;
                    out.push((buffer >> bits) as u8);
                }
            }
            String::from_utf8_lossy(&out).into_owned()
        }
        "@hex" => text.bytes().map(|b| format!("{b:02x}")).collect(),
        "@hexd" => {
            let digits: Vec<u8> = text.bytes().filter(|b| !b.is_ascii_whitespace()).collect();
            if !digits.len().is_multiple_of(2) {
                return Err(error("invalid hex string"));
            }
            let mut out = Vec::new();
            for pair in digits.chunks(2) {
                let hi = (pair[0] as char)
                    .to_digit(16)
                    .ok_or_else(|| error("invalid hex digit"))?;
                let lo = (pair[1] as char)
                    .to_digit(16)
                    .ok_or_else(|| error("invalid hex digit"))?;
                out.push((hi * 16 + lo) as u8);
            }
            String::from_utf8_lossy(&out).into_owned()
        }
        "@sh" => {
            let values = match input {
                Value::Array(a) => a.as_ref().clone(),
                v => vec![v.clone()],
            };
            let mut result = Vec::new();
            for v in values {
                result.push(match v {
                    Value::String(s) => format!("'{}'", s.replace('\'', "'\\''")),
                    Value::Array(_) | Value::Object(_) => {
                        return Err(error("cannot escape container for shell"));
                    }
                    v => v.to_compact_json(),
                });
            }
            result.join(" ")
        }
        "@csv" | "@tsv" => {
            let Value::Array(values) = input else {
                return Err(error("CSV/TSV formatting requires an array"));
            };
            let mut fields = Vec::new();
            for v in values.iter() {
                fields.push(match v {
                    Value::Null => String::new(),
                    Value::String(s) if name == "@csv" => format!("\"{}\"", s.replace('"', "\"\"")),
                    Value::String(s) => s
                        .replace('\\', "\\\\")
                        .replace('\t', "\\t")
                        .replace('\r', "\\r")
                        .replace('\n', "\\n"),
                    Value::Array(_) | Value::Object(_) => {
                        return Err(error(format!(
                            "{} ({}) is not valid in a {} row",
                            v.type_name(),
                            crate::jq::vm::value::truncated_repr(v),
                            name.trim_start_matches('@'),
                        )));
                    }
                    v => v.to_compact_json(),
                });
            }
            fields.join(if name == "@csv" { "," } else { "\t" })
        }
        _ => return Err(error("unknown format")),
    };
    Ok(Value::String(result.into()))
}
macro_rules! formats { ($($function:ident => $name:literal),* $(,)?)=>{$(pub(crate) fn $function(input:&Value,_:&[Value])->Result<Value,JqError>{format(input,$name)})*}; }
formats!(text=>"@text",as_json=>"@json",html=>"@html",htmld=>"@htmld",uri=>"@uri",urid=>"@urid",base64=>"@base64",base64d=>"@base64d",base32=>"@base32",base32d=>"@base32d",hex=>"@hex",hexd=>"@hexd",sh=>"@sh",csv=>"@csv",tsv=>"@tsv");

fn hex_digest(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
pub(crate) fn sha1(input: &Value, _: &[Value]) -> Result<Value, JqError> {
    use sha1::{Digest, Sha1};
    let mut hasher = Sha1::new();
    hasher.update(string(input)?.as_bytes());
    Ok(Value::String(hex_digest(&hasher.finalize()).into()))
}
pub(crate) fn sha256(input: &Value, _: &[Value]) -> Result<Value, JqError> {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(string(input)?.as_bytes());
    Ok(Value::String(hex_digest(&hasher.finalize()).into()))
}
pub(crate) fn sha512(input: &Value, _: &[Value]) -> Result<Value, JqError> {
    use sha2::{Digest, Sha512};
    let mut hasher = Sha512::new();
    hasher.update(string(input)?.as_bytes());
    Ok(Value::String(hex_digest(&hasher.finalize()).into()))
}
/// jq's `ascii`: a codepoint number to its single-character string.
pub(crate) fn ascii(input: &Value, _: &[Value]) -> Result<Value, JqError> {
    let n = input
        .as_number()
        .ok_or_else(|| error("ascii requires a number"))?;
    char::from_u32(n as u32)
        .map(|c| Value::String(c.to_string().into()))
        .ok_or_else(|| error("invalid codepoint"))
}
