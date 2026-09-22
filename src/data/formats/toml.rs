//! Hand-rolled streaming TOML 1.0 parser & serializer (no `toml` crate).
//!
//! The parser emits `StreamItem`s as soon as each `key = value` line is
//! parsed, and treats table headers (`[a.b]`, `[[a.b]]`) as a path jump:
//! switching to an unrelated table finalizes the previous one immediately,
//! emitting an `EmptyObject` for it if it never received a direct key (e.g.
//! a header immediately followed by another header). Because a TOML table
//! only really "closes" once the file moves on to a different one (or hits
//! EOF), the document as a whole only closes at EOF - mirroring the same
//! rule YAML/`document::events` already follow for their own containers.
use super::{DataError, ParseOutput, Parser, PathItem, Serializer, StreamItem, Value};
use crate::{
    data::{ArrayIndex, core::decimal::Decimal, core::escape, traits::CharReader},
    io::{Input, Output},
    render, strs,
};
use indexmap::IndexMap;
use std::{cell::RefCell, collections::VecDeque, io::Write, rc::Rc};

/// Tracks, for a given dotted key/header name, whether it has been opened as
/// an array of tables (and how many elements it has so far) - needed to
/// resolve later dotted references (`[[a]]` then `a.x = 1` then `[[a]]`
/// again) to the right path.
#[derive(Default)]
struct TomlNode {
    children: std::collections::HashMap<String, NodeRef>,
    /// `Some(n)` once this key has been opened via `[[...]]`: `n` elements
    /// have been started so far, and `children` describes the *current*
    /// (last) element's own namespace.
    array_count: Option<ArrayIndex>,
}
type NodeRef = Rc<RefCell<TomlNode>>;

pub struct TomlParser<'a> {
    reader: CharReader<'a>,
    filename: String,
    line: u32,
    col: u32,
    queue: VecDeque<StreamItem>,
    done: bool,
    root: NodeRef,
    /// The table tree node the *current* header/dotted-key context resolves
    /// dotted keys against - fixed by the most recent `[header]`.
    current_node: NodeRef,
    /// The exact path of the most recent `[header]`/`[[header]]` - the base
    /// path onto which subsequent bare/dotted keys are appended.
    table_path: Vec<PathItem>,
    /// Stack of currently "open" container frames, deepest last -
    /// `open_path` is their concatenated keys/indices and `open_last_child`
    /// records, per frame, the last key/index emitted directly under it.
    /// Every leaf/table event reconciles this stack against its own path
    /// first (`reopen_to`), closing whatever no longer applies (emitting
    /// that frame's own close event - `EmptyObject` if it never got a
    /// child) and opening whatever is newly needed - this is what turns a
    /// header/dotted-key "jump" into the right close/open event sequence
    /// for every nesting depth, not just the table directly named by a
    /// header.
    open_path: Vec<PathItem>,
    open_last_child: Vec<Option<PathItem>>,
    /// Same bookkeeping as `open_last_child`, but for the implicit root
    /// table (which isn't itself represented as a frame).
    root_last_child: Option<PathItem>,
}

impl<'a> TomlParser<'a> {
    #[must_use]
    pub fn new(input: Input<'a>) -> Self {
        let filename = input.filename().to_owned();
        let root: NodeRef = Rc::new(RefCell::new(TomlNode::default()));
        Self {
            reader: CharReader::new(input),
            filename,
            line: 1,
            col: 1,
            queue: VecDeque::new(),
            done: false,
            current_node: root.clone(),
            root,
            table_path: Vec::new(),
            open_path: Vec::new(),
            open_last_child: Vec::new(),
            root_last_child: None,
        }
    }

    fn bump(&mut self) -> Option<char> {
        let c = self.reader.bump();
        if let Some(ch) = c {
            if ch == '\n' {
                self.line += 1;
                self.col = 1;
            } else {
                self.col += 1;
            }
        }
        c
    }

    fn peek(&mut self) -> Option<char> {
        self.reader.peek()
    }

    fn peek2(&mut self) -> Option<char> {
        self.reader.peek2()
    }

