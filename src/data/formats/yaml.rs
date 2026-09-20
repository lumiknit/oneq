//! Small, JSON-shaped YAML 1.2 reader and writer.
use super::{DataError, ParseOutput, Parser, PathItem, Serializer, StreamItem, Value};
use crate::data::core::decimal::Decimal;
use crate::{
    io::{Input, Output},
    render, strs,
};
use indexmap::IndexMap;
use std::{
    collections::{HashMap, VecDeque},
    io::{BufRead, BufReader, Write},
    rc::Rc,
};

#[derive(Clone)]
struct Line {
    no: u32,
    indent: usize,
    text: String,
}

pub struct YamlParser<'a> {
    input: BufReader<Input<'a>>,
    filename: String,
    pending: String,
    queue: VecDeque<StreamItem>,
    done: bool,
    /// An error from `document()`, held back until `queue` - which may
    /// already hold events for whatever successfully parsed before the
    /// error was hit - has been fully drained.
    pending_error: Option<DataError>,
    line: u32,
}
impl<'a> YamlParser<'a> {
    pub fn new(input: Input<'a>) -> Self {
        let filename = input.filename().to_owned();
        Self {
            input: BufReader::new(input),
            filename,
            pending: String::new(),
            queue: VecDeque::new(),
            done: false,
            pending_error: None,
            line: 0,
        }
    }
    fn document(&mut self) -> Result<(), DataError> {
        let mut source = std::mem::take(&mut self.pending);
        if source
            .lines()
            .next()
            .is_some_and(|x| document_marker(x, "---"))
        {
            source = source.lines().skip(1).collect();
        }
        let mut content = source.lines().any(is_content);
        loop {
            let mut line = String::new();
            let n = self
                .input
                .read_line(&mut line)
                .map_err(DataError::IOError)?;
            if n == 0 {
                self.done = true;
                break;
            }
            self.line += 1;
            if document_marker(&line, "---") && content {
                self.pending = line;
                break;
            }
            if document_marker(&line, "---")
                || document_marker(&line, "...")
                || line.trim_start().starts_with('%')
            {
                continue;
            }
            content |= is_content(&line);
            source.push_str(&line);
        }
        if !content {
            return Ok(());
        }
        let mut lines = Vec::new();
        for (i, raw) in source.lines().enumerate() {
            let indent = raw.bytes().take_while(|b| *b == b' ').count();
            let text = raw[indent..].trim_end();
            if text.is_empty() || text.trim_start().starts_with('#') {
                continue;
            }
            lines.push(Line {
                no: i as u32 + 1,
                indent,
                text: strip_comment(text),
            });
        }
        let mut p = Reader {
            lines,
            pos: 0,
            anchors: HashMap::new(),
            filename: &self.filename,
        };
        let indent = p.lines.first().map_or(0, |x| x.indent);
        let mut path = Vec::new();
        let value = p.node_eager(indent, &mut path, &mut self.queue)?;
        if p.pos != p.lines.len() {
            return Err(p.err("unexpected YAML content"));
        }
        // A later explicit key can replace a merged subtree. Successful
        // documents must stream the resolved value, not obsolete merge leaves.
        // On errors, keep the partial events produced above for --stream-errors.
        self.queue.clear();
        super::document::events(value, &mut path, &mut self.queue);
        Ok(())
    }
}
impl<'a> Iterator for YamlParser<'a> {
    type Item = ParseOutput;
    fn next(&mut self) -> Option<Self::Item> {
        loop {
            if let Some(x) = self.queue.pop_front() {
                return Some(Ok(x));
            }
            if let Some(e) = self.pending_error.take() {
                self.done = true;
                return Some(Err(e));
            }
            if self.done {
                return None;
            }
            if let Err(e) = self.document() {
                self.pending_error = Some(e);
            }
        }
    }
}
impl<'a> Parser for YamlParser<'a> {}

