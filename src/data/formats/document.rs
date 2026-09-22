//! Emit relative traversal events for a completed document.
use crate::data::{ArrayIndex, PathItem, StreamItem, Value};
use std::collections::VecDeque;

pub(super) fn events(value: Value, out: &mut VecDeque<StreamItem>) {
    match value {
        Value::Array(items) if !items.is_empty() => {
            for (i, value) in items.iter().enumerate() {
                out.push_back(StreamItem::Push(PathItem::new_idx(i as ArrayIndex)));
                events(value.clone(), out);
            }
            out.push_back(StreamItem::Close);
        }
        Value::Object(items) if !items.is_empty() => {
            for (key, value) in items.iter() {
                out.push_back(StreamItem::Push(PathItem::new_key(*key)));
                events(value.clone(), out);
            }
            out.push_back(StreamItem::Close);
        }
        value => out.push_back(StreamItem::Value(value)),
    }
}
