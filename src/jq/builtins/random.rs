use std::sync::{Mutex, OnceLock};

use chrono::Utc;
use rand_chacha::{
    ChaCha8Rng,
    rand_core::{Rng, SeedableRng},
};

use crate::{data::Value, jq::vm::JqError};

static RNG: OnceLock<Mutex<ChaCha8Rng>> = OnceLock::new();

fn rng() -> &'static Mutex<ChaCha8Rng> {
    RNG.get_or_init(|| {
        let nanos = Utc::now().timestamp_nanos_opt().unwrap_or_default() as u64;
        let seed = nanos ^ nanos.rotate_left(29) ^ 0x9e3779b97f4a7c15;
        let mut bytes = [0u8; 32];
        for (i, chunk) in bytes.as_chunks_mut::<8>().0.iter_mut().enumerate() {
            let value =
                seed.rotate_left((i * 13) as u32) ^ (i as u64).wrapping_mul(0x9e3779b97f4a7c15);
            chunk.copy_from_slice(&value.to_le_bytes());
        }
        Mutex::new(ChaCha8Rng::from_seed(bytes))
    })
}

fn next_u64() -> u64 {
    rng()
        .lock()
        .expect("random generator mutex poisoned")
        .next_u64()
}

fn number_error(name: &str, value: &Value) -> JqError {
    JqError::Runtime(Value::String(
        format!("{name} expects a number, got {}", value.type_name()).into(),
    ))
}

pub fn random(_: &Value, _: &[Value]) -> Result<Value, JqError> {
    Ok(Value::Float(
        (next_u64() >> 11) as f64 / 9_007_199_254_740_992.0,
    ))
}

pub fn randint2(_: &Value, args: &[Value]) -> Result<Value, JqError> {
    randint_range(args, 1)
}

fn randint_range(args: &[Value], offset: usize) -> Result<Value, JqError> {
    let a = if offset == 0 {
        0.0
    } else {
        args[0]
            .as_number()
            .ok_or_else(|| number_error("randint", &args[0]))?
    };
    let b = args[offset]
        .as_number()
        .ok_or_else(|| number_error("randint", &args[offset]))?;
    if !a.is_finite() || !b.is_finite() || a.fract() != 0.0 || b.fract() != 0.0 {
        return Err(JqError::Runtime(Value::String(
            "randint requires integer bounds with b > a"
                .to_string()
                .into(),
        )));
    }
    let (a, b) = if a < b { (a, b) } else { (b, a) };
    let width = (b - a) as u64;
    Ok(Value::int(a as i64 + next_u64().wrapping_rem(width) as i64))
}

pub fn choice(input: &Value, _: &[Value]) -> Result<Value, JqError> {
    let Value::Array(values) = input else {
        return Err(JqError::Runtime(Value::String(
            "choice expects an array".to_string().into(),
        )));
    };
    if values.is_empty() {
        return Err(JqError::Runtime(Value::String(
            "choice cannot choose from an empty array"
                .to_string()
                .into(),
        )));
    }
    Ok(values[next_u64().wrapping_rem(values.len() as u64) as usize].clone())
}

fn format_uuid(bits: u128) -> Value {
    let hex = format!("{bits:032x}");
    Value::String(
        format!(
            "{}-{}-{}-{}-{}",
            &hex[..8],
            &hex[8..12],
            &hex[12..16],
            &hex[16..20],
            &hex[20..]
        )
        .into(),
    )
}

fn random_u128() -> u128 {
    (u128::from(next_u64()) << 64) | u128::from(next_u64())
}

/// Sets the version nibble and the RFC 9562 variant bits.
fn with_version(bits: u128, version: u128) -> u128 {
    (bits & !(0xf << 76) & !(0b11 << 62)) | (version << 76) | (0b10 << 62)
}

pub fn uuidv4(_: &Value, _: &[Value]) -> Result<Value, JqError> {
    Ok(format_uuid(with_version(random_u128(), 4)))
}

/// Last (unix_ms, rand_a) handed out, so UUIDv7s stay ordered within one
/// millisecond (RFC 9562 §6.2, method 1 with a 12-bit counter).
static LAST_V7: Mutex<(u64, u16)> = Mutex::new((0, 0));

pub fn uuidv7(_: &Value, _: &[Value]) -> Result<Value, JqError> {
    let now = Utc::now().timestamp_millis().max(0) as u64;
    let (ms, counter) = {
        let mut last = LAST_V7.lock().expect("uuidv7 mutex poisoned");
        let next = if now > last.0 {
            (now, (next_u64() & 0x7ff) as u16)
        } else if last.1 < 0xfff {
            (last.0, last.1 + 1)
        } else {
            // Counter exhausted: borrow the next millisecond.
            (last.0 + 1, (next_u64() & 0x7ff) as u16)
        };
        *last = next;
        next
    };
    let bits = (u128::from(ms & 0xffff_ffff_ffff) << 80)
        | (u128::from(counter) << 64)
        | u128::from(next_u64());
    Ok(format_uuid(with_version(bits, 7)))
}
