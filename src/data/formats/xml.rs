//! XML documents represented as ordered element/text trees.
//!
//! Parsing is driven by an explicit stack of currently-open elements (see
//! `Frame`/`step_element`): each element's `name`/`attributes` events are
//! emitted as soon as its opening tag is read, and its `children` events
//! are emitted incrementally as each child element/text run completes,
//! rather than building the whole document as a `Value` tree first and
//! replaying it afterward. This means a document that fails to parse
//! partway through (e.g. a missing closing tag near the end of a large
//! file) still yields events for every element that *did* close
//! successfully before the error is returned.
use super::{DataError, ParseOutput, Parser, PathItem, Serializer, StreamItem, Value};
use crate::{
    io::{Input, Output},
    render, strs,
};
use std::{
    collections::{HashSet, VecDeque},
    io::{BufReader, Read, Write},
};

fn is_name_start_char(c: char) -> bool {
    matches!(c, ':' | '_' | 'A'..='Z' | 'a'..='z' | '\u{c0}'..='\u{d6}' | '\u{d8}'..='\u{f6}' | '\u{f8}'..='\u{2ff}' | '\u{370}'..='\u{37d}' | '\u{37f}'..='\u{1fff}' | '\u{200c}'..='\u{200d}' | '\u{2070}'..='\u{218f}' | '\u{2c00}'..='\u{2fef}' | '\u{3001}'..='\u{d7ff}' | '\u{f900}'..='\u{fdcf}' | '\u{fdf0}'..='\u{fffd}' | '\u{10000}'..='\u{effff}')
}
fn is_name_char(c: char) -> bool {
    is_name_start_char(c)
        || matches!(c, '-' | '.' | '0'..='9' | '\u{b7}' | '\u{300}'..='\u{36f}' | '\u{203f}'..='\u{2040}')
}
fn is_xml10_char(c: char) -> bool {
    matches!(c, '\t' | '\n' | '\r' | '\u{20}'..='\u{d7ff}' | '\u{e000}'..='\u{fffd}' | '\u{10000}'..='\u{10ffff}')
}
fn ws(c: char) -> bool {
    matches!(c, ' ' | '\t' | '\r' | '\n')
}

fn append(path: &[PathItem], item: PathItem) -> Vec<PathItem> {
    let mut v = path.to_vec();
    v.push(item);
    v
}

/// An element currently open on the parse stack: its own `name`/
/// `attributes` events have already been emitted, and `child_count`/
/// `pending_text` track its still-open `children` array.
///
/// A frame's own path is *not* stored here - it's always exactly the
/// parser's shared `path` stack at the point this frame is the innermost
/// open one (see `XmlParser::path`), pushed/popped in lock-step with this
/// frame as it's opened/closed.
struct Frame {
    tag: String,
    child_count: isize,
    /// Text/CDATA read since the last flush point (a child element start,
    /// or this frame's own closing tag) - merged the same way the old
    /// tree-building parser merged consecutive text nodes.
    pending_text: Option<String>,
}

enum Stage {
    Preamble,
    InElement,
    Trailing,
}

pub struct XmlParser<'a> {
    input: Option<Input<'a>>,
    source: String,
    pos: usize,
    filename: String,
    stage: Stage,
    stack: Vec<Frame>,
    /// The innermost currently-open frame's own path - `[]` whenever
    /// `stack` is empty (before the root opens, or after it closes).
    /// Threaded through the parsing methods as a `&mut Vec<PathItem>` so
    /// descending into (and returning from) a child element is a
    /// push/pop pair instead of cloning the whole path.
    path: Vec<PathItem>,
    queue: VecDeque<StreamItem>,
    done: bool,
}

impl<'a> XmlParser<'a> {
    pub fn new(input: Input<'a>) -> Self {
        Self {
            input: Some(input),
            source: String::new(),
            pos: 0,
            filename: String::new(),
            stage: Stage::Preamble,
            stack: Vec::new(),
            path: Vec::new(),
            queue: VecDeque::new(),
            done: false,
        }
    }