    fn peek_at(&mut self, offset: usize) -> Option<char> {
        self.reader.peek_at(offset)
    }

    fn starts_with(&mut self, s: &str) -> bool {
        self.reader.starts_with(s)
    }

    fn err(&mut self, message: impl Into<String>) -> DataError {
        if let Some(e) = self.reader.take_read_error() {
            return DataError::IOError(std::io::Error::other(e));
        }
        DataError::ParseError {
            path: self.filename.clone(),
            line: self.line,
            col: self.col,
            message: message.into(),
        }
    }

    fn skip_inline_ws(&mut self) {
        while matches!(self.peek(), Some(' ' | '\t')) {
            self.bump();
        }
    }

    fn skip_ws_nl_comments(&mut self) {
        loop {
            match self.peek() {
                Some(' ' | '\t' | '\r' | '\n') => {
                    self.bump();
                }
                Some('#') => {
                    while !matches!(self.peek(), None | Some('\n')) {
                        self.bump();
                    }
                }
                _ => break,
            }
        }
    }

    fn expect_char(&mut self, expected: char) -> Result<(), DataError> {
        if self.peek() == Some(expected) {
            self.bump();
            Ok(())
        } else {
            Err(self.err(format!("expected '{expected}'")))
        }
    }

    fn expect_eol_or_comment(&mut self) -> Result<(), DataError> {
        self.skip_inline_ws();
        if self.peek() == Some('#') {
            while !matches!(self.peek(), None | Some('\n')) {
                self.bump();
            }
        }
        match self.peek() {
            None => Ok(()),
            Some('\n') => {
                self.bump();
                Ok(())
            }
            Some('\r') if self.peek2() == Some('\n') => {
                self.bump();
                self.bump();
                Ok(())
            }
            Some(c) => Err(self.err(format!("expected end of line, found '{c}'"))),
        }
    }

    // -- keys --

    fn parse_key_name(&mut self) -> Result<String, DataError> {
        match self.peek() {
            Some('"') => {
                if self.starts_with("\"\"\"") {
                    return Err(self.err("multi-line strings are not allowed as keys"));
                }
                self.parse_basic_string(false)
            }
            Some('\'') => {
                if self.starts_with("'''") {
                    return Err(self.err("multi-line strings are not allowed as keys"));
                }
                self.parse_literal_string(false)
            }
            Some(c) if is_bare_key_char(c) => {
                let mut s = String::new();
                while let Some(c) = self.peek() {
                    if is_bare_key_char(c) {
                        s.push(c);
                        self.bump();
                    } else {
                        break;
                    }
                }
                Ok(s)
            }
            Some(c) => Err(self.err(format!("unexpected character '{c}' in key"))),
            None => Err(self.err("unexpected end of input in key")),
        }
    }

    fn parse_dotted_key_names(&mut self) -> Result<Vec<String>, DataError> {
        let mut names = vec![self.parse_key_name()?];
        loop {
            self.skip_inline_ws();
            if self.peek() == Some('.') {
                self.bump();
                self.skip_inline_ws();
                names.push(self.parse_key_name()?);
            } else {
                break;
            }
        }
        Ok(names)
    }

    // -- strings --

