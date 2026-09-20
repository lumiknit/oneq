//! Decides what to do with the sequence of per-document `Value`s a
//! `ValueBuilder` produces - independent of how each of those documents was
//! shaped (default vs. `--stream`). This is what `--slurp` actually
//! controls: jq's own `--stream --slurp` combination shows the two are
//! orthogonal, collecting the `--stream` `[path, value]`/`[path]` tuples
//! into one array rather than changing their shape.

use std::rc::Rc;

use crate::data::DataError;
use crate::data::value::Value;

enum CollectorKind {
    Passthrough,
    Slurp { items: Vec<Value>, done: bool },
}

/// Iterator adapter: pulls documents from an inner `ValueBuilder` iterator
/// `B` and either passes each one straight through (`Passthrough`), or
/// collects all of them into a single array yielded once `B` is exhausted
/// (`Slurp`). Any `Err` coming out of `B` is passed straight through.
pub struct ValueCollector<B> {
    builder: B,
    kind: CollectorKind,
}

impl<B> ValueCollector<B>
where
    B: Iterator<Item = Result<Value, DataError>>,
{
    pub fn new(builder: B, slurp: bool) -> Self {
        if slurp {
            Self::new_slurp(builder)
        } else {
            Self::new_passthrough(builder)
        }
    }

    pub fn new_passthrough(builder: B) -> Self {
        Self {
            builder,
            kind: CollectorKind::Passthrough,
        }
    }

    pub fn new_slurp(builder: B) -> Self {
        Self {
            builder,
            kind: CollectorKind::Slurp {
                items: Vec::new(),
                done: false,
            },
        }
    }
}

impl<B> Iterator for ValueCollector<B>
where
    B: Iterator<Item = Result<Value, DataError>>,
{
    type Item = Result<Value, DataError>;

    fn next(&mut self) -> Option<Self::Item> {
        match &mut self.kind {
            CollectorKind::Passthrough => self.builder.next(),

            CollectorKind::Slurp { items, done } => {
                if *done {
                    return None;
                }
                loop {
                    match self.builder.next() {
                        Some(Ok(v)) => items.push(v),
                        Some(Err(e)) => {
                            *done = true;
                            return Some(Err(e));
                        }
                        None => {
                            *done = true;
                            let collected = std::mem::take(items);
                            return Some(Ok(Value::Array(Rc::new(collected))));
                        }
                    }
                }
            }
        }
    }
}
