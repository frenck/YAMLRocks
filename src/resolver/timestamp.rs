//! Implicit timestamp resolution, PyYAML-compatible.
//!
//! YAML 1.2's core schema has no timestamp type, so a scalar like `2024-01-15`
//! is a plain string by default. PyYAML (following YAML 1.1's type repository)
//! instead reads a plain scalar matching a timestamp shape as a `datetime.date`
//! or `datetime.datetime`. Enabling that is opt-in here: `OPT_PYYAML_COMPAT`
//! turns it on to match PyYAML, and the standalone `OPT_TIMESTAMPS` flag turns
//! it on under any schema. Only *plain* scalars resolve; a quoted `"2024-01-15"`
//! stays a string, exactly as in PyYAML.
//!
//! The grammar mirrors PyYAML 6.0.3's implicit-resolver regex bug-for-bug,
//! including its quirk that a *date-only* value needs two-digit month and day
//! (`2024-01-05` is a date, `2024-1-5` is a string), while a value with a time
//! part accepts one or two digits. One deliberate divergence: PyYAML matches the
//! shape and then *constructs*, so an out-of-range value such as `2024-13-01` or
//! `25:00:00` raises `ValueError` out of the load. Crashing a load on malformed
//! input is a footgun, so [`parse`] validates the calendar and ranges and falls
//! back to a plain string (returns `None`) when they do not hold.

/// A resolved timestamp: a calendar date, or a date-time with an optional UTC
/// offset (naive when no zone is given).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Timestamp {
    /// A date with no time part (`2024-01-15`).
    Date { year: i32, month: u8, day: u8 },
    /// A date-time. `offset_minutes` is the UTC offset in minutes (`Some(0)` for
    /// `Z`), or `None` for a naive datetime with no zone.
    DateTime {
        year: i32,
        month: u8,
        day: u8,
        hour: u8,
        minute: u8,
        second: u8,
        microsecond: u32,
        offset_minutes: Option<i32>,
    },
}

impl Timestamp {
    /// The ISO 8601 rendering, matching Python's `date`/`datetime` `isoformat()`:
    /// `T` separator, fractional seconds only when non-zero, and a `+HH:MM`
    /// offset when zone-aware. Used where a resolved timestamp must be written
    /// back as text (JSON output, the fast emitter's total match).
    pub fn to_iso(&self) -> String {
        match self {
            Timestamp::Date { year, month, day } => format!("{year:04}-{month:02}-{day:02}"),
            Timestamp::DateTime {
                year,
                month,
                day,
                hour,
                minute,
                second,
                microsecond,
                offset_minutes,
            } => {
                let mut s =
                    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}");
                if *microsecond > 0 {
                    s.push_str(&format!(".{microsecond:06}"));
                }
                if let Some(offset) = offset_minutes {
                    let (sign, abs) = if *offset < 0 {
                        ('-', -offset)
                    } else {
                        ('+', *offset)
                    };
                    s.push_str(&format!("{sign}{:02}:{:02}", abs / 60, abs % 60));
                }
                s
            }
        }
    }
}

