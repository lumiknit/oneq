//! CSV/TSV (and their header variants) reader/writer.
//!
//! Quoting is the relaxed dialect many CSV dialects converge on: a field is
//! only ever treated as quoted if it starts with `"` (right after the
//! separator, a newline, or the start of input); `""` inside such a field
//! escapes a literal quote; any `"` that shows up anywhere else - inside an
//! unquoted field, or trailing after a quoted field's closing quote - is
//! just literal text. This never rejects input on stray quotes.

use std::{
    collections::{HashSet, VecDeque},
    io::{BufReader, Read, Write},
};

use super::{DataError, ParseOutput, Parser, PathItem, Serializer, StreamItem, Value};
use crate::{
    data::{ArrayIndex, ObjectKey},
    io::{Input, Output},
    render, strs,
};

/// Whether this format's parser/serializer treats the first row as a header.
const fn has_header(format: crate::data::DataFormat) -> bool {
    matches!(
        format,
        crate::data::DataFormat::Csvh | crate::data::DataFormat::Tsvh
    )
}

const fn separator(format: crate::data::DataFormat) -> char {
    match format {
        crate::data::DataFormat::Tsv | crate::data::DataFormat::Tsvh => '\t',
        _ => ',',
    }
}

pub struct SvParser<'a> {
    input: BufReader<Input<'a>>,
    format: crate::data::DataFormat,
    field: String,
    line: ArrayIndex,
    col: ArrayIndex,
    field_started: bool,
    row_has_field: bool,
    quote_mode: bool,
    quote_pending: bool,
    skip_lf: bool,
    pending_char: Option<char>,
    header: bool,
    with_header: bool,
    header_keys: Vec<ObjectKey>,
    done: bool,
    queue: VecDeque<StreamItem>,
}
impl<'a> SvParser<'a> {
    #[must_use]
    pub fn new(input: Input<'a>, format: crate::data::DataFormat) -> Self {
        Self {
            input: BufReader::new(input),
            format,
            field: String::new(),
            line: 0,
            col: 0,
            field_started: false,
            row_has_field: false,
            quote_mode: false,
            quote_pending: false,
            skip_lf: false,
            pending_char: None,
            header: has_header(format),
            with_header: has_header(format),
            header_keys: Vec::new(),
            done: false,
            queue: VecDeque::new(),
        }
    }

    fn read_char(&mut self) -> Result<Option<char>, DataError> {
        loop {
            let mut bytes = [0u8; 4];
            let mut n = 1;
            let mut first = [0u8; 1];
            let read = self.input.read(&mut first).map_err(DataError::IOError)?;
            if read == 0 {
                return Ok(None);
            }
            bytes[0] = first[0];
            let width = if first[0] < 0x80 {
                1
            } else if first[0] & 0xe0 == 0xc0 {
                2
            } else if first[0] & 0xf0 == 0xe0 {
                3
            } else if first[0] & 0xf8 == 0xf0 {
                4
            } else {
                1
            };
            while n < width {
                let read = self
                    .input
                    .read(&mut bytes[n..=n])
                    .map_err(DataError::IOError)?;
                if read == 0 {
                    return Ok(None);
                }
                n += 1;
            }
            if let Some(c) = std::str::from_utf8(&bytes[..n])
                .ok()
                .and_then(|s| s.chars().next())
            {
                return Ok(Some(c));
            }
        }
    }

    fn key_for_col(&self) -> ObjectKey {
        self.header_keys
            .get(self.col as usize)
            .copied()
            .unwrap_or_else(|| strs::intern(&format!("_{}", self.col)))
    }

    fn path_column(&self) -> PathItem {
        if self.with_header {
            PathItem::new_key(self.key_for_col())
        } else {
            PathItem::new_idx(self.col)
        }
    }

    fn reset_field(&mut self) {
        self.field.clear();
        self.field_started = false;
        self.quote_mode = false;
        self.quote_pending = false;
    }

