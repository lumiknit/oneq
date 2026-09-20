//! jq source as a data format. A document is one JSON-compatible Pair tree.
use crate::{
    data::{ArrayIndex, DataError, ParseOutput, Parser, PathItem, Serializer, StreamItem, Value},
    io::{Input, Output},
    jq::parser::{
        pair::{Pair, pairs_to_value},
        parse_pairs,
    },
    render,
};
use std::{
    collections::VecDeque,
    io::{Read, Write},
};

pub struct JqParser {
    values: VecDeque<ParseOutput>,
}
impl JqParser {
    pub fn new(mut input: Input) -> Self {
        let path = input.filename().to_string();
        let mut source = String::new();
        let value = input
            .read_to_string(&mut source)
            .map_err(DataError::IOError)
            .and_then(|_| {
                parse_pairs(path.clone(), &source)
                    .map_err(|message| DataError::ParseError {
                        path,
                        line: 0,
                        col: 0,
                        message,
                    })
                    .map(|(files, root)| pairs_to_value(&[root], &files))
            });
        let mut values = VecDeque::new();
        match value {
            Ok(value) => flatten(value, Vec::new(), &mut values),
            Err(e) => values.push_back(Err(e)),
        };
        Self { values }
    }
}
impl Iterator for JqParser {
    type Item = ParseOutput;
    fn next(&mut self) -> Option<Self::Item> {
        self.values.pop_front()
    }
}
impl Parser for JqParser {}

pub struct JqSerializer {
    output: Output,
    options: render::Options,
}
impl JqSerializer {
    pub fn new(output: Output, options: render::Options) -> Self {
        Self { output, options }
    }
}
impl Serializer for JqSerializer {
    fn put(&mut self, value: Value) -> Result<(), DataError> {
        let Value::Array(pairs) = value else {
            return Err(DataError::UnableToSerializeValueType {
                value_type: value.type_name(),
            });
        };
        let files = crate::jq::parser::pair::FileSet::new();
        let mut text = String::new();
        for value in pairs.iter() {
            let pair = Pair::from_value(value).map_err(|message| DataError::ParseError {
                path: "<jq>".into(),
                line: 0,
                col: 0,
                message,
            })?;
            let rendered = crate::jq::parser::printer::Printer::with_render(&files, &self.options)
                .print(&pair)
                .map_err(|message| DataError::ParseError {
                    path: "<jq>".into(),
                    line: 0,
                    col: 0,
                    message,
                })?;
            if !text.is_empty() {
                text.push('\n');
            }
            text.push_str(&rendered);
        }
        self.output
            .write_all(text.as_bytes())
            .map_err(DataError::IOError)
    }
}

fn flatten(value: Value, path: Vec<PathItem>, out: &mut VecDeque<ParseOutput>) {
    match value {
        Value::Null => out.push_back(Ok(StreamItem {
            path,
            value: Some(Value::Null),
        })),
        Value::Bool(x) => out.push_back(Ok(StreamItem {
            path,
            value: Some(Value::Bool(x)),
        })),
        Value::Decimal(x) => out.push_back(Ok(StreamItem {
            path,
            value: Some(Value::Decimal(x)),
        })),
        Value::Float(x) => out.push_back(Ok(StreamItem {
            path,
            value: Some(Value::Float(x)),
        })),
        Value::String(x) => out.push_back(Ok(StreamItem {
            path,
            value: Some(Value::String(x)),
        })),
        Value::Array(xs) => {
            if xs.is_empty() {
                out.push_back(Ok(StreamItem {
                    path,
                    value: Some(Value::empty_array()),
                }))
            } else {
                for (i, x) in xs.iter().cloned().enumerate() {
                    let mut p = path.clone();
                    p.push(PathItem::new_idx(i as ArrayIndex));
                    flatten(x, p, out)
                }
                let mut last = path.clone();
                last.push(PathItem::new_idx((xs.len() - 1) as ArrayIndex));
                out.push_back(Ok(StreamItem {
                    path: last,
                    value: None,
                }));
            }
        }
        Value::Object(xs) => {
            if xs.is_empty() {
                out.push_back(Ok(StreamItem {
                    path,
                    value: Some(Value::empty_object()),
                }))
            } else {
                for (k, x) in xs.iter() {
                    let mut p = path.clone();
                    p.push(PathItem::new_key(*k));
                    flatten(x.clone(), p, out)
                }
                let (key, _) = xs.last().unwrap();
                let mut last = path.clone();
                last.push(PathItem::new_key(*key));
                out.push_back(Ok(StreamItem {
                    path: last,
                    value: None,
                }));
            }
        }
    }
}
