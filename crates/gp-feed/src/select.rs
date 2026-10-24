//! Which instruments to subscribe to (V1.4): the index, the nearest future, and options on the
//! next `expiries` expiries within ±`band_pct` of a reference price, at most `max` in all.

use crate::{Date, Instrument, Kind};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Selection {
    /// Option expiries to include, from the first one on or after `today`.
    pub expiries: usize,
    /// Half-width of the strike band around the reference, in percent.
    pub band_pct: f64,
    /// Upper bound on the number of instruments, index and future included. The index
    /// and the nearest future are always selected, even when `max` is smaller; `max`
    /// only limits how many option strikes are added after them.
    pub max: usize,
}

impl Default for Selection {
    fn default() -> Self {
        Self {
            expiries: 2,
            band_pct: 10.0,
            max: 1_500,
        }
    }
}

/// Instruments to subscribe to on `today`, given a reference price (gp-capture passes the
/// index spot). A non-finite or non-positive reference selects no options.
///
/// When the band holds more than `max`, the strikes farthest from `reference` are dropped
/// first; a strike's call and put are kept or dropped together.
pub fn select(
    instruments: &[Instrument],
    today: Date,
    reference: f64,
    sel: Selection,
) -> Vec<&Instrument> {
    let mut out: Vec<&Instrument> = instruments.iter().filter(|i| i.kind == Kind::Idx).collect();
    if let Some(fut) = instruments
        .iter()
        .filter(|i| i.kind == Kind::Fut && i.expiry.is_some_and(|e| e >= today))
        .min_by_key(|i| i.expiry)
    {
        out.push(fut);
    }

    let mut expiries: Vec<Date> = instruments
        .iter()
        .filter(|i| i.kind.is_option())
        .filter_map(|i| i.expiry)
        .filter(|&e| e >= today)
        .collect();
    expiries.sort();
    expiries.dedup();
    expiries.truncate(sel.expiries);

    let half = reference * sel.band_pct / 100.0;
    // (distance, expiry, strike) groups: both sides of a strike share one group.
    let mut groups: Vec<(f64, Date, f64, Vec<&Instrument>)> = Vec::new();
    for i in instruments.iter().filter(|i| i.kind.is_option()) {
        let (Some(e), Some(k)) = (i.expiry, i.strike) else {
            continue;
        };
        if !(reference.is_finite() && reference > 0.0)
            || !expiries.contains(&e)
            || (k - reference).abs() > half
        {
            continue;
        }
        match groups.iter_mut().find(|g| g.1 == e && g.2 == k) {
            Some(g) => g.3.push(i),
            None => groups.push(((k - reference).abs(), e, k, vec![i])),
        }
    }
    groups.sort_by(|a, b| {
        (a.0, a.1, a.2)
            .partial_cmp(&(b.0, b.1, b.2))
            .expect("finite")
    });
    for (_, _, _, members) in groups {
        if out.len() + members.len() > sel.max {
            break;
        }
        out.extend(members);
    }
    out.sort_by(|a, b| {
        (a.expiry, a.strike.unwrap_or(0.0), a.kind)
            .partial_cmp(&(b.expiry, b.strike.unwrap_or(0.0), b.kind))
            .expect("finite")
    });
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opt(key: usize, expiry: Date, strike: f64, kind: Kind) -> Instrument {
        Instrument {
            key: format!("NSE_FO|{key}"),
            underlying: "NIFTY".into(),
            kind,
            expiry: Some(expiry),
            strike: Some(strike),
            lot_size: 65,
            tick_size: Some(0.05),
        }
    }

    fn universe() -> Vec<Instrument> {
        let mut v = vec![Instrument {
            key: crate::NIFTY_INDEX_KEY.into(),
            underlying: "NIFTY".into(),
            kind: Kind::Idx,
            expiry: None,
            strike: None,
            lot_size: 0,
            tick_size: Some(0.05),
        }];
        let expiries = [
            Date::from_ymd(2026, 10, 13), // expired
            Date::from_ymd(2026, 10, 19),
            Date::from_ymd(2026, 10, 27),
            Date::from_ymd(2026, 11, 3),
        ];
        for (n, &e) in expiries.iter().enumerate() {
            for s in 0..=40 {
                let k = 23_000.0 + 100.0 * s as f64;
                v.push(opt(10_000 * n + 2 * s, e, k, Kind::Ce));
                v.push(opt(10_000 * n + 2 * s + 1, e, k, Kind::Pe));
            }
            let mut fut = opt(90_000 + n, e, 0.0, Kind::Fut);
            fut.strike = None;
            v.push(fut);
        }
        v
    }

    #[test]
    fn current_and_next_expiry_in_band() {
        let u = universe();
        let today = Date::from_ymd(2026, 10, 17);
        let picked = select(&u, today, 25_000.0, Selection::default());
        assert_eq!(picked[0].kind, Kind::Idx);
        let fut: Vec<_> = picked.iter().filter(|i| i.kind == Kind::Fut).collect();
        assert_eq!(fut.len(), 1);
        assert_eq!(fut[0].expiry, Some(Date::from_ymd(2026, 10, 19)));
        let opts: Vec<_> = picked.iter().filter(|i| i.kind.is_option()).collect();
        // 22,500–27,500 band, listed strikes 23,000–27,000: 41 strikes × 2 sides × 2 expiries
        assert_eq!(opts.len(), 164);
        assert!(
            opts.iter()
                .all(|i| i.expiry.unwrap() <= Date::from_ymd(2026, 10, 27))
        );
        assert!(opts.iter().all(|i| i.expiry.unwrap() >= today));
    }

    #[test]
    fn narrow_band_and_expiry_day() {
        let u = universe();
        let sel = Selection {
            band_pct: 1.0,
            ..Selection::default()
        };
        // On expiry day the expiring chain still counts as current.
        let picked = select(&u, Date::from_ymd(2026, 10, 19), 25_010.0, sel);
        let strikes: Vec<f64> = picked
            .iter()
            .filter(|i| i.kind == Kind::Ce && i.expiry == Some(Date::from_ymd(2026, 10, 19)))
            .map(|i| i.strike.unwrap())
            .collect();
        assert_eq!(strikes, [24_800.0, 24_900.0, 25_000.0, 25_100.0, 25_200.0]);
    }

    #[test]
    fn non_finite_reference_selects_no_options() {
        let u = universe();
        for r in [f64::NAN, f64::INFINITY, 0.0] {
            let picked = select(&u, Date::from_ymd(2026, 10, 17), r, Selection::default());
            assert_eq!(picked.len(), 2, "{r}"); // index + future
        }
    }

    #[test]
    fn cap_drops_farthest_pairs() {
        let u = universe();
        let sel = Selection {
            max: 2 + 4 * 3 + 1, // index + future + 3 strikes per expiry, plus one spare slot
            ..Selection::default()
        };
        let picked = select(&u, Date::from_ymd(2026, 10, 17), 25_040.0, sel);
        assert_eq!(picked.len(), 14);
        let mut strikes: Vec<f64> = picked.iter().filter_map(|i| i.strike).collect();
        strikes.dedup();
        assert_eq!(
            strikes,
            [24_900.0, 25_000.0, 25_100.0, 24_900.0, 25_000.0, 25_100.0]
        );
        // calls and puts stay paired
        assert_eq!(
            picked.iter().filter(|i| i.kind == Kind::Ce).count(),
            picked.iter().filter(|i| i.kind == Kind::Pe).count()
        );
    }
}
