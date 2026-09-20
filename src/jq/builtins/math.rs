use super::scalar::number;
use crate::{data::Value, jq::vm::JqError};
use std::rc::Rc;
macro_rules! unary { ($($name:ident),* $(,)?)=>{$(pub(crate) fn $name(input:&Value,_:&[Value])->Result<Value,JqError>{Ok(Value::Float(libm::$name(number(input)?)))})*}; }
unary!(
    acos, acosh, asin, asinh, atan, atanh, cos, cosh, sin, sinh, tan, tanh, exp, exp2, expm1, log,
    log2, log10, log1p, sqrt, cbrt, floor, ceil, trunc, round, fabs, erf, erfc, lgamma, tgamma, j0,
    j1, y0, y1
);
macro_rules! binary { ($($name:ident),* $(,)?)=>{$(pub(crate) fn $name(_:&Value,args:&[Value])->Result<Value,JqError>{Ok(Value::Float(libm::$name(number(&args[0])?,number(&args[1])?)))})*}; }
binary!(
    atan2, pow, hypot, fmod, remainder, copysign, nextafter, fdim, fmax, fmin
);
pub(crate) fn exp10(input: &Value, _: &[Value]) -> Result<Value, JqError> {
    Ok(Value::Float(10f64.powf(number(input)?)))
}
pub(crate) fn significand(input: &Value, _: &[Value]) -> Result<Value, JqError> {
    let n = number(input)?;
    Ok(Value::Float(n / 2f64.powf(n.abs().log2().floor())))
}
pub(crate) fn logb(input: &Value, _: &[Value]) -> Result<Value, JqError> {
    Ok(Value::Float(number(input)?.abs().log2().floor()))
}
pub(crate) fn nearbyint(input: &Value, _: &[Value]) -> Result<Value, JqError> {
    Ok(Value::Float(number(input)?.round_ties_even()))
}
pub(crate) fn rint(input: &Value, args: &[Value]) -> Result<Value, JqError> {
    nearbyint(input, args)
}
pub(crate) fn frexp(input: &Value, _: &[Value]) -> Result<Value, JqError> {
    let (fraction, exponent) = libm::frexp(number(input)?);
    Ok(Value::Array(Rc::new(vec![
        Value::Float(fraction),
        Value::int(exponent as i64),
    ])))
}
pub(crate) fn modf(input: &Value, _: &[Value]) -> Result<Value, JqError> {
    let (fraction, integer) = libm::modf(number(input)?);
    Ok(Value::Array(Rc::new(vec![
        Value::Float(fraction),
        Value::Float(integer),
    ])))
}
pub(crate) fn ldexp(_: &Value, args: &[Value]) -> Result<Value, JqError> {
    Ok(Value::Float(libm::ldexp(
        number(&args[0])?,
        number(&args[1])? as i32,
    )))
}
pub(crate) fn jn(_: &Value, args: &[Value]) -> Result<Value, JqError> {
    Ok(Value::Float(libm::jn(
        number(&args[0])? as i32,
        number(&args[1])?,
    )))
}
pub(crate) fn yn(_: &Value, args: &[Value]) -> Result<Value, JqError> {
    Ok(Value::Float(libm::yn(
        number(&args[0])? as i32,
        number(&args[1])?,
    )))
}
pub(crate) fn fma(_: &Value, args: &[Value]) -> Result<Value, JqError> {
    Ok(Value::Float(libm::fma(
        number(&args[0])?,
        number(&args[1])?,
        number(&args[2])?,
    )))
}
pub(crate) fn isnan(input: &Value, _: &[Value]) -> Result<Value, JqError> {
    Ok(Value::Bool(input.as_number().is_some_and(f64::is_nan)))
}
pub(crate) fn isinfinite(input: &Value, _: &[Value]) -> Result<Value, JqError> {
    Ok(Value::Bool(input.as_number().is_some_and(f64::is_infinite)))
}
pub(crate) fn isnormal(input: &Value, _: &[Value]) -> Result<Value, JqError> {
    Ok(Value::Bool(input.as_number().is_some_and(f64::is_normal)))
}
pub(crate) fn infinite(_: &Value, _: &[Value]) -> Result<Value, JqError> {
    Ok(Value::Float(f64::INFINITY))
}
pub(crate) fn nan(_: &Value, _: &[Value]) -> Result<Value, JqError> {
    Ok(Value::Float(f64::NAN))
}
pub(crate) fn lgamma_r(input: &Value, _: &[Value]) -> Result<Value, JqError> {
    let (value, sign) = libm::lgamma_r(number(input)?);
    Ok(Value::Array(Rc::new(vec![
        Value::Float(value),
        Value::int(sign as i64),
    ])))
}
pub(crate) fn scalb(_: &Value, args: &[Value]) -> Result<Value, JqError> {
    Ok(Value::Float(libm::scalbn(
        number(&args[0])?,
        number(&args[1])? as i32,
    )))
}
pub(crate) fn ilogb(input: &Value, _: &[Value]) -> Result<Value, JqError> {
    Ok(Value::int(libm::ilogb(number(input)?) as i64))
}
