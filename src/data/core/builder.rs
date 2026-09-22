//! Assembles the relative `StreamItem` stream a `Parser` produces into the
//! per-document `Value`(s) a `ValueCollector` receives - one output value
//! per top-level input document, shaped per `--stream` vs. the default.

use std::rc::Rc;

use crate::data::DataError;
use crate::data::stream::{PathItem, StreamItem};
use crate::data::traits::ParseOutput;
use crate::data::value::Value;
use crate::strs;

/// jq `--stream`-style `[path, value]` pair, where each path segment is a
/// JSON string (object key) or number (array index).
fn path_to_value(path: &[PathItem]) -> Value {
    let items: Vec<Value> = path
        .iter()
        .map(|item| {
            let (i, is_key) = item.unpack();
            if is_key {
                let key = strs::resolve(i).expect("interned path key must resolve");
                Value::String(key.to_string().into())
            } else {
                Value::int(i as i64)
            }
        })
        .collect();
    Value::Array(Rc::new(items))
}

#[derive(Default, Clone, Copy)]
pub enum StreamOption {
    #[default]
    Default,
    Stream,
    StreamError,
}

/// Default mode owns a stack of open containers, deepest last. Stream modes
/// materialize jq-style paths without assembling the document.
pub enum BuilderKind {
    StreamError, // Same as stream, but pass parse error
    Stream,
    Default { containers: Vec<Value> },
}

/// Iterator adapter: pulls `ParseOutput`s from an inner `Parser` iterator
/// `P` and maintains one reusable path. Default mode yields complete root
/// documents; stream modes yield each value/close event. Parser errors are
/// forwarded, or encoded as values in `StreamError` mode.
pub struct ValueBuilder<P> {
    parser: P,
    kind: BuilderKind,
    path: Vec<PathItem>,
    last_item: Option<PathItem>,
}

impl<P> ValueBuilder<P>
where
    P: Iterator<Item = ParseOutput>,
{
    pub fn new(parser: P, opt: StreamOption) -> Self {
        Self {
            parser,
            path: Vec::with_capacity(8),
            last_item: None,
            kind: match opt {
                StreamOption::Default => BuilderKind::Default {
                    // Scalars and empty containers need no stack allocation.
                    containers: Vec::new(),
                },
                StreamOption::Stream => BuilderKind::Stream,
                StreamOption::StreamError => BuilderKind::StreamError,
            },
        }
    }
}

impl<P> Iterator for ValueBuilder<P>
where
    P: Iterator<Item = ParseOutput>,
{
    type Item = Result<Value, DataError>;

    fn next(&mut self) -> Option<Self::Item> {
        match &mut self.kind {
            BuilderKind::Default { containers } => {
                Self::next_default(&mut self.parser, &mut self.path, containers)
            }
            kind @ (BuilderKind::Stream | BuilderKind::StreamError) => Self::next_stream(
                &mut self.parser,
                &mut self.path,
                &mut self.last_item,
                matches!(kind, BuilderKind::StreamError),
            ),
        }
    }
}

impl<P: Iterator<Item = ParseOutput>> ValueBuilder<P> {
    fn next_default(
        parser: &mut P,
        path: &mut Vec<PathItem>,
        containers: &mut Vec<Value>,
    ) -> Option<Result<Value, DataError>> {
        loop {
            let value = match parser.next()? {
                Err(error) => return Some(Err(error)),
                Ok(StreamItem::Push(item)) => {
                    // A second Push before Value/Close enters another
                    // container. Siblings reuse the existing top frame.
                    if containers.len() == path.len() {
                        let child = if let Some(parent) = containers.last_mut() {
                            let key = *path.last().expect("nested container has a path");
                            match parent.child_mut(key) {
                                Ok(slot) => std::mem::take(slot),
                                Err(error) => return Some(Err(error)),
                            }
                        } else {
                            Value::Null
                        };
                        // Move, never clone, the existing subtree. Leaving
                        // Null in its parent preserves object insertion order
                        // and allows repeated keys/table paths to be extended.
                        containers.push(child);
                    }
                    path.push(item);
                    continue;
                }
                Ok(StreamItem::Value(value)) => value,
                Ok(StreamItem::Close) => containers.pop().expect("Close has an open container"),
            };
            let Some(item) = path.pop() else {
                return Some(Ok(value));
            };
            let parent = containers.last_mut().expect("child has an open parent");
            match parent.child_mut(item) {
                Ok(slot) => *slot = value,
                Err(error) => return Some(Err(error)),
            }
        }
    }

    fn next_stream(
        parser: &mut P,
        path: &mut Vec<PathItem>,
        last_item: &mut Option<PathItem>,
        stream_errors: bool,
    ) -> Option<Result<Value, DataError>> {
        loop {
            let items = match parser.next()? {
                Err(error) => {
                    return Some(if stream_errors {
                        Ok(Value::Array(Rc::new(vec![Value::String(
                            error.to_string().into(),
                        )])))
                    } else {
                        Err(error)
                    });
                }
                Ok(StreamItem::Push(item)) => {
                    path.push(item);
                    continue;
                }
                Ok(StreamItem::Value(value)) => vec![path_to_value(path), value],
                Ok(StreamItem::Close) => {
                    let child = last_item.expect("Close must follow a completed child");
                    path.push(child);
                    let output_path = path_to_value(path);
                    path.pop();
                    vec![output_path]
                }
            };
            *last_item = path.pop();
            return Some(Ok(Value::Array(Rc::new(items))));
        }
    }
}
