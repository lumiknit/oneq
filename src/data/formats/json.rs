//! JSON parser & serializer.
//!
//! The serializer only ever emits strict JSON - `render::Options` controls
//! its layout (compact level, indent, ascii-only escaping, sort-keys,
//! doc_begin/doc_end/flush, colors), but never its grammar.
//!
//! The parser is intentionally more permissive, along two independent
//! axes:
//! - `LooseLevel` - how forgiving the grammar itself is: `Strict` is plain
//!   JSON; `Json5` adds `//`/`/* */` comments, trailing commas, single- or
//!   unquoted-key object keys, and single-quoted strings; `Loose` on top of
//!   that treats commas between elements as decoration rather than
//!   structure - any number of them (including zero) between array/object
//!   members is accepted.
//! - `KeywordPreset` - which literal words parse as `null`/`true`/`false`,
//!   independent of how strict the grammar around them is (e.g. `PyLit`
//!   recognizes `None`/`True`/`False`). `undefined` is *not* part of a
//!   preset - like JSON5 itself, it's available once `loose_level` is at
//!   least `Json5`, and it isn't a value at all: the key (or array slot)
//!   it's assigned to is simply dropped.

use std::collections::VecDeque;
use std::io::Write;

use indexmap::IndexMap;

use crate::data::core::decimal::Decimal;
use crate::data::core::escape;
use crate::data::stream::{PathItem, StreamItem};
use crate::data::traits::CharReader;
use crate::data::value::{ObjectKey, Value};
use crate::data::{ArrayIndex, DataError, ParseOutput, Parser, Serializer};
use crate::io::{Input, Output};
use crate::render::{self, ThemeIdx};
use crate::strs;

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default)]
#[repr(u8)]
pub enum LooseLevel {
    /// Plain JSON.
    #[default]
    Strict = 0,

    /// JSON5-ish: `//`/`/* */` comments, trailing commas, single-quoted
    /// strings, and bare/single-quoted object keys. Also unlocks the
    /// `undefined` literal (see module docs).
    Json5 = 1,

    /// Everything `Json5` allows, plus commas between array/object members
    /// become fully decorative: any run of zero or more of them between
    /// members is accepted, so a missing comma or a doubled comma is not
    /// an error.
    Loose = 2,
}

/// Which literal words denote `null`/`true`/`false` - independent of
/// `LooseLevel`, which governs the surrounding grammar instead.
#[derive(Clone, Copy)]
pub struct KeywordPreset {
    null_words: &'static [&'static str],
    true_words: &'static [&'static str],
    false_words: &'static [&'static str],
    /// Unsigned `Infinity`/`NaN` spellings - a leading `-` (or, once
    /// `LooseLevel::Json5` unlocks unary `+` on numbers, `+`) is handled
    /// separately in `parse_value`, the same way it is for ordinary numbers.
    infinity_words: &'static [&'static str],
    nan_words: &'static [&'static str],
    emit_nonfinite_words: bool,
    case_insensitive: bool,
    /// Separate from `case_insensitive`: real jq's own JSON parser matches
    /// `Infinity`/`NaN` case-insensitively (`nan`, `INFINITY`, ... all
    /// parse) even in otherwise-strict mode, while still requiring exact
    /// case for `null`/`true`/`false` - verified against real jq.
    numeric_case_insensitive: bool,
}

enum Keyword {
    Null,
    True,
    False,
    Infinity,
    NaN,
}

impl KeywordPreset {
    pub const JSON: Self = Self {
        null_words: &["null"],
        true_words: &["true"],
        false_words: &["false"],
        // Not part of the JSON grammar, but jq's own JSON parser always
        // accepts these (input like `[Infinity,-Infinity,NaN,-NaN]`), so
        // strict mode matches jq rather than plain RFC 8259 - and, unlike
        // `null`/`true`/`false`, jq matches them case-insensitively even
        // here (`nan`, `INFINITY`, `iNfInItY`, ... all parse in real jq).
        infinity_words: &["Infinity"],
        nan_words: &["NaN"],
        case_insensitive: false,
        numeric_case_insensitive: true,
        emit_nonfinite_words: false,
    };

    pub const JSON5: Self = Self {
        infinity_words: &["Infinity"],
        nan_words: &["NaN"],
        emit_nonfinite_words: true,
        ..Self::JSON
    };

    pub const YAML: Self = Self {
        infinity_words: &[".inf"],
        nan_words: &[".nan"],
        emit_nonfinite_words: true,
        ..Self::JSON
    };