/// Parse a plain scalar as a PyYAML-style timestamp, or `None` if it is not one
/// (wrong shape, or out-of-range components). The caller resolves `None` to a
/// plain string.
pub fn parse(value: &str) -> Option<Timestamp> {
    let bytes = value.as_bytes();
    let mut cur = Cursor { bytes, pos: 0 };

    // Date: YYYY-M(M)-D(D). Year is exactly four digits.
    let year: i32 = cur.digits(4, 4)?.parse().ok()?;
    cur.byte(b'-')?;
    let (month_str, month_digits) = cur.digits_counted(1, 2)?;
    let month: u8 = month_str.parse().ok()?;
    cur.byte(b'-')?;
    let (day_str, day_digits) = cur.digits_counted(1, 2)?;
    let day: u8 = day_str.parse().ok()?;

    // A date with no time part must have two-digit month and day (PyYAML's
    // date-only alternative), and consumes the whole scalar.
    if cur.done() {
        if month_digits != 2 || day_digits != 2 || !valid_date(year, month, day) {
            return None;
        }
        return Some(Timestamp::Date { year, month, day });
    }

    // Otherwise a `T`/`t` or one-or-more spaces/tabs separates the time.
    if !cur.time_separator() {
        return None;
    }

    let hour: u8 = cur.digits(1, 2)?.parse().ok()?;
    cur.byte(b':')?;
    let minute: u8 = cur.digits(2, 2)?.parse().ok()?;
    cur.byte(b':')?;
    let second: u8 = cur.digits(2, 2)?.parse().ok()?;

    // Optional fractional seconds, truncated to microseconds (6 digits) like
    // PyYAML; further digits are consumed but ignored.
    let mut microsecond = 0u32;
    if cur.peek() == Some(b'.') {
        cur.pos += 1;
        let frac = cur.digits(0, usize::MAX)?;
        if !frac.is_empty() {
            let mut micros = String::with_capacity(6);
            micros.push_str(&frac[..frac.len().min(6)]);
            while micros.len() < 6 {
                micros.push('0');
            }
            microsecond = micros.parse().ok()?;
        }
    }

    // Optional timezone: leading whitespace, then `Z` or `(+|-)H(H)(:MM)`.
    cur.spaces();
    let offset_minutes = if cur.done() {
        None
    } else if matches!(cur.peek(), Some(b'Z' | b'z')) {
        cur.pos += 1;
        Some(0)
    } else {
        let sign = match cur.peek() {
            Some(b'+') => 1,
            Some(b'-') => -1,
            _ => return None,
        };
        cur.pos += 1;
        let tz_hour: i32 = cur.digits(1, 2)?.parse().ok()?;
        let tz_minute: i32 = if cur.peek() == Some(b':') {
            cur.pos += 1;
            cur.digits(2, 2)?.parse().ok()?
        } else {
            0
        };
        Some(sign * (tz_hour * 60 + tz_minute))
    };

    // Trailing whitespace is allowed after the zone; anything else is not a
    // timestamp.
    cur.spaces();
    if !cur.done() {
        return None;
    }

    if !valid_date(year, month, day)
        || hour > 23
        || minute > 59
        || second > 59
        || offset_minutes.is_some_and(|o| o.abs() >= 24 * 60)
    {
        return None;
    }

    Some(Timestamp::DateTime {
        year,
        month,
        day,
        hour,
        minute,
        second,
        microsecond,
        offset_minutes,
    })
}

/// Whether `year-month-day` is a real calendar date (matching `datetime.date`'s
/// validation, so a value PyYAML would reject is rejected here too).
fn valid_date(year: i32, month: u8, day: u8) -> bool {
    if !(1..=9999).contains(&year) || !(1..=12).contains(&month) || day < 1 {
        return false;
    }
    day <= days_in_month(year, month)
}

fn days_in_month(year: i32, month: u8) -> u8 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if is_leap_year(year) => 29,
        2 => 28,
        _ => 0,
    }
}

fn is_leap_year(year: i32) -> bool {
    year % 4 == 0 && (year % 100 != 0 || year % 400 == 0)
}

