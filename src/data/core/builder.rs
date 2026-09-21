//! Assembles the flat `StreamItem` stream a `Parser` produces into the
//! per-document `Value`(s) a `ValueCollector` receives - one output value
//! per top-level input document, shaped per `--stream` vs. the default.

use std::rc::Rc;

use crate::data::DataError;
use crate::data::stream::{PathItem, StreamItem};
use crate::data::traits::ParseOutput;
use crate::data::value::Value;
use crate::strs;

/// Mirrors jq `--stream`'s own rule for when a leaf/close event finishes a
/// top-level document:
/// - a leaf (`has_value`) closes its document iff its path is empty - it
///   *is* the whole (scalar, or empty array/object) root value.
/// - a close event (no value) reports the path of the container's *last
///   child*, one level deeper than the container itself - so a close event
///   finishes the root container iff that path has exactly one segment.
///
/// All data parsers normalize to this encoding. YAML waits for its next
/// document marker or EOF and TOML waits for EOF before emitting events,
/// so source-specific table/container endings cannot finish a document early.
fn is_document_boundary(path: &[PathItem], has_value: bool) -> bool {
    if has_value {
        path.is_empty()
    } else {
        path.len() <= 1
    }
}

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

/// Turns the event stream a `Parser` emits into the `Value`(s) a
/// `ValueCollector` (and from there, a `Serializer`) should receive - one
/// output value per top-level input document.
///
/// - `Stream`: every event is independent - each one becomes its own value
///   right away, `[path, value]` for a leaf or `[path]` for a close event
///   (jq `--stream` shape).
/// - `Default`: folds leaf events into a single `Value` via
///   `Value::set_path` (close events carry no data, so they only ever
///   affect document-boundary tracking), yielding the folded value once a
///   document-boundary event arrives.
///
/// This is deliberately unaware of `--slurp` - jq's own `--stream --slurp`
/// shows the two are independent: `--stream` picks *this* per-document
/// shape, `--slurp` (a `ValueCollector`) then decides whether each of those
/// documents is passed on immediately or collected into one array.
pub enum BuilderKind {
    StreamError, // Same as stream, but pass parse error
    Stream,
    Default { current: Value },
}

/// Iterator adapter: pulls `ParseOutput`s from an inner `Parser` iterator
/// `P` and yields the resulting per-document `Value`s. Any `Err` coming out
/// of `P` is passed straight through as this iterator's next (and last)
/// item.
pub struct ValueBuilder<P> {
    parser: P,
    kind: BuilderKind,
}

impl<P> ValueBuilder<P>
where
    P: Iterator<Item = ParseOutput>,
{
    pub fn new(parser: P, opt: StreamOption) -> Self {
        Self {
            parser,
            kind: match opt {
                StreamOption::Default => BuilderKind::Default {
                    current: Value::Null,
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
        loop {
            match &mut self.kind {
                BuilderKind::StreamError => {
                    return {
                        let mut items: Vec<Value> = vec![];
                        match self.parser.next()? {
                            Ok(StreamItem { path, value }) => {
                                items.push(path_to_value(&path));
                                if let Some(value) = value {
                                    items.push(value);
                                }
                            }
                            Err(e) => {
                                items.push(Value::String(e.to_string().into()));
                            }
                        };
                        Some(Ok(Value::Array(Rc::new(items))))
                    };
                }

                BuilderKind::Stream => {
                    return {
                        match self.parser.next()? {
                            Ok(StreamItem { path, value }) => {
                                let mut items = vec![path_to_value(&path)];
                                if let Some(value) = value {
                                    items.push(value);
                                }
                                Some(Ok(Value::Array(Rc::new(items))))
                            }
                            Err(e) => Some(Err(e)),
                        }
                    };
                }

                BuilderKind::Default { current } => {
                    let event = match self.parser.next()? {
                        Ok(event) => event,
                        Err(e) => return Some(Err(e)),
                    };
                    let StreamItem { path, value } = event;
                    let boundary = is_document_boundary(&path, value.is_some());
                    if let Some(value) = value
                        && let Err(e) = current.set_path_mut(&path, value)
                    {
                        return Some(Err(e));
                    }
                    if boundary {
                        return Some(Ok(std::mem::replace(current, Value::Null)));
                    }
                }
            }
        }
    }
}