    pub const PY_LIT: Self = Self {
        null_words: &["None"],
        true_words: &["True"],
        false_words: &["False"],
        infinity_words: &["inf", "Infinity"],
        nan_words: &["nan", "NaN"],
        case_insensitive: false,
        numeric_case_insensitive: false,
        emit_nonfinite_words: true,
    };

    /// Common synonyms people actually type, matched case-insensitively.
    pub const LOOSE: Self = Self {
        null_words: &["null", "nil", "none"],
        true_words: &["true", "yes", "on"],
        false_words: &["false", "no", "off"],
        infinity_words: &["Infinity", "inf"],
        nan_words: &["NaN", "nan"],
        case_insensitive: true,
        numeric_case_insensitive: true,
        emit_nonfinite_words: true,
    };

    fn word_matches(word: &str, list: &[&str], case_insensitive: bool) -> bool {
        if case_insensitive {
            list.iter().any(|w| w.eq_ignore_ascii_case(word))
        } else {
            list.contains(&word)
        }
    }

    pub fn null_word(&self) -> &'static str {
        self.null_words[0]
    }

    pub fn true_word(&self) -> &'static str {
        self.true_words[0]
    }

    pub fn false_word(&self) -> &'static str {
        self.false_words[0]
    }

    pub fn infinity_word(&self) -> Option<&'static str> {
        self.infinity_words.first().copied()
    }

    pub fn nan_word(&self) -> Option<&'static str> {
        self.nan_words.first().copied()
    }

    fn classify(&self, word: &str) -> Option<Keyword> {
        if Self::word_matches(word, self.null_words, self.case_insensitive) {
            Some(Keyword::Null)
        } else if Self::word_matches(word, self.true_words, self.case_insensitive) {
            Some(Keyword::True)
        } else if Self::word_matches(word, self.false_words, self.case_insensitive) {
            Some(Keyword::False)
        } else if Self::word_matches(word, self.infinity_words, self.numeric_case_insensitive) {
            Some(Keyword::Infinity)
        } else if Self::word_matches(word, self.nan_words, self.numeric_case_insensitive) {
            Some(Keyword::NaN)
        } else {
            None
        }
    }
}

impl Default for KeywordPreset {
    fn default() -> Self {
        Self::JSON
    }
}

#[derive(Clone, Copy, Default)]
pub struct JsonParserOptions {
    pub loose_level: LooseLevel,
    pub keywords: KeywordPreset,
}

fn is_ident_start(c: char) -> bool {
    c == '_' || c == '$' || c.is_alphabetic()
}

fn is_ident_part(c: char) -> bool {
    c == '_' || c == '$' || c.is_alphanumeric()
}

/// Pull-style JSON reader: `next_event` hands back one `StreamItem` at a
/// time, but under the hood a whole top-level document is parsed in one go
/// into `queue` as soon as it's needed - `CharReader` still only reads as
/// many bytes as parsing that document actually touches, so a value on a
/// still-open stream is available as soon as it's complete.
pub struct JsonParser<'a> {
    reader: CharReader<'a>,
    filename: String,
    options: JsonParserOptions,
    queue: VecDeque<StreamItem>,
    eof: bool,
    line: u32,
    col: u32,
}