    fn rest(&self) -> &str {
        &self.source[self.pos..]
    }
    fn error(&self, message: &str) -> DataError {
        let prefix = &self.source[..self.pos];
        DataError::ParseError {
            path: self.filename.clone(),
            line: prefix.bytes().filter(|&b| b == b'\n').count() as u32 + 1,
            col: prefix.rsplit('\n').next().unwrap_or("").chars().count() as u32 + 1,
            message: message.into(),
        }
    }
    fn take(&mut self, s: &str) -> bool {
        if self.rest().starts_with(s) {
            self.pos += s.len();
            true
        } else {
            false
        }
    }
    fn expect(&mut self, s: &str) -> Result<(), DataError> {
        if self.take(s) {
            Ok(())
        } else {
            Err(self.error(&format!("expected {s:?}")))
        }
    }
    fn spaces(&mut self) -> bool {
        let start = self.pos;
        while let Some(c) = self.rest().chars().next().filter(|&c| ws(c)) {
            self.pos += c.len_utf8();
        }
        self.pos != start
    }
    fn read_name(&mut self) -> Result<String, DataError> {
        let start = self.pos;
        let Some(c) = self
            .rest()
            .chars()
            .next()
            .filter(|&c| is_name_start_char(c))
        else {
            return Err(self.error("expected XML name"));
        };
        self.pos += c.len_utf8();
        while let Some(c) = self.rest().chars().next().filter(|&c| is_name_char(c)) {
            self.pos += c.len_utf8();
        }
        Ok(self.source[start..self.pos].into())
    }
    fn through(&mut self, end: &str) -> Result<String, DataError> {
        let n = self
            .rest()
            .find(end)
            .ok_or_else(|| self.error("unterminated XML construct"))?;
        let text = self.rest()[..n].to_owned();
        self.pos += n + end.len();
        Ok(text)
    }
    fn misc(&mut self) -> Result<bool, DataError> {
        if self.take("<!--") {
            let text = self.through("-->")?;
            if text.contains("--") || text.ends_with('-') {
                return Err(self.error("invalid XML comment"));
            }
            return Ok(true);
        }
        if self.take("<?") {
            let target = self.read_name()?;
            if target.eq_ignore_ascii_case("xml") {
                return Err(self.error("XML declaration must appear at the start"));
            }
            if !self.take("?>") {
                if !self.spaces() {
                    return Err(
                        self.error("expected whitespace after processing instruction target")
                    );
                }
                self.through("?>")?;
            }
            return Ok(true);
        }
        Ok(false)
    }
    fn reference(&mut self) -> Result<char, DataError> {
        let entity = self.through(";")?;
        let c = match entity.as_str() {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "apos" => Some('\''),
            "quot" => Some('"'),
            _ => {
                let (digits, radix) = if let Some(s) = entity.strip_prefix("#x") {
                    (s, 16)
                } else if let Some(s) = entity.strip_prefix('#') {
                    (s, 10)
                } else {
                    return Err(self.error("unknown XML entity"));
                };
                if digits.is_empty() || !digits.chars().all(|c| c.is_digit(radix)) {
                    return Err(self.error("invalid character reference"));
                }
                u32::from_str_radix(digits, radix)
                    .ok()
                    .and_then(char::from_u32)
            }
        };
        c.filter(|&c| is_xml10_char(c))
            .ok_or_else(|| self.error("invalid XML character reference"))
    }
    fn text(&mut self, quote: Option<char>) -> Result<String, DataError> {
        let mut out = String::new();
        while let Some(c) = self.rest().chars().next() {
            if Some(c) == quote || (quote.is_none() && c == '<') {
                break;
            }
            if c == '<' || (quote.is_none() && self.rest().starts_with("]]>")) {
                return Err(self.error("invalid XML text"));
            }
            self.pos += c.len_utf8();
            if c == '&' {
                out.push(self.reference()?);
            } else {
                out.push(if quote.is_some() && ws(c) { ' ' } else { c });
            }
        }
        Ok(out)
    }
    fn attribute(&mut self) -> Result<(String, String), DataError> {
        let key = self.read_name()?;
        self.spaces();
        self.expect("=")?;
        self.spaces();
        let quote = self
            .rest()
            .chars()
            .next()
            .filter(|c| matches!(c, '\'' | '"'))
            .ok_or_else(|| self.error("attribute value must be quoted"))?;
        self.pos += 1;
        let value = self.text(Some(quote))?;
        self.expect(&quote.to_string())?;
        Ok((key, value))
    }
    fn declaration_attribute(&mut self) -> Result<(String, String), DataError> {
        let start = self.pos;
        let result = self.attribute()?;
        if self.source[start..self.pos].contains('&') {
            return Err(self.error("references are not allowed in XML declarations"));
        }
        Ok(result)
    }

