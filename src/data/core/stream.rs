use crate::{data::value::Value, strs};

#[derive(Clone, Copy, Debug)]
#[repr(transparent)]
pub struct PathItem(isize);

impl PathItem {
    const TAG_BIT: isize = 1isize;

    #[inline]
    fn new(val: isize, tag: isize) -> Self {
        // Must be i63
        let v = val << 1;
        assert_eq!(
            v >> 1,
            val,
            "PathItem value overflow must be 63-bit signed int"
        );
        Self(v | tag)
    }

    #[inline]
    #[must_use]
    pub fn new_key(val: isize) -> Self {
        Self::new(val, Self::TAG_BIT)
    }

    #[inline]
    #[must_use]
    pub fn new_key_str(val: &str) -> Self {
        Self::new_key(strs::intern(val))
    }

    #[inline]
    #[must_use]
    pub fn new_idx(val: isize) -> Self {
        Self::new(val, 0)
    }

    /// Returns the true value and `is_key`
    #[inline]
    #[must_use]
    pub const fn unpack(&self) -> (isize, bool) {
        (self.0 >> 1, (self.0 & Self::TAG_BIT) != 0)
    }
}

/// Relative traversal events. `Value` and `Close` both finish the current
/// node and pop one path segment (if present), saving it as the last child.
/// Empty containers are Values and must not be followed by Close. A Value
/// or Close at an empty path completes a root document.
#[derive(Debug)]
pub enum StreamItem {
    /// Enter a child of the current container.
    Push(PathItem),
    /// Set the current path, then pop it. A root value has nothing to pop.
    Value(Value),
    /// Close a nonempty container. Its stream path includes the last popped
    /// child before this event pops the container's own path segment.
    Close,
}