impl<'a> JsonParser<'a> {
    pub fn new(input: Input<'a>, options: JsonParserOptions) -> Self {
        let filename = input.filename().to_string();
        Self {
            reader: CharReader::new(input),
            filename,
            options,
            queue: VecDeque::new(),
            eof: false,
            line: 1,
            col: 1,
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

    fn skip_ws(&mut self) -> Result<(), DataError> {
        loop {
            match self.peek() {
                Some(c) if c.is_whitespace() || c == '\u{FEFF}' => {
                    self.bump();
                }
                Some('/')
                    if self.options.loose_level >= LooseLevel::Json5
                        && self.peek2() == Some('/') =>
                {
                    self.bump();
                    self.bump();
                    while let Some(c) = self.peek() {
                        if c == '\n' {
                            break;
                        }
                        self.bump();
                    }
                }
                Some('/')
                    if self.options.loose_level >= LooseLevel::Json5
                        && self.peek2() == Some('*') =>
                {
                    self.bump();
                    self.bump();
                    loop {
                        match self.peek() {
                            None => return Err(self.err("unterminated block comment")),
                            Some('*') => {
                                self.bump();
                                if self.peek() == Some('/') {
                                    self.bump();
                                    break;
                                }
                            }
                            Some(_) => {
                                self.bump();
                            }
                        }
                    }
                }
                _ => break,
            }
        }
        Ok(())
    }

    fn read_word(&mut self) -> String {
        let mut s = String::new();
        while let Some(c) = self.peek() {
            if is_ident_part(c) {
                s.push(c);
                self.bump();
            } else {
                break;
            }
        }
        s
    }

    fn parse_key(&mut self) -> Result<String, DataError> {
        match self.peek() {
            Some('"') => self.parse_string('"'),
            Some('\'') if self.options.loose_level >= LooseLevel::Json5 => self.parse_string('\''),
            Some(c) if self.options.loose_level >= LooseLevel::Json5 && is_ident_start(c) => {
                Ok(self.read_word())
            }
            Some(c) => Err(self.err(format!("unexpected character '{}' in object key", c))),
            None => Err(self.err("unexpected end of input in object key")),
        }
    }

    fn parse_hex4(&mut self) -> Result<u32, DataError> {
        let mut v: u32 = 0;
        for _ in 0..4 {
            let c = match self.bump() {
                Some(c) => c,
                None => return Err(self.err("unterminated unicode escape")),
            };
            let d = c
                .to_digit(16)
                .ok_or_else(|| self.err("invalid hex digit in unicode escape"))?;
            v = v * 16 + d;
        }
        Ok(v)
    }

    fn parse_string(&mut self, quote: char) -> Result<String, DataError> {
        self.bump(); // opening quote
        let mut s = String::new();
        loop {
            match self.bump() {
                None => return Err(self.err("unterminated string literal")),
                Some(c) if c == quote => break,
                Some('\\') => match self.bump() {
                    Some('"') => s.push('"'),
                    Some('\'') => s.push('\''),
                    Some('\\') => s.push('\\'),
                    Some('/') => s.push('/'),
                    Some('b') => s.push('\u{08}'),
                    Some('f') => s.push('\u{0c}'),
                    Some('n') => s.push('\n'),
                    Some('r') => s.push('\r'),
                    Some('t') => s.push('\t'),
                    // JSON5 line continuation: backslash-newline is elided.
                    Some('\n') if self.options.loose_level >= LooseLevel::Json5 => {}
                    Some('u') => {
                        let hi = self.parse_hex4()?;
                        if (0xD800..=0xDBFF).contains(&hi) {
                            if self.peek() == Some('\\') && self.peek2() == Some('u') {
                                self.bump();
                                self.bump();
                                let lo = self.parse_hex4()?;
                                if !(0xDC00..=0xDFFF).contains(&lo) {
                                    return Err(self.err("invalid low surrogate"));
                                }
                                let cp = 0x10000 + ((hi - 0xD800) << 10) + (lo - 0xDC00);
                                match char::from_u32(cp) {
                                    Some(ch) => s.push(ch),
                                    None => return Err(self.err("invalid unicode escape")),
                                }
                            } else {
                                return Err(self.err("unpaired surrogate in unicode escape"));
                            }
                        } else {
                            match char::from_u32(hi) {
                                Some(ch) => s.push(ch),
                                None => return Err(self.err("invalid unicode escape")),
                            }
                        }
                    }
                    Some(c) => return Err(self.err(format!("invalid escape '\\{}'", c))),
                    None => return Err(self.err("unterminated escape sequence")),
                },
                Some(c) => s.push(c),
            }
        }
        Ok(s)
    }

    fn parse_number(&mut self) -> Result<Value, DataError> {
        let mut s = String::new();
        let mut has_digits = false;

        if self.peek() == Some('-')
            || (self.options.loose_level >= LooseLevel::Json5 && self.peek() == Some('+'))
        {
            s.push(self.bump().unwrap());
        }

        while let Some(c) = self.peek() {
            if c.is_ascii_digit() {
                s.push(c);
                self.bump();
                has_digits = true;
            } else {
                break;
            }
        }

        if self.peek() == Some('.') {
            s.push('.');
            self.bump();
            while let Some(c) = self.peek() {
                if c.is_ascii_digit() {
                    s.push(c);
                    self.bump();
                    has_digits = true;
                } else {
                    break;
                }
            }
        }

        if matches!(self.peek(), Some('e') | Some('E')) {
            s.push(self.bump().unwrap());
            if matches!(self.peek(), Some('+') | Some('-')) {
                s.push(self.bump().unwrap());
            }
            let mut exp_digits = false;
            while let Some(c) = self.peek() {
                if c.is_ascii_digit() {
                    s.push(c);
                    self.bump();
                    exp_digits = true;
                } else {
                    break;
                }
            }
            if !exp_digits {
                return Err(self.err("invalid number: missing exponent digits"));
            }
        }

        if !has_digits {
            return Err(self.err("invalid number literal"));
        }

        // Both integer- and float-shaped literals become `Decimal` - jq's
        // decNum mode preserves a fractional/exponent literal's exact
        // spelling (e.g. `1.50`) just like it does for big integers, only
        // losing precision once arithmetic touches the value (see
        // `Value::Decimal`'s docs).
        let decimal = Decimal::parse(&s).ok_or_else(|| self.err("invalid number literal"))?;
        Ok(Value::decimal(decimal))
    }

    /// Parses one value at `path`, queuing whatever `StreamItem`(s) it
    /// produces. Returns `false` iff the value was a bare `undefined` -
    /// nothing was queued, and the caller (array/object loop) should treat
    /// this slot as if it never existed.
    fn parse_value(&mut self, path: Vec<PathItem>) -> Result<bool, DataError> {
        self.skip_ws()?;
        match self.peek() {
            Some('{') => {
                self.parse_object(path)?;
                Ok(true)
            }
            Some('[') => {
                self.parse_array(path)?;
                Ok(true)
            }
            Some('"') => {
                let s = self.parse_string('"')?;
                self.queue.push_back(StreamItem {
                    path,
                    value: Some(Value::String(s.into())),
                });
                Ok(true)
            }
            Some('\'') if self.options.loose_level >= LooseLevel::Json5 => {
                let s = self.parse_string('\'')?;
                self.queue.push_back(StreamItem {
                    path,
                    value: Some(Value::String(s.into())),
                });
                Ok(true)
            }
            // A signed `Infinity`/`NaN` spelling: same idea as an ordinary
            // signed number, but the sign is followed by a keyword instead
            // of digits. `-` is always allowed (jq itself always accepts
            // `-Infinity`/`-NaN`); a leading `+` only unlocks alongside
            // `+123`-style numbers, at `Json5` and up.
            Some(c @ ('-' | '+'))
                if (c == '-' || self.options.loose_level >= LooseLevel::Json5)
                    && self.peek2().is_some_and(is_ident_start) =>
            {
                self.bump();
                let word = self.read_word();
                let pv = match self.options.keywords.classify(&word) {
                    Some(Keyword::Infinity) => Value::Float(if c == '-' {
                        f64::NEG_INFINITY
                    } else {
                        f64::INFINITY
                    }),
                    Some(Keyword::NaN) => Value::Float(f64::NAN),
                    _ => return Err(self.err(format!("unexpected token '{c}{word}'"))),
                };
                self.queue.push_back(StreamItem {
                    path,
                    value: Some(pv),
                });
                Ok(true)
            }
            Some(c) if c == '-' || c == '+' || c == '.' || c.is_ascii_digit() => {
                let n = self.parse_number()?;
                self.queue.push_back(StreamItem {
                    path,
                    value: Some(n),
                });
                Ok(true)
            }
            Some(c) if is_ident_start(c) => {
                let word = self.read_word();
                if let Some(kw) = self.options.keywords.classify(&word) {
                    let pv = match kw {
                        Keyword::Null => Value::Null,
                        Keyword::True => Value::Bool(true),
                        Keyword::False => Value::Bool(false),
                        Keyword::Infinity => Value::Float(f64::INFINITY),
                        Keyword::NaN => Value::Float(f64::NAN),
                    };
                    self.queue.push_back(StreamItem {
                        path,
                        value: Some(pv),
                    });
                    Ok(true)
                } else if self.options.loose_level >= LooseLevel::Json5 && word == "undefined" {
                    Ok(false)
                } else {
                    Err(self.err(format!("unexpected token '{}'", word)))
                }
            }
            Some(c) => Err(self.err(format!("unexpected character '{}'", c))),
            None => Err(self.err("unexpected end of input")),
        }
    }

    /// Handles the run of separators between array/object members: a
    /// single required `,` in `Strict`, an optionally-trailing one in
    /// `Json5`, and any number (including zero) of them in `Loose`.
    /// Returns `true` iff the closer (`]`/`}`) was consumed.
    fn consume_separator(&mut self, closer: char) -> Result<bool, DataError> {
        self.skip_ws()?;
        match self.peek() {
            Some(c) if c == closer => {
                self.bump();
                Ok(true)
            }
            Some(',') => {
                self.bump();
                if self.options.loose_level >= LooseLevel::Loose {
                    loop {
                        self.skip_ws()?;
                        if self.peek() == Some(',') {
                            self.bump();
                        } else {
                            break;
                        }
                    }
                }
                self.skip_ws()?;
                if self.options.loose_level >= LooseLevel::Json5 && self.peek() == Some(closer) {
                    self.bump();
                    return Ok(true);
                }
                Ok(false)
            }
            _ => {
                if self.options.loose_level >= LooseLevel::Loose {
                    // Missing comma - `Loose` treats it as decoration
                    // anyway, so just carry on to the next member.
                    Ok(false)
                } else {
                    Err(self.err(format!("expected ',' or '{}'", closer)))
                }
            }
        }
    }

    fn parse_array(&mut self, path: Vec<PathItem>) -> Result<(), DataError> {
        self.bump(); // '['
        self.skip_ws()?;
        if self.peek() == Some(']') {
            self.bump();
            self.queue.push_back(StreamItem {
                path,
                value: Some(Value::empty_array()),
            });
            return Ok(());
        }

        let mut idx: ArrayIndex = 0;
        // Only the *last* child's index is needed for the close event, so
        // remember that rather than keeping a clone of its whole path.
        let mut last_child_idx = None;
        loop {
            self.skip_ws()?;
            let mut child_path = path.clone();
            child_path.push(PathItem::new_idx(idx));
            if self.parse_value(child_path)? {
                last_child_idx = Some(idx);
                idx += 1;
            }
            if self.consume_separator(']')? {
                break;
            }
        }

        self.queue.push_back(match last_child_idx {
            Some(li) => StreamItem {
                path: {
                    let mut lp = path;
                    lp.push(PathItem::new_idx(li));
                    lp
                },
                value: None,
            },
            // every element was `undefined` - the array is effectively empty.
            None => StreamItem {
                path,
                value: Some(Value::empty_array()),
            },
        });
        Ok(())
    }

    fn parse_object(&mut self, path: Vec<PathItem>) -> Result<(), DataError> {
        self.bump(); // '{'
        self.skip_ws()?;
        if self.peek() == Some('}') {
            self.bump();
            self.queue.push_back(StreamItem {
                path,
                value: Some(Value::empty_object()),
            });
            return Ok(());
        }

        // As in `parse_array`: the close event only needs the last child's
        // key, not a second copy of its path.
        let mut last_child_key = None;
        loop {
            self.skip_ws()?;
            let key = self.parse_key()?;
            self.skip_ws()?;
            match self.peek() {
                Some(':') => {
                    self.bump();
                }
                _ => return Err(self.err("Objects must consist of key:value pairs")),
            }
            self.skip_ws()?;

            let item = PathItem::new_key_str(&key);
            let mut child_path = path.clone();
            child_path.push(item);
            if self.parse_value(child_path)? {
                last_child_key = Some(item);
            }
            if self.consume_separator('}')? {
                break;
            }
        }

        self.queue.push_back(match last_child_key {
            Some(lk) => StreamItem {
                path: {
                    let mut lp = path;
                    lp.push(lk);
                    lp
                },
                value: None,
            },
            // every member's value was `undefined` - the object is effectively empty.
            None => StreamItem {
                path,
                value: Some(Value::empty_object()),
            },
        });
        Ok(())
    }
}

impl<'a> Iterator for JsonParser<'a> {
    type Item = ParseOutput;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            if let Some(item) = self.queue.pop_front() {
                return Some(Ok(item));
            }
            if self.eof {
                return None;
            }
            if let Err(e) = self.skip_ws() {
                self.eof = true;
                return Some(Err(e));
            }
            if self.peek().is_none() {
                self.eof = true;
                return None;
            }
            // A bare top-level `undefined` queues nothing - loop back
            // around to parse the next document rather than reporting a
            // (nonexistent) event for it.
            if let Err(e) = self.parse_value(Vec::new()) {
                self.eof = true;
                return Some(Err(e));
            }
        }
    }
}

