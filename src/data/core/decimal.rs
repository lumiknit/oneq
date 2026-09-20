//! An arbitrary-precision decimal literal, matching jq's own "decNum" mode:
//! a JSON number literal's exact value (including "insignificant" trailing
//! zeros like `1.50`'s) is preserved verbatim until it's touched by
//! arithmetic, at which point it's converted to `f64` like any other
//! number (see `Value::as_number`). This is *not* the same as literal
//! spelling preservation (`have_literal_numbers`, which oneq doesn't
//! implement) - `1e2` round-trips as `1E+2`, not `1e2`, because printing
//! always goes through this type's own canonical formatter.
//!
//! Deliberately simpler than a general-purpose bignum: no `BigUint`, just
//! the significant digits as a plain byte vector plus a power-of-ten
//! exponent (`value = sign * digits * 10^exponent`). Arithmetic never
//! operates on this representation directly - callers only ever need
//! `to_f64`, `to_canonical_string`, and exact comparison (for cases like
//! `13911860366432393 == 13911860366432392`, which must stay `false` even
//! though both round to the same `f64`).
use std::cmp::Ordering;
use std::fmt;

#[derive(Clone, Debug)]
pub struct Decimal {
    /// `1` or `-1`. Zero values still carry a sign (`-0.00` round-trips as
    /// `-0.00`, matching real jq's decNum parsing), never `0`.
    pub sign: i8,
    /// Significant digits, most-significant first, each `0..=9`. Exactly
    /// `[0]` for a zero value; never has extra leading zeros otherwise.
    pub digits: Vec<u8>,
    /// `value = sign * (digits read as an integer) * 10^exponent`.
    pub exponent: isize,
}

impl Decimal {
    pub fn from_i64(n: i64) -> Self {
        let sign = if n < 0 { -1 } else { 1 };
        let s = n.unsigned_abs().to_string();
        Decimal {
            sign,
            digits: s.bytes().map(|b| b - b'0').collect(),
            exponent: 0,
        }
    }

    /// Parses a JSON/jq-style number literal: optional leading `-`/`+`,
    /// digits, an optional `.` fraction, and an optional `[eE][+-]?digits`
    /// exponent. Returns `None` for anything else (callers only feed this
    /// already-tokenized number text, so a mismatch means a caller bug).
    pub fn parse(text: &str) -> Option<Decimal> {
        let bytes = text.as_bytes();
        let mut i = 0;
        let sign = match bytes.first() {
            Some(b'-') => {
                i += 1;
                -1
            }
            Some(b'+') => {
                i += 1;
                1
            }
            _ => 1,
        };
        let int_start = i;
        while bytes.get(i).is_some_and(u8::is_ascii_digit) {
            i += 1;
        }
        let int_part = &text[int_start..i];
        let mut frac_part = "";
        if bytes.get(i) == Some(&b'.') {
            i += 1;
            let frac_start = i;
            while bytes.get(i).is_some_and(u8::is_ascii_digit) {
                i += 1;
            }
            frac_part = &text[frac_start..i];
        }
        let mut exp: isize = 0;
        if matches!(bytes.get(i), Some(b'e' | b'E')) {
            i += 1;
            let exp_sign = match bytes.get(i) {
                Some(b'+') => {
                    i += 1;
                    1
                }
                Some(b'-') => {
                    i += 1;
                    -1
                }
                _ => 1,
            };
            let exp_start = i;
            while bytes.get(i).is_some_and(u8::is_ascii_digit) {
                i += 1;
            }
            let exp_digits = &text[exp_start..i];
            if exp_digits.is_empty() {
                return None;
            }
            exp = exp_sign * exp_digits.parse::<isize>().ok()?;
        }
        if i != bytes.len() || (int_part.is_empty() && frac_part.is_empty()) {
            return None;
        }
        // Leading zeros aren't significant (and don't change the value at
        // a fixed exponent), but trailing zeros - wherever they came from,
        // integer or fractional part - are, so only the front gets
        // trimmed.
        let combined: String = format!("{int_part}{frac_part}");
        let stripped = combined.trim_start_matches('0');
        let digits = if stripped.is_empty() {
            vec![0]
        } else {
            stripped.bytes().map(|b| b - b'0').collect()
        };
        let exponent = exp - frac_part.len() as isize;
        Some(Decimal {
            sign,
            digits,
            exponent,
        })
    }

    pub fn is_zero(&self) -> bool {
        self.digits.iter().all(|&d| d == 0)
    }

    /// Exact negation - preserves precision, unlike arithmetic (matches
    /// real jq: "Unary negation preserves numerical precision").
    pub fn negate(&self) -> Decimal {
        Decimal {
            sign: -self.sign,
            ..self.clone()
        }
    }

    pub fn abs(&self) -> Decimal {
        Decimal {
            sign: 1,
            ..self.clone()
        }
    }

