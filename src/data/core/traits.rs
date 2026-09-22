use std::io::Read;

use crate::{
    data::{DataError, stream::StreamItem, value::Value},
    io::Input,
};

pub type ParseOutput = Result<StreamItem, DataError>;

/// Parser emitting relative Push/Value/Close traversal events.
pub trait Parser
where
    Self: Iterator<Item = ParseOutput>,
{
}

pub trait Serializer {
    /// Pass value to the output. The implementer may format the value based on their
    /// style and write it to the output. The value is consumed.
    fn put(&mut self, value: Value) -> Result<(), DataError>;
}

pub struct CharReader<'a> {
    reader: Input<'a>,
    /// Set once `reader` has reported EOF (or an error - see `read_error`),
    /// so `ensure` stops trying to pull more bytes.
    eof: bool,
    /// A read error from `reader`, surfaced the next time parsing actually
    /// needs a byte past what's already buffered (rather than eagerly).
    read_error: Option<String>,
    /// Bytes read but not yet decoded into `chars` - only ever a partial
    /// UTF-8 sequence left over at a chunk boundary.
    pending_bytes: Vec<u8>,
    /// Decoded characters not yet consumed, plus whatever lookahead
    /// `ensure` has pulled in. Consumed characters are dropped once
    /// `pos` passes `COMPACT_THRESHOLD`, so a large input doesn't keep
    /// the whole decoded document (4 bytes per character) resident.
    chars: Vec<char>,
    pos: usize,
}

/// How many consumed characters may pile up in `chars` before `ensure`
/// drops them. Parsing only ever looks a character or two ahead, so the
/// retained tail is tiny and the periodic `drain` is O(1) amortized.
const COMPACT_THRESHOLD: usize = 4096;

impl<'a> CharReader<'a> {
    /// No bytes are read here - `new` can't fail and doesn't block; each
    /// `peek`/`bump` call pulls only as many bytes as it needs, so a
    /// value on a still-open stream (e.g. a line typed into stdin) is
    /// available as soon as it's complete.
    #[must_use]
    pub const fn new(reader: Input<'a>) -> Self {
        CharReader {
            reader,
            eof: false,
            read_error: None,
            pending_bytes: Vec::new(),
            chars: Vec::new(),
            pos: 0,
        }
    }

    /// Takes the pending read error, if any - call this when hitting an
    /// unexpected EOF, so the real cause (rather than a generic "end of
    /// input") gets surfaced.
    pub const fn take_read_error(&mut self) -> Option<String> {
        self.read_error.take()
    }

    /// Makes sure at least `self.pos + want` characters are available in
    /// `self.chars` (or that EOF/an error has been hit trying). Reads in
    /// small chunks so a blocking source only blocks for data that's
    /// actually needed, not the whole input.
    fn ensure(&mut self, want: usize) {
        if self.pos >= COMPACT_THRESHOLD {
            self.chars.drain(..self.pos);
            self.pos = 0;
        }
        while !self.eof && self.chars.len() < self.pos + want {
            let mut chunk = [0u8; 256];
            match self.reader.read(&mut chunk) {
                Ok(0) => self.eof = true,
                Ok(n) => self.decode(&chunk[..n]),
                Err(e) => {
                    self.read_error = Some(e.to_string());
                    self.eof = true;
                }
            }
        }
    }

    /// Appends `bytes` to any leftover partial UTF-8 sequence and decodes
    /// as many complete characters as possible into `self.chars`.
    fn decode(&mut self, bytes: &[u8]) {
        self.pending_bytes.extend_from_slice(bytes);
        loop {
            match std::str::from_utf8(&self.pending_bytes) {
                Ok(s) => {
                    self.chars.extend(s.chars());
                    self.pending_bytes.clear();
                    return;
                }
                Err(e) => {
                    let valid_len = e.valid_up_to();
                    if valid_len > 0 {
                        // Safe: `valid_up_to()` guarantees this prefix is valid UTF-8.
                        let valid = std::str::from_utf8(&self.pending_bytes[..valid_len]).unwrap();
                        self.chars.extend(valid.chars());
                    }
                    match e.error_len() {
                        // Sequence cut off at the chunk boundary - keep the
                        // tail and wait for the rest on the next read.
                        None => {
                            self.pending_bytes.drain(..valid_len);
                            return;
                        }
                        // Genuinely invalid UTF-8 - drop the offending
                        // bytes and keep decoding the rest of the chunk.
                        Some(bad_len) => {
                            self.pending_bytes.drain(..valid_len + bad_len);
                        }
                    }
                }
            }
        }
    }

    /// The character `offset` positions past the current one, without
    /// consuming anything - `peek_at(0)` is `peek()`.
    pub fn peek_at(&mut self, offset: usize) -> Option<char> {
        self.ensure(offset + 1);
        self.chars.get(self.pos + offset).copied()
    }

    pub fn peek(&mut self) -> Option<char> {
        self.peek_at(0)
    }

    pub fn peek2(&mut self) -> Option<char> {
        self.peek_at(1)
    }

    pub fn bump(&mut self) -> Option<char> {
        let c = self.peek();
        if c.is_some() {
            self.pos += 1;
        }
        c
    }

    /// Character-by-character, so a mismatch on the very first character
    /// doesn't force blocking for lookahead as far out as `s` is long.
    pub fn starts_with(&mut self, s: &str) -> bool {
        for (i, expected) in s.chars().enumerate() {
            self.ensure(i + 1);
            if self.chars.get(self.pos + i) != Some(&expected) {
                return false;
            }
        }
        true
    }
}
