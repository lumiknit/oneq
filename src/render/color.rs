// Split by ':', order:
// (0) null - (1) false - (2) true - (3) numbers -
// (4) strings - (5) arrays - (6) objects - (7) object keys -
// Default JQ_COLORS="0;90:0;39:0;39:0;39:0;32:1;39:1;39:1;34".

use std::fmt::{self};

/// ANSI reset sequence, to be emitted after any styled text.
pub const RESET: &str = "\x1b[0m";
pub const ANSI_STYLE_ESC: &str = "\x1b[";

pub const ENV_JQ_COLORS: &str = "JQ_COLORS";
pub const DEFAULT_JQ_COLORS: &str = concat!(
    "0;90:", // null - bright black
    "0;39:", // false - default
    "0;39:", // true - default
    "0;39:", // numbers - default
    "0;32:", // strings - green
    "1;39:", // arrays - bright default
    "1;39:", // objects - bright default
    "1;34:", // object keys - bright blue
    // jq script highlighting
    "3;90:",  // comments - bright black
    "1;35:",  // keywords - bright magenta
    "1;36:",  // functions - bright cyan
    "1;33:",  // variables - bright yellow
    "1;31:",  // operators - bright red
    "0;33:",  // string escapes - yellow
    "1;3;36"  // format specifiers - cyan
);

// Escape Helper

#[repr(u8)]
#[derive(Default, Clone, Copy)]
pub enum Deco {
    #[default]
    Normal = 0,
    Bright = 1,
    Dim = 2,
    Italic = 3,
    Underscore = 4,
    Blink = 5,
    Reverse = 7,
    Hidden = 8,
}

#[repr(u8)]
#[derive(Default, Clone, Copy)]
pub enum Color {
    Black = 0,
    Red = 1,
    Green = 2,
    Yellow = 3,
    Blue = 4,
    Magenta = 5,
    Cyan = 6,
    White = 7,
    #[default]
    Reset = 9,
}

#[derive(Clone)]
pub struct TextStyle(String);

impl Default for TextStyle {
    fn default() -> Self {
        Self("0;39".to_string())
    }
}

impl TryFrom<&str> for TextStyle {
    type Error = String;

    /// Validate the input string and create a `TextStyle` instance.
    fn try_from(s: &str) -> Result<Self, Self::Error> {
        // Split the input string by semicolons and parse each part as a u8.
        for part in s.split(';') {
            if part.parse::<u8>().is_err() {
                return Err(format!("Invalid ANSI code: {part}"));
            }
        }
        let mut t = String::with_capacity(2 + s.len());
        t.push_str("0;");
        t.push_str(s);
        Ok(Self(t))
    }
}

impl fmt::Display for TextStyle {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str(ANSI_STYLE_ESC)?;
        f.write_str(&self.0)?;
        f.write_str("m")
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum ThemeIdx {
    Null = 0,
    False,
    True,
    Number,
    String,
    Array,
    Object,
    ObjectKey, // 7

    // Custom for syntax highlighting of JQ scripts, not part of the original JQ_COLORS spec.
    JQComment,
    JQKeyword,
    JQFunction,
    JQVariable,
    JQOperator,     // Pipe and operators, including 'and', 'or', 'not', etc.
    JQStringEscape, // backslash escape sequences in string, including interpolation
    JQFormat,       // @csv, etc.

    Max, // 14
}

/// Styleset for JQ
#[derive(Clone)]
pub struct ColorTheme {
    pub styles: [TextStyle; ThemeIdx::Max as usize],
}

impl ColorTheme {
    pub fn parse(s: &str) -> Result<Self, String> {
        let mut styles: [TextStyle; ThemeIdx::Max as usize] = Default::default();

        for (i, style_str) in s.split(':').enumerate() {
            if i >= styles.len() {
                break;
            }
            styles[i] = TextStyle::try_from(style_str)?;
        }

        Ok(Self { styles })
    }
}

impl Default for ColorTheme {
    fn default() -> Self {
        Self::parse(DEFAULT_JQ_COLORS).unwrap()
    }
}
