//! A synthetic Nifty market for tests, benchmarks and dry runs. Nothing here is real data.

use serde_json::json;

use crate::{Date, Instrument, Kind, NIFTY_INDEX_KEY};

/// First synthetic exchange token; keys are `NSE_FO|<token>`.
const FIRST_TOKEN: u32 = 700_000;

/// Next `n` Tuesdays on or after `from` (weekly Nifty expiries, holidays ignored).
pub fn tuesdays(from: Date, n: usize) -> Vec<Date> {
    let first = from.add_days((1 + 7 - from.weekday() as i32) % 7);
    (0..n).map(|i| first.add_days(7 * i as i32)).collect()
}

/// A Nifty universe around `spot` on `date`: the index, one future per expiry, and calls and
/// puts every `step` points within ±`band_pct` on the next `expiries` Tuesdays.
pub fn universe(
    date: Date,
    spot: f64,
    expiries: usize,
    band_pct: f64,
    step: f64,
) -> Vec<Instrument> {
    let mut out = vec![Instrument {
        key: NIFTY_INDEX_KEY.into(),
        underlying: "NIFTY".into(),
        kind: Kind::Idx,
        expiry: None,
        strike: None,
        lot_size: 0,
        tick_size: 0.0, // the real master has no tick size for the index
        weekly: false,
    }];
    let lo = ((spot * (1.0 - band_pct / 100.0)) / step).ceil() as i64;
    let hi = ((spot * (1.0 + band_pct / 100.0)) / step).floor() as i64;
    let mut token = FIRST_TOKEN;
    let mut next_key = || {
        token += 1;
        format!("NSE_FO|{token}")
    };
    for e in tuesdays(date, expiries) {
        out.push(Instrument {
            key: next_key(),
            underlying: "NIFTY".into(),
            kind: Kind::Fut,
            expiry: Some(e),
            strike: None,
            lot_size: 65,
            tick_size: 0.1,
            weekly: false,
        });
        for s in lo..=hi {
            for kind in [Kind::Ce, Kind::Pe] {
                out.push(Instrument {
                    key: next_key(),
                    underlying: "NIFTY".into(),
                    kind,
                    expiry: Some(e),
                    strike: Some(s as f64 * step),
                    lot_size: 65,
                    tick_size: 0.05,
                    weekly: true,
                });
            }
        }
    }
    out
}

/// The universe as an instrument master JSON array, readable by [`crate::parse_master`].
pub fn master_json(instruments: &[Instrument]) -> Vec<u8> {
    let rows: Vec<_> = instruments
        .iter()
        .map(|i| match i.kind {
            Kind::Idx => json!({
                "segment": "NSE_INDEX", "name": "Nifty 50", "exchange": "NSE",
                "instrument_type": "INDEX", "instrument_key": i.key, "trading_symbol": "NIFTY",
            }),
            kind => json!({
                "segment": "NSE_FO", "name": "NIFTY", "exchange": "NSE",
                "instrument_type": kind.as_str(), "instrument_key": i.key,
                "underlying_key": NIFTY_INDEX_KEY, "weekly": i.weekly,
                // 23:59:59 IST on the expiry day, as Upstox stamps it
                "expiry": i.expiry.map(|e| e.ist_unix_ms(24.0 * 60.0) - 1_000),
                "strike_price": i.strike.unwrap_or(0.0),
                "lot_size": i.lot_size, "tick_size": (i.tick_size * 100.0).round(),
            }),
        })
        .collect();
    serde_json::to_vec(&rows).expect("plain JSON values")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tuesdays_from_any_day() {
        let sat = Date::from_ymd(2026, 10, 17);
        assert_eq!(
            tuesdays(sat, 2),
            [Date::from_ymd(2026, 10, 20), Date::from_ymd(2026, 10, 27)]
        );
        let tue = Date::from_ymd(2026, 10, 20);
        assert_eq!(tuesdays(tue, 1), [tue]);
    }

    #[test]
    fn universe_round_trips_through_master_parser() {
        let date = Date::from_ymd(2026, 10, 19);
        let u = universe(date, 25_000.0, 2, 2.0, 50.0);
        // 24,500..=25,500 every 50: 21 strikes × 2 sides, plus a future, per expiry; plus the index
        assert_eq!(u.len(), 1 + 2 * (1 + 21 * 2));
        let parsed = crate::parse_master(&master_json(&u)).unwrap();
        let mut expected = u.clone();
        expected.sort_by(|a, b| {
            (a.expiry, a.strike.unwrap_or(0.0), a.kind)
                .partial_cmp(&(b.expiry, b.strike.unwrap_or(0.0), b.kind))
                .unwrap()
        });
        assert_eq!(parsed, expected);
    }
}
