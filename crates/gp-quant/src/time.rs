//! Time to expiry in trading years: T = trading minutes left to 15:30 on expiry / (252 × 375),
//! floored at one minute. Rates are act/365 (`research/rates.py`), so `e^{-rT}` mixes day counts;
//! the effect over a few sessions is negligible (PREREGISTRATION.md, Deviations, 24 Oct 2026).

/// 09:15 IST, minutes after midnight.
pub const SESSION_OPEN: f64 = 9.0 * 60.0 + 15.0;
/// 15:30 IST, minutes after midnight.
pub const SESSION_CLOSE: f64 = 15.0 * 60.0 + 30.0;
pub const SESSION_MINUTES: f64 = SESSION_CLOSE - SESSION_OPEN;
pub const MINUTES_PER_YEAR: f64 = 252.0 * SESSION_MINUTES;
/// Lower bound on time to expiry: one trading minute, in years.
pub const MIN_T_YEARS: f64 = 1.0 / MINUTES_PER_YEAR;

/// Trading minutes from `minute_of_day` (IST, fractional allowed) to 15:30 on expiry.
/// `sessions_after_today` counts trading days after today up to and including expiry
/// (0 on expiry day). The caller applies the holiday calendar.
pub fn minutes_to_expiry(minute_of_day: f64, sessions_after_today: u32) -> f64 {
    let today = (SESSION_CLOSE - minute_of_day.max(SESSION_OPEN)).clamp(0.0, SESSION_MINUTES);
    today + SESSION_MINUTES * f64::from(sessions_after_today)
}

/// Trading minutes to years, floored at one minute.
pub fn t_years(minutes: f64) -> f64 {
    minutes.max(1.0) / MINUTES_PER_YEAR
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expiry_day() {
        assert_eq!(minutes_to_expiry(14.0 * 60.0, 0), 90.0);
        assert_eq!(minutes_to_expiry(8.0 * 60.0, 0), 375.0);
        assert_eq!(minutes_to_expiry(15.0 * 60.0 + 45.0, 0), 0.0);
    }

    #[test]
    fn days_before() {
        // Friday 10:15, expiry Tuesday: Mon and Tue still to come
        assert_eq!(minutes_to_expiry(10.0 * 60.0 + 15.0, 2), 315.0 + 750.0);
    }

    #[test]
    fn floor_one_minute() {
        assert_eq!(t_years(0.0), MIN_T_YEARS);
        assert_eq!(t_years(375.0), 1.0 / 252.0);
    }
}
