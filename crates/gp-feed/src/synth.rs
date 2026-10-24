//! A synthetic Nifty market for tests, benchmarks and dry runs. Nothing here is real data.

use std::collections::HashMap;

use gp_quant::OptType;
use gp_quant::black76::price;
use gp_quant::time::{MINUTES_PER_YEAR, minutes_to_expiry, t_years};
use serde_json::json;

use crate::date::ist_minute_of_day;
use crate::proto::{
    Feed, FeedResponse, FullFeed, IndexFullFeed, Ltpc, MarketFullFeed, MarketLevel, Quote,
    RequestMode, Type, feed::FeedUnion, full_feed::FullFeedUnion,
};
use crate::{DEPTH, Date, Instrument, Kind, NIFTY_INDEX_KEY};

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

/// SplitMix64: tiny, seedable and identical on every platform.
#[derive(Debug, Clone)]
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Self {
        Self(seed)
    }

    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// Uniform in [0, 1).
    pub fn uniform(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64
    }

    /// Uniform integer in `lo..=hi`.
    pub fn range(&mut self, lo: i64, hi: i64) -> i64 {
        lo + (self.next_u64() % (hi - lo + 1) as u64) as i64
    }

    /// Standard normal (Box–Muller).
    pub fn normal(&mut self) -> f64 {
        let u1 = 1.0 - self.uniform();
        let u2 = self.uniform();
        (-2.0 * u1.ln()).sqrt() * (std::f64::consts::TAU * u2).cos()
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MarketConfig {
    pub seed: u64,
    pub date: Date,
    /// Starting clock, minutes after IST midnight (555 = 09:15). The first frame is
    /// stamped one `step_ms` later.
    pub start_minute: f64,
    pub spot: f64,
    /// At-the-money annual vol.
    pub vol: f64,
    pub r: f64,
    /// Exchange time between frames.
    pub step_ms: i64,
    /// Instruments carried by each frame (the index is always one of them).
    pub per_frame: usize,
}

impl MarketConfig {
    pub fn new(date: Date) -> Self {
        Self {
            seed: 20_261_019,
            date,
            start_minute: 9.0 * 60.0 + 15.0,
            spot: 25_000.0,
            vol: 0.12,
            r: 0.065,
            step_ms: 250,
            per_frame: 40,
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct State {
    cum_volume: i64,
    /// Open interest in index units (lot size × contracts), as the feed is assumed to send it.
    oi_units: f64,
}

/// A deterministic stream of `FeedResponse` frames for a fixed universe.
#[derive(Debug, Clone)]
pub struct Market {
    cfg: MarketConfig,
    instruments: Vec<Instrument>,
    state: Vec<State>,
    sessions_to: HashMap<Date, u32>,
    rng: Rng,
    now_ms: i64,
    spot: f64,
    cursor: usize,
}

fn round_to(x: f64, tick: f64) -> f64 {
    ((x / tick).round() * tick * 100.0).round() / 100.0
}

/// Weekdays after `from` up to and including `to` (holidays ignored).
fn sessions_between(from: Date, to: Date) -> u32 {
    (from.0 + 1..=to.0)
        .filter(|&d| Date(d).weekday() < 5)
        .count() as u32
}

impl Market {
    pub fn new(cfg: MarketConfig, instruments: Vec<Instrument>) -> Self {
        let mut rng = Rng::new(cfg.seed);
        let state = instruments
            .iter()
            .map(|i| {
                let lots = match i.strike {
                    Some(k) => {
                        let x = (k - cfg.spot) / (0.03 * cfg.spot);
                        2_000.0 + 60_000.0 * (-x * x).exp() * (0.5 + rng.uniform())
                    }
                    None => 0.0,
                };
                State {
                    cum_volume: 0,
                    oi_units: (lots.round()) * f64::from(i.lot_size),
                }
            })
            .collect();
        let sessions_to = instruments
            .iter()
            .filter_map(|i| i.expiry)
            .map(|e| (e, sessions_between(cfg.date, e)))
            .collect();
        Self {
            now_ms: cfg.date.ist_unix_ms(cfg.start_minute),
            spot: cfg.spot,
            cfg,
            instruments,
            state,
            sessions_to,
            rng,
            cursor: 0,
        }
    }

    pub fn instruments(&self) -> &[Instrument] {
        &self.instruments
    }

    /// Exchange time of the last frame from `next_frame`, Unix ms; the starting clock
    /// before the first frame.
    pub fn now_ms(&self) -> i64 {
        self.now_ms
    }

    pub fn spot(&self) -> f64 {
        self.spot
    }

    fn t_years(&self, expiry: Date) -> f64 {
        let minute = ist_minute_of_day(self.now_ms);
        t_years(minutes_to_expiry(minute, self.sessions_to[&expiry]))
    }

    fn ltpc(&mut self, ltp: f64, lot: i32) -> Ltpc {
        Ltpc {
            ltp,
            ltt: self.now_ms - self.rng.range(0, self.cfg.step_ms.max(1) - 1),
            ltq: i64::from(lot.max(1)) * self.rng.range(1, 20),
            cp: 0.0,
            iep: None,
        }
    }

    fn quote_feed(&mut self, idx: usize, fair: f64) -> MarketFullFeed {
        let ins = &self.instruments[idx];
        let (tick, lot_size, is_option) =
            (ins.tick_size.max(0.05), ins.lot_size, ins.kind.is_option());
        let lot = i64::from(lot_size.max(1));
        let ltp = round_to(fair.max(tick), tick);
        let half = round_to((ltp * 0.0025).max(tick), tick);
        let levels = (0..DEPTH)
            .map(|l| {
                let off = half + tick * l as f64;
                Quote {
                    bid_q: lot * self.rng.range(1, 40),
                    bid_p: round_to((ltp - off).max(0.0), tick),
                    ask_q: lot * self.rng.range(1, 40),
                    ask_p: round_to(ltp + off, tick),
                }
            })
            .collect();
        let ltpc = self.ltpc(ltp, lot_size);
        let st = &mut self.state[idx];
        st.cum_volume += ltpc.ltq;
        if is_option {
            let drift = self.rng.range(-3, 4) as f64 * lot as f64;
            st.oi_units = (st.oi_units + drift).max(0.0);
        }
        MarketFullFeed {
            ltpc: Some(ltpc),
            market_level: Some(MarketLevel {
                bid_ask_quote: levels,
            }),
            vtt: st.cum_volume,
            oi: st.oi_units,
            ..Default::default()
        }
    }

    fn feed_for(&mut self, idx: usize) -> FeedUnion {
        let ins = self.instruments[idx].clone();
        let full = match ins.kind {
            Kind::Idx => {
                let ltpc = self.ltpc(round_to(self.spot, 0.05), 0);
                FullFeedUnion::IndexFf(IndexFullFeed {
                    ltpc: Some(Ltpc { ltq: 0, ..ltpc }),
                    market_ohlc: None,
                })
            }
            Kind::Fut => {
                let t = self.t_years(ins.expiry.expect("future has expiry"));
                let fair = self.spot * (self.cfg.r * t).exp();
                FullFeedUnion::MarketFf(self.quote_feed(idx, fair))
            }
            Kind::Ce | Kind::Pe => {
                let (e, k) = (
                    ins.expiry.expect("option expiry"),
                    ins.strike.expect("strike"),
                );
                let t = self.t_years(e);
                let f = self.spot * (self.cfg.r * t).exp();
                let m = (k / f).ln();
                let sigma = self.cfg.vol * (1.0 + 4.0 * m * m - 0.3 * m);
                let opt = if ins.kind == Kind::Ce {
                    OptType::Call
                } else {
                    OptType::Put
                };
                let fair = price(f, k, t, self.cfg.r, sigma, opt);
                FullFeedUnion::MarketFf(self.quote_feed(idx, fair))
            }
        };
        FeedUnion::FullFeed(FullFeed {
            full_feed_union: Some(full),
        })
    }

    /// Advance the clock by one step and return the next frame.
    pub fn next_frame(&mut self) -> FeedResponse {
        self.now_ms += self.cfg.step_ms;
        let dt = self.cfg.step_ms as f64 / 60_000.0 / MINUTES_PER_YEAR;
        self.spot *= (self.cfg.vol * dt.sqrt() * self.rng.normal()).exp();

        let n = self.instruments.len();
        let mut picked: Vec<usize> = self
            .instruments
            .iter()
            .position(|i| i.kind == Kind::Idx)
            .into_iter()
            .collect();
        while picked.len() < self.cfg.per_frame.min(n) {
            let i = self.cursor % n;
            self.cursor += 1 + self.rng.range(0, 2) as usize;
            if !picked.contains(&i) {
                picked.push(i);
            }
        }
        let mut feeds = HashMap::with_capacity(picked.len());
        for i in picked {
            let feed = Feed {
                feed_union: Some(self.feed_for(i)),
                request_mode: RequestMode::FullD5 as i32,
            };
            feeds.insert(self.instruments[i].key.clone(), feed);
        }
        FeedResponse {
            r#type: Type::LiveFeed as i32,
            feeds,
            current_ts: self.now_ms + 5,
            market_info: None,
        }
    }
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

    fn market() -> Market {
        let date = Date::from_ymd(2026, 10, 19);
        Market::new(
            MarketConfig::new(date),
            universe(date, 25_000.0, 2, 5.0, 50.0),
        )
    }

    #[test]
    fn same_seed_same_bytes() {
        use prost::Message;
        let (mut a, mut b) = (market(), market());
        for _ in 0..50 {
            let (fa, fb) = (a.next_frame(), b.next_frame());
            assert_eq!(fa, fb);
            // map order differs between runs, but the decoded content does not
            let (ua, ub) = (crate::updates(&fa), crate::updates(&fb));
            assert_eq!(ua, ub);
            assert!(!fa.encode_to_vec().is_empty());
        }
    }

    #[test]
    fn frames_are_plausible() {
        let mut m = market();
        let start = m.now_ms();
        for step in 1..=400 {
            let frame = m.next_frame();
            assert_eq!(m.now_ms(), start + 250 * step);
            let us = crate::updates(&frame);
            assert_eq!(us.len(), 40);
            let idx = us
                .iter()
                .find(|u| u.instrument_key == NIFTY_INDEX_KEY)
                .unwrap();
            assert!((idx.ltp - m.spot()).abs() < 0.05);
            for u in us.iter().filter(|u| u.instrument_key != NIFTY_INDEX_KEY) {
                assert!(u.ltt_ms <= m.now_ms() && u.ltt_ms > m.now_ms() - 250);
                assert!(u.ltp >= 0.05, "{u:?}");
                assert!(u.bid_px[0] < u.ask_px[0], "{u:?}");
                assert!(u.bid_px.windows(2).all(|w| w[0] >= w[1]));
                assert!(u.ask_px.windows(2).all(|w| w[0] < w[1]));
                assert_eq!(u.ltq % 65, 0);
            }
        }
        // 100 s of a 12% vol walk stays within a few tens of points
        assert!((m.spot() - 25_000.0).abs() < 300.0);
    }

    #[test]
    fn option_prices_follow_moneyness() {
        let mut m = market();
        let mut last: HashMap<String, f64> = HashMap::new();
        for _ in 0..200 {
            for u in crate::updates(&m.next_frame()) {
                last.insert(u.instrument_key, u.ltp);
            }
        }
        let u = m.instruments().to_vec();
        let price_of = |k: f64, kind: Kind| {
            let i = u
                .iter()
                .find(|i| {
                    i.strike == Some(k)
                        && i.kind == kind
                        && i.expiry == Some(Date::from_ymd(2026, 10, 20))
                })
                .unwrap();
            last[&i.key]
        };
        assert!(price_of(24_500.0, Kind::Ce) > price_of(25_500.0, Kind::Ce));
        assert!(price_of(25_500.0, Kind::Pe) > price_of(24_500.0, Kind::Pe));
    }

    #[test]
    fn rng_is_stable() {
        let mut r = Rng::new(1);
        assert_eq!(r.next_u64(), 0x910a_2dec_8902_5cc1);
        let mean: f64 = (0..10_000).map(|_| r.normal()).sum::<f64>() / 10_000.0;
        assert!(mean.abs() < 0.05);
        assert!((0..1000).all(|_| (3..=5).contains(&r.range(3, 5))));
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
