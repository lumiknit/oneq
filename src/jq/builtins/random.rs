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
        for (i, chunk) in bytes.chunks_exact_mut(8).enumerate() {
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

pub(crate) fn random(_: &Value, _: &[Value]) -> Result<Value, JqError> {
    Ok(Value::Float(
        (next_u64() >> 11) as f64 / 9_007_199_254_740_992.0,
    ))
}

pub(crate) fn randint2(_: &Value, args: &[Value]) -> Result<Value, JqError> {
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

pub(crate) fn choice(input: &Value, _: &[Value]) -> Result<Value, JqError> {
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
