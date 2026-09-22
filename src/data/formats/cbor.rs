//! CBOR input/output.  CBOR is a byte-oriented format, so render options and
//! document separators intentionally do not apply.
use crate::data::{DataError, ParseOutput, Parser, Serializer, StreamItem, Value};
use crate::io::{Input, Output};
use crate::strs;
use indexmap::IndexMap;
use std::collections::VecDeque;
use std::io::{Read, Write};
use std::rc::Rc;

pub struct CborParser<'a> {
    input: Input<'a>,
    done: bool,
    queue: VecDeque<StreamItem>,
}
impl<'a> CborParser<'a> {
    #[must_use]
    pub const fn new(input: Input<'a>) -> Self {
        Self {
            input,
            done: false,
            queue: VecDeque::new(),
        }
    }
}
impl Iterator for CborParser<'_> {
    type Item = ParseOutput;
    fn next(&mut self) -> Option<Self::Item> {
        if let Some(x) = self.queue.pop_front() {
            return Some(Ok(x));
        }
        if self.done {
            return None;
        }
        self.done = true;
        let mut bytes = Vec::new();
        if let Err(e) = self.input.read_to_end(&mut bytes) {
            return Some(Err(DataError::IOError(e)));
        }
        match decode(&bytes).and_then(|(v, n)| {
            if n == bytes.len() {
                Ok(v)
            } else {
                Err("trailing bytes".into())
            }
        }) {
            Ok(v) => {
                super::document::events(v, &mut self.queue);
                self.queue.pop_front().map(Ok)
            }
            Err(message) => Some(Err(DataError::ParseError {
                path: String::new(),
                line: 1,
                col: 1,
                message,
            })),
        }
    }
}
impl Parser for CborParser<'_> {}

fn read_ai(b: &[u8], ai: u8) -> Result<(u64, usize), String> {
    match ai {
        0..=23 => Ok((u64::from(ai), 0)),
        24 => Ok((u64::from(b[0]), 1)),
        25 => Ok((u64::from(u16::from_be_bytes([b[0], b[1]])), 2)),
        26 => Ok((u64::from(u32::from_be_bytes(b[..4].try_into().unwrap())), 4)),
        27 => Ok((u64::from_be_bytes(b[..8].try_into().unwrap()), 8)),
        _ => Err("indefinite-length CBOR is not supported".into()),
    }
}
fn decode(b: &[u8]) -> Result<(Value, usize), String> {
    if b.is_empty() {
        return Err("unexpected end of CBOR".into());
    }
    let h = b[0];
    let major = h >> 5;
    let (n, k) = read_ai(&b[1..], h & 31)?;
    let mut p = 1 + k;
    let need = |p: usize, n: usize| -> Result<(), String> {
        if p + n <= b.len() {
            Ok(())
        } else {
            Err("unexpected end of CBOR".into())
        }
    };
    match major {
        0 => Ok((Value::int(n as i64), p)),
        1 => Ok((Value::int(-1 - (n as i64)), p)),
        2 => {
            need(p, n as usize)?;
            let a = b[p..p + n as usize]
                .iter()
                .map(|x| Value::int(i64::from(*x)))
                .collect();
            Ok((Value::Array(Rc::new(a)), p + n as usize))
        }
        3 => {
            need(p, n as usize)?;
            Ok((
                Value::String(
                    String::from_utf8(b[p..p + n as usize].to_vec())
                        .map_err(|_| "invalid UTF-8")?
                        .into(),
                ),
                p + n as usize,
            ))
        }
        4 => {
            let mut a = Vec::new();
            for _ in 0..n {
                let (v, z) = decode(&b[p..])?;
                p += z;
                a.push(v);
            }
            Ok((Value::Array(Rc::new(a)), p))
        }
        5 => {
            let mut m = IndexMap::new();
            for _ in 0..n {
                let (k, z) = decode(&b[p..])?;
                p += z;
                let (v, z) = decode(&b[p..])?;
                p += z;
                let Value::String(key) = k else {
                    return Err("CBOR map key must be a string".into());
                };
                m.insert(strs::intern(&key), v);
            }
            Ok((Value::Object(Rc::new(m)), p))
        }
        7 => match h {
            244 => Ok((Value::Bool(false), p)),
            245 => Ok((Value::Bool(true), p)),
            246 => Ok((Value::Null, p)),
            249 => {
                need(p, 2)?;
                Ok((Value::Float(f16(b[p], b[p + 1])), p + 2))
            }
            250 => {
                need(p, 4)?;
                Ok((
                    Value::Float(f64::from(f32::from_bits(u32::from_be_bytes(
                        b[p..p + 4].try_into().unwrap(),
                    )))),
                    p + 4,
                ))
            }
            251 => {
                need(p, 8)?;
                Ok((
                    Value::Float(f64::from_bits(u64::from_be_bytes(
                        b[p..p + 8].try_into().unwrap(),
                    ))),
                    p + 8,
                ))
            }
            _ => Err("unsupported CBOR simple value".into()),
        },
        _ => Err("unsupported CBOR type".into()),
    }
}
fn f16(a: u8, b: u8) -> f64 {
    let x = (u16::from(a) << 8) | u16::from(b);
    let s = if x & 0x8000 != 0 { -1.0 } else { 1.0 };
    let e = (x >> 10) & 31;
    let f = x & 1023;
    if e == 0 {
        s * f64::from(f) * 2f64.powi(-24)
    } else if e == 31 {
        if f == 0 { s * f64::INFINITY } else { f64::NAN }
    } else {
        s * (1.0 + f64::from(f) / 1024.0) * 2f64.powi(i32::from(e) - 15)
    }
}

pub struct CborSerializer {
    output: Output,
}
impl CborSerializer {
    #[must_use]
    pub const fn new(output: Output) -> Self {
        Self { output }
    }
    pub(crate) fn finish(self) -> std::io::Result<()> {
        self.output.finish()
    }
}
impl Serializer for CborSerializer {
    fn put(&mut self, value: Value) -> Result<(), DataError> {
        let mut b = Vec::new();
        encode(&value, &mut b)?;
        self.output.write_all(&b).map_err(DataError::IOError)
    }
}
fn head(b: &mut Vec<u8>, m: u8, n: u64) {
    if n < 24 {
        b.push(m << 5 | n as u8);
    } else if n <= 255 {
        b.extend([m << 5 | 24, n as u8]);
    } else {
        b.extend([m << 5 | 25]);
        b.extend((n as u16).to_be_bytes());
    }
}
fn encode(v: &Value, b: &mut Vec<u8>) -> Result<(), DataError> {
    match v {
        Value::Null => b.push(0xf6),
        Value::Bool(x) => b.push(if *x { 0xf5 } else { 0xf4 }),
        Value::Decimal(x) => {
            let n = x.to_f64() as i64;
            if n >= 0 {
                head(b, 0, n as u64);
            } else {
                head(b, 1, (-1 - n) as u64);
            }
        }
        Value::Float(x) => {
            b.push(0xfb);
            b.extend(x.to_be_bytes());
        }
        Value::String(s) => {
            head(b, 3, s.len() as u64);
            b.extend(s.as_bytes());
        }
        Value::Array(a) => {
            head(b, 4, a.len() as u64);
            for x in a.iter() {
                encode(x, b)?;
            }
        }
        Value::Object(o) => {
            head(b, 5, o.len() as u64);
            for (k, x) in o.iter() {
                let s = strs::resolve(*k).unwrap();
                head(b, 3, s.len() as u64);
                b.extend(s.as_bytes());
                encode(x, b)?;
            }
        }
    }
    Ok(())
}