    fn parse_basic_string(&mut self, multiline: bool) -> Result<String, DataError> {
        if multiline {
            self.bump();
            self.bump();
            self.bump();
            if self.peek() == Some('\n') {
                self.bump();
            } else if self.peek() == Some('\r') && self.peek2() == Some('\n') {
                self.bump();
                self.bump();
            }
        } else {
            self.bump();
        }
        let mut s = String::new();
        loop {
            match self.peek() {
                None => return Err(self.err("unterminated string literal")),
                Some('"') => {
                    if multiline {
                        if self.starts_with("\"\"\"") {
                            self.bump();
                            self.bump();
                            self.bump();
                            break;
                        }
                        s.push('"');
                        self.bump();
                    } else {
                        self.bump();
                        break;
                    }
                }
                Some('\n') if !multiline => return Err(self.err("unterminated string literal")),
                Some('\\') => {
                    self.bump();
                    match self.peek() {
                        Some('n') => {
                            s.push('\n');
                            self.bump();
                        }
                        Some('t') => {
                            s.push('\t');
                            self.bump();
                        }
                        Some('r') => {
                            s.push('\r');
                            self.bump();
                        }
                        Some('b') => {
                            s.push('\u{08}');
                            self.bump();
                        }
                        Some('f') => {
                            s.push('\u{0c}');
                            self.bump();
                        }
                        Some('"') => {
                            s.push('"');
                            self.bump();
                        }
                        Some('\\') => {
                            s.push('\\');
                            self.bump();
                        }
                        Some('u') => {
                            self.bump();
                            let cp = self.parse_hex_n(4)?;
                            s.push(
                                char::from_u32(cp)
                                    .ok_or_else(|| self.err("invalid unicode escape"))?,
                            );
                        }
                        Some('U') => {
                            self.bump();
                            let cp = self.parse_hex_n(8)?;
                            s.push(
                                char::from_u32(cp)
                                    .ok_or_else(|| self.err("invalid unicode escape"))?,
                            );
                        }
                        Some(c) if multiline && matches!(c, ' ' | '\t' | '\n' | '\r') => {
                            while matches!(self.peek(), Some(' ' | '\t' | '\n' | '\r')) {
                                self.bump();
                            }
                        }
                        Some(c) => return Err(self.err(format!("invalid escape '\\{c}'"))),
                        None => return Err(self.err("unterminated escape sequence")),
                    }
                }
                Some(c) => {
                    s.push(c);
                    self.bump();
                }
            }
        }
        Ok(s)
    }

    fn parse_hex_n(&mut self, n: usize) -> Result<u32, DataError> {
        let mut v: u32 = 0;
        for _ in 0..n {
            let c = self
                .bump()
                .ok_or_else(|| self.err("unterminated unicode escape"))?;
            let d = c
                .to_digit(16)
                .ok_or_else(|| self.err("invalid hex digit in unicode escape"))?;
            v = v * 16 + d;
        }
        Ok(v)
    }

    fn parse_literal_string(&mut self, multiline: bool) -> Result<String, DataError> {
        if multiline {
            self.bump();
            self.bump();
            self.bump();
            if self.peek() == Some('\n') {
                self.bump();
            } else if self.peek() == Some('\r') && self.peek2() == Some('\n') {
                self.bump();
                self.bump();
            }
        } else {
            self.bump();
        }
        let mut s = String::new();
        loop {
            match self.peek() {
                None => return Err(self.err("unterminated string literal")),
                Some('\'') => {
                    if multiline {
                        if self.starts_with("'''") {
                            self.bump();
                            self.bump();
                            self.bump();
                            break;
                        }
                        s.push('\'');
                        self.bump();
                    } else {
                        self.bump();
                        break;
                    }
                }
                Some('\n') if !multiline => return Err(self.err("unterminated string literal")),
                Some(c) => {
                    s.push(c);
                    self.bump();
                }
            }
        }
        Ok(s)
    }

    // -- values --

    fn parse_value(&mut self) -> Result<Value, DataError> {
        match self.peek() {
            Some('"') => {
                let multiline = self.starts_with("\"\"\"");
                Ok(Value::String(self.parse_basic_string(multiline)?.into()))
            }
            Some('\'') => {
                let multiline = self.starts_with("'''");
                Ok(Value::String(self.parse_literal_string(multiline)?.into()))
            }
            Some('[') => self.parse_array(),
            Some('{') => self.parse_inline_table(),
            Some(c) if c == '+' || c == '-' || c == '.' || c.is_ascii_alphanumeric() => {
                let tok = self.read_bare_token();
                if tok.is_empty() {
                    return Err(self.err("expected value"));
                }
                self.classify_token(&tok)
            }
            Some(c) => Err(self.err(format!("unexpected character '{c}' in value"))),
            None => Err(self.err("unexpected end of input in value")),
        }
    }