    fn finish_field(&mut self) -> Option<StreamItem> {
        let value = std::mem::take(&mut self.field);
        self.field_started = false;
        self.quote_mode = false;
        self.quote_pending = false;
        self.row_has_field = true;

        if self.header {
            self.header_keys.push(strs::intern(&value));
            None
        } else {
            if self.col == 0 {
                self.queue
                    .push_back(StreamItem::Push(PathItem::new_idx(self.line)));
            }
            let column = self.path_column();
            self.queue.push_back(StreamItem::Push(column));
            self.queue
                .push_back(StreamItem::Value(Value::String(value.into())));
            self.queue.pop_front()
        }
    }

    fn finish_row(&mut self) {
        if self.header {
            self.header = false;
        } else {
            self.queue.push_back(StreamItem::Close);
            self.line += 1;
        }
        self.col = 0;
        self.row_has_field = false;
        self.reset_field();
    }

    fn finish_document(&mut self) {
        self.queue.push_back(if self.line == 0 {
            // Preserve the existing null result for an empty/header-only
            // document; there is no child that a Close could refer to.
            StreamItem::Value(Value::Null)
        } else {
            StreamItem::Close
        });
        self.done = true;
    }
}
impl Iterator for SvParser<'_> {
    type Item = ParseOutput;
    fn next(&mut self) -> Option<Self::Item> {
        if let Some(event) = self.queue.pop_front() {
            return Some(Ok(event));
        }
        if self.done {
            return None;
        }

        let sep = separator(self.format);
        loop {
            if self.skip_lf {
                self.skip_lf = false;
                match self.read_char() {
                    Ok(Some('\n')) => continue,
                    Ok(Some(c)) => self.pending_char = Some(c),
                    Ok(None) => return self.next(),
                    Err(e) => return Some(Err(e)),
                }
            }

            let c = match self.pending_char.take() {
                Some(c) => c,
                None => match self.read_char() {
                    Ok(Some(c)) => c,
                    Ok(None) => {
                        if self.row_has_field || self.field_started {
                            if let Some(event) = self.finish_field() {
                                self.queue.push_back(StreamItem::Close);
                                self.line += 1;
                                self.col = 0;
                                self.row_has_field = false;
                                self.reset_field();
                                self.finish_document();
                                return Some(Ok(event));
                            }
                            self.finish_row();
                        }
                        self.finish_document();
                        return self.queue.pop_front().map(Ok);
                    }
                    Err(e) => return Some(Err(e)),
                },
            };

            if self.quote_pending {
                self.quote_pending = false;
                if c == '"' {
                    self.field.push('"');
                    continue;
                }
                self.quote_mode = false;
            }

            if self.quote_mode {
                if c == '"' {
                    self.quote_pending = true;
                } else {
                    self.field.push(c);
                }
                continue;
            }

            if !self.field_started && self.field.is_empty() && c == '"' {
                self.field_started = true;
                self.quote_mode = true;
                continue;
            }

            if c == sep {
                let event = self.finish_field();
                self.col += 1;
                if let Some(event) = event {
                    return Some(Ok(event));
                }
                continue;
            }

            if c == '\n' || c == '\r' {
                let blank = !self.row_has_field && self.field.trim().is_empty();
                if c == '\r' {
                    self.skip_lf = true;
                }
                if blank {
                    self.reset_field();
                    self.finish_document();
                    return self.queue.pop_front().map(Ok);
                }

                let event = self.finish_field();
                self.finish_row();
                if let Some(event) = event {
                    return Some(Ok(event));
                }
                continue;
            }

            self.field_started = true;
            self.field.push(c);
        }
    }
}
impl Parser for SvParser<'_> {}

fn cell(value: &Value) -> String {
    match value {
        Value::String(s) => s.to_string(),
        other => other.to_string(),
    }
}

fn quote_cell(out: &mut String, cell: &str, sep: char) {
    if cell.is_empty()
        || cell.contains(sep)
        || cell.contains('"')
        || cell.contains('\n')
        || cell.contains('\r')
    {
        out.push('"');
        for c in cell.chars() {
            if c == '"' {
                out.push('"');
            }
            out.push(c);
        }
        out.push('"');
    } else {
        out.push_str(cell);
    }
}

fn write_row(
    out: &mut String,
    row: &[String],
    sep: char,
    opts: &render::Options,
    idx: render::ThemeIdx,
) {
    for (i, field) in row.iter().enumerate() {
        if i > 0 {
            out.push(sep);
        }
        let mut buf = String::new();
        quote_cell(&mut buf, field, sep);
        super::json::push_styled(out, opts, idx, &buf);
    }
    out.push('\n');
}

