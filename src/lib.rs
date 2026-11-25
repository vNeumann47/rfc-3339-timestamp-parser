//! A parser for RFC 3339 timestamps into a UTC instant.
//!
//! The parser accepts the full RFC 3339 date-time grammar, including
//! fractional seconds and numeric timezone offsets.  All values are
//! normalised to UTC, so the resulting `Timestamp` is independent of the
//! original offset.
//!
//! The implementation deliberately rejects leap seconds (`:60`), because
//! RFC 3339 permits them but they are not representable as a simple
//! seconds-since-epoch value.  This is a documented trade-off: accepting
//! them would require a leap-second table, which is not available in the
//! standard library and would make the parser stateful.

use std::fmt;

/// A UTC instant with nanosecond precision.
///
/// `seconds` is the number of seconds since the Unix epoch (1970-01-01T00:00:00Z),
/// ignoring leap seconds.  `nanos` is the fractional part in the range
/// 0..1_000_000_000.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Timestamp {
    pub seconds: i64,
    pub nanos: u32,
}

impl Timestamp {
    /// Creates a timestamp from whole seconds and nanoseconds.
    ///
    /// Panics if `nanos` is not less than 1_000_000_000.
    pub fn new(seconds: i64, nanos: u32) -> Self {
        assert!(nanos < 1_000_000_000, "nanos out of range");
        Timestamp { seconds, nanos }
    }
}

/// Errors that can occur while parsing an RFC 3339 timestamp.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParseError {
    /// The input is empty or too short to be a valid timestamp.
    Empty,
    /// The date or time components are malformed.
    Format(String),
    /// A component is out of its valid range (e.g. month 13).
    OutOfRange(String),
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ParseError::Empty => write!(f, "empty input"),
            ParseError::Format(msg) => write!(f, "format error: {}", msg),
            ParseError::OutOfRange(msg) => write!(f, "out of range: {}", msg),
        }
    }
}

impl std::error::Error for ParseError {}