    fn date_space_time_follows(&mut self) -> bool {
        // offset 0 is the space itself; expect `DD:DD:DD` right after it.
        const PATTERN: [bool; 8] = [true, true, false, true, true, false, true, true];
        for (i, is_digit) in PATTERN.iter().enumerate() {
            let Some(c) = self.peek_at(1 + i) else {
                return false;
            };
            if *is_digit {
                if !c.is_ascii_digit() {
                    return false;
                }
            } else if c != ':' {
                return false;
            }
        }
        true
    }

    fn read_bare_token(&mut self) -> String {
        let mut s = String::new();
        loop {
            match self.peek() {
                Some(c) if is_bare_token_char(c) => {
                    s.push(c);
                    self.bump();
                }
                Some(' ') if is_full_date(&s) && self.date_space_time_follows() => {
                    s.push(' ');
                    self.bump();
                }
                _ => break,
            }
        }
        s
    }

    fn classify_token(&mut self, token: &str) -> Result<Value, DataError> {
        match token {
            "true" => return Ok(Value::Bool(true)),
            "false" => return Ok(Value::Bool(false)),
            "inf" | "+inf" => return Ok(Value::Float(f64::INFINITY)),
            "-inf" => return Ok(Value::Float(f64::NEG_INFINITY)),
            "nan" | "+nan" | "-nan" => return Ok(Value::Float(f64::NAN)),
            _ => {}
        }
        let has_interior_dash = token.char_indices().skip(1).any(|(_, c)| c == '-');
        if token.contains(':') || has_interior_dash {
            // Local time / local date / local datetime / offset datetime -
            // kept verbatim as a string, same as the previous `toml` crate
            // based implementation did.
            return Ok(Value::String(token.to_string().into()));
        }
        let (neg, body) = match token.as_bytes().first() {
            Some(b'-') => (true, &token[1..]),
            Some(b'+') => (false, &token[1..]),
            _ => (false, token),
        };
        for (prefix, radix) in [("0x", 16), ("0o", 8), ("0b", 2)] {
            if let Some(rest) = body.strip_prefix(prefix) {
                let digits: String = rest.chars().filter(|&c| c != '_').collect();
                // TOML integers are spec'd as 64-bit signed, so unlike the
                // plain-decimal case below (arbitrary precision via
                // `Decimal`), `i64` is both sufficient and more correct.
                let n = i64::from_str_radix(&digits, radix)
                    .map_err(|_| self.err(format!("invalid integer literal '{token}'")))?;
                return Ok(Value::int(if neg { -n } else { n }));
            }
        }
        let cleaned: String = token.chars().filter(|&c| c != '_').collect();
        if cleaned.contains('.') || cleaned.contains('e') || cleaned.contains('E') {
            let n = Decimal::parse(&cleaned)
                .ok_or_else(|| self.err(format!("invalid float literal '{token}'")))?;
            return Ok(Value::decimal(n));
        }
        // `cleaned` is derived from the *whole* token, sign included (unlike
        // `body` above, which had the sign stripped into `neg` already), so
        // `Decimal::parse` already captures it - negating again here would
        // double-flip it.
        let n = Decimal::parse(&cleaned)
            .ok_or_else(|| self.err(format!("invalid integer literal '{token}'")))?;
        Ok(Value::decimal(n))
    }

    fn parse_array(&mut self) -> Result<Value, DataError> {
        self.bump(); // '['
        let mut items = Vec::new();
        loop {
            self.skip_ws_nl_comments();
            if self.peek() == Some(']') {
                self.bump();
                break;
            }
            items.push(self.parse_value()?);
            self.skip_ws_nl_comments();
            match self.peek() {
                Some(',') => {
                    self.bump();
                }
                Some(']') => {
                    self.bump();
                    break;
                }
                Some(c) => {
                    return Err(self.err(format!("expected ',' or ']' in array, found '{c}'")));
                }
                None => return Err(self.err("unterminated array")),
            }
        }
        Ok(Value::Array(Rc::new(items)))
    }

