//! String escaping / unescaping utilities.

use std::{fmt, str::Chars};

/// Escape a string according to JSON escaping rules.
///
/// `quote` is the delimiter used by the caller. JSON itself only permits `"`,
/// but allowing `'` here is useful when generating quoted fragments.
///
/// The returned string does not include the surrounding quotes.
pub fn escape_string_json<W: fmt::Write>(input: &str, quote: char, out: &mut W) -> fmt::Result {
    for c in input.chars() {
        match c {
            '\\' => out.write_str(r"\\")?,
            '"' if quote == '"' => out.write_str(r#"\""#)?,
            '\'' if quote == '\'' => out.write_str(r"\'")?,

            '\u{08}' => out.write_str(r"\b")?,
            '\u{0C}' => out.write_str(r"\f")?,
            '\n' => out.write_str(r"\n")?,
            '\r' => out.write_str(r"\r")?,
            '\t' => out.write_str(r"\t")?,

            // JSON requires all U+0000..=U+001F to be escaped.
            c if c <= '\u{1F}' => {
                write!(out, r"\u{:04x}", c as u32)?;
            }

            c => out.write_char(c)?,
        }
    }

    Ok(())
}

/// Escape a string according to JSON rules, producing ASCII-only output.
///
/// Non-ASCII Unicode scalar values are emitted as `\uXXXX` sequences.
/// Supplementary-plane characters are emitted as UTF-16 surrogate pairs.
pub fn escape_string_json_ascii<W: fmt::Write>(
    input: &str,
    quote: char,
    out: &mut W,
) -> fmt::Result {
    for c in input.chars() {
        match c {
            '\\' => out.write_str(r"\\")?,
            '"' if quote == '"' => out.write_str(r#"\""#)?,
            '\'' if quote == '\'' => out.write_str(r"\'")?,

            '\u{08}' => out.write_str(r"\b")?,
            '\u{0C}' => out.write_str(r"\f")?,
            '\n' => out.write_str(r"\n")?,
            '\r' => out.write_str(r"\r")?,
            '\t' => out.write_str(r"\t")?,

            c if c <= '\u{1F}' => {
                write!(out, r"\u{:04x}", c as u32)?;
            }

            c if c.is_ascii() => out.write_char(c)?,

            c => {
                let n = c as u32;

                if n <= 0xFFFF {
                    write!(out, r"\u{:04x}", n)?;
                } else {
                    // Encode as UTF-16 surrogate pair.
                    let n = n - 0x10000;
                    let hi = 0xD800 + ((n >> 10) & 0x3FF);
                    let lo = 0xDC00 + (n & 0x3FF);

                    write!(out, r"\u{:04x}\u{:04x}", hi, lo)?;
                }
            }
        }
    }

    Ok(())
}

/// Escape a string so that it can safely be used as one POSIX-shell word.
///
/// This function writes the contents only; it does not add quotes.
///
/// Safe characters are left untouched. Shell metacharacters, whitespace,
/// control characters and backslashes are escaped with `\`.
///
/// This is intentionally shell-oriented rather than C-oriented:
/// `\n` is NOT used because POSIX shells generally do not interpret it as
/// a newline in an unquoted word.
pub fn escape_string_general<W: fmt::Write>(input: &str, out: &mut W) -> fmt::Result {
    for c in input.chars() {
        match c {
            // Characters that are safe in an unquoted POSIX shell word.
            'a'..='z'
            | 'A'..='Z'
            | '0'..='9'
            | '_'
            | '-'
            | '.'
            | '/'
            | ':'
            | '@'
            | '%'
            | '+'
            | '='
            | ',' => out.write_char(c)?,

            // Everything else gets backslash-escaped.
            c if c.is_ascii() => {
                out.write_char('\\')?;
                out.write_char(c)?;
            }

            // Non-ASCII characters are ordinary shell characters.
            c => out.write_char(c)?,
        }
    }

    Ok(())
}

/// Escape a string for CSV.
///
/// Every occurrence of `quote` is doubled. Everything else is unchanged.
///
/// Usually `quote` is `"`.
pub fn escape_string_csv<W: fmt::Write>(input: &str, quote: char, out: &mut W) -> fmt::Result {
    for c in input.chars() {
        if c == quote {
            out.write_char(quote)?;
            out.write_char(quote)?;
        } else {
            out.write_char(c)?;
        }
    }

    Ok(())
}

/// Unescape common JSON/C/shell-style escapes.
///
/// Supported:
///
/// - `\\`
/// - `\"`, `\'`
/// - `\b`, `\f`, `\n`, `\r`, `\t`
/// - `\a`, `\v`, `\e`
/// - `\xHH`
/// - `\uHHHH`
/// - `\UHHHHHHHH`
/// - octal `\0` .. `\777`
///
/// Unknown escapes are treated as the escaped character itself.
/// For example, `\q` becomes `q`.
pub fn unescape_string<W: fmt::Write>(input: &str, out: &mut W) -> fmt::Result {
    let mut chars = input.chars().peekable();

    while let Some(c) = chars.next() {
        if c != '\\' {
            out.write_char(c)?;
            continue;
        }

        let Some(next) = chars.next() else {
            // Preserve a trailing backslash rather than silently dropping it.
            out.write_char('\\')?;
            break;
        };

        match next {
            '\\' => out.write_char('\\')?,
            '"' => out.write_char('"')?,
            '\'' => out.write_char('\'')?,

            'a' => out.write_char('\x07')?,
            'b' => out.write_char('\x08')?,
            'e' => out.write_char('\x1b')?,
            'f' => out.write_char('\x0c')?,
            'n' => out.write_char('\n')?,
            'r' => out.write_char('\r')?,
            't' => out.write_char('\t')?,
            'v' => out.write_char('\x0b')?,

            'x' => {
                let mut value = 0u32;
                let mut count = 0;

                while count < 2 {
                    let Some(&c) = chars.peek() else {
                        break;
                    };

                    let Some(digit) = c.to_digit(16) else {
                        break;
                    };

                    chars.next();
                    value = value * 16 + digit;
                    count += 1;
                }

                if count == 2 {
                    if let Some(c) = char::from_u32(value) {
                        out.write_char(c)?;
                    }
                } else {
                    out.write_char('\\')?;
                    out.write_char('x')?;
                }
            }

            'u' => {
                if let Some(value) = read_hex(&mut chars, 4) {
                    decode_unicode_escape(value, &mut chars, out)?;
                } else {
                    out.write_char('\\')?;
                    out.write_char('u')?;
                }
            }

            'U' => {
                if let Some(value) = read_hex(&mut chars, 8) {
                    if let Some(c) = char::from_u32(value) {
                        out.write_char(c)?;
                    } else {
                        // Invalid Unicode scalar.
                        out.write_char('\\')?;
                        out.write_char('U')?;
                        write!(out, "{value:08x}")?;
                    }
                } else {
                    out.write_char('\\')?;
                    out.write_char('U')?;
                }
            }

            // C/POSIX-style octal escape.
            '0'..='7' => {
                let mut value = (next as u32) - ('0' as u32);
                let mut count = 1;

                while count < 3 {
                    let Some(&c) = chars.peek() else {
                        break;
                    };

                    if !('0'..='7').contains(&c) {
                        break;
                    }

                    chars.next();
                    value = value * 8 + (c as u32 - '0' as u32);
                    count += 1;
                }

                if let Some(c) = char::from_u32(value) {
                    out.write_char(c)?;
                }
            }

            // Shell escaping: \anything -> anything.
            other => out.write_char(other)?,
        }
    }

    Ok(())
}

/// Unescape CSV quote doubling.
///
/// `""` becomes `"`, for example.
/// No other escape processing is performed.
pub fn unescape_string_csv<W: fmt::Write>(input: &str, quote: char, out: &mut W) -> fmt::Result {
    let mut chars = input.chars().peekable();

    while let Some(c) = chars.next() {
        if c == quote && chars.peek() == Some(&quote) {
            chars.next();
            out.write_char(quote)?;
        } else {
            out.write_char(c)?;
        }
    }

    Ok(())
}

fn read_hex<'a>(chars: &mut std::iter::Peekable<Chars<'a>>, n: usize) -> Option<u32> {
    let mut value = 0u32;

    for _ in 0..n {
        let c = chars.next()?;
        let digit = c.to_digit(16)?;
        value = value * 16 + digit;
    }

    Some(value)
}

fn decode_unicode_escape<'a, W: fmt::Write>(
    value: u32,
    chars: &mut std::iter::Peekable<Chars<'a>>,
    out: &mut W,
) -> fmt::Result {
    let mut clone = chars.clone();

    // High surrogate: try to consume a following \uXXXX low surrogate.
    if (0xD800..=0xDBFF).contains(&value)
        && clone.next() == Some('\\')
        && clone.next() == Some('u')
        && let Some(low) = read_hex(&mut clone, 4)
        && (0xDC00..=0xDFFF).contains(&low)
    {
        chars.next(); // '\'
        chars.next(); // 'u'
        read_hex(chars, 4);

        let code = 0x10000 + ((value - 0xD800) << 10) + (low - 0xDC00);

        if let Some(c) = char::from_u32(code) {
            out.write_char(c)?;
            return Ok(());
        }
    }

    if let Some(c) = char::from_u32(value) {
        out.write_char(c)?;
    }

    Ok(())
}