/// Parses an RFC 3339 timestamp string into a UTC `Timestamp`.
///
/// The accepted grammar is:
///
/// ```text
/// date-time = full-date "T" full-time
/// full-date = 4DIGIT "-" 2DIGIT "-" 2DIGIT
/// full-time = 2DIGIT ":" 2DIGIT ":" 2DIGIT [ "." 1*DIGIT ] ( "Z" / ( "+" / "-" ) 2DIGIT ":" 2DIGIT )
/// ```
///
/// The `T` may be either `T` or `t`.  The timezone offset is interpreted as
/// the local time minus UTC, so `+01:00` means the local time is one hour
/// ahead of UTC.  The returned `Timestamp` is always in UTC.
///
/// Leap seconds are rejected (`:60`).
///
/// # Examples
///
/// ```
/// use rfc_3339_timestamp_parser::{parse, Timestamp};
/// let ts = parse("2024-01-02T03:04:05Z").unwrap();
/// assert_eq!(ts, Timestamp::new(1704164645, 0));
/// ```
pub fn parse(input: &str) -> Result<Timestamp, ParseError> {
    let bytes = input.as_bytes();
    if bytes.is_empty() {
        return Err(ParseError::Empty);
    }

    // Minimum length: YYYY-MM-DDTHH:MM:SSZ is 20 bytes.
    if bytes.len() < 20 {
        return Err(ParseError::Format("too short".into()));
    }

    // Date part: YYYY-MM-DD
    let year = parse_digits(&bytes[0..4])? as i32;
    if bytes[4] != b'-' {
        return Err(ParseError::Format("expected '-' after year".into()));
    }
    let month = parse_digits(&bytes[5..7])? as u32;
    if bytes[7] != b'-' {
        return Err(ParseError::Format("expected '-' after month".into()));
    }
    let day = parse_digits(&bytes[8..10])? as u32;

    // Separator: 'T' or 't'
    let sep = bytes[10];
    if sep != b'T' && sep != b't' {
        return Err(ParseError::Format("expected 'T' separator".into()));
    }

    // Time part: HH:MM:SS
    let hour = parse_digits(&bytes[11..13])? as u32;
    if bytes[13] != b':' {
        return Err(ParseError::Format("expected ':' after hour".into()));
    }
    let minute = parse_digits(&bytes[14..16])? as u32;
    if bytes[16] != b':' {
        return Err(ParseError::Format("expected ':' after minute".into()));
    }
    let second = parse_digits(&bytes[17..19])? as u32;

    // Validate ranges before doing arithmetic.
    if !(1..=12).contains(&month) {
        return Err(ParseError::OutOfRange(format!("month {}", month)));
    }
    if day < 1 || day > days_in_month(year, month) {
        return Err(ParseError::OutOfRange(format!("day {}", day)));
    }
    if hour > 23 {
        return Err(ParseError::OutOfRange(format!("hour {}", hour)));
    }
    if minute > 59 {
        return Err(ParseError::OutOfRange(format!("minute {}", minute)));
    }
    // Leap seconds are explicitly rejected; see module docs.
    if second > 59 {
        return Err(ParseError::OutOfRange(format!("second {}", second)));
    }

    // Fractional seconds: optional '.' followed by one or more digits.
    let mut idx = 19;
    let mut nanos: u32 = 0;
    if idx < bytes.len() && bytes[idx] == b'.' {
        idx += 1;
        let frac_start = idx;
        while idx < bytes.len() && bytes[idx].is_ascii_digit() {
            idx += 1;
        }
        if idx == frac_start {
            return Err(ParseError::Format("empty fractional part".into()));
        }
        // Parse up to 9 digits; ignore any beyond nanosecond precision.
        let frac = &bytes[frac_start..idx];
        let take = frac.len().min(9);
        let mut value: u32 = 0;
        for &b in &frac[..take] {
            value = value * 10 + (b - b'0') as u32;
        }
        // Scale to nanoseconds: if fewer than 9 digits, multiply by 10^(9-take).
        for _ in take..9 {
            value *= 10;
        }
        nanos = value;
    }

    // Timezone: 'Z', 'z', or +/-HH:MM
    if idx >= bytes.len() {
        return Err(ParseError::Format("missing timezone".into()));
    }
    let tz = bytes[idx];
    let offset_seconds: i64 = match tz {
        b'Z' | b'z' => {
            idx += 1;
            0
        }
        b'+' | b'-' => {
            if idx + 6 > bytes.len() {
                return Err(ParseError::Format("truncated timezone offset".into()));
            }
            let sign = if tz == b'+' { 1 } else { -1 };
            let oh = parse_digits(&bytes[idx + 1..idx + 3])? as i64;
            if bytes[idx + 3] != b':' {
                return Err(ParseError::Format("expected ':' in timezone offset".into()));
            }
            let om = parse_digits(&bytes[idx + 4..idx + 6])? as i64;
            if oh > 23 {
                return Err(ParseError::OutOfRange(format!("timezone hour {}", oh)));
            }
            if om > 59 {
                return Err(ParseError::OutOfRange(format!("timezone minute {}", om)));
            }
            idx += 6;
            sign * (oh * 3600 + om * 60)
        }
        _ => return Err(ParseError::Format("invalid timezone".into())),
    };

    if idx != bytes.len() {
        return Err(ParseError::Format("trailing characters".into()));
    }

    // Convert to seconds since epoch, then subtract the offset to get UTC.
    let days = days_from_civil(year, month, day);
    let local_seconds = days * 86400 + (hour as i64) * 3600 + (minute as i64) * 60 + (second as i64);
    let utc_seconds = local_seconds - offset_seconds;

    Ok(Timestamp::new(utc_seconds, nanos))
}

/// Parses a slice of ASCII digits into a `u32`.
///
/// The slice must be non-empty and contain only digits.
fn parse_digits(bytes: &[u8]) -> Result<u32, ParseError> {
    if bytes.is_empty() {
        return Err(ParseError::Format("empty numeric field".into()));
    }
    let mut value: u32 = 0;
    for &b in bytes {
        if !b.is_ascii_digit() {
            return Err(ParseError::Format("expected digit".into()));
        }
        value = value * 10 + (b - b'0') as u32;
    }
    Ok(value)
}

/// Returns the number of days in the given month of the given year.
fn days_in_month(year: i32, month: u32) -> u32 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 => {
            if is_leap_year(year) {
                29
            } else {
                28
            }
        }
        _ => 0,
    }
}

fn is_leap_year(year: i32) -> bool {
    (year % 4 == 0 && year % 100 != 0) || year % 400 == 0
}