struct Reader<'a> {
    lines: Vec<Line>,
    pos: usize,
    anchors: HashMap<String, Value>,
    filename: &'a str,
}
impl Reader<'_> {
    fn err(&self, s: impl Into<String>) -> DataError {
        DataError::ParseError {
            path: self.filename.to_owned(),
            line: self.lines.get(self.pos).map_or(1, |x| x.no),
            col: 1,
            message: s.into(),
        }
    }
    /// Pure ("atomic") recursive-descent dispatch: computes and returns a
    /// whole subtree's `Value` with no side effects. Used only where a
    /// value must be fully materialized before it can be used (a YAML
    /// anchor's definition, reachable again later via `*alias`/`<<` from
    /// anywhere else in the document) - see `node_eager` for the
    /// self-streaming counterpart used everywhere else.
    fn node(&mut self, indent: usize) -> Result<Value, DataError> {
        let Some(l) = self.lines.get(self.pos).cloned() else {
            return Ok(Value::Null);
        };
        if l.indent < indent {
            return Ok(Value::Null);
        }
        if l.indent != indent {
            return Err(self.err("inconsistent YAML indentation"));
        }
        if l.text == "-" || l.text.starts_with("- ") {
            self.seq_atomic(indent)
        } else if split_colon(&l.text).is_some() {
            self.map_atomic(indent)
        } else {
            self.pos += 1;
            self.text_value(&l.text, indent)
        }
    }
    fn seq_atomic(&mut self, indent: usize) -> Result<Value, DataError> {
        let mut a = Vec::new();
        while let Some(l) = self.lines.get(self.pos).cloned() {
            if l.indent != indent || !(l.text == "-" || l.text.starts_with("- ")) {
                break;
            }
            self.pos += 1;
            let s = l.text[1..].trim();
            if s.is_empty() {
                a.push(self.child(indent)?)
            } else if split_colon(s).is_some() {
                a.push(self.map_first_atomic(indent + 2, s)?)
            } else {
                a.push(self.text_value(s, indent + 2)?)
            }
        }
        Ok(Value::Array(Rc::new(a)))
    }
    fn map_atomic(&mut self, indent: usize) -> Result<Value, DataError> {
        self.map_first_atomic(indent, "")
    }
    fn map_first_atomic(&mut self, indent: usize, first: &str) -> Result<Value, DataError> {
        let mut m = IndexMap::new();
        let mut first = (!first.is_empty()).then(|| first.to_owned());
        loop {
            let l = if let Some(s) = first.take() {
                Line {
                    no: 0,
                    indent,
                    text: s,
                }
            } else {
                let Some(x) = self.lines.get(self.pos).cloned() else {
                    break;
                };
                if x.indent != indent || split_colon(&x.text).is_none() {
                    break;
                }
                self.pos += 1;
                x
            };
            let (k, r) = split_colon(&l.text).unwrap();
            let key = self.key(k)?;
            let v = if r.trim().is_empty() {
                self.child(indent)?
            } else {
                self.text_value(r.trim(), indent + 2)?
            };
            if k.trim() == "<<" {
                self.merge(&mut m, v)?
            } else {
                m.insert(strs::intern(&key), v);
            }
        }
        Ok(Value::Object(Rc::new(m)))
    }
    fn child(&mut self, parent: usize) -> Result<Value, DataError> {
        let Some(l) = self.lines.get(self.pos) else {
            return Ok(Value::Null);
        };
        if l.indent < parent {
            return Ok(Value::Null);
        }
        if l.indent == parent && !(l.text == "-" || l.text.starts_with("- ")) {
            return Ok(Value::Null);
        }
        self.node(l.indent)
    }

    /// Self-streaming counterpart of `node`: guarantees that, by the time
    /// it returns `Ok`, every event needed to represent "the value at
    /// `path`" has already been pushed onto `queue` - a single leaf event
    /// for a scalar, or (via `seq`/`map`) one event per child plus the
    /// container's own close/`Empty*` event. This means a later sibling
    /// (or nested value) failing to parse doesn't discard events for
    /// whatever already-resolved data preceded it.
    fn node_eager(
        &mut self,
        indent: usize,
        path: &mut Vec<PathItem>,
        queue: &mut VecDeque<StreamItem>,
    ) -> Result<Value, DataError> {
        let Some(l) = self.lines.get(self.pos).cloned() else {
            super::document::events(Value::Null, path, queue);
            return Ok(Value::Null);
        };
        if l.indent < indent {
            super::document::events(Value::Null, path, queue);
            return Ok(Value::Null);
        }
        if l.indent != indent {
            return Err(self.err("inconsistent YAML indentation"));
        }
        if l.text == "-" || l.text.starts_with("- ") {
            self.seq(indent, path, queue)
        } else if split_colon(&l.text).is_some() {
            self.map(indent, path, queue)
        } else {
            self.pos += 1;
            let v = self.text_value(&l.text, indent)?;
            super::document::events(v.clone(), path, queue);
            Ok(v)
        }
    }
    /// Each item is streamed (recursively, via `child_eager`/`map`, or
    /// explicitly for a plain scalar) as soon as it's resolved, so items
    /// before a later parse failure are preserved in `queue`. `path` is a
    /// scratch stack shared with every caller up the recursion - pushed
    /// before descending into each item and popped right after, so no
    /// intermediate path ever needs to be cloned just to pass it down.
    fn seq(
        &mut self,
        indent: usize,
        path: &mut Vec<PathItem>,
        queue: &mut VecDeque<StreamItem>,
    ) -> Result<Value, DataError> {
        let mut a = Vec::new();
        let mut i: isize = 0;
        while let Some(l) = self.lines.get(self.pos).cloned() {
            if l.indent != indent || !(l.text == "-" || l.text.starts_with("- ")) {
                break;
            }
            self.pos += 1;
            let s = l.text[1..].trim();
            path.push(PathItem::new_idx(i));
            let v = if s.is_empty() {
                self.child_eager(indent, path, queue)?
            } else if split_colon(s).is_some() {
                self.map_first(indent + 2, s, path, queue)?
            } else {
                let v = self.text_value(s, indent + 2)?;
                super::document::events(v.clone(), path, queue);
                v
            };
            path.pop();
            a.push(v);
            i += 1;
        }
        if a.is_empty() {
            queue.push_back(StreamItem {
                path: path.clone(),
                value: Some(Value::empty_array()),
            });
        } else {
            path.push(PathItem::new_idx(i - 1));
            queue.push_back(StreamItem {
                path: path.clone(),
                value: None,
            });
            path.pop();
        }
        Ok(Value::Array(Rc::new(a)))
    }
    fn map(
        &mut self,
        indent: usize,
        path: &mut Vec<PathItem>,
        queue: &mut VecDeque<StreamItem>,
    ) -> Result<Value, DataError> {
        self.map_first(indent, "", path, queue)
    }
    /// Each key - explicit or merged via `<<` - is streamed as soon as it's
    /// resolved, in document order. This is simpler than (and, for a `<<`
    /// followed later by an explicit key of the same name, technically
    /// looser than) the real YAML rule that an explicit key always wins
    /// regardless of position: here, whichever assignment is processed
    /// last simply re-emits at the same path, exactly like any other
    /// duplicate key - the final `Value` this returns is unaffected either
    /// way, since `merge` never overwrites a key `m` already has.
    fn map_first(
        &mut self,
        indent: usize,
        first: &str,
        path: &mut Vec<PathItem>,
        queue: &mut VecDeque<StreamItem>,
    ) -> Result<Value, DataError> {
        let mut m = IndexMap::new();
        let mut first = (!first.is_empty()).then(|| first.to_owned());
        loop {
            let l = if let Some(s) = first.take() {
                Line {
                    no: 0,
                    indent,
                    text: s,
                }
            } else {
                let Some(x) = self.lines.get(self.pos).cloned() else {
                    break;
                };
                if x.indent != indent || split_colon(&x.text).is_none() {
                    break;
                }
                self.pos += 1;
                x
            };
            let (k, r) = split_colon(&l.text).unwrap();
            let key = self.key(k)?;
            if k.trim() == "<<" {
                // `<<` is a YAML-only directive, not a real data key, so its
                // own value is resolved atomically (never appears as an
                // event itself) - only the keys it contributes to `m` are
                // streamed, immediately, as they're merged in.
                let v = if r.trim().is_empty() {
                    self.child(indent)?
                } else {
                    self.text_value(r.trim(), indent + 2)?
                };
                self.merge_eager(&mut m, v, path, queue)?;
                continue;
            }
            let key_sym = strs::intern(&key);
            path.push(PathItem::new_key(key_sym));
            let v = if r.trim().is_empty() {
                self.child_eager(indent, path, queue)?
            } else {
                let v = self.text_value(r.trim(), indent + 2)?;
                super::document::events(v.clone(), path, queue);
                v
            };
            path.pop();
            m.insert(key_sym, v);
        }
        if let Some(&last_key) = m.keys().last() {
            path.push(PathItem::new_key(last_key));
            queue.push_back(StreamItem {
                path: path.clone(),
                value: None,
            });
            path.pop();
        } else {
            queue.push_back(StreamItem {
                path: path.clone(),
                value: Some(Value::empty_object()),
            });
        }
        Ok(Value::Object(Rc::new(m)))
    }
    fn child_eager(
        &mut self,
        parent: usize,
        path: &mut Vec<PathItem>,
        queue: &mut VecDeque<StreamItem>,
    ) -> Result<Value, DataError> {
        let Some(l) = self.lines.get(self.pos) else {
            super::document::events(Value::Null, path, queue);
            return Ok(Value::Null);
        };
        if l.indent < parent {
            super::document::events(Value::Null, path, queue);
            return Ok(Value::Null);
        }
        if l.indent == parent && !(l.text == "-" || l.text.starts_with("- ")) {
            super::document::events(Value::Null, path, queue);
            return Ok(Value::Null);
        }
        let indent = l.indent;
        self.node_eager(indent, path, queue)
    }
    fn key(&mut self, s: &str) -> Result<String, DataError> {
        match self.text_value(s.trim(), 0)? {
            Value::String(x) => Ok(x.to_string()),
            Value::Decimal(x) => Ok(x.to_string()),
            Value::Bool(x) => Ok(x.to_string()),
            Value::Null => Ok("null".into()),
            _ => Err(self.err("YAML mapping keys must be scalar")),
        }
    }
    /// Resolves a `<<` value (an alias, or a list of them) to the mapping(s)
    /// it merges in.
    fn merge_sources(&self, v: Value) -> Result<Vec<Value>, DataError> {
        Ok(match v {
            Value::Array(x) => x.as_ref().clone(),
            Value::String(s) if s.starts_with('*') => vec![
                self.anchors
                    .get(&s[1..])
                    .cloned()
                    .ok_or_else(|| self.err("unknown YAML alias"))?,
            ],
            x => vec![x],
        })
    }
    /// Atomic (non-streaming) merge, used only for the inline `{<<: *a}`
    /// flow-mapping form, which never has a real path to stream into.
    fn merge(&self, m: &mut IndexMap<isize, Value>, v: Value) -> Result<(), DataError> {
        for x in self.merge_sources(v)? {
            let Value::Object(x) = x else {
                return Err(self.err(format!(
                    "YAML merge requires a mapping, got {}",
                    x.type_name()
                )));
            };
            for (k, v) in x.iter() {
                m.entry(*k).or_insert_with(|| v.clone());
            }
        }
        Ok(())
    }
    /// Same merge semantics as `merge`, but streams each newly-merged key
    /// immediately (see `map_first`'s doc comment for the trade-off this
    /// makes versus strict YAML precedence).
    fn merge_eager(
        &self,
        m: &mut IndexMap<isize, Value>,
        v: Value,
        path: &mut Vec<PathItem>,
        queue: &mut VecDeque<StreamItem>,
    ) -> Result<(), DataError> {
        for x in self.merge_sources(v)? {
            let Value::Object(x) = x else {
                return Err(self.err(format!(
                    "YAML merge requires a mapping, got {}",
                    x.type_name()
                )));
            };
            for (k, v) in x.iter() {
                if !m.contains_key(k) {
                    m.insert(*k, v.clone());
                    path.push(PathItem::new_key(*k));
                    super::document::events(v.clone(), path, queue);
                    path.pop();
                }
            }
        }
        Ok(())
    }
    fn text_value(&mut self, raw: &str, indent: usize) -> Result<Value, DataError> {
        let mut s = raw.trim();
        let mut anchor = None;
        let mut tag = None;
        loop {
            if let Some(x) = s.strip_prefix('&') {
                let n = x.split_whitespace().next().unwrap_or("");
                if n.is_empty() {
                    return Err(self.err("invalid YAML anchor"));
                }
                anchor = Some(n.to_owned());
                s = x[n.len()..].trim();
                continue;
            }
            if let Some(x) = s.strip_prefix('!') {
                let n = x.split_whitespace().next().unwrap_or("");
                tag = Some(n.trim_start_matches('!').to_owned());
                s = x[n.len()..].trim();
                continue;
            }
            break;
        }
        let mut v = if let Some(n) = s.strip_prefix('*') {
            self.anchors
                .get(n.trim())
                .cloned()
                .ok_or_else(|| self.err("unknown YAML alias"))?
        } else if s.is_empty() {
            self.child(indent.saturating_sub(2))?
        } else if s.starts_with('|') || s.starts_with('>') {
            self.block(indent.saturating_sub(2), s.starts_with('>'))?
        } else if s == "-" || s.starts_with("- ") {
            self.inline_seq(s)?
        } else if s.starts_with('[') || s.starts_with('{') {
            let mut f = Flow {
                s,
                pos: 0,
                reader: self,
            };
            f.value()?
        } else {
            scalar(s, false)
        };
        if tag.as_deref() == Some("str") {
            v = Value::String(s.to_owned().into());
        }
        if let Some(n) = anchor {
            self.anchors.insert(n, v.clone());
        }
        Ok(v)
    }
    fn block(&mut self, parent: usize, fold: bool) -> Result<Value, DataError> {
        let mut a = Vec::new();
        while let Some(x) = self.lines.get(self.pos).cloned() {
            if x.indent <= parent {
                break;
            }
            self.pos += 1;
            a.push(x.text)
        }
        let mut s = if fold { a.join(" ") } else { a.join("\n") };
        if !fold {
            s.push('\n')
        }
        Ok(Value::String(s.into()))
    }
    fn inline_seq(&mut self, s: &str) -> Result<Value, DataError> {
        let x = s[1..].trim();
        let v = if x.is_empty() {
            Value::Null
        } else if x == "-" || x.starts_with("- ") {
            self.inline_seq(x)?
        } else {
            self.text_value(x, 0)?
        };
        Ok(Value::Array(Rc::new(vec![v])))
    }
}