    fn parse_inline_table(&mut self) -> Result<Value, DataError> {
        self.bump(); // '{'
        self.skip_inline_ws();
        let mut map: IndexMap<strs::Symbol, Value> = IndexMap::new();
        if self.peek() == Some('}') {
            self.bump();
            return Ok(Value::Object(Rc::new(map)));
        }
        loop {
            self.skip_inline_ws();
            let names = self.parse_dotted_key_names()?;
            self.skip_inline_ws();
            self.expect_char('=')?;
            self.skip_inline_ws();
            let value = self.parse_value()?;
            insert_dotted(&mut map, &names, value);
            self.skip_inline_ws();
            match self.peek() {
                Some(',') => {
                    self.bump();
                }
                Some('}') => {
                    self.bump();
                    break;
                }
                Some(c) => {
                    return Err(
                        self.err(format!("expected ',' or '}}' in inline table, found '{c}'"))
                    );
                }
                None => return Err(self.err("unterminated inline table")),
            }
        }
        Ok(Value::Object(Rc::new(map)))
    }

    // -- table tree resolution --

    fn resolve(
        &self,
        start: NodeRef,
        names: &[String],
        is_array: bool,
    ) -> (Vec<PathItem>, NodeRef) {
        let mut node = start;
        let mut path = Vec::new();
        let n = names.len();
        for (i, name) in names.iter().enumerate() {
            let last = i + 1 == n;
            let child = {
                let mut nb = node.borrow_mut();
                nb.children
                    .entry(name.clone())
                    .or_insert_with(|| Rc::new(RefCell::new(TomlNode::default())))
                    .clone()
            };
            path.push(PathItem::new_key_str(name));
            if last && is_array {
                let idx = {
                    let mut cb = child.borrow_mut();
                    let idx = cb.array_count.unwrap_or(0);
                    cb.array_count = Some(idx + 1);
                    cb.children.clear();
                    idx
                };
                path.push(PathItem::new_idx(idx));
            } else {
                let idx_opt = child.borrow().array_count.map(|c| c - 1);
                if let Some(idx) = idx_opt {
                    path.push(PathItem::new_idx(idx));
                }
            }
            node = child;
        }
        (path, node)
    }

    // -- streaming close/open bookkeeping --

    /// Records `child` as the last (direct) child of whatever frame is
    /// currently deepest - or of the implicit root, if none is open.
    fn record_child(&mut self, child: PathItem) {
        if let Some(last) = self.open_last_child.last_mut() {
            *last = Some(child);
        } else {
            self.root_last_child = Some(child);
        }
    }

    /// Reconciles the open-frame stack against `target`: closes every
    /// currently open frame that isn't a prefix of `target` (deepest
    /// first, each emitting its own close event - `EmptyObject` if it
    /// never got a child), then opens frames for whatever's left of
    /// `target` beyond the common prefix. Afterwards `open_path == target`.
    fn reopen_to(&mut self, target: &[PathItem]) {
        let mut common = 0;
        while common < self.open_path.len()
            && common < target.len()
            && self.open_path[common].unpack() == target[common].unpack()
        {
            common += 1;
        }
        while self.open_path.len() > common {
            let last_child = self.open_last_child.pop().unwrap();
            self.open_path.pop();
            self.queue.push_back(if last_child.is_some() {
                StreamItem::Close
            } else {
                StreamItem::Value(Value::empty_object())
            });
        }
        while self.open_path.len() < target.len() {
            let next = target[self.open_path.len()];
            self.record_child(next);
            self.queue.push_back(StreamItem::Push(next));
            self.open_path.push(next);
            self.open_last_child.push(None);
        }
    }

    fn push_value_event(&mut self, path: Vec<PathItem>, value: Value) {
        self.reopen_to(&path[..path.len() - 1]);
        self.record_child(path[path.len() - 1]);
        self.queue.push_back(StreamItem::Push(path[path.len() - 1]));
        super::document::events(value, &mut self.queue);
    }

    fn finish(&mut self) {
        self.reopen_to(&[]);
        self.queue.push_back(if self.root_last_child.is_some() {
            StreamItem::Close
        } else {
            StreamItem::Value(Value::empty_object())
        });
    }

    // -- line-level grammar --

