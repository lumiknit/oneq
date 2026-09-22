use std::{cmp::Ordering, fmt::Display, rc::Rc};

use crate::{
    data::{DataError, escape::StringEscape},
    strs,
};

use super::decimal::Decimal;
use super::stream::PathItem;
use indexmap::IndexMap;

/// `ObjectKey` uses isize, which is symbol table index.
pub type ObjectKey = strs::Symbol;
pub type ArrayIndex = isize;

// Handle for array.
// As jq, if idx >= 0, use as is, if idx < 0, use size + idx.
// Only for negative case, bound check is needed.
pub const fn resolve_index(idx: ArrayIndex, size: usize) -> Result<usize, DataError> {
    if idx >= 0 {
        let idx = idx as usize;
        Ok(idx)
    } else {
        let idx = (size as isize) + idx;
        if idx >= 0 {
            Ok(idx as usize)
        } else {
            Err(DataError::OutOfBoundsNegativeArrayIndex)
        }
    }
}

pub type ArrayInner = Rc<Vec<Value>>;
pub type ObjectInner = Rc<IndexMap<ObjectKey, Value>>;

/// JSON compatible value
/// Size must be 2 words
#[derive(Clone, Debug, Default)]
pub enum Value {
    #[default]
    Null,
    Bool(bool),
    Decimal(Rc<Decimal>),
    Float(f64),
    String(Rc<String>), // Use Rc<String> for small payload
    Array(ArrayInner),
    Object(ObjectInner),
}

impl Value {
    #[must_use]
    pub fn int(n: i64) -> Self {
        Self::Decimal(Rc::new(Decimal::from_i64(n)))
    }
    #[must_use]
    pub fn decimal(d: Decimal) -> Self {
        Self::Decimal(Rc::new(d))
    }

    #[must_use]
    pub fn empty_array() -> Self {
        Self::Array(Rc::new(Vec::new()))
    }

    #[must_use]
    pub fn empty_object() -> Self {
        Self::Object(Rc::new(IndexMap::new()))
    }
}

impl Value {
    /// Iterate object entries in insertion order or lexical key order.
    /// Non-object values return `None`; sorting never mutates the object.
    #[must_use]
    pub fn object_iter(&self, sorted: bool) -> Option<std::vec::IntoIter<(&ObjectKey, &Self)>> {
        let Self::Object(map) = self else {
            return None;
        };
        Some(Self::object_entries(map, sorted))
    }

    pub(crate) fn object_entries(
        map: &IndexMap<ObjectKey, Self>,
        sorted: bool,
    ) -> std::vec::IntoIter<(&ObjectKey, &Self)> {
        let mut entries: Vec<_> = map.iter().collect();
        if sorted {
            let mut order: Vec<(ObjectKey, usize)> =
                entries.iter().enumerate().map(|(i, (k, _))| (**k, i)).collect();
            strs::sort_symbols_by_str(&mut order);
            entries = order.into_iter().map(|(_, i)| entries[i]).collect();
        }
        entries.into_iter()
    }

    #[must_use]
    pub const fn is_null(&self) -> bool {
        matches!(self, Self::Null)
    }

    /// Return true iff the value is not null or false. Follwing 'jq' rule.
    #[must_use]
    pub const fn is_truthy(&self) -> bool {
        match self {
            Self::Null => false,
            Self::Bool(b) => *b,
            _ => true,
        }
    }

    #[must_use]
    pub fn as_number(&self) -> Option<f64> {
        match self {
            Self::Decimal(d) => Some(d.to_f64()),
            Self::Float(f) => Some(*f),
            _ => None,
        }
    }