    /// Parses an opening tag (`<name attr="v" ...>` or self-closing
    /// `<name attr="v" ... />`) at `path`, emitting its `name` and
    /// `attributes` events immediately. Returns `(tag, self_closing)`.
    fn open_tag(&mut self, path: &[PathItem]) -> Result<(String, bool), DataError> {
        self.expect("<")?;
        let tag = self.read_name()?;
        self.queue.push_back(StreamItem {
            path: append(path, PathItem::new_key_str("name")),
            value: Some(Value::String(tag.clone().into())),
        });
        let mut attrs: Vec<(String, String)> = Vec::new();
        let mut seen = HashSet::new();
        let empty;
        loop {
            let spaced = self.spaces();
            if self.take("/>") {
                empty = true;
                break;
            }
            if self.take(">") {
                empty = false;
                break;
            }
            if !spaced {
                return Err(self.error("expected whitespace before attribute"));
            }
            let (key, value) = self.attribute()?;
            if !seen.insert(key.clone()) {
                return Err(self.error("duplicate XML attribute"));
            }
            attrs.push((key, value));
        }
        let attrs_path = append(path, PathItem::new_key_str("attributes"));
        if attrs.is_empty() {
            self.queue.push_back(StreamItem {
                path: attrs_path,
                value: Some(Value::empty_object()),
            });
        } else {
            let mut last = attrs_path.clone();
            for (key, value) in &attrs {
                last = append(&attrs_path, PathItem::new_key_str(key));
                self.queue.push_back(StreamItem {
                    path: last.clone(),
                    value: Some(Value::String(value.clone().into())),
                });
            }
            self.queue.push_back(StreamItem {
                path: last,
                value: None,
            });
        }
        Ok((tag, empty))
    }

    /// Emits the (already-known) `children` events for an element at
    /// `path` with `child_count` children, then the element's own closing
    /// event - the last two events every element produces, regardless of
    /// whether its children turned out to be empty or not.
    fn close_children(&mut self, path: &[PathItem], child_count: isize) {
        let children_path = append(path, PathItem::new_key_str("children"));
        if child_count == 0 {
            self.queue.push_back(StreamItem {
                path: children_path.clone(),
                value: Some(Value::empty_array()),
            });
        } else {
            let mut last = children_path.clone();
            last.push(PathItem::new_idx(child_count - 1));
            self.queue.push_back(StreamItem {
                path: last,
                value: None,
            });
        }
        self.queue.push_back(StreamItem {
            path: children_path,
            value: None,
        });
    }

    /// `path` is the frame's own path (the parser's shared path stack).
    fn flush_pending_text(frame: &mut Frame, path: &[PathItem], queue: &mut VecDeque<StreamItem>) {
        let Some(text) = frame.pending_text.take() else {
            return;
        };
        if text.is_empty() {
            return;
        }
        let idx = frame.child_count;
        frame.child_count += 1;
        let mut path = append(path, PathItem::new_key_str("children"));
        path.push(PathItem::new_idx(idx));
        path.push(PathItem::new_key_str("text"));
        queue.push_back(StreamItem {
            path: path.clone(),
            value: Some(Value::String(text.into())),
        });
        queue.push_back(StreamItem { path, value: None });
    }

    fn merge_text(frame: &mut Frame, text: String) {
        frame.pending_text = Some(match frame.pending_text.take() {
            Some(mut buf) => {
                buf.push_str(&text);
                buf
            }
            None => text,
        });
    }

    /// Processes exactly one syntactic unit (a piece of text, a comment/PI,
    /// a child element's opening tag, or the top frame's own closing tag)
    /// against the innermost currently-open element. `path` is the parser's
    /// shared path stack, kept equal to the top frame's own path (or `[]`
    /// while `stack` is empty) - see `XmlParser::path`.
    fn step_element(&mut self, path: &mut Vec<PathItem>) -> Result<(), DataError> {
        if self.take("</") {
            let name = self.read_name()?;
            if name != self.stack.last().unwrap().tag {
                return Err(self.error("mismatched XML closing tag"));
            }
            self.spaces();
            self.expect(">")?;
            let mut frame = self.stack.pop().unwrap();
            Self::flush_pending_text(&mut frame, path, &mut self.queue);
            self.close_children(path, frame.child_count);
            if !self.stack.is_empty() {
                path.truncate(path.len() - 2); // pop "children", idx
            }
            return Ok(());
        }
        if self.rest().is_empty() {
            return Err(self.error("unclosed XML element"));
        }
        if self.misc()? {
            return Ok(());
        }
        if self.take("<![CDATA[") {
            let text = self.through("]]>")?;
            Self::merge_text(self.stack.last_mut().unwrap(), text);
            return Ok(());
        }
        if self.rest().starts_with('<') {
            if self.stack.len() >= 128 {
                return Err(self.error("XML nesting exceeds 128 levels"));
            }
            let frame = self.stack.last_mut().unwrap();
            Self::flush_pending_text(frame, path, &mut self.queue);
            let idx = frame.child_count;
            frame.child_count += 1;
            path.push(PathItem::new_key_str("children"));
            path.push(PathItem::new_idx(idx));
            let (tag, empty) = self.open_tag(path)?;
            if empty {
                self.close_children(path, 0);
                path.truncate(path.len() - 2); // pop "children", idx
            } else {
                self.stack.push(Frame {
                    tag,
                    child_count: 0,
                    pending_text: None,
                });
            }
            return Ok(());
        }
        let text = self.text(None)?;
        Self::merge_text(self.stack.last_mut().unwrap(), text);
        Ok(())
    }

