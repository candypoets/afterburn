//! UTC timestamps in the format the Python reference used: `YYYY-MM-DDTHH:MM:SSZ`.
//!
//! `chrono`/`time` would be a dependency for one function, so the civil-date conversion is done
//! here directly (Howard Hinnant's `civil_from_days` algorithm).

use std::time::{SystemTime, UNIX_EPOCH};

/// The current UTC time as `YYYY-MM-DDTHH:MM:SSZ`.
pub fn now_iso() -> String {
    format_iso(now_secs())
}

/// Seconds since the Unix epoch.
pub fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

/// Format seconds since the Unix epoch as an ISO-8601 UTC timestamp.
pub fn format_iso(seconds: u64) -> String {
    let seconds = seconds as i64;
    let days = seconds.div_euclid(86_400);
    let time = seconds.rem_euclid(86_400);
    let (year, month, day) = civil_from_days(days);
    let hour = time / 3_600;
    let minute = (time % 3_600) / 60;
    let second = time % 60;
    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}Z")
}

/// Convert days since 1970-01-01 to a proleptic Gregorian `(year, month, day)`.
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let day_of_era = z - era * 146_097; // [0, 146096]
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let mut year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_prime = (5 * day_of_year + 2) / 153;
    let day = (day_of_year - (153 * month_prime + 2) / 5 + 1) as u32;
    let month = (month_prime + if month_prime < 10 { 3 } else { -9 }) as u32;
    if month <= 2 {
        year += 1;
    }
    (year, month, day)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn epoch_is_iso() {
        assert_eq!(format_iso(0), "1970-01-01T00:00:00Z");
    }

    #[test]
    fn known_timestamp() {
        // The `wP` record created by the Python service.
        assert_eq!(format_iso(1_792_605_391), "2026-10-21T17:56:31Z");
    }

    #[test]
    fn leap_day() {
        assert_eq!(format_iso(951_782_400), "2000-02-29T00:00:00Z");
    }
}
