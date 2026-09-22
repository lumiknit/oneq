//! Date/time helpers backing calculation and runtime builtins, built on
//! `chrono` rather than hand-rolled calendar math.
//!
//! Not yet covered: `strflocaltime`.

use chrono::{
    DateTime, Datelike, Local, NaiveDate, NaiveDateTime, NaiveTime, TimeZone, Timelike, Utc,
};

/// Current time in seconds since the epoch, as jq's `now` (with sub-second
/// precision, unlike the other builtins here which take whole seconds).
#[must_use]
pub fn now() -> f64 {
    let dt = Utc::now();
    dt.timestamp() as f64 + f64::from(dt.timestamp_subsec_nanos()) / 1e9
}

/// Parses a strict `YYYY-MM-DDTHH:MM:SSZ` timestamp into seconds since the
/// epoch, matching jq's `fromdateiso8601`. `None` on any malformed input.
#[must_use]
pub fn parse_iso8601(s: &str) -> Option<i64> {
    let dt = NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%SZ").ok()?;
    Some(dt.and_utc().timestamp())
}

/// Formats seconds since the epoch as `YYYY-MM-DDTHH:MM:SSZ`, matching jq's
/// `todateiso8601`. Empty string on an out-of-range timestamp.
#[must_use]
pub fn format_iso8601(secs: f64) -> String {
    match Utc.timestamp_opt(secs.floor() as i64, 0).single() {
        Some(dt) => dt.format("%Y-%m-%dT%H:%M:%SZ").to_string(),
        None => String::new(),
    }
}

/// `(weekday, day-of-year)` for a civil date, both 0-indexed as jq/`struct
/// tm` expect (weekday 0 = Sunday, day-of-year 0 = Jan 1st).
#[must_use]
pub fn weekday_and_yday(y: i64, m: i64, d: i64) -> (i64, i64) {
    match NaiveDate::from_ymd_opt(y as i32, m as u32, d as u32) {
        Some(date) => (
            i64::from(date.weekday().num_days_from_sunday()),
            i64::from(date.ordinal0()),
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
#[must_use]
pub fn strptime(s: &str, fmt: &str) -> Option<(i64, i64, i64, i64, i64, i64)> {
    if let Ok(dt) = NaiveDateTime::parse_from_str(s, fmt) {
        return Some((
            i64::from(dt.year()),
            i64::from(dt.month()),
            i64::from(dt.day()),
            i64::from(dt.hour()),
            i64::from(dt.minute()),
            i64::from(dt.second()),
        ));
    }
    if let Ok(d) = NaiveDate::parse_from_str(s, fmt) {
        return Some((
            i64::from(d.year()),
            i64::from(d.month()),
            i64::from(d.day()),
            0,
            0,
            0,
        ));
    }
    None
}

/// `(year, month, day, hour, min, sec)` for seconds since the epoch, as
/// jq's `gmtime` would produce. `None` on an out-of-range timestamp.
#[must_use]
pub fn seconds_to_ymdhms(secs: f64) -> Option<(i64, i64, i64, i64, i64, i64)> {
    let dt = Utc.timestamp_opt(secs.floor() as i64, 0).single()?;
    Some((
        i64::from(dt.year()),
        i64::from(dt.month()),
        i64::from(dt.day()),
        i64::from(dt.hour()),
        i64::from(dt.minute()),
        i64::from(dt.second()),
    ))
}

/// Formats a broken-down time (year, month 1-12, day, hour, min, sec) with a
/// `strftime`-style format string, matching jq's `strftime`. `None` if the
/// date/time fields are out of range.
#[must_use]
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
        f64::from(dt.year()),
        f64::from(dt.month() - 1),
        f64::from(dt.day()),
        f64::from(dt.hour()),
        f64::from(dt.minute()),
        f64::from(dt.second()) + frac,
        f64::from(dt.weekday().num_days_from_sunday()),
        f64::from(dt.ordinal0()),
    )
}

/// Converts seconds since the epoch into broken-down GMT time, as jq's
/// `gmtime`. `None` on an out-of-range timestamp.
#[must_use]
pub fn gmtime(secs: f64) -> Option<BrokenDown> {
    let whole = secs.floor();
    let dt = Utc.timestamp_opt(whole as i64, 0).single()?;
    Some(broken_down(dt, secs - whole))
}

/// Like `gmtime`, but in the process's local timezone, as jq's `localtime`.
#[must_use]
pub fn localtime(secs: f64) -> Option<BrokenDown> {
    let whole = secs.floor();
    let dt = Utc.timestamp_opt(whole as i64, 0).single()?;
    Some(broken_down(dt.with_timezone(&Local), secs - whole))
}

/// Converts broken-down time (year, month 0-based, day, hour, min, sec) into
/// seconds since the epoch, as jq's `mktime`. `wday`/`yday` are ignored, like
/// jq's own `mktime` (`timegm` recomputes them). `None` if the date/time
/// fields are out of range.
#[must_use]
pub fn mktime(y: i64, mo0: i64, d: i64, h: i64, mi: i64, s: f64) -> Option<f64> {
    let whole = s.floor();
    let date = NaiveDate::from_ymd_opt(y as i32, (mo0 + 1) as u32, d as u32)?;
    let time = NaiveTime::from_hms_opt(h as u32, mi as u32, whole as u32)?;
    let dt = NaiveDateTime::new(date, time).and_utc();
    Some(dt.timestamp() as f64 + (s - whole))
}