    /// Drives parsing forward until at least one event has been queued, or
    /// the document is fully consumed.
    fn pump(&mut self) -> Result<(), DataError> {
        loop {
            match self.stage {
                Stage::Preamble => {
                    self.take("\u{feff}");
                    if self.rest().starts_with("<?xml") && self.rest()[5..].starts_with(ws) {
                        self.pos += 5;
                        self.spaces();
                        let (key, version) = self.declaration_attribute()?;
                        if key != "version" || version != "1.0" {
                            return Err(self.error("only XML 1.0 is supported"));
                        }
                        let mut encoding = false;
                        let mut standalone = false;
                        loop {
                            let spaced = self.spaces();
                            if self.take("?>") {
                                break;
                            }
                            if !spaced {
                                return Err(self.error("expected whitespace in XML declaration"));
                            }
                            let (key, value) = self.declaration_attribute()?;
                            match key.as_str() {
                                "encoding"
                                    if !encoding
                                        && !standalone
                                        && value.eq_ignore_ascii_case("utf-8") =>
                                {
                                    encoding = true
                                }
                                "standalone"
                                    if !standalone && matches!(value.as_str(), "yes" | "no") =>
                                {
                                    standalone = true
                                }
                                _ => {
                                    return Err(self.error(
                                        "invalid XML declaration (only UTF-8 is supported)",
                                    ));
                                }
                            }
                        }
                    }
                    loop {
                        self.spaces();
                        if !self.misc()? {
                            break;
                        }
                    }
                    if self.rest().starts_with("<!DOCTYPE") {
                        return Err(self.error("DOCTYPE and custom entities are not supported"));
                    }
                    let (tag, empty) = self.open_tag(&[])?;
                    if empty {
                        self.close_children(&[], 0);
                        self.stage = Stage::Trailing;
                    } else {
                        self.stack.push(Frame {
                            tag,
                            child_count: 0,
                            pending_text: None,
                        });
                        self.stage = Stage::InElement;
                    }
                }
                Stage::InElement => {
                    let mut path = std::mem::take(&mut self.path);
                    let result = self.step_element(&mut path);
                    self.path = path;
                    result?;
                    if self.stack.is_empty() {
                        self.stage = Stage::Trailing;
                    }
                }
                Stage::Trailing => {
                    loop {
                        self.spaces();
                        if !self.misc()? {
                            break;
                        }
                    }
                    if !self.rest().is_empty() {
                        return Err(self.error("unexpected content after XML root"));
                    }
                    self.done = true;
                    return Ok(());
                }
            }
            if !self.queue.is_empty() {
                return Ok(());
            }
        }
    }
}
impl<'a> Iterator for XmlParser<'a> {
    type Item = ParseOutput;
    fn next(&mut self) -> Option<Self::Item> {
        loop {
            if let Some(item) = self.queue.pop_front() {
                return Some(Ok(item));
            }
            if self.done {
                return None;
            }
            if let Some(input) = self.input.take() {
                let filename = input.filename().to_owned();
                let mut source = String::new();
                if let Err(e) = BufReader::new(input).read_to_string(&mut source) {
                    self.done = true;
                    return Some(Err(DataError::IOError(e)));
                }
                self.source = source.replace("\r\n", "\n").replace('\r', "\n");
                self.filename = filename;
                if let Some((pos, _)) = self.source.char_indices().find(|&(_, c)| !is_xml10_char(c))
                {
                    self.pos = pos;
                    self.done = true;
                    return Some(Err(self.error("invalid XML character")));
                }
            }
            if let Err(e) = self.pump() {
                self.done = true;
                return Some(Err(e));
            }
        }
    }
}
impl<'a> Parser for XmlParser<'a> {}