    fn parse_header(&mut self) -> Result<(), DataError> {
        self.bump(); // '['
        let is_array = if self.peek() == Some('[') {
            self.bump();
            true
        } else {
            false
        };
        self.skip_inline_ws();
        let names = self.parse_dotted_key_names()?;
        self.skip_inline_ws();
        self.expect_char(']')?;
        if is_array {
            self.expect_char(']')?;
        }
        self.expect_eol_or_comment()?;
        let root = self.root.clone();
        let (path, node) = self.resolve(root, &names, is_array);
        self.reopen_to(&path);
        self.table_path = path;
        self.current_node = node;
        Ok(())
    }

    fn parse_keyval_line(&mut self) -> Result<(), DataError> {
        let names = self.parse_dotted_key_names()?;
        self.skip_inline_ws();
        self.expect_char('=')?;
        self.skip_inline_ws();
        let value = self.parse_value()?;
        let node = self.current_node.clone();
        let (rel, _child) = self.resolve(node, &names, false);
        let mut full = self.table_path.clone();
        full.extend(rel);
        self.push_value_event(full, value);
        self.expect_eol_or_comment()?;
        Ok(())
    }

    fn pump(&mut self) -> Result<(), DataError> {
        loop {
            self.skip_ws_nl_comments();
            match self.peek() {
                None => {
                    self.finish();
                    self.done = true;
                    return Ok(());
                }
                Some('[') => self.parse_header()?,
                Some(_) => self.parse_keyval_line()?,
            }
            if !self.queue.is_empty() {
                return Ok(());
            }
        }
    }
}

const fn is_bare_key_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_' || c == '-'
}

const fn is_bare_token_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.' | '_' | ':')
}

fn is_full_date(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() == 10
        && b[..4].iter().all(u8::is_ascii_digit)
        && b[4] == b'-'
        && b[5..7].iter().all(u8::is_ascii_digit)
        && b[7] == b'-'
        && b[8..10].iter().all(u8::is_ascii_digit)
}

fn insert_dotted(map: &mut IndexMap<strs::Symbol, Value>, names: &[String], value: Value) {
    let key = strs::intern(&names[0]);
    if names.len() == 1 {
        map.insert(key, value);
        return;
    }
    let entry = map
        .entry(key)
        .or_insert_with(|| Value::Object(Rc::new(IndexMap::new())));
    let Value::Object(inner) = entry else {
        *entry = Value::Object(Rc::new(IndexMap::new()));
        let Value::Object(inner) = entry else {
            unreachable!()
        };
        let mut inner_map = (**inner).clone();
        insert_dotted(&mut inner_map, &names[1..], value);
        *entry = Value::Object(Rc::new(inner_map));
        return;
    };
    let mut inner_map = (**inner).clone();
    insert_dotted(&mut inner_map, &names[1..], value);
    *entry = Value::Object(Rc::new(inner_map));
}

impl Iterator for TomlParser<'_> {
    type Item = ParseOutput;
    fn next(&mut self) -> Option<Self::Item> {
        loop {
            if let Some(item) = self.queue.pop_front() {
                return Some(Ok(item));
            }
            if self.done {
                return None;
            }
            if let Err(e) = self.pump() {
                self.done = true;
                return Some(Err(e));
            }
        }
    }
}
impl Parser for TomlParser<'_> {}

// -- serializer --

fn is_bare_key(s: &str) -> bool {
    !s.is_empty()
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

fn write_basic_string(out: &mut String, s: &str) {
    out.push('"');
    // `String: fmt::Write` never fails.
    escape::escape_string_json(s, '"', out).unwrap();
    out.push('"');
}

/// Writes `s` as a TOML multi-line basic string (`"""..."""`), keeping
/// newlines and tabs literal for readability. Every `"` is still escaped
/// (rather than only when it would collide with the closing delimiter) so
/// the result can never accidentally contain an unescaped `"""` sequence.
fn write_multiline_basic_string(out: &mut String, s: &str) {
    out.push_str("\"\"\"\n");
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' | '\t' => out.push(c),
            '\u{08}' => out.push_str("\\b"),
            '\u{0C}' => out.push_str("\\f"),
            '\r' => out.push_str("\\r"),
            c if (c as u32) <= 0x1F => {
                use std::fmt::Write;
                write!(out, "\\u{:04x}", c as u32).unwrap();
            }
            c => out.push(c),
        }
    }
    out.push_str("\"\"\"");
}