struct Flow<'a, 'r, 'f> {
    s: &'a str,
    pos: usize,
    reader: &'r mut Reader<'f>,
}
impl<'a, 'r, 'f> Flow<'a, 'r, 'f> {
    fn ws(&mut self) {
        while self.s[self.pos..]
            .chars()
            .next()
            .is_some_and(|c| c.is_whitespace())
        {
            self.pos += 1
        }
    }
    fn value(&mut self) -> Result<Value, DataError> {
        self.ws();
        match self.s.as_bytes().get(self.pos) {
            Some(b'[') => self.array(),
            Some(b'{') => self.object(),
            Some(b'\'' | b'"') => self.quote(),
            _ => {
                let st = self.pos;
                while let Some(c) = self.s[self.pos..].chars().next() {
                    if ",]}:".contains(c) {
                        break;
                    }
                    self.pos += c.len_utf8()
                }
                let t = self.s[st..self.pos].trim();
                if let Some(n) = t.strip_prefix('*') {
                    self.reader
                        .anchors
                        .get(n)
                        .cloned()
                        .ok_or_else(|| self.reader.err("unknown YAML alias"))
                } else if let Some(n) = t.strip_prefix("!!str ") {
                    Ok(Value::String(n.to_string().into()))
                } else {
                    Ok(scalar(t, true))
                }
            }
        }
    }
    fn quote(&mut self) -> Result<Value, DataError> {
        let q = self.s.as_bytes()[self.pos] as char;
        self.pos += 1;
        let mut o = String::new();
        while self.pos < self.s.len() {
            let c = self.s.as_bytes()[self.pos] as char;
            self.pos += 1;
            if c == q {
                return Ok(Value::String(o.into()));
            }
            if c == '\\' && q == '"' {
                if self.pos >= self.s.len() {
                    break;
                }
                let e = self.s.as_bytes()[self.pos] as char;
                self.pos += 1;
                o.push(match e {
                    'n' => '\n',
                    'r' => '\r',
                    't' => '\t',
                    '"' => '"',
                    '\\' => '\\',
                    x => x,
                })
            } else {
                o.push(c)
            }
        }
        Err(self.reader.err("unterminated YAML string"))
    }
    fn array(&mut self) -> Result<Value, DataError> {
        self.pos += 1;
        let mut a = Vec::new();
        loop {
            self.ws();
            if self.s.as_bytes().get(self.pos) == Some(&b']') {
                self.pos += 1;
                break;
            }
            a.push(self.value()?);
            self.ws();
            if self.s.as_bytes().get(self.pos) == Some(&b',') {
                self.pos += 1
            } else if self.s.as_bytes().get(self.pos) != Some(&b']') {
                return Err(self.reader.err("expected ',' or ']'"));
            }
        }
        Ok(Value::Array(Rc::new(a)))
    }
    fn object(&mut self) -> Result<Value, DataError> {
        self.pos += 1;
        let mut m = IndexMap::new();
        loop {
            self.ws();
            if self.s.as_bytes().get(self.pos) == Some(&b'}') {
                self.pos += 1;
                break;
            }
            let merge_key = self.s[self.pos..].starts_with("<<");
            let k = self.value()?;
            self.ws();
            if self.s.as_bytes().get(self.pos) != Some(&b':') {
                return Err(self.reader.err("expected ':'"));
            }
            self.pos += 1;
            let v = self.value()?;
            let k = match k {
                Value::String(x) => x.to_string(),
                Value::Decimal(x) => x.to_string(),
                Value::Bool(x) => x.to_string(),
                Value::Null => "null".into(),
                _ => return Err(self.reader.err("flow key must be scalar")),
            };
            if merge_key && k == "<<" {
                self.reader.merge(&mut m, v)?
            } else {
                m.insert(strs::intern(&k), v);
            }
            self.ws();
            if self.s.as_bytes().get(self.pos) == Some(&b',') {
                self.pos += 1
            } else if self.s.as_bytes().get(self.pos) != Some(&b'}') {
                return Err(self.reader.err("expected ',' or '}'"));
            }
        }
        Ok(Value::Object(Rc::new(m)))
    }
}