impl<'a> Parser for JsonParser<'a> {}

pub(super) fn push_styled(out: &mut String, opts: &render::Options, idx: ThemeIdx, s: &str) {
    if let Some(style) = opts.style_ansi(idx) {
        out.push_str(&style.to_string());
        out.push_str(s);
        if let Some(reset) = opts.style_reset() {
            out.push_str(reset);
        }
    } else {
        out.push_str(s);
    }
}

pub(super) fn push_json_string(out: &mut String, s: &str, ascii_only: bool) {
    out.push('"');
    // `String: fmt::Write` never fails.
    if ascii_only {
        escape::escape_string_json_ascii(s, '"', out)
    } else {
        escape::escape_string_json(s, '"', out)
    }
    .unwrap();
    out.push('"');
}

fn push_indent(out: &mut String, opts: &render::Options, depth: usize) {
    for _ in 0..depth {
        out.push_str(opts.out.indent);
    }
}

/// Formats a finite `f64` the way jq's own printer does: shortest
/// round-trip digits, then plain decimal unless that would be unreasonably
/// sparse - the exact rule (`decpt <= -4 || decpt > digit_count + 15`,
/// where `decpt` is the position of the decimal point relative to the
/// first significant digit) is David Gay's classic `g_fmt.c` heuristic,
/// which jq's own `jvp_dtoa_fmt.c` is directly based on. Verified against
/// real jq across the plain/scientific boundary in both directions (e.g.
/// `1e15` prints plain, `1e16` doesn't, `13911860366432382.0` - 17
/// significant digits - prints plain even though its exponent is 16).
fn format_float(f: f64) -> String {
    if f == 0.0 {
        return if f.is_sign_negative() {
            "-0".to_string()
        } else {
            "0".to_string()
        };
    }
    let neg = f.is_sign_negative();
    // Rust's `{:e}` already produces the shortest digit string that
    // round-trips back to `f`, normalized to one digit before the point.
    let rendered = format!("{:e}", f.abs());
    let (mantissa, exp_str) = rendered.split_once('e').expect("float exponential form");
    let exp: i64 = exp_str.parse().expect("float exponent");
    let digits: String = mantissa.chars().filter(|c| *c != '.').collect();
    let precision = digits.len() as i64;
    let decpt = exp + 1;
    let mut out = String::new();
    if neg {
        out.push('-');
    }
    if decpt <= -4 || decpt > precision + 15 {
        out.push_str(&digits[..1]);
        if digits.len() > 1 {
            out.push('.');
            out.push_str(&digits[1..]);
        }
        if exp >= 0 {
            out.push_str(&format!("e+{:02}", exp));
        } else {
            out.push_str(&format!("e-{:02}", -exp));
        }
    } else if decpt <= 0 {
        out.push_str("0.");
        out.push_str(&"0".repeat((-decpt) as usize));
        out.push_str(&digits);
    } else if decpt as usize >= digits.len() {
        out.push_str(&digits);
        out.push_str(&"0".repeat(decpt as usize - digits.len()));
    } else {
        out.push_str(&digits[..decpt as usize]);
        out.push('.');
        out.push_str(&digits[decpt as usize..]);
    }
    out
}

