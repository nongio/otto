//! Calendar arithmetic for `modified:`, without a date crate.
//!
//! Only what the query language needs: civil dates to days since the epoch
//! and back, and the local UTC offset. Days are proleptic Gregorian, per
//! Howard Hinnant's `days_from_civil` and `civil_from_days`.

// Rust guideline compliant 2026-02-21

/// Seconds in a day.
pub const DAY: i64 = 86_400;

/// Days since 1970-01-01 of a civil date. `month` is 1 to 12.
pub fn days_from_civil(year: i32, month: u32, day: u32) -> i64 {
    let year = i64::from(year) - i64::from(month <= 2);
    let era = year.div_euclid(400);
    let year_of_era = year - era * 400;
    let month = i64::from(month);
    let day_of_year = (153 * (month + if month > 2 { -3 } else { 9 }) + 2) / 5 + i64::from(day) - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

/// The civil date `(year, month, day)` of a day count since 1970-01-01.
pub fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let days = days + 719_468;
    let era = days.div_euclid(146_097);
    let day_of_era = days - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_index = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_index + 2) / 5 + 1;
    let month = if month_index < 10 {
        month_index + 3
    } else {
        month_index - 9
    };
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "month is 1..=12 and day 1..=31 by construction"
    )]
    (year, month as u32, day as u32)
}

pub(crate) fn is_leap(year: i32) -> bool {
    (year % 4 == 0 && year % 100 != 0) || year % 400 == 0
}

/// Days in `month` (1 to 12) of `year`.
pub(crate) fn days_in_month(year: i32, month: u32) -> u32 {
    match month {
        2 if is_leap(year) => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

/// Seconds since the epoch as an `xsd:dateTime` in UTC.
pub(crate) fn xsd_datetime(epoch: i64) -> String {
    let (year, month, day) = civil_from_days(epoch.div_euclid(DAY));
    let second_of_day = epoch.rem_euclid(DAY);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        second_of_day / 3600,
        second_of_day / 60 % 60,
        second_of_day % 60
    )
}

/// Seconds since the epoch as RFC 3339 in local time, with its offset:
/// `2026-03-14T13:00:00+01:00`.
pub(crate) fn rfc3339_local(epoch: i64) -> String {
    let offset = local_offset(epoch);
    let local = epoch + offset;
    let (year, month, day) = civil_from_days(local.div_euclid(DAY));
    let second_of_day = local.rem_euclid(DAY);
    let sign = if offset < 0 { '-' } else { '+' };
    let minutes = offset.abs() / 60;
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}{sign}{:02}:{:02}",
        second_of_day / 3600,
        second_of_day / 60 % 60,
        second_of_day % 60,
        minutes / 60,
        minutes % 60
    )
}

/// The local offset from UTC at `epoch`, in seconds east.
///
/// Read from the C library, which knows the time zone rules; zero if it
/// cannot say.
pub fn local_offset(epoch: i64) -> i64 {
    let time: libc::time_t = epoch;
    // SAFETY: `tm` is plain data that `localtime_r` fills in; all-zero is a
    // valid value for it to start from.
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    // SAFETY: both pointers are valid for the duration of the call, and
    // `localtime_r` is the reentrant form that writes only through them.
    let filled = unsafe { libc::localtime_r(&raw const time, &raw mut tm) };
    if filled.is_null() {
        return 0;
    }
    tm.tm_gmtoff
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn days_round_trip_across_eras_and_leap_days() {
        for (y, m, d) in [
            (1970, 1, 1),
            (2000, 2, 29),
            (2024, 12, 31),
            (1969, 12, 31),
            (2100, 3, 1),
        ] {
            let days = days_from_civil(y, m, d);
            assert_eq!(civil_from_days(days), (i64::from(y), m, d));
        }
        assert_eq!(days_from_civil(1970, 1, 1), 0);
        assert_eq!(
            days_from_civil(2000, 3, 1) - days_from_civil(2000, 2, 28),
            2
        );
    }

    #[test]
    fn datetimes_are_utc_xsd() {
        assert_eq!(xsd_datetime(0), "1970-01-01T00:00:00Z");
        assert_eq!(xsd_datetime(1_709_251_199), "2024-02-29T23:59:59Z");
    }

    #[test]
    fn local_rfc3339_carries_its_offset() {
        let text = rfc3339_local(1_709_251_199);
        assert_eq!(text.len(), "2024-02-29T23:59:59+00:00".len(), "{text}");
        let offset = local_offset(1_709_251_199);
        let sign = if offset < 0 { '-' } else { '+' };
        assert_eq!(text.as_bytes()[19], sign as u8);
        if offset == 0 {
            assert_eq!(text, "2024-02-29T23:59:59+00:00");
        }
    }

    #[test]
    fn month_lengths() {
        assert_eq!(days_in_month(2024, 2), 29);
        assert_eq!(days_in_month(1900, 2), 28);
        assert_eq!(days_in_month(2000, 2), 29);
        assert_eq!(days_in_month(2025, 4), 30);
    }
}