fn scalar(s: &str, _flow: bool) -> Value {
    let s = s.trim();
    if s.starts_with('"') || s.starts_with('\'') {
        return Value::String(s.trim_matches(['"', '\'']).to_string().into());
    }
    if matches!(s, "" | "~" | "null" | "Null" | "NULL") {
        Value::Null
    } else if matches!(s, "true" | "True" | "TRUE") {
        Value::Bool(true)
    } else if matches!(s, "false" | "False" | "FALSE") {
        Value::Bool(false)
    } else if (s.starts_with("0x")
        || s.starts_with("0o")
        || (!s.contains('.') && !s.contains('e') && !s.contains('E')))
        && let Some(n) = integer(s)
    {
        // `Decimal::parse` (unlike the old integer-only check this used to
        // be) also accepts fractional/exponent text, so plain-decimal
        // literals need to explicitly stay integer-shaped here - otherwise
        // a YAML float like `1.25e+3` would round-trip as a
        // literal-preserving `Decimal` (`1.25E+3`) instead of the plain
        // `Value::Float` (`1250.0`) it's always been. Hex/octal literals
        // are exempt from that guard since `e`/`E` can be an ordinary hex
        // digit there (`0xE5`), not an exponent marker.
        Value::decimal(n)
    } else if s.contains('.') || s.contains('e') || s.contains('E') {
        match Decimal::parse(s) {
            Some(n) => Value::decimal(n),
            None => Value::String(s.to_string().into()),
        }
    } else {
        Value::String(s.to_string().into())
    }
}
fn integer(s: &str) -> Option<Decimal> {
    // Hex/octal are YAML 1.2 core-schema sugar with no fractional form, so
    // parsing via `i64` (rather than `Decimal`, which only understands
    // decimal-literal grammar) is both sufficient and simpler.
    if let Some(x) = s.strip_prefix("0x") {
        return i64::from_str_radix(x, 16).ok().map(Decimal::from_i64);
    }
    if let Some(x) = s.strip_prefix("0o") {
        return i64::from_str_radix(x, 8).ok().map(Decimal::from_i64);
    }
    Decimal::parse(s)
}
fn document_marker(s: &str, m: &str) -> bool {
    s.strip_prefix(m)
        .is_some_and(|x| x.is_empty() || x.starts_with([' ', '\t', '\r', '\n']))
}
fn is_content(s: &str) -> bool {
    let x = s.trim();
    !x.is_empty()
        && !x.starts_with('#')
        && !x.starts_with('%')
        && !document_marker(x, "...")
        && !document_marker(x, "---")
}
fn strip_comment(s: &str) -> String {
    let mut q = None;
    for (i, c) in s.char_indices() {
        if c == '\'' || c == '"' {
            if q == Some(c) {
                q = None
            } else if q.is_none() {
                q = Some(c)
            }
        } else if c == '#' && q.is_none() && (i == 0 || s.as_bytes()[i - 1].is_ascii_whitespace()) {
            return s[..i].trim_end().into();
        }
    }
    s.into()
}
fn split_colon(s: &str) -> Option<(&str, &str)> {
    let mut q = None;
    let mut d = 0;
    for (i, c) in s.char_indices() {
        if c == '\'' || c == '"' {
            if q == Some(c) {
                q = None
            } else if q.is_none() {
                q = Some(c)
            }
        } else if q.is_none() {
            if "[{".contains(c) {
                d += 1
            } else if "]}".contains(c) {
                d -= 1
            } else if c == ':'
                && d == 0
                && (i + 1 == s.len() || s[i + 1..].starts_with([' ', '\t']))
            {
                return Some((&s[..i], &s[i + 1..]));
            }
        }
    }
    None
}

