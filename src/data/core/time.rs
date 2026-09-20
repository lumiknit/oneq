//! Date/time helpers backing calculation and runtime builtins, built on
//! `chrono` rather than hand-rolled calendar math.
//!
//! Not yet covered: `strflocaltime`.

use chrono::{
    DateTime, Datelike, Local, NaiveDate, NaiveDateTime, NaiveTime, TimeZone, Timelike, Utc,
};

/// Current time in seconds since the epoch, as jq's `now` (with sub-second
/// precision, unlike the other builtins here which take whole seconds).
pub fn now() -> f64 {
    let dt = Utc::now();
    dt.timestamp() as f64 + dt.timestamp_subsec_nanos() as f64 / 1e9
}

/// Parses a strict `YYYY-MM-DDTHH:MM:SSZ` timestamp into seconds since the
/// epoch, matching jq's `fromdateiso8601`. `None` on any malformed input.
pub fn parse_iso8601(s: &str) -> Option<i64> {
    let dt = NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%SZ").ok()?;
    Some(dt.and_utc().timestamp())
}

/// Formats seconds since the epoch as `YYYY-MM-DDTHH:MM:SSZ`, matching jq's
/// `todateiso8601`. Empty string on an out-of-range timestamp.
pub fn format_iso8601(secs: f64) -> String {
    match Utc.timestamp_opt(secs.floor() as i64, 0).single() {
        Some(dt) => dt.format("%Y-%m-%dT%H:%M:%SZ").to_string(),
        None => String::new(),
    }
}

/// `(weekday, day-of-year)` for a civil date, both 0-indexed as jq/`struct
/// tm` expect (weekday 0 = Sunday, day-of-year 0 = Jan 1st).
pub fn weekday_and_yday(y: i64, m: i64, d: i64) -> (i64, i64) {
    match NaiveDate::from_ymd_opt(y as i32, m as u32, d as u32) {
        Some(date) => (
            date.weekday().num_days_from_sunday() as i64,
            date.ordinal0() as i64,
        ),
        None => (0, 0),
    }
}

/// Parses `s` against a `strptime`-style format string, returning `(year,
/// month, day, hour, min, sec)`. Delegates to chrono's format parser (whose
/// directive set closely, but not perfectly, matches C `strptime`); falls
/// back to a date-only parse (time defaults to midnight) if that fails.
///
/// TODO: only spot-checked so far; needs a proper test pass, especially
/// around `%Z`/`%z` (chrono can't reliably parse a bare timezone name back
/// into an offset, so those are effectively ignored) and locale-dependent
/// weekday/month names.
pub fn strptime(s: &str, fmt: &str) -> Option<(i64, i64, i64, i64, i64, i64)> {
    if let Ok(dt) = NaiveDateTime::parse_from_str(s, fmt) {
        return Some((
            dt.year() as i64,
            dt.month() as i64,
            dt.day() as i64,
            dt.hour() as i64,
            dt.minute() as i64,
            dt.second() as i64,
        ));
    }
    if let Ok(d) = NaiveDate::parse_from_str(s, fmt) {
        return Some((d.year() as i64, d.month() as i64, d.day() as i64, 0, 0, 0));
    }
    None
}

/// `(year, month, day, hour, min, sec)` for seconds since the epoch, as
/// jq's `gmtime` would produce. `None` on an out-of-range timestamp.
pub fn seconds_to_ymdhms(secs: f64) -> Option<(i64, i64, i64, i64, i64, i64)> {
    let dt = Utc.timestamp_opt(secs.floor() as i64, 0).single()?;
    Some((
        dt.year() as i64,
        dt.month() as i64,
        dt.day() as i64,
        dt.hour() as i64,
        dt.minute() as i64,
        dt.second() as i64,
    ))
}

/// Formats a broken-down time (year, month 1-12, day, hour, min, sec) with a
/// `strftime`-style format string, matching jq's `strftime`. `None` if the
/// date/time fields are out of range.
pub fn strftime(y: i64, mo: i64, d: i64, h: i64, mi: i64, s: i64, fmt: &str) -> Option<String> {
    let date = NaiveDate::from_ymd_opt(y as i32, mo as u32, d as u32)?;
    let time = NaiveTime::from_hms_opt(h as u32, mi as u32, s as u32)?;
    Some(NaiveDateTime::new(date, time).format(fmt).to_string())
}

/// jq's broken-down time: `(year, month 0-based, day, hour, min, sec, wday,
/// yday)`; `sec`/wday/yday keep the fractional part of the input seconds,
/// mirroring `gmtime`'s sub-second precision.
type BrokenDown = (f64, f64, f64, f64, f64, f64, f64, f64);

fn broken_down<Tz: TimeZone>(dt: DateTime<Tz>, frac: f64) -> BrokenDown {
    (
        dt.year() as f64,
        (dt.month() - 1) as f64,
        dt.day() as f64,
        dt.hour() as f64,
        dt.minute() as f64,
        dt.second() as f64 + frac,
        dt.weekday().num_days_from_sunday() as f64,
        dt.ordinal0() as f64,
    )
}

/// Converts seconds since the epoch into broken-down GMT time, as jq's
/// `gmtime`. `None` on an out-of-range timestamp.
pub fn gmtime(secs: f64) -> Option<BrokenDown> {
    let whole = secs.floor();
    let dt = Utc.timestamp_opt(whole as i64, 0).single()?;
    Some(broken_down(dt, secs - whole))
}

/// Like `gmtime`, but in the process's local timezone, as jq's `localtime`.
pub fn localtime(secs: f64) -> Option<BrokenDown> {
    let whole = secs.floor();
    let dt = Utc.timestamp_opt(whole as i64, 0).single()?;
    Some(broken_down(dt.with_timezone(&Local), secs - whole))
}

/// Converts broken-down time (year, month 0-based, day, hour, min, sec) into
/// seconds since the epoch, as jq's `mktime`. `wday`/`yday` are ignored, like
/// jq's own `mktime` (`timegm` recomputes them). `None` if the date/time
/// fields are out of range.
pub fn mktime(y: i64, mo0: i64, d: i64, h: i64, mi: i64, s: f64) -> Option<f64> {
    let whole = s.floor();
    let date = NaiveDate::from_ymd_opt(y as i32, (mo0 + 1) as u32, d as u32)?;
    let time = NaiveTime::from_hms_opt(h as u32, mi as u32, whole as u32)?;
    let dt = NaiveDateTime::new(date, time).and_utc();
    Some(dt.timestamp() as f64 + (s - whole))
}
