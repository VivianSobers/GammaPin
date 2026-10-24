//! Calendar dates as days since 1970-01-01 (Arrow `date32`), and IST conversions.

use std::fmt;

/// IST is UTC+05:30 all year.
pub(crate) const IST_OFFSET_MS: i64 = 19_800_000;
const MS_PER_DAY: i64 = 86_400_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Date(pub i32);

impl Date {
    /// Proleptic Gregorian date; `month` 1–12, `day` 1–31 (not range-checked).
    pub fn from_ymd(year: i32, month: u32, day: u32) -> Self {
        // Howard Hinnant's days_from_civil.
        let y = i64::from(year) - i64::from(month <= 2);
        let era = y.div_euclid(400);
        let yoe = y - era * 400;
        let m = i64::from(month);
        let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + i64::from(day) - 1;
        let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
        Self((era * 146_097 + doe - 719_468) as i32)
    }

    pub fn ymd(self) -> (i32, u32, u32) {
        let z = i64::from(self.0) + 719_468;
        let era = z.div_euclid(146_097);
        let doe = z - era * 146_097;
        let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
        let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
        let mp = (5 * doy + 2) / 153;
        let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
        let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
        let year = (yoe + era * 400 + i64::from(month <= 2)) as i32;
        (year, month, day)
    }

    /// Parse `YYYY-MM-DD`.
    pub fn parse(s: &str) -> Option<Self> {
        let mut it = s.split('-');
        let (y, m, d) = (it.next()?, it.next()?, it.next()?);
        if it.next().is_some() || y.len() != 4 || m.len() != 2 || d.len() != 2 {
            return None;
        }
        let (y, m, d) = (y.parse().ok()?, m.parse().ok()?, d.parse().ok()?);
        let date = Self::from_ymd(y, m, d);
        (date.ymd() == (y, m, d)).then_some(date)
    }

    /// The IST calendar date of a Unix-ms instant.
    pub fn ist_from_unix_ms(ms: i64) -> Self {
        Self((ms + IST_OFFSET_MS).div_euclid(MS_PER_DAY) as i32)
    }

    /// Unix ms of `minute_of_day` (IST) on this date.
    pub fn ist_unix_ms(self, minute_of_day: f64) -> i64 {
        i64::from(self.0) * MS_PER_DAY - IST_OFFSET_MS + (minute_of_day * 60_000.0).round() as i64
    }

    /// 0 = Monday … 6 = Sunday.
    pub fn weekday(self) -> u32 {
        (i64::from(self.0) + 3).rem_euclid(7) as u32
    }

    pub fn add_days(self, n: i32) -> Self {
        Self(self.0 + n)
    }
}

/// Minutes after IST midnight of a Unix-ms instant, fractional.
pub fn ist_minute_of_day(ms: i64) -> f64 {
    (ms + IST_OFFSET_MS).rem_euclid(MS_PER_DAY) as f64 / 60_000.0
}

impl fmt::Display for Date {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let (y, m, d) = self.ymd();
        write!(f, "{y:04}-{m:02}-{d:02}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_dates() {
        assert_eq!(Date::from_ymd(1970, 1, 1), Date(0));
        assert_eq!(Date::from_ymd(2000, 3, 1).0, 11_017);
        assert_eq!(Date::from_ymd(2026, 10, 19).to_string(), "2026-10-19");
        assert_eq!(Date::from_ymd(1969, 12, 31), Date(-1));
        assert_eq!(Date::from_ymd(2026, 10, 19).weekday(), 0); // Monday
        assert_eq!(Date::from_ymd(2026, 10, 20).weekday(), 1);
    }

    #[test]
    fn round_trip_every_day_for_a_century() {
        for n in -20_000..40_000 {
            let (y, m, d) = Date(n).ymd();
            assert_eq!(Date::from_ymd(y, m, d), Date(n));
        }
    }

    #[test]
    fn parse_and_reject() {
        assert_eq!(Date::parse("2024-02-29"), Some(Date::from_ymd(2024, 2, 29)));
        for bad in [
            "2023-02-29",
            "2026-13-01",
            "2026-1-01",
            "20261019",
            "2026-10-19-1",
            "",
        ] {
            assert_eq!(Date::parse(bad), None, "{bad}");
        }
    }

    #[test]
    fn ist_conversions() {
        // Upstox expiry stamp: 23:59:59 IST on 2026-10-20
        assert_eq!(
            Date::ist_from_unix_ms(1_792_520_999_000),
            Date::from_ymd(2026, 10, 20)
        );
        let open = Date::from_ymd(2026, 10, 19).ist_unix_ms(9.0 * 60.0 + 15.0);
        assert_eq!(open, 1_792_381_500_000);
        assert_eq!(ist_minute_of_day(open), 555.0);
        assert_eq!(Date::ist_from_unix_ms(open), Date::from_ymd(2026, 10, 19));
    }
}