pub(super) fn write_value(
    out: &mut String,
    value: &Value,
    opts: &render::Options,
    depth: usize,
    keywords: KeywordPreset,
) {
    match value {
        Value::Null => push_styled(out, opts, ThemeIdx::Null, keywords.null_word()),
        Value::Bool(true) => push_styled(out, opts, ThemeIdx::True, keywords.true_word()),
        Value::Bool(false) => push_styled(out, opts, ThemeIdx::False, keywords.false_word()),
        Value::Decimal(n) => push_styled(out, opts, ThemeIdx::Number, &n.to_string()),
        Value::Float(f) => {
            if f.is_nan() {
                if keywords.emit_nonfinite_words {
                    if let Some(word) = keywords.nan_word() {
                        push_styled(out, opts, ThemeIdx::Number, word);
                        return;
                    }
                }
                push_styled(out, opts, ThemeIdx::Null, "null");
                return;
            }
            if !f.is_finite() {
                if keywords.emit_nonfinite_words {
                    if let Some(word) = keywords.infinity_word() {
                        let word = if f.is_sign_negative() {
                            format!("-{word}")
                        } else {
                            word.to_string()
                        };
                        push_styled(out, opts, ThemeIdx::Number, &word);
                        return;
                    }
                }
            }
            let v = if f.is_finite() {
                *f
            } else if f.is_sign_positive() {
                f64::MAX
            } else {
                f64::MIN
            };
            let formatted = format_float(v);
            push_styled(out, opts, ThemeIdx::Number, &formatted);
        }
        Value::String(s) => {
            let mut buf = String::new();
            push_json_string(&mut buf, s, opts.out.ascii_only);
            push_styled(out, opts, ThemeIdx::String, &buf);
        }
        Value::Array(items) => write_array(out, items, opts, depth, keywords),
        Value::Object(map) => write_object(out, map, opts, depth, keywords),
    }
}