fn write_key(out: &mut String, s: &str) {
    if is_bare_key(s) {
        out.push_str(s);
    } else {
        write_basic_string(out, s);
    }
}

fn write_key_themed(out: &mut String, s: &str, opts: &render::Options) {
    let mut buf = String::new();
    write_key(&mut buf, s);
    super::json::push_styled(out, opts, render::ThemeIdx::ObjectKey, &buf);
}

fn write_float(out: &mut String, f: f64) {
    if f.is_nan() {
        out.push_str("nan");
    } else if f.is_infinite() {
        out.push_str(if f > 0.0 { "inf" } else { "-inf" });
    } else {
        let mut buf = ryu::Buffer::new();
        out.push_str(buf.format(f));
    }
}

/// TOML has no `null` - `null` members are simply dropped (see
/// `write_table`/the object & array branches here), so this is only ever
/// reached for a `null` that's the *sole* content of an array slot that
/// itself can't be dropped without changing the array's length; callers
/// otherwise filter nulls out before recursing here.
fn write_value_inline(
    out: &mut String,
    value: &Value,
    opts: &render::Options,
    top: bool,
) -> Result<(), DataError> {
    let sorted = opts.out.sort_keys;
    match value {
        Value::Null => super::json::push_styled(out, opts, render::ThemeIdx::Null, "\"<null>\""),
        Value::Bool(b) => super::json::push_styled(
            out,
            opts,
            if *b {
                render::ThemeIdx::True
            } else {
                render::ThemeIdx::False
            },
            if *b { "true" } else { "false" },
        ),
        Value::Decimal(n) => {
            super::json::push_styled(out, opts, render::ThemeIdx::Number, &n.to_string());
        }
        Value::Float(f) => {
            let mut buf = String::new();
            write_float(&mut buf, *f);
            super::json::push_styled(out, opts, render::ThemeIdx::Number, &buf);
        }
        Value::String(s) => {
            let mut buf = String::new();
            if top && opts.out.compact_level == render::CompactLevel::Pretty && s.contains('\n') {
                write_multiline_basic_string(&mut buf, s);
            } else {
                write_basic_string(&mut buf, s);
            }
            super::json::push_styled(out, opts, render::ThemeIdx::String, &buf);
        }
        Value::Array(items) => {
            out.push('[');
            let mut first = true;
            for item in items.iter() {
                if matches!(item, Value::Null) {
                    continue;
                }
                if !first {
                    out.push_str(", ");
                }
                first = false;
                write_value_inline(out, item, opts, false)?;
            }
            out.push(']');
        }
        Value::Object(_) => {
            let entries: Vec<_> = value
                .object_iter(sorted)
                .unwrap()
                .filter(|(_, v)| !matches!(v, Value::Null))
                .collect();
            if entries.is_empty() {
                out.push_str("{}");
            } else {
                out.push_str("{ ");
                for (i, (k, v)) in entries.iter().enumerate() {
                    if i > 0 {
                        out.push_str(", ");
                    }
                    write_key_themed(out, strs::resolve(**k).unwrap(), opts);
                    out.push_str(" = ");
                    write_value_inline(out, v, opts, false)?;
                }
                out.push_str(" }");
            }
        }
    }
    Ok(())
}

fn key_repr(s: &str) -> String {
    if is_bare_key(s) {
        s.to_string()
    } else {
        let mut b = String::new();
        write_basic_string(&mut b, s);
        b
    }
}