/// Builds the (optional header, rows) table for a value, per the format's
/// documented shape rules: singleton for scalars, `key`/`value` rows for an
/// object, and one of three array shapes depending on what's inside it.
fn build_table(
    value: &Value,
    sorted: bool,
    header: bool,
) -> (Option<Vec<String>>, Vec<Vec<String>>) {
    match value {
        Value::Object(_) => {
            let head = header.then(|| vec!["key".to_owned(), "value".to_owned()]);
            let rows = value
                .object_iter(sorted)
                .unwrap()
                .map(|(k, v)| vec![strs::resolve(*k).unwrap().to_owned(), cell(v)])
                .collect();
            (head, rows)
        }
        Value::Array(items)
            if !items.is_empty() && items.iter().all(|v| matches!(v, Value::Object(_))) =>
        {
            let mut keys: Vec<ObjectKey> = Vec::new();
            let mut seen = HashSet::new();
            for item in items.iter() {
                if let Value::Object(m) = item {
                    for k in m.keys() {
                        if seen.insert(*k) {
                            keys.push(*k);
                        }
                    }
                }
            }
            if sorted {
                let mut order: Vec<(ObjectKey, usize)> = keys.iter().map(|k| (*k, 0)).collect();
                strs::sort_symbols_by_str(&mut order);
                keys = order.into_iter().map(|(k, _)| k).collect();
            }
            let head = header.then(|| {
                keys.iter()
                    .map(|k| strs::resolve(*k).unwrap().to_owned())
                    .collect()
            });
            let rows = items
                .iter()
                .map(|item| {
                    let Value::Object(m) = item else {
                        unreachable!()
                    };
                    keys.iter()
                        .map(|k| m.get(k).map(cell).unwrap_or_default())
                        .collect()
                })
                .collect();
            (head, rows)
        }
        Value::Array(items)
            if !items.is_empty() && items.iter().all(|v| matches!(v, Value::Array(_))) =>
        {
            let max_len = items
                .iter()
                .map(|v| match v {
                    Value::Array(a) => a.len(),
                    _ => 0,
                })
                .max()
                .unwrap_or(0);
            let head = header.then(|| (0..max_len).map(|i| i.to_string()).collect());
            let rows = items
                .iter()
                .map(|item| {
                    let Value::Array(a) = item else {
                        unreachable!()
                    };
                    (0..max_len)
                        .map(|i| a.get(i).map(cell).unwrap_or_default())
                        .collect()
                })
                .collect();
            (head, rows)
        }
        Value::Array(items) => {
            // Mixed shapes: fall back to one JSON document per row.
            (
                None,
                items.iter().map(|item| vec![item.to_string()]).collect(),
            )
        }
        other => (None, vec![vec![cell(other)]]),
    }
}

pub struct SvSerializer {
    output: Output,
    options: render::Options,
    sep: char,
    header: bool,
}
impl SvSerializer {
    #[must_use]
    pub const fn new(
        output: Output,
        options: render::Options,
        format: crate::data::DataFormat,
    ) -> Self {
        Self {
            output,
            options,
            sep: separator(format),
            header: has_header(format),
        }
    }
    pub(crate) fn finish(self) -> std::io::Result<()> {
        self.output.finish()
    }
}
impl Serializer for SvSerializer {
    fn put(&mut self, value: Value) -> Result<(), DataError> {
        if self.options.out.quiet {
            return Ok(());
        }
        let (head, rows) = build_table(&value, self.options.out.sort_keys, self.header);
        let mut text = String::new();
        if let Some(s) = self.options.out.doc_begin {
            text.push_str(s);
        }
        if let Some(head) = &head {
            write_row(
                &mut text,
                head,
                self.sep,
                &self.options,
                render::ThemeIdx::ObjectKey,
            );
        }
        for row in &rows {
            write_row(
                &mut text,
                row,
                self.sep,
                &self.options,
                render::ThemeIdx::String,
            );
        }
        match self.options.out.doc_end {
            Some(s) => text.push_str(s),
            None => text.push('\n'),
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