    #[must_use]
    pub const fn type_name(&self) -> &'static str {
        match self {
            Self::Null => "null",
            Self::Bool(_) => "boolean",
            Self::Decimal(_) | Self::Float(_) => "number",
            Self::String(_) => "string",
            Self::Array(_) => "array",
            Self::Object(_) => "object",
        }
    }

    #[must_use]
    pub const fn rank(&self) -> u8 {
        match self {
            Self::Null => 0,
            Self::Bool(false) => 1,
            Self::Bool(true) => 2,
            Self::Decimal(_) | Self::Float(_) => 3,
            Self::String(_) => 4,
            Self::Array(_) => 5,
            Self::Object(_) => 6,
        }
    }

    /// Resolve a negative array index (-1 is the last element) without
    /// overflowing when the index lies before the start of the array.
    fn resolve_ref_index(len: usize, n: isize) -> Result<usize, String> {
        let v = len.checked_add_signed(n).ok_or("array out of range")?;
        if v >= len {
            return Err("array out of range".to_string());
        }
        Ok(v)
    }

    /// Return the value at path, or Err if path does not exist.
    pub fn get_path(&self, path: &[PathItem]) -> Result<&Self, String> {
        let mut current = self;
        for item in path {
            let (i, is_key) = item.unpack();
            if is_key {
                if let Self::Object(obj) = current {
                    current = obj
                        .get(&i)
                        .ok_or_else(|| format!("key {} not found", strs::resolve(i).unwrap()))?;
                }
            } else if let Self::Array(arr) = current {
                if i >= 0 {
                    current = arr
                        .get(i as usize)
                        .ok_or_else(|| "array index out of bounds".to_string())?;
                } else {
                    let idx = Self::resolve_ref_index(arr.len(), i)?;
                    current = &arr[idx];
                }
            }
        }
        Ok(current)
    }

    /// In-place version of `set_path`: mutates through `Rc::make_mut`, so a
    /// uniquely-owned tree (the usual case while a `ValueBuilder` is folding
    /// one document together) is updated without copying its containers.
    /// `set_path` clones every container along the path, which makes building
    /// an N-element document out of N leaf events quadratic; this is O(depth).
    pub fn set_path_mut(&mut self, path: &[PathItem], value: Self) -> Result<(), DataError> {
        let Some((head, rest)) = path.split_first() else {
            *self = value;
            return Ok(());
        };

        self.child_mut(*head)?.set_path_mut(rest, value)
    }

    /// Access one child, creating a container for Null and inserting missing
    /// entries. Shared with the builder so detached subtrees keep the same
    /// index, type-error, and copy-on-write semantics as `set_path_mut`.
    pub(super) fn child_mut(&mut self, item: PathItem) -> Result<&mut Self, DataError> {
        let (i, is_key) = item.unpack();

        if is_key {
            if matches!(self, Self::Null) {
                *self = Self::Object(Rc::new(IndexMap::new()));
            }
            match self {
                Self::Object(obj) => {
                    let entries = Rc::make_mut(obj);
                    Ok(entries.entry(i).or_insert(Self::Null))
                }
                other => Err(DataError::UnexpectedObjectKeyType {
                    index_type: other.type_name(),
                }),
            }
        } else {
            if matches!(self, Self::Null) {
                *self = Self::Array(Rc::new(Vec::new()));
            }
            match self {
                Self::Array(arr) => {
                    let items = Rc::make_mut(arr);
                    let idx = resolve_index(i, items.len())?;
                    if items.len() <= idx {
                        items.resize(idx + 1, Self::Null);
                    }
                    Ok(&mut items[idx])
                }
                other => Err(DataError::UnexpectedArrayIndexType {
                    index_type: other.type_name(),
                }),
            }
        }
    }

    /// Return new Value with the value at path set to value.
    /// If the path does not exists, it'll try to create array/object as needed.
    /// If index is invalid (wrong type or out of bounds), it'll return Err.
    pub fn set_path(&self, path: &[PathItem], value: Self) -> Result<Self, DataError> {
        let Some((head, rest)) = path.split_first() else {
            return Ok(value);
        };

        let (i, is_key) = head.unpack();

        if is_key {
            let key = i;
            let mut entries: IndexMap<ObjectKey, Self> = match self {
                Self::Object(obj) => (**obj).clone(),
                Self::Null => IndexMap::new(),
                other => {
                    return Err(DataError::UnexpectedObjectKeyType {
                        index_type: other.type_name(),
                    });
                }
            };
            let child = match entries.get(&key) {
                Some(existing) => existing.set_path(rest, value)?,
                None => Self::Null.set_path(rest, value)?,
            };
            entries.insert(key, child);
            Ok(Self::Object(Rc::new(entries)))
        } else {
            let mut items: Vec<Self> = match self {
                Self::Array(arr) => (**arr).clone(),
                Self::Null => Vec::new(),
                other => {
                    return Err(DataError::UnexpectedArrayIndexType {
                        index_type: other.type_name(),
                    });
                }
            };
            let idx = resolve_index(i, items.len())?;
            if items.len() <= idx {
                items.resize(idx + 1, Self::Null);
            }
            items[idx] = items[idx].set_path(rest, value)?;
            Ok(Self::Array(Rc::new(items)))
        }
    }
}

impl Display for Value {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Null => f.write_str("null"),
            Self::Bool(b) => f.write_str(if *b { "true" } else { "false" }),
            Self::Decimal(n) => n.fmt(f),
            Self::Float(n) => n.fmt(f),
            Self::String(s) => {
                f.write_str("\"")?;
                s.escape_json_ascii('"', f)?;
                f.write_str("\"")
            }
            Self::Array(v) => {
                f.write_str("[")?;
                for (i, v) in v.iter().enumerate() {
                    if i > 0 {
                        f.write_str(",")?;
                    }
                    v.fmt(f)?;
                }
                f.write_str("]")
            }
            Self::Object(v) => {
                f.write_str("{")?;
                // Sort iteration by key for consistent output, but don't mutate the object.
                let v: Vec<_> = v.iter().collect();
                let mut order: Vec<(ObjectKey, usize)> =
                    v.iter().enumerate().map(|(i, (k, _))| (**k, i)).collect();
                strs::sort_symbols_by_str(&mut order);
                let v: Vec<_> = order.into_iter().map(|(_, i)| v[i]).collect();
                for (i, v) in v.iter().enumerate() {
                    if i > 0 {
                        f.write_str(",")?;
                    }
                    let key = strs::resolve(*v.0).unwrap();
                    f.write_str("\"")?;
                    key.escape_json_ascii('"', f)?;
                    f.write_str("\":")?;
                    v.1.fmt(f)?;
                }
                f.write_str("}")
            }
        }
    }
}

