//! Shell env / dotenv reader & writer.
//!
//! Parsing accepts both plain shell `export`-listing output and dotenv
//! files: a leading `export\s+` is dropped, `#` starts a line comment
//! (unless escaped as `\#` or inside quotes), `\` escapes any single
//! character, and values may be bare, single-quoted or double-quoted.
//! `Env` and `ExportEnv` parse identically - the two formats only differ
//! on write, where `ExportEnv` prefixes every line with `export `.

use std::io::{BufRead, BufReader, Write};

use super::{DataError, ParseOutput, Parser, PathItem, Serializer, StreamItem, Value};
use crate::{
    data::DataFormat,
    io::{Input, Output},
    render, strs,
};

fn strip_export(line: &str) -> &str {
    let trimmed = line.trim_start();
    let Some(rest) = trimmed.strip_prefix("export") else {
        return trimmed;
    };
    let rest_trimmed = rest.trim_start();
    if rest_trimmed.len() == rest.len() {
        // "export" wasn't followed by whitespace, so it's not the keyword.
        trimmed
    } else {
        rest_trimmed
    }
}

/// Parses one `KEY=VALUE` line, honoring quotes/escapes/comments. Returns
/// `None` for blank lines, comment-only lines, or lines with no `=`.
fn parse_line(line: &str) -> Option<(String, String)> {
    let line = strip_export(line);

    let mut chars = line.chars().peekable();
    let mut key = String::new();
    while let Some(&c) = chars.peek() {
        if c == '=' {
            break;
        }
        if c == '#' {
            return None;
        }
        if c.is_whitespace() {
            chars.next();
            continue;
        }
        key.push(c);
        chars.next();
    }
    if chars.next() != Some('=') {
        return None;
    }
    if key.is_empty() {
        return None;
    }

    // Skip whitespace before the value.
    while let Some(&c) = chars.peek() {
        if c.is_whitespace() {
            chars.next();
        } else {
            break;
        }
    }

    let mut value = String::new();
    match chars.peek() {
        Some(&q) if q == '"' || q == '\'' => {
            chars.next();
            loop {
                match chars.next() {
                    None => break,
                    Some(c) if c == q => break,
                    Some('\\') => {
                        if let Some(next) = chars.next() {
                            value.push(next);
                        }
                    }
                    Some(c) => value.push(c),
                }
            }
        }
        _ => {
            while let Some(&c) = chars.peek() {
                if c == '#' {
                    break;
                }
                if c == '\\' {
                    chars.next();
                    if let Some(next) = chars.next() {
                        value.push(next);
                    }
                    continue;
                }
                value.push(c);
                chars.next();
            }
            while value.ends_with(|c: char| c.is_whitespace()) {
                value.pop();
            }
        }
    }

    Some((key, value))
}

pub struct EnvParser<'a> {
    input: BufReader<Input<'a>>,
    done: bool,
    has_entries: bool,
    pending_value: Option<Value>,
}
impl<'a> EnvParser<'a> {
    #[must_use]
    pub fn new(input: Input<'a>) -> Self {
        Self {
            input: BufReader::new(input),
            done: false,
            has_entries: false,
            pending_value: None,
        }
    }
}
impl Iterator for EnvParser<'_> {
    type Item = ParseOutput;
    fn next(&mut self) -> Option<Self::Item> {
        if let Some(value) = self.pending_value.take() {
            return Some(Ok(StreamItem::Value(value)));
        }
        if self.done {
            return None;
        }
        loop {
            let mut line = String::new();
            match self.input.read_line(&mut line) {
                Ok(0) => {
                    self.done = true;
                    return Some(Ok(if self.has_entries {
                        StreamItem::Close
                    } else {
                        StreamItem::Value(Value::empty_object())
                    }));
                }
                Ok(_) => {
                    let Some((key, value)) = parse_line(&line) else {
                        continue;
                    };
                    let key = PathItem::new_key(strs::intern(&key));
                    self.has_entries = true;
                    self.pending_value = Some(Value::String(value.into()));
                    return Some(Ok(StreamItem::Push(key)));
                }
                Err(e) => {
                    self.done = true;
                    return Some(Err(DataError::IOError(e)));
                }
            }
        }
    }
}
impl Parser for EnvParser<'_> {}

/// Shell single-quotes `s`, escaping any embedded `'` as `'\''`.
fn shell_quote(out: &mut String, s: &str) {
    out.push('\'');
    for c in s.chars() {
        if c == '\'' {
            out.push_str("'\\''");
        } else {
            out.push(c);
        }
    }
    out.push('\'');
}

fn write_value(out: &mut String, value: &Value) {
    match value {
        Value::Null => out.push_str("null"),
        Value::Bool(true) => out.push_str("true"),
        Value::Bool(false) => out.push_str("false"),
        Value::Decimal(n) => out.push_str(&n.to_string()),
        Value::Float(n) => out.push_str(&n.to_string()),
        Value::String(s) => shell_quote(out, s),
        // Nested structures are not flattened - dump them as JSON, quoted
        // for shell-safety.
        Value::Array(_) | Value::Object(_) => shell_quote(out, &value.to_string()),
    }
}

pub struct EnvSerializer {
    output: Output,
    options: render::Options,
    export: bool,
}
impl EnvSerializer {
    #[must_use]
    pub const fn new(output: Output, options: render::Options, format: DataFormat) -> Self {
        Self {
            output,
            options,
            export: matches!(format, DataFormat::ExportEnv),
        }
    }

    fn write_entry(&self, out: &mut String, key: &str, value: &Value) {
        if self.export {
            out.push_str("export ");
        }
        out.push_str(key);
        out.push('=');
        write_value(out, value);
        out.push('\n');
    }
    pub(crate) fn finish(self) -> std::io::Result<()> {
        self.output.finish()
    }
}
impl Serializer for EnvSerializer {
    fn put(&mut self, value: Value) -> Result<(), DataError> {
        if self.options.out.quiet {
            return Ok(());
        }
        let mut text = String::new();
        if let Some(s) = self.options.out.doc_begin {
            text.push_str(s);
        }
        match &value {
            Value::Object(_) => {
                for (key, v) in value.object_iter(self.options.out.sort_keys).unwrap() {
                    self.write_entry(&mut text, strs::resolve(*key).unwrap(), v);
                }
            }
            Value::Array(items) => {
                for (i, v) in items.iter().enumerate() {
                    self.write_entry(&mut text, &format!("_{i}"), v);
                }
            }
            other => self.write_entry(&mut text, "_", other),
        }
        if let Some(s) = self.options.out.doc_end {
            text.push_str(s);
        }
        self.output
            .write_all(text.as_bytes())
            .map_err(DataError::IOError)?;
        if self.options.out.doc_end_flush {
            self.output.flush().map_err(DataError::IOError)?;
        }
        Ok(())
    }
}
