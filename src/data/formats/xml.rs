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
use std::fmt::Write as _;
use std::{
    collections::{HashSet, VecDeque},
    io::{BufReader, Read, Write},
};

const fn is_name_start_char(c: char) -> bool {
    matches!(c, ':' | '_' | 'A'..='Z' | 'a'..='z' | '\u{c0}'..='\u{d6}' | '\u{d8}'..='\u{f6}' | '\u{f8}'..='\u{2ff}' | '\u{370}'..='\u{37d}' | '\u{37f}'..='\u{1fff}' | '\u{200c}'..='\u{200d}' | '\u{2070}'..='\u{218f}' | '\u{2c00}'..='\u{2fef}' | '\u{3001}'..='\u{d7ff}' | '\u{f900}'..='\u{fdcf}' | '\u{fdf0}'..='\u{fffd}' | '\u{10000}'..='\u{effff}')
}
const fn is_name_char(c: char) -> bool {
    is_name_start_char(c)
        || matches!(c, '-' | '.' | '0'..='9' | '\u{b7}' | '\u{300}'..='\u{36f}' | '\u{203f}'..='\u{2040}')
}
const fn is_xml10_char(c: char) -> bool {
    matches!(c, '\t' | '\n' | '\r' | '\u{20}'..='\u{d7ff}' | '\u{e000}'..='\u{fffd}' | '\u{10000}'..='\u{10ffff}')
}
const fn ws(c: char) -> bool {
    matches!(c, ' ' | '\t' | '\r' | '\n')
}

/// An open element and its pending children/text. Relative events leave
/// its children array open until `close_children` finishes the element.
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
    queue: VecDeque<StreamItem>,
    done: bool,
}

impl<'a> XmlParser<'a> {
    #[must_use]
    pub const fn new(input: Input<'a>) -> Self {
        Self {
            input: Some(input),
            source: String::new(),
            pos: 0,
            filename: String::new(),
            stage: Stage::Preamble,
            stack: Vec::new(),
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
    /// `<name attr="v" ... />`), emitting its `name` and
    /// `attributes` events immediately. Returns `(tag, self_closing)`.
    fn open_tag(&mut self) -> Result<(String, bool), DataError> {
        self.expect("<")?;
        let tag = self.read_name()?;
        self.queue
            .push_back(StreamItem::Push(PathItem::new_key_str("name")));
        self.queue
            .push_back(StreamItem::Value(Value::String(tag.clone().into())));
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
        self.queue
            .push_back(StreamItem::Push(PathItem::new_key_str("attributes")));
        if attrs.is_empty() {
            self.queue
                .push_back(StreamItem::Value(Value::empty_object()));
        } else {
            for (key, value) in attrs {
                self.queue
                    .push_back(StreamItem::Push(PathItem::new_key_str(&key)));
                self.queue
                    .push_back(StreamItem::Value(Value::String(value.into())));
            }
            self.queue.push_back(StreamItem::Close);
        }
        self.queue
            .push_back(StreamItem::Push(PathItem::new_key_str("children")));
        Ok((tag, empty))
    }

    /// Finish the children array, then the element object itself.
    fn close_children(&mut self, child_count: isize) {
        self.queue.push_back(if child_count == 0 {
            StreamItem::Value(Value::empty_array())
        } else {
            StreamItem::Close
        });
        self.queue.push_back(StreamItem::Close);
    }

    fn flush_pending_text(frame: &mut Frame, queue: &mut VecDeque<StreamItem>) {
        let Some(text) = frame.pending_text.take() else {
            return;
        };
        if text.is_empty() {
            return;
        }
        let idx = frame.child_count;
        frame.child_count += 1;
        queue.push_back(StreamItem::Push(PathItem::new_idx(idx)));
        queue.push_back(StreamItem::Push(PathItem::new_key_str("text")));
        queue.push_back(StreamItem::Value(Value::String(text.into())));
        queue.push_back(StreamItem::Close);
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

    /// Process one text/comment/tag unit against the innermost element.
    fn step_element(&mut self) -> Result<(), DataError> {
        if self.take("</") {
            let name = self.read_name()?;
            if name != self.stack.last().unwrap().tag {
                return Err(self.error("mismatched XML closing tag"));
            }
            self.spaces();
            self.expect(">")?;
            let mut frame = self.stack.pop().unwrap();
            Self::flush_pending_text(&mut frame, &mut self.queue);
            self.close_children(frame.child_count);
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
            Self::flush_pending_text(frame, &mut self.queue);
            let idx = frame.child_count;
            frame.child_count += 1;
            self.queue
                .push_back(StreamItem::Push(PathItem::new_idx(idx)));
            let (tag, empty) = self.open_tag()?;
            if empty {
                self.close_children(0);
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
                                    encoding = true;
                                }
                                "standalone"
                                    if !standalone && matches!(value.as_str(), "yes" | "no") =>
                                {
                                    standalone = true;
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
                    let (tag, empty) = self.open_tag()?;
                    if empty {
                        self.close_children(0);
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
                    self.step_element()?;
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
impl Iterator for XmlParser<'_> {
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
impl Parser for XmlParser<'_> {}

const fn invalid(message: &'static str) -> DataError {
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
            '\n' | '\t' if attr => {
                let _ = write!(out, "&#{};", c as u32);
            }
            c if ascii && !c.is_ascii() => {
                let _ = write!(out, "&#{};", c as u32);
            }
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
    #[must_use]
    pub const fn new(output: Output, options: render::Options) -> Self {
        Self { output, options }
    }
    pub(crate) fn finish(self) -> std::io::Result<()> {
        self.output.finish()
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
