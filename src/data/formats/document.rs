//! Convert a completed document to the same leaf/last-child-close events as JSON.
use crate::data::{
    ArrayIndex,
    core::{PathItem, StreamItem, Value},
};
use std::collections::VecDeque;

/// Streams `value` (as if it sat at `path`) into `out`. `path` is used as a
/// shared scratch stack - `push`ed before recursing into each child and
/// `pop`ped right after - so a deeply nested document costs one `Vec` clone
/// per *emitted* event rather than one per level of nesting descended
/// through.
pub(super) fn events(value: Value, path: &mut Vec<PathItem>, out: &mut VecDeque<StreamItem>) {
    let leaf = match value {
        Value::Null => Value::Null,
        Value::Bool(v) => Value::Bool(v),
        Value::Decimal(v) => Value::Decimal(v),
        Value::Float(v) => Value::Float(v),
        Value::String(v) => Value::String(v),
        Value::Array(v) if v.is_empty() => Value::empty_array(),
        Value::Object(v) if v.is_empty() => Value::empty_object(),
        Value::Array(v) => {
            for (i, value) in v.iter().enumerate() {
                path.push(PathItem::new_idx(i as ArrayIndex));
                events(value.clone(), path, out);
                path.pop();
            }
            let mut last = path.clone();
            last.push(PathItem::new_idx((v.len() - 1) as ArrayIndex));
            out.push_back(StreamItem {
                path: last,
                value: None,
            });
            return;
        }
        Value::Object(v) => {
            let mut last_key = None;
            for (key, value) in v.iter() {
                last_key = Some(*key);
                path.push(PathItem::new_key(*key));
                events(value.clone(), path, out);
                path.pop();
            }
            let mut last = path.clone();
            last.push(PathItem::new_key(last_key.unwrap()));
            out.push_back(StreamItem {
                path: last,
                value: None,
            });
            return;
        }
    };
    out.push_back(StreamItem {
        path: path.clone(),
        value: Some(leaf),
    });
}