pub struct YamlSerializer {
    output: Output,
    options: render::Options,
    emitted: bool,
}
impl YamlSerializer {
    pub fn new(output: Output, options: render::Options) -> Self {
        Self {
            output,
            options,
            emitted: false,
        }
    }
}
fn block(v: &Value) -> bool {
    matches!(v,Value::Array(a)if !a.is_empty()) || matches!(v,Value::Object(m)if !m.is_empty())
}
/// Whether `s` can be written as a bare (unquoted) YAML plain scalar and
/// still read back as the identical string through this module's reader -
/// i.e. it doesn't collide with any syntax this reader gives special
/// meaning to (seq/map markers, flow indicators, comments, anchors/tags,
/// block scalars) or with a value `scalar()` would parse as a different type.
fn yaml_plain_safe(s: &str) -> bool {
    if s.is_empty() || s.trim() != s || s == "---" || s == "..." {
        return false;
    }
    if s == "-" || s.starts_with("- ") {
        return false;
    }
    if s.starts_with([
        '?', ':', ',', '[', ']', '{', '}', '#', '&', '*', '!', '|', '>', '\'', '"', '%', '@', '`',
    ]) {
        return false;
    }
    if s.chars().any(|c| c.is_control()) {
        return false;
    }
    if s.contains(": ") || s.ends_with(':') || s.contains(" #") {
        return false;
    }
    matches!(scalar(s, false), Value::String(x) if &*x == s)
}
fn write_string_themed(o: &mut String, s: &str, x: &render::Options, idx: render::ThemeIdx) {
    if yaml_plain_safe(s) {
        super::json::push_styled(o, x, idx, s);
    } else {
        let mut buf = String::new();
        super::json::push_json_string(&mut buf, s, x.out.ascii_only);
        super::json::push_styled(o, x, idx, &buf);
    }
}
fn write_scalar(o: &mut String, v: &Value, x: &render::Options) {
    if let Value::String(s) = v {
        write_string_themed(o, s, x, render::ThemeIdx::String);
        return;
    }
    super::json::write_value(o, v, x, 0, super::json::KeywordPreset::YAML);
}
fn write_block(o: &mut String, v: &Value, x: &render::Options, d: usize) {
    let i = if x.out.indent.is_empty() || !x.out.indent.chars().all(|c| c == ' ') {
        "  "
    } else {
        x.out.indent
    };
    let p = i.repeat(d);
    match v {
        Value::Array(a) if !a.is_empty() => {
            for z in a.iter() {
                o.push_str(&p);
                o.push('-');
                if block(z) {
                    o.push(' ');
                    let mut child = String::new();
                    write_block(&mut child, z, x, d + 1);
                    let child_indent = i.repeat(d + 1);
                    o.push_str(child.strip_prefix(&child_indent).unwrap_or(&child));
                } else {
                    o.push(' ');
                    write_scalar(o, z, x);
                    o.push('\n')
                }
            }
        }
        Value::Object(_) => {
            for (k, z) in v.object_iter(x.out.sort_keys).unwrap() {
                o.push_str(&p);
                write_string_themed(
                    o,
                    strs::resolve(*k).unwrap(),
                    x,
                    render::ThemeIdx::ObjectKey,
                );
                o.push(':');
                if block(z) {
                    o.push('\n');
                    write_block(o, z, x, d + 1)
                } else {
                    o.push(' ');
                    write_scalar(o, z, x);
                    o.push('\n')
                }
            }
        }
        _ => {
            o.push_str(&p);
            write_scalar(o, v, x);
            o.push('\n');
        }
    }
}
impl Serializer for YamlSerializer {
    fn put(&mut self, v: Value) -> Result<(), DataError> {
        if self.options.out.quiet {
            return Ok(());
        }
        let mut o = String::new();
        if self.emitted {
            if self.options.out.doc_end.is_none() {
                o.push_str("---\n")
            } else {
                o.push_str("\n---\n")
            }
        } else if let Some(s) = self.options.out.doc_begin {
            o.push_str(s)
        }
        self.emitted = true;
        if self.options.out.compact_level == render::CompactLevel::Pretty {
            write_block(&mut o, &v, &self.options, 0);
            o.pop();
        } else {
            super::json::write_value(
                &mut o,
                &v,
                &self.options,
                0,
                super::json::KeywordPreset::YAML,
            )
        }
        o.push_str(self.options.out.doc_end.unwrap_or("\n"));
        self.output
            .write_all(o.as_bytes())
            .map_err(DataError::IOError)?;
        if self.options.out.doc_end_flush {
            self.output.flush().map_err(DataError::IOError)?
        }
        Ok(())
    }
}