impl Value {
    fn to_raw_string_(&self, f: &mut String) {
        use std::fmt::Write;
        match self {
            Self::Null => f.push_str("null"),
            Self::Bool(b) => f.push_str(if *b { "true" } else { "false" }),
            Self::Decimal(n) => write!(f, "{n}").unwrap(),
            Self::Float(n) if n.is_nan() => f.push_str("null"),
            Self::Float(n) => {
                let n = if n.is_finite() {
                    *n
                } else if n.is_sign_negative() {
                    f64::MIN
                } else {
                    f64::MAX
                };
                // The same formatter the output path uses: Rust's `Display`
                // never switches to an exponent, so `tostring` would spell
                // `1e+20` out as twenty-one digits.
                f.push_str(&crate::data::formats::json::format_float(n));
            }
            Self::String(s) => {
                f.push('"');
                s.escape_json('"', f).unwrap();
                f.push('"');
            }
            Self::Array(v) => {
                f.push('[');
                let mut i = v.iter();
                if let Some(v) = i.next() {
                    v.to_raw_string_(f);
                }
                for v in i {
                    f.push(',');
                    v.to_raw_string_(f);
                }
                f.push(']');
            }
            Self::Object(v) => {
                f.push('{');
                let mut i = v.iter();
                if let Some(v) = i.next() {
                    let key = strs::resolve(*v.0).unwrap();
                    f.push('"');
                    key.escape_json('"', f).unwrap();
                    f.push_str("\":");
                    v.1.to_raw_string_(f);
                }
                for v in i {
                    let key = strs::resolve(*v.0).unwrap();
                    f.push_str(",\"");
                    key.escape_json('"', f).unwrap();
                    f.push_str("\":");
                    v.1.to_raw_string_(f);
                }
                f.push('}');
            }
        }
    }

    /// Unlike display, unsorted and allow unicode
    #[must_use]
    pub fn to_compact_json(&self) -> String {
        let mut s = String::with_capacity(8);
        self.to_raw_string_(&mut s);
        s
    }
}

impl PartialOrd for Value {
    fn partial_cmp(&self, b: &Self) -> Option<Ordering> {
        if self.rank() != b.rank() {
            return self.rank().partial_cmp(&b.rank());
        }
        match (self, b) {
            (Self::Null, Self::Null) => Some(Ordering::Equal),
            (Self::Bool(a), Self::Bool(b)) => a.partial_cmp(b),
            // Exact comparison between two literal decimals (never rounds
            // through `f64`) - matters for e.g.
            // `13911860366432393 == 13911860366432392`, which stays
            // `false` even though both round to the same nearest double.
            (Self::Decimal(a), Self::Decimal(b)) => Some(a.compare(b)),
            (Self::Decimal(_) | Self::Float(_), Self::Decimal(_) | Self::Float(_)) => {
                self.as_number().partial_cmp(&b.as_number())
            }
            (Self::String(a), Self::String(b)) => a.partial_cmp(b),
            (Self::Array(a), Self::Array(b)) => {
                for (a, b) in a.iter().zip(b.iter()) {
                    let c = (a).partial_cmp(b)?;
                    if c != Ordering::Equal {
                        return Some(c);
                    }
                }
                a.len().partial_cmp(&b.len())
            }
            (Self::Object(a), Self::Object(b)) => {
                let mut ak: Vec<_> = a.keys().map(|k| strs::resolve(*k).unwrap_or("")).collect();
                ak.sort_unstable();
                let mut bk: Vec<_> = b.keys().map(|k| strs::resolve(*k).unwrap_or("")).collect();
                bk.sort_unstable();

                let c = ak.partial_cmp(&bk)?;
                if c != Ordering::Equal {
                    return Some(c);
                }

                for (k, v) in Self::object_entries(a, true) {
                    let c = v.partial_cmp(&b[k])?;
                    if c != Ordering::Equal {
                        return Some(c);
                    }
                }
                Some(Ordering::Equal)
            }
            _ => None,
        }
    }
}

impl PartialEq for Value {
    fn eq(&self, other: &Self) -> bool {
        self.partial_cmp(other) == Some(Ordering::Equal)
    }
}
