use crate::{
    data::{
        DataError, Value, ValueBuilder,
        builder::StreamOption,
        json::{JsonParser, JsonParserOptions},
    },
    io::Input,
};

/// Returns strict JSON parser with input
#[must_use]
pub fn json_parser(i: Input<'_>) -> JsonParser<'_> {
    JsonParser::new(i, JsonParserOptions::default())
}

/// Like [`parse_json_str`], but keeps the parser's structured error
/// (with line/column) instead of collapsing it to a string, so callers
/// that need jq-compatible "Invalid numeric literal at line L, column C"
/// wording can build it themselves.
pub fn parse_json_str_detailed(s: &str) -> Result<Value, DataError> {
    let input = Input::new_str(s);
    let parser = JsonParser::new(input, JsonParserOptions::default());
    let mut values = ValueBuilder::new(parser, StreamOption::default());
    let value = values.next().ok_or(DataError::EOF)??;
    match values.next() {
        None => Ok(value),
        Some(Err(e)) => Err(e),
        Some(Ok(_)) => Err(DataError::ParseError {
            path: String::new(),
            line: 0,
            col: 0,
            message: "expected exactly one JSON value".into(),
        }),
    }
}

pub fn parse_json_str(s: &str) -> Result<Value, String> {
    parse_json_str_detailed(s).map_err(|e| e.to_string())
}