fn write_array(
    out: &mut String,
    items: &[Value],
    opts: &render::Options,
    depth: usize,
    keywords: KeywordPreset,
) {
    if items.is_empty() {
        push_styled(out, opts, ThemeIdx::Array, "[]");
        return;
    }

    let pretty = opts.out.compact_level == render::CompactLevel::Pretty;
    push_styled(out, opts, ThemeIdx::Array, "[");
    for (i, item) in items.iter().enumerate() {
        if i > 0 {
            push_styled(out, opts, ThemeIdx::Array, ",");
            if !pretty && opts.out.compact_level != render::CompactLevel::Compact {
                out.push(' ');
            }
        }
        if pretty {
            out.push('\n');
            push_indent(out, opts, depth + 1);
        }
        write_value(out, item, opts, depth + 1, keywords);
    }
    if pretty {
        out.push('\n');
        push_indent(out, opts, depth);
    }
    push_styled(out, opts, ThemeIdx::Array, "]");
}

fn write_object(
    out: &mut String,
    map: &IndexMap<ObjectKey, Value>,
    opts: &render::Options,
    depth: usize,
    keywords: KeywordPreset,
) {
    if map.is_empty() {
        push_styled(out, opts, ThemeIdx::Object, "{}");
        return;
    }

    let entries = Value::object_entries(map, opts.out.sort_keys);

    let pretty = opts.out.compact_level == render::CompactLevel::Pretty;
    push_styled(out, opts, ThemeIdx::Object, "{");
    for (i, (key, value)) in entries.into_iter().enumerate() {
        if i > 0 {
            push_styled(out, opts, ThemeIdx::Object, ",");
            if !pretty && opts.out.compact_level != render::CompactLevel::Compact {
                out.push(' ');
            }
        }
        if pretty {
            out.push('\n');
            push_indent(out, opts, depth + 1);
        }

        let key_str = strs::resolve(*key).unwrap_or("");
        let mut kbuf = String::new();
        push_json_string(&mut kbuf, key_str, opts.out.ascii_only);
        push_styled(out, opts, ThemeIdx::ObjectKey, &kbuf);

        push_styled(out, opts, ThemeIdx::Object, ":");
        if opts.out.compact_level != render::CompactLevel::Compact {
            out.push(' ');
        }
        write_value(out, value, opts, depth + 1, keywords);
    }
    if pretty {
        out.push('\n');
        push_indent(out, opts, depth);
    }
    push_styled(out, opts, ThemeIdx::Object, "}");
}

