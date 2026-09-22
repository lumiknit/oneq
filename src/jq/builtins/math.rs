use super::scalar::number;
use crate::{data::Value, jq::vm::JqError};
use std::rc::Rc;
macro_rules! unary { ($($name:ident),* $(,)?)=>{$(pub fn $name(input:&Value,_:&[Value])->Result<Value,JqError>{Ok(Value::Float(libm::$name(number(input)?)))})*}; }
unary!(
    acos, acosh, asin, asinh, atan, atanh, cos, cosh, sin, sinh, tan, tanh, exp, exp2, expm1, log,
    log2, log10, log1p, sqrt, cbrt, floor, ceil, trunc, round, fabs, erf, erfc, lgamma, tgamma, j0,
    j1, y0, y1
);
macro_rules! binary { ($($name:ident),* $(,)?)=>{$(pub fn $name(_:&Value,args:&[Value])->Result<Value,JqError>{Ok(Value::Float(libm::$name(number(&args[0])?,number(&args[1])?)))})*}; }
binary!(
    atan2, pow, hypot, fmod, remainder, copysign, nextafter, fdim, fmax, fmin
);
pub fn exp10(input: &Value, _: &[Value]) -> Result<Value, JqError> {
    Ok(Value::Float(10f64.powf(number(input)?)))
}
pub fn significand(input: &Value, _: &[Value]) -> Result<Value, JqError> {
    let n = number(input)?;
    Ok(Value::Float(n / n.abs().log2().floor().exp2()))
}
pub fn logb(input: &Value, _: &[Value]) -> Result<Value, JqError> {
    Ok(Value::Float(number(input)?.abs().log2().floor()))
}
pub fn nearbyint(input: &Value, _: &[Value]) -> Result<Value, JqError> {
    Ok(Value::Float(number(input)?.round_ties_even()))
}
pub fn rint(input: &Value, args: &[Value]) -> Result<Value, JqError> {
    nearbyint(input, args)
}
pub fn frexp(input: &Value, _: &[Value]) -> Result<Value, JqError> {
    let (fraction, exponent) = libm::frexp(number(input)?);
    Ok(Value::Array(Rc::new(vec![
        Value::Float(fraction),
        Value::int(i64::from(exponent)),
    ])))
}
pub fn modf(input: &Value, _: &[Value]) -> Result<Value, JqError> {
    let (fraction, integer) = libm::modf(number(input)?);
    Ok(Value::Array(Rc::new(vec![
        Value::Float(fraction),
        Value::Float(integer),
    ])))
}
pub fn ldexp(_: &Value, args: &[Value]) -> Result<Value, JqError> {
    Ok(Value::Float(libm::ldexp(
        number(&args[0])?,
        number(&args[1])? as i32,
    )))
}
pub fn jn(_: &Value, args: &[Value]) -> Result<Value, JqError> {
    Ok(Value::Float(libm::jn(
        number(&args[0])? as i32,
        number(&args[1])?,
    )))
}
pub fn yn(_: &Value, args: &[Value]) -> Result<Value, JqError> {
    Ok(Value::Float(libm::yn(
        number(&args[0])? as i32,
        number(&args[1])?,
    )))
}
pub fn fma(_: &Value, args: &[Value]) -> Result<Value, JqError> {
    Ok(Value::Float(libm::fma(
        number(&args[0])?,
        number(&args[1])?,
        number(&args[2])?,
    )))
}
pub fn isnan(input: &Value, _: &[Value]) -> Result<Value, JqError> {
    Ok(Value::Bool(input.as_number().is_some_and(f64::is_nan)))
}
pub fn isinfinite(input: &Value, _: &[Value]) -> Result<Value, JqError> {
    Ok(Value::Bool(input.as_number().is_some_and(f64::is_infinite)))
}
pub fn isnormal(input: &Value, _: &[Value]) -> Result<Value, JqError> {
    Ok(Value::Bool(input.as_number().is_some_and(f64::is_normal)))
}
pub const fn infinite(_: &Value, _: &[Value]) -> Result<Value, JqError> {
    Ok(Value::Float(f64::INFINITY))
}
pub const fn nan(_: &Value, _: &[Value]) -> Result<Value, JqError> {
    Ok(Value::Float(f64::NAN))
}
pub fn lgamma_r(input: &Value, _: &[Value]) -> Result<Value, JqError> {
    let (value, sign) = libm::lgamma_r(number(input)?);
    Ok(Value::Array(Rc::new(vec![
        Value::Float(value),
        Value::int(i64::from(sign)),
    ])))
}
pub fn scalb(_: &Value, args: &[Value]) -> Result<Value, JqError> {
    Ok(Value::Float(libm::scalbn(
        number(&args[0])?,
        number(&args[1])? as i32,
    )))
}
pub fn ilogb(input: &Value, _: &[Value]) -> Result<Value, JqError> {
    Ok(Value::int(i64::from(libm::ilogb(number(input)?))))
}
