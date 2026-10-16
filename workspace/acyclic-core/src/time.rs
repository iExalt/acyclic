//! Times as people read them. The model keeps milliseconds since the Unix epoch; output shows RFC 3339 in UTC.

use crate::state::Millis;

/// The UTC calendar date of a time: year, month and day.
#[must_use]
pub fn civil(at: Millis) -> (i64, i64, i64) {
    // Howard Hinnant's days-to-civil algorithm.
    let days = at.div_euclid(86_400_000);
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    let day = doy - (153 * mp + 2) / 5 + 1;
    (year, month, day)
}

/// A time's UTC calendar date: `YYYY-MM-DD`.
#[must_use]
pub fn date(at: Millis) -> String {
    let (year, month, day) = civil(at);
    format!("{year:04}-{month:02}-{day:02}")
}

/// A time as RFC 3339 in UTC, to the second: `YYYY-MM-DDTHH:MM:SSZ`.
#[must_use]
pub fn rfc3339(at: Millis) -> String {
    let (year, month, day) = civil(at);
    let second = at.div_euclid(1000).rem_euclid(86_400);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        second / 3600,
        second / 60 % 60,
        second % 60
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::Error;

    #[test]
    fn times_read_as_rfc3339_in_utc() {
        assert_eq!(rfc3339(0), "1970-01-01T00:00:00Z");
        assert_eq!(rfc3339(-1), "1969-12-31T23:59:59Z");
        assert_eq!(rfc3339(951_782_400_999), "2000-02-29T00:00:00Z");
        assert_eq!(rfc3339(1_792_080_415_990), "2026-10-15T16:06:55Z");
    }

    #[test]
    fn errors_name_times_as_dates() {
        let message = Error::ClockJump { now: 1000, last: 1_792_080_415_990 }.to_string();
        assert!(message.contains("2026-10-15T16:06:55Z"), "{message}");
        assert!(
            !message.as_bytes().windows(5).any(|w| w.iter().all(u8::is_ascii_digit)),
            "{message}"
        );
    }
}
