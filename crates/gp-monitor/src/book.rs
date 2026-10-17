//! The latest quote per contract, arranged as one option chain per expiry.

use std::collections::BTreeMap;

use gp_feed::{Date, Event, Kind};
use gp_quant::{Quote, StrikeRow};

/// Strikes are keyed in paise so that they order and compare exactly.
fn strike_key(strike: f64) -> i64 {
    (strike * 100.0).round() as i64
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct Leg {
    quote: Quote,
    ltt_ms: i64,
}

#[derive(Debug, Clone, Copy, Default, PartialEq)]
struct Pair {
    call: Option<Leg>,
    put: Option<Leg>,
}

#[derive(Debug, Clone, Default)]
struct Chain {
    lot_size: i32,
    strikes: BTreeMap<i64, Pair>,
}

#[derive(Debug, Clone, Default)]
pub struct Book {
    spot: Option<(f64, i64)>,
    chains: BTreeMap<Date, Chain>,
}

/// L1 of an event as a C3 quote. A side with no quantity counts as absent (price 0).
pub fn quote(e: &Event) -> Quote {
    let side = |px: f64, qty: i64| if qty > 0 && px > 0.0 { px } else { 0.0 };
    Quote {
        bid: side(e.bid_px[0], e.bid_qty[0]),
        ask: side(e.ask_px[0], e.ask_qty[0]),
        oi: e.oi.max(0) as f64,
    }
}

impl Book {
    pub fn new() -> Self {
        Self::default()
    }

    /// Take `e` if it is newer than what the book holds for its contract. Returns whether
    /// it was taken: a duplicate from the second node, or an older update, is not.
    pub fn apply(&mut self, e: &Event) -> bool {
        match e.kind {
            Kind::Idx => {
                if self.spot.is_some_and(|(_, t)| t >= e.ltt_ms) || e.ltp <= 0.0 {
                    return false;
                }
                self.spot = Some((e.ltp, e.ltt_ms));
                true
            }
            Kind::Fut => false,
            Kind::Ce | Kind::Pe => {
                let (Some(expiry), Some(strike)) = (e.expiry, e.strike) else {
                    return false;
                };
                let chain = self.chains.entry(expiry).or_default();
                chain.lot_size = e.lot_size;
                let pair = chain.strikes.entry(strike_key(strike)).or_default();
                let slot = if e.kind == Kind::Ce {
                    &mut pair.call
                } else {
                    &mut pair.put
                };
                let leg = Leg {
                    quote: quote(e),
                    ltt_ms: e.ltt_ms,
                };
                if slot.is_some_and(|l| l.ltt_ms > e.ltt_ms || l == leg) {
                    return false;
                }
                *slot = Some(leg);
                true
            }
        }
    }

    /// Index LTP and its exchange time.
    pub fn spot(&self) -> Option<(f64, i64)> {
        self.spot
    }

    pub fn expiries(&self) -> impl Iterator<Item = Date> + '_ {
        self.chains.keys().copied()
    }

    /// The nearest expiry on or after `today`.
    pub fn front_expiry(&self, today: Date) -> Option<Date> {
        self.chains.range(today..).next().map(|(d, _)| *d)
    }

    pub fn lot_size(&self, expiry: Date) -> Option<i32> {
        self.chains.get(&expiry).map(|c| c.lot_size)
    }

    /// C3 rows for `expiry`, strikes ascending. A side never quoted is an empty quote,
    /// which the filters reject.
    pub fn rows(&self, expiry: Date) -> Vec<StrikeRow> {
        let Some(chain) = self.chains.get(&expiry) else {
            return Vec::new();
        };
        chain
            .strikes
            .iter()
            .map(|(&k, p)| StrikeRow {
                strike: k as f64 / 100.0,
                call: p.call.map(|l| l.quote).unwrap_or_default(),
                put: p.put.map(|l| l.quote).unwrap_or_default(),
            })
            .collect()
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use gp_feed::{DEPTH, NIFTY_INDEX_KEY};

    pub fn ev(kind: Kind, strike: f64, ltt_ms: i64, bid: f64, ask: f64) -> Event {
        let mut bid_px = [0.0; DEPTH];
        let mut ask_px = [0.0; DEPTH];
        bid_px[0] = bid;
        ask_px[0] = ask;
        Event {
            instrument_key: format!("NSE_FO|{}{}", strike, kind.as_str()),
            underlying: "NIFTY".into(),
            kind,
            expiry: (kind != Kind::Idx).then(|| Date::from_ymd(2026, 10, 20)),
            strike: kind.is_option().then_some(strike),
            lot_size: 75,
            ltt_ms,
            recv_ns: ltt_ms * 1_000_000 + 40_000_000,
            ltp: if kind == Kind::Idx {
                strike
            } else {
                0.5 * (bid + ask)
            },
            ltq: 0,
            cum_volume: 0,
            oi: 1_000,
            bid_px,
            bid_qty: [75; DEPTH],
            ask_px,
            ask_qty: [75; DEPTH],
            src_mask: 1,
        }
    }

    pub fn index(spot: f64, ltt_ms: i64) -> Event {
        Event {
            instrument_key: NIFTY_INDEX_KEY.into(),
            expiry: None,
            lot_size: 0,
            ..ev(Kind::Idx, spot, ltt_ms, 0.0, 0.0)
        }
    }

    #[test]
    fn keeps_the_latest_quote_per_contract() {
        let mut b = Book::new();
        assert!(b.apply(&index(25_000.0, 10)));
        assert!(!b.apply(&index(24_990.0, 9)));
        assert_eq!(b.spot(), Some((25_000.0, 10)));

        assert!(b.apply(&ev(Kind::Ce, 25_100.0, 10, 40.0, 41.0)));
        assert!(b.apply(&ev(Kind::Pe, 24_900.0, 10, 35.0, 36.0)));
        // the same update again (the other node's copy) is not news
        assert!(!b.apply(&ev(Kind::Pe, 24_900.0, 10, 35.0, 36.0)));
        assert!(!b.apply(&ev(Kind::Ce, 25_100.0, 5, 1.0, 2.0)));
        assert!(b.apply(&ev(Kind::Ce, 25_100.0, 11, 42.0, 43.0)));

        let d = Date::from_ymd(2026, 10, 20);
        assert_eq!(b.front_expiry(Date::from_ymd(2026, 10, 19)), Some(d));
        assert_eq!(b.front_expiry(Date::from_ymd(2026, 10, 21)), None);
        let rows = b.rows(d);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].strike, 24_900.0);
        assert_eq!(rows[0].call, Quote::default());
        assert_eq!(rows[1].call.bid, 42.0);
        assert_eq!(rows[1].call.oi, 1_000.0);
        assert_eq!(b.lot_size(d), Some(75));
    }

    #[test]
    fn empty_sides_are_zero() {
        let mut e = ev(Kind::Ce, 25_000.0, 1, 10.0, 11.0);
        e.bid_qty[0] = 0;
        assert_eq!(quote(&e).bid, 0.0);
        assert_eq!(quote(&e).ask, 11.0);
    }
}