/// A tiny byte cursor over the scalar text.
struct Cursor<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl Cursor<'_> {
    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.pos).copied()
    }

    fn done(&self) -> bool {
        self.pos >= self.bytes.len()
    }

    /// Consume the exact byte `b`, or fail.
    fn byte(&mut self, b: u8) -> Option<()> {
        if self.peek() == Some(b) {
            self.pos += 1;
            Some(())
        } else {
            None
        }
    }

    /// Consume between `min` and `max` ASCII digits, returning them as text.
    fn digits(&mut self, min: usize, max: usize) -> Option<&str> {
        let start = self.pos;
        while self.pos - start < max && matches!(self.peek(), Some(b'0'..=b'9')) {
            self.pos += 1;
        }
        let count = self.pos - start;
        if count < min {
            return None;
        }
        // The slice is ASCII digits, so it is valid UTF-8.
        std::str::from_utf8(&self.bytes[start..self.pos]).ok()
    }

    /// Like [`Self::digits`] but also returns how many digits were consumed
    /// (one byte each, so the length is the count).
    fn digits_counted(&mut self, min: usize, max: usize) -> Option<(&str, usize)> {
        let text = self.digits(min, max)?;
        let count = text.len();
        Some((text, count))
    }

    /// Consume a `T`/`t` separator, or one or more spaces/tabs.
    fn time_separator(&mut self) -> bool {
        match self.peek() {
            Some(b'T' | b't') => {
                self.pos += 1;
                true
            }
            Some(b' ' | b'\t') => {
                self.spaces();
                true
            }
            _ => false,
        }
    }

    /// Consume any run of spaces and tabs.
    fn spaces(&mut self) {
        while matches!(self.peek(), Some(b' ' | b'\t')) {
            self.pos += 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // A datetime constructor for the tests; mirrors the struct's fields.
    #[allow(clippy::too_many_arguments)]
    fn dt(
        year: i32,
        month: u8,
        day: u8,
        hour: u8,
        minute: u8,
        second: u8,
        microsecond: u32,
        offset_minutes: Option<i32>,
    ) -> Timestamp {
        Timestamp::DateTime {
            year,
            month,
            day,
            hour,
            minute,
            second,
            microsecond,
            offset_minutes,
        }
    }

    #[test]
    fn date_only_needs_two_digit_month_and_day() {
        assert_eq!(
            parse("2024-01-15"),
            Some(Timestamp::Date {
                year: 2024,
                month: 1,
                day: 15
            })
        );
        // PyYAML's date-only alternative requires two digits each.
        assert_eq!(parse("2024-1-5"), None);
        assert_eq!(parse("2024-01-5"), None);
        assert_eq!(parse("2024-1-15"), None);
    }

    #[test]
    fn datetime_separators_and_case() {
        let want = dt(2024, 1, 15, 13, 30, 45, 0, None);
        assert_eq!(parse("2024-01-15T13:30:45"), Some(want.clone()));
        assert_eq!(parse("2024-01-15t13:30:45"), Some(want.clone()));
        assert_eq!(parse("2024-01-15 13:30:45"), Some(want.clone()));
        // A time part allows one-or-two-digit month/day/hour.
        assert_eq!(
            parse("2024-1-5 3:30:45"),
            Some(dt(2024, 1, 5, 3, 30, 45, 0, None))
        );
    }

    #[test]
    fn fractional_seconds_truncate_to_microseconds() {
        assert_eq!(
            parse("2024-01-15T13:30:45.123456789"),
            Some(dt(2024, 1, 15, 13, 30, 45, 123456, None))
        );
        assert_eq!(
            parse("2024-01-15T13:30:45.5"),
            Some(dt(2024, 1, 15, 13, 30, 45, 500000, None))
        );
    }

    #[test]
    fn timezones() {
        assert_eq!(
            parse("2024-01-15T13:30:45Z"),
            Some(dt(2024, 1, 15, 13, 30, 45, 0, Some(0)))
        );
        assert_eq!(
            parse("2024-01-15T13:30:45+5"),
            Some(dt(2024, 1, 15, 13, 30, 45, 0, Some(300)))
        );
        assert_eq!(
            parse("2024-01-15T13:30:45+05:30"),
            Some(dt(2024, 1, 15, 13, 30, 45, 0, Some(330)))
        );
        assert_eq!(
            parse("2024-01-15T13:30:45-08:00"),
            Some(dt(2024, 1, 15, 13, 30, 45, 0, Some(-480)))
        );
        // Extra spaces around the separator and the zone are allowed.
        assert_eq!(
            parse("2024-01-15  13:30:45  +02:00"),
            Some(dt(2024, 1, 15, 13, 30, 45, 0, Some(120)))
        );
    }

    #[test]
    fn out_of_range_falls_back_to_string() {
        // Timestamp-shaped but invalid: PyYAML raises; we return None (string).
        assert_eq!(parse("2024-13-01"), None);
        assert_eq!(parse("2024-02-30"), None);
        assert_eq!(parse("2024-01-15T25:00:00"), None);
        assert_eq!(parse("2024-01-15T13:60:00"), None);
        assert_eq!(parse("2024-01-15T13:30:45+25:00"), None);
    }

    #[test]
    fn non_timestamps() {
        assert_eq!(parse("not-a-date"), None);
        assert_eq!(parse("2024-01-15X"), None);
        assert_eq!(parse("2024-01-15T13:30:45 garbage"), None);
        assert_eq!(parse(""), None);
        assert_eq!(parse("2024"), None);
    }

    #[test]
    fn leap_year() {
        assert_eq!(
            parse("2024-02-29"),
            Some(Timestamp::Date {
                year: 2024,
                month: 2,
                day: 29
            })
        );
        assert_eq!(parse("2023-02-29"), None);
    }
}