#[derive(Clone, Copy, Default)]
pub struct JsonSerializerOptions {
    pub keywords: KeywordPreset,
}

pub struct JsonSerializer {
    output: Output,
    render_options: render::Options,
    options: JsonSerializerOptions,
}

impl JsonSerializer {
    pub fn new(output: Output, render_options: render::Options) -> Self {
        Self::with_options(output, render_options, JsonSerializerOptions::default())
    }

    pub fn with_options(
        output: Output,
        render_options: render::Options,
        options: JsonSerializerOptions,
    ) -> Self {
        Self {
            output,
            render_options,
            options,
        }
    }
}

impl Serializer for JsonSerializer {
    fn put(&mut self, value: Value) -> Result<(), DataError> {
        if self.render_options.out.quiet {
            return Ok(());
        }

        let mut out = String::new();
        if let Some(s) = self.render_options.out.doc_begin {
            out.push_str(s);
        }
        write_value(
            &mut out,
            &value,
            &self.render_options,
            0,
            self.options.keywords,
        );
        match self.render_options.out.doc_end {
            Some(s) => out.push_str(s),
            None => out.push('\n'),
        }

        self.output
            .write_all(out.as_bytes())
            .map_err(DataError::IOError)?;
        if self.render_options.out.doc_end_flush {
            self.output.flush().map_err(DataError::IOError)?;
        }
        Ok(())
    }
}