    /// Lossy conversion for arithmetic - builds a plain scientific-notation
    /// string and lets Rust's own (correctly-rounded) float parser do the
    /// decimal-to-binary conversion, rather than reimplementing it.
    pub fn to_f64(&self) -> f64 {
        // Up to 19 integer digits fit in u64. One integer-to-float conversion
        // rounds exactly as the parser does, without allocating two strings
        // every time a VM instruction uses an integer literal in arithmetic.
        if self.exponent == 0 && self.digits.len() <= 19 {
            let integer = self
                .digits
                .iter()
                .fold(0u64, |n, &digit| n * 10 + digit as u64);
            let number = integer as f64;
            return if self.sign < 0 { -number } else { number };
        }
        let mut s = String::with_capacity(self.digits.len() + 8);
        if self.sign < 0 {
            s.push('-');
        }
        for &d in &self.digits {
            s.push((b'0' + d) as char);
        }
        s.push('e');
        s.push_str(&self.exponent.to_string());
        s.parse().unwrap_or(0.0)
    }

    /// Exact value comparison (never rounds through `f64`) - needed so
    /// e.g. `13911860366432393 == 13911860366432392` stays `false` even
    /// though both literals round to the same nearest double.
    pub fn compare(&self, other: &Decimal) -> Ordering {
        let (a_zero, b_zero) = (self.is_zero(), other.is_zero());
        match (a_zero, b_zero) {
            (true, true) => return Ordering::Equal,
            (true, false) => {
                return if other.sign > 0 {
                    Ordering::Less
                } else {
                    Ordering::Greater
                };
            }
            (false, true) => {
                return if self.sign > 0 {
                    Ordering::Greater
                } else {
                    Ordering::Less
                };
            }
            (false, false) => {}
        }
        if self.sign != other.sign {
            return self.sign.cmp(&other.sign);
        }
        let magnitude = Self::compare_magnitude(self, other);
        if self.sign > 0 {
            magnitude
        } else {
            magnitude.reverse()
        }
    }

    /// Compares `|self|` and `|other|` by aligning both to the smaller of
    /// the two exponents (equivalent to padding the coarser one with
    /// trailing zeros) and comparing digit-by-digit - this way, differing
    /// numbers of "significant" trailing zeros (`1.50` vs `1.5`) never
    /// affect the *value* comparison, only formatting does.
    fn compare_magnitude(a: &Decimal, b: &Decimal) -> Ordering {
        let min_exp = a.exponent.min(b.exponent);
        let a_len = a.digits.len() as isize + (a.exponent - min_exp);
        let b_len = b.digits.len() as isize + (b.exponent - min_exp);
        if a_len != b_len {
            return a_len.cmp(&b_len);
        }
        for i in 0..a_len as usize {
            let da = a.digits.get(i).copied().unwrap_or(0);
            let db = b.digits.get(i).copied().unwrap_or(0);
            if da != db {
                return da.cmp(&db);
            }
        }
        Ordering::Equal
    }

    /// jq's own decNum `toString`: General Decimal Arithmetic spec's
    /// classic rule - plain notation unless `exponent > 0` (an integer
    /// literal written with a positive exponent, e.g. `1e2`) or the
    /// adjusted exponent (`exponent + digit_count - 1`, i.e. the power of
    /// ten of the leading digit) is below -6. Verified against real jq
    /// across the boundary in both directions for several shapes
    /// (`1e2` -> `1E+2`, `100` -> `100`, `0.000001` -> `0.000001`,
    /// `1e-7` -> `1E-7`, `1.000E+1000` round-trips unchanged).
    pub fn to_canonical_string(&self) -> String {
        let digits = &self.digits;
        let adjusted = self.exponent + digits.len() as isize - 1;
        let mut out = String::new();
        if self.sign < 0 {
            out.push('-');
        }
        if self.exponent > 0 || adjusted < -6 {
            out.push((b'0' + digits[0]) as char);
            if digits.len() > 1 {
                out.push('.');
                for &d in &digits[1..] {
                    out.push((b'0' + d) as char);
                }
            }
            out.push('E');
            out.push(if adjusted >= 0 { '+' } else { '-' });
            out.push_str(&adjusted.abs().to_string());
        } else {
            let point = digits.len() as isize + self.exponent;
            if point <= 0 {
                out.push_str("0.");
                for _ in 0..(-point) {
                    out.push('0');
                }
                for &d in digits {
                    out.push((b'0' + d) as char);
                }
            } else if point as usize >= digits.len() {
                for &d in digits {
                    out.push((b'0' + d) as char);
                }
                for _ in 0..(point as usize - digits.len()) {
                    out.push('0');
                }
            } else {
                for &d in &digits[..point as usize] {
                    out.push((b'0' + d) as char);
                }
                out.push('.');
                for &d in &digits[point as usize..] {
                    out.push((b'0' + d) as char);
                }
            }
        }
        out
    }
}

impl fmt::Display for Decimal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_canonical_string())
    }
}

impl PartialEq for Decimal {
    fn eq(&self, other: &Self) -> bool {
        self.compare(other) == Ordering::Equal
    }
}
impl Eq for Decimal {}
impl PartialOrd for Decimal {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.compare(other))
    }
}
impl Ord for Decimal {
    fn cmp(&self, other: &Self) -> Ordering {
        self.compare(other)
    }
}