/// Converts a civil date to days since 1970-01-01.
///
/// Uses the algorithm from Howard Hinnant's date library, which is
/// well-known and avoids any table lookups.
fn days_from_civil(y: i32, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = (y - era * 400) as u32; // [0, 399]
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1; // [0, 365]
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy; // [0, 146096]
    (era as i64) * 146097 + (doe as i64) - 719468
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_epoch() {
        assert_eq!(parse("1970-01-01T00:00:00Z").unwrap(), Timestamp::new(0, 0));
    }

    #[test]
    fn parses_basic_utc() {
        // 2024-01-02T03:04:05Z
        assert_eq!(parse("2024-01-02T03:04:05Z").unwrap(), Timestamp::new(1704164645, 0));
    }

    #[test]
    fn parses_lowercase_t_and_z() {
        assert_eq!(parse("2024-01-02t03:04:05z").unwrap(), Timestamp::new(1704164645, 0));
    }

    #[test]
    fn parses_fractional_seconds() {
        assert_eq!(parse("2024-01-02T03:04:05.123Z").unwrap(), Timestamp::new(1704164645, 123_000_000));
        assert_eq!(parse("2024-01-02T03:04:05.123456789Z").unwrap(), Timestamp::new(1704164645, 123_456_789));
    }

    #[test]
    fn truncates_fractional_seconds_beyond_nanoseconds() {
        // 10 digits: the last digit is ignored.
        assert_eq!(parse("2024-01-02T03:04:05.1234567890Z").unwrap(), Timestamp::new(1704164645, 123_456_789));
    }

    #[test]
    fn parses_positive_offset() {
        // 2024-01-02T03:04:05+01:00 is 2024-01-02T02:04:05Z
        assert_eq!(parse("2024-01-02T03:04:05+01:00").unwrap(), Timestamp::new(1704161045, 0));
    }

    #[test]
    fn parses_negative_offset() {
        // 2024-01-02T03:04:05-05:00 is 2024-01-02T08:04:05Z
        assert_eq!(parse("2024-01-02T03:04:05-05:00").unwrap(), Timestamp::new(1704182645, 0));
    }

    #[test]
    fn rejects_leap_second() {
        assert!(matches!(parse("2024-01-02T03:04:60Z"), Err(ParseError::OutOfRange(_))));
    }

    #[test]
    fn rejects_invalid_month() {
        assert!(matches!(parse("2024-13-02T03:04:05Z"), Err(ParseError::OutOfRange(_))));
    }

    #[test]
    fn rejects_invalid_day_for_month() {
        assert!(matches!(parse("2024-02-30T03:04:05Z"), Err(ParseError::OutOfRange(_))));
        assert!(matches!(parse("2024-04-31T03:04:05Z"), Err(ParseError::OutOfRange(_))));
    }

    #[test]
    fn accepts_leap_day() {
        assert!(parse("2024-02-29T00:00:00Z").is_ok());
    }

    #[test]
    fn rejects_trailing_characters() {
        assert!(matches!(parse("2024-01-02T03:04:05Zx"), Err(ParseError::Format(_))));
    }

    #[test]
    fn rejects_empty() {
        assert!(matches!(parse(""), Err(ParseError::Empty)));
    }

    #[test]
    fn rejects_missing_timezone() {
        assert!(matches!(parse("2024-01-02T03:04:05"), Err(ParseError::Format(_))));
    }

    #[test]
    fn rejects_bad_separator() {
        assert!(matches!(parse("2024-01-02 03:04:05Z"), Err(ParseError::Format(_))));
    }

    #[test]
    fn rejects_offset_out_of_range() {
        assert!(matches!(parse("2024-01-02T03:04:05+24:00"), Err(ParseError::OutOfRange(_))));
        assert!(matches!(parse("2024-01-02T03:04:05+00:60"), Err(ParseError::OutOfRange(_))));
    }

    #[test]
    fn parses_fractional_with_offset() {
        assert_eq!(parse("2024-01-02T03:04:05.5+01:00").unwrap(), Timestamp::new(1704161045, 500_000_000));
    }

    #[test]
    fn parses_known_timestamp() {
        // 2001-09-09T01:46:40Z is 1_000_000_000 seconds since epoch.
        assert_eq!(parse("2001-09-09T01:46:40Z").unwrap(), Timestamp::new(1_000_000_000, 0));
    }
}
