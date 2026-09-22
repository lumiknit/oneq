//! Line-by-line splitter

use std::io::{BufRead, BufReader, Read, Write};

use crate::data::stream::StreamItem;
use crate::data::value::Value;
use crate::data::{DataError, ParseOutput};
use crate::io::{Input, Output};
use crate::render;

const BUF_SIZE: usize = 8192;

/// `RawParser` is takes string and split it into each line.
/// Each value is a line with only its final LF removed; CR is data.
pub struct RawParser<'a> {
    input: BufReader<Input<'a>>,
}

impl<'a> RawParser<'a> {
    #[must_use]
    pub fn new(input: Input<'a>) -> Self {
        RawParser {
            input: BufReader::with_capacity(BUF_SIZE, input),
        }
    }
}

impl RawParser<'_> {
    fn read_line(&mut self) -> Result<Option<String>, DataError> {
        let mut line = String::new();
        self.input
            .read_line(&mut line)
            .map_err(DataError::IOError)?;
        if line.is_empty() {
            return Ok(None);
        }

        if line.ends_with('\n') {
            line.pop();
        }
        Ok(Some(line))
    }
}

impl Iterator for RawParser<'_> {
    type Item = ParseOutput;

    fn next(&mut self) -> Option<Self::Item> {
        match self.read_line() {
            Ok(Some(s)) => Some(Ok(StreamItem::Value(Value::String(s.into())))),
            Ok(None) => None,
            Err(e) => Some(Err(e)),
        }
    }
}

impl super::Parser for RawParser<'_> {}

/// `RawSlurpParser` is takes string and split it into each line.
/// Each value is a line with only its final LF removed; CR is data.
pub struct RawSlurpParser<'a> {
    input: BufReader<Input<'a>>,
    done: bool,
}

impl<'a> RawSlurpParser<'a> {
    #[must_use]
    pub fn new(input: Input<'a>) -> Self {
        RawSlurpParser {
            input: BufReader::with_capacity(BUF_SIZE, input),
            done: false,
        }
    }
}

impl Iterator for RawSlurpParser<'_> {
    type Item = ParseOutput;

    fn next(&mut self) -> Option<Self::Item> {
        if self.done {
            None
        } else {
            let mut content = String::new();
            self.done = true;
            if let Err(error) = self.input.read_to_string(&mut content) {
                return Some(Err(DataError::IOError(error)));
            }
            Some(Ok(StreamItem::Value(Value::String(content.into()))))
        }
    }
}

impl super::Parser for RawSlurpParser<'_> {}

/// `RawSerializer` implements jq's `-r`/`--raw-output`/`--join-output`: a
/// string prints unquoted, but every other type still prints as JSON
/// (honoring the usual compact/indent/color/sort-keys render options) rather
/// than erroring.
pub struct RawSerializer {
    output: Output,
    render_options: render::Options,
}

impl RawSerializer {
    #[must_use]
    pub const fn new(output: Output, render_options: render::Options) -> Self {
        Self {
            output,
            render_options,
        }
    }
    pub(crate) fn finish(self) -> std::io::Result<()> {
        self.output.finish()
    }
}

impl super::Serializer for RawSerializer {
    fn put(&mut self, value: Value) -> Result<(), DataError> {
        if let Some(s) = self.render_options.out.doc_begin {
            write!(self.output, "{s}").map_err(DataError::IOError)?;
        }

        match &value {
            Value::String(s) => write!(self.output, "{s}").map_err(DataError::IOError)?,
            other => {
                let mut out = String::new();
                super::json::write_value(
                    &mut out,
                    other,
                    &self.render_options,
                    0,
                    super::json::KeywordPreset::JSON,
                );
                self.output
                    .write_all(out.as_bytes())
                    .map_err(DataError::IOError)?;
            }
        }

        if let Some(s) = self.render_options.out.doc_end {
            write!(self.output, "{s}").map_err(DataError::IOError)?;
        } else {
            writeln!(self.output).map_err(DataError::IOError)?;
        }

        Ok(())
    }
}