/// Writes `obj`'s direct scalar keys first, then recurses into non-empty
/// nested tables / arrays-of-tables as `[prefix.key]` / `[[prefix.key]]`
/// sections - the conventional "pretty" TOML table layout.
fn write_table(
    out: &mut String,
    obj: &Value,
    prefix: &[String],
    opts: &render::Options,
) -> Result<(), DataError> {
    let sorted = opts.out.sort_keys;
    let entries: Vec<_> = obj.object_iter(sorted).unwrap().collect();
    let mut nested: Vec<(String, &Value)> = Vec::new();
    for (k, v) in entries {
        // TOML has no `null` - drop `null`-valued members entirely rather
        // than erroring (the top-level document itself is handled
        // separately in `put`, since it can't just be dropped).
        if matches!(v, Value::Null) {
            continue;
        }
        let key = strs::resolve(*k).unwrap().to_string();
        let is_nested_table = matches!(v, Value::Object(m) if !m.is_empty());
        let is_array_of_tables = matches!(v, Value::Array(items) if {
            let mut non_null = items.iter().filter(|i| !matches!(i, Value::Null));
            non_null.clone().next().is_some() && non_null.all(|i| matches!(i, Value::Object(_)))
        });
        if is_nested_table || is_array_of_tables {
            nested.push((key, v));
        } else {
            write_key_themed(out, &key, opts);
            out.push_str(" = ");
            write_value_inline(out, v, opts, true)?;
            out.push('\n');
        }
    }
    for (key, v) in nested {
        let mut path = prefix.to_vec();
        path.push(key);
        let header: String = path
            .iter()
            .map(|s| key_repr(s))
            .collect::<Vec<_>>()
            .join(".");
        if opts.out.compact_level == render::CompactLevel::Pretty && !out.is_empty() {
            out.push('\n');
        }
        match v {
            Value::Object(_) => {
                out.push('[');
                out.push_str(&header);
                out.push_str("]\n");
                write_table(out, v, &path, opts)?;
            }
            Value::Array(items) => {
                for (i, item) in items
                    .iter()
                    .filter(|i| !matches!(i, Value::Null))
                    .enumerate()
                {
                    if i > 0 && opts.out.compact_level == render::CompactLevel::Pretty {
                        out.push('\n');
                    }
                    out.push_str("[[");
                    out.push_str(&header);
                    out.push_str("]]\n");
                    write_table(out, item, &path, opts)?;
                }
            }
            _ => unreachable!(),
        }
    }
    Ok(())
}

/// Separate root tables with `+++` by default.
pub struct TomlSerializer {
    output: Output,
    options: render::Options,
    written: bool,
}
impl TomlSerializer {
    #[must_use]
    pub const fn new(output: Output, options: render::Options) -> Self {
        Self {
            output,
            options,
            written: false,
        }
    }
    pub(crate) fn finish(self) -> std::io::Result<()> {
        self.output.finish()
    }
}
impl Serializer for TomlSerializer {
    fn put(&mut self, value: Value) -> Result<(), DataError> {
        if self.options.out.quiet {
            return Ok(());
        }
        let mut text = String::new();
        match &value {
            // A `null` root can't just be dropped like a `null` member
            // elsewhere - there'd be nothing left to write at all.
            Value::Null => text.push_str("_ = \"<null>\"\n"),
            Value::Object(_) => {
                write_table(&mut text, &value, &[], &self.options)?;
            }
            // TOML documents are always tables - a non-object root (a
            // scalar, or an array) is wrapped under a synthetic `_` key
            // instead of erroring.
            _ => {
                let mut root = IndexMap::new();
                root.insert(strs::keyword_underscore(), value);
                write_table(&mut text, &Value::Object(Rc::new(root)), &[], &self.options)?;
            }
        }
        let mut text = text.trim_end_matches('\n').to_owned();
        if self.written {
            if self.options.out.compact_level == render::CompactLevel::Pretty {
                text.insert_str(0, "\n+++\n\n");
            } else {
                text.insert_str(0, "+++\n");
            }
        }
        if let Some(begin) = self.options.out.doc_begin {
            text.insert_str(0, begin);
        }
        text.push_str(self.options.out.doc_end.unwrap_or("\n"));
        self.output
            .write_all(text.as_bytes())
            .map_err(DataError::IOError)?;
        self.written = true;
        if self.options.out.doc_end_flush {
            self.output.flush().map_err(DataError::IOError)?;
        }
        Ok(())
    }
}