fn invalid(message: &'static str) -> DataError {
    DataError::UnableToSerializeValueType {
        value_type: message,
    }
}

fn name(s: &str) -> Result<&str, DataError> {
    let mut chars = s.chars();
    if !chars.next().is_some_and(is_name_start_char) || !chars.all(is_name_char) {
        return Err(invalid("invalid XML name"));
    }
    Ok(s)
}

fn escaped(out: &mut String, s: &str, attr: bool, ascii: bool) -> Result<(), DataError> {
    for c in s.chars() {
        if !is_xml10_char(c) {
            return Err(invalid("invalid XML character"));
        }
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' if attr => out.push_str("&quot;"),
            '\r' => out.push_str("&#13;"),
            '\n' | '\t' if attr => out.push_str(&format!("&#{};", c as u32)),
            c if ascii && !c.is_ascii() => out.push_str(&format!("&#{};", c as u32)),
            c => out.push(c),
        }
    }
    Ok(())
}

fn write_node(
    out: &mut String,
    value: &Value,
    depth: usize,
    opts: &render::Options,
) -> Result<(), DataError> {
    let Value::Object(map) = value else {
        return Err(invalid("XML node must be an object"));
    };
    let get = |key: &str| map.get(&strs::intern(key));
    if let Some(Value::String(text)) = get("text") {
        if depth == 0 || map.len() != 1 {
            return Err(invalid(
                "XML text node requires only text and cannot be the root",
            ));
        }
        return escaped(out, text, false, opts.out.ascii_only);
    }
    if depth >= 128 {
        return Err(invalid("XML nesting exceeds 128 levels"));
    }
    let (Some(Value::String(tag)), Some(Value::Object(attrs)), Some(Value::Array(children))) =
        (get("name"), get("attributes"), get("children"))
    else {
        return Err(invalid(
            "XML element requires string name, object attributes, and array children",
        ));
    };
    if map.len() != 3 {
        return Err(invalid("unexpected XML element field"));
    }
    out.push('<');
    out.push_str(name(tag)?);
    for (key, value) in Value::object_entries(attrs, opts.out.sort_keys) {
        let Value::String(value) = value else {
            return Err(invalid("XML attribute must be a string"));
        };
        out.push(' ');
        out.push_str(name(strs::resolve(*key).unwrap_or(""))?);
        out.push_str("=\"");
        escaped(out, value, true, opts.out.ascii_only)?;
        out.push('"');
    }
    if children.is_empty() {
        out.push_str("/>");
    } else {
        out.push('>');
        for child in children.iter() {
            write_node(out, child, depth + 1, opts)?;
        }
        out.push_str("</");
        out.push_str(tag);
        out.push('>');
    }
    Ok(())
}

/// Parses `text` purely to validate it (used to sanity-check serializer
/// output before writing any bytes) - errors are collapsed to a generic
/// "invalid XML document", since the interesting error is why `write_node`
/// was asked to serialize something bad in the first place.
fn validate(text: &str) -> Result<(), DataError> {
    XmlParser::new(Input::new_string(text.to_owned()))
        .collect::<Result<Vec<_>, _>>()
        .map(|_| ())
}

pub struct XmlSerializer {
    output: Output,
    options: render::Options,
}
impl XmlSerializer {
    pub fn new(output: Output, options: render::Options) -> Self {
        Self { output, options }
    }
}
impl Serializer for XmlSerializer {
    fn put(&mut self, value: Value) -> Result<(), DataError> {
        let mut text = String::new();
        write_node(&mut text, &value, 0, &self.options)?;
        // Validate the complete document before writing any bytes.
        validate(&text).map_err(|_| invalid("invalid XML document"))?;
        if self.options.out.quiet {
            return Ok(());
        }
        if let Some(begin) = self.options.out.doc_begin {
            text.insert_str(0, begin);
        }
        text.push_str(self.options.out.doc_end.unwrap_or("\n"));
        self.output
            .write_all(text.as_bytes())
            .map_err(DataError::IOError)?;
        if self.options.out.doc_end_flush {
            self.output.flush().map_err(DataError::IOError)?;
        }
        Ok(())
    }
}