pub trait StringEscape {
    fn escape_json<W: fmt::Write>(&self, quote: char, out: &mut W) -> fmt::Result;
    fn escape_json_ascii<W: fmt::Write>(&self, quote: char, out: &mut W) -> fmt::Result;
    fn escape_general<W: fmt::Write>(&self, out: &mut W) -> fmt::Result;
    fn escape_csv<W: fmt::Write>(&self, quote: char, out: &mut W) -> fmt::Result;

    fn unescape<W: fmt::Write>(&self, out: &mut W) -> fmt::Result;
    fn unescape_csv<W: fmt::Write>(&self, quote: char, out: &mut W) -> fmt::Result;
}

impl StringEscape for str {
    fn escape_json<W: fmt::Write>(&self, quote: char, out: &mut W) -> fmt::Result {
        escape_string_json(self, quote, out)
    }

    fn escape_json_ascii<W: fmt::Write>(&self, quote: char, out: &mut W) -> fmt::Result {
        escape_string_json_ascii(self, quote, out)
    }

    fn escape_general<W: fmt::Write>(&self, out: &mut W) -> fmt::Result {
        escape_string_general(self, out)
    }

    fn escape_csv<W: fmt::Write>(&self, quote: char, out: &mut W) -> fmt::Result {
        escape_string_csv(self, quote, out)
    }

    fn unescape<W: fmt::Write>(&self, out: &mut W) -> fmt::Result {
        unescape_string(self, out)
    }

    fn unescape_csv<W: fmt::Write>(&self, quote: char, out: &mut W) -> fmt::Result {
        unescape_string_csv(self, quote, out)
    }
}
