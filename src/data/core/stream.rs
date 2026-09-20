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
    pub fn new_key(val: isize) -> Self {
        Self::new(val, Self::TAG_BIT)
    }

    #[inline]
    pub fn new_key_str(val: &str) -> Self {
        Self::new_key(strs::intern(val))
    }

    #[inline]
    pub fn new_idx(val: isize) -> Self {
        Self::new(val, 0)
    }

    /// Returns the true value and is_key
    #[inline]
    pub fn unpack(&self) -> (isize, bool) {
        (self.0 >> 1, (self.0 & Self::TAG_BIT) != 0)
    }
}

#[derive(Debug, Default)]
pub struct StreamItem {
    pub path: Vec<PathItem>,

    /// The value at the path. If array/object closing event, this will be None.
    pub value: Option<Value>,
}
