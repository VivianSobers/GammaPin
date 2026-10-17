//! The live engine: events in, a C2 snapshot out on request.
//!
//! Live and replay feed it the same way; only the snapshot clock differs (wall time live,
//! the stream's exchange time in replay).

use std::collections::BTreeMap;

use gp_feed::date::ist_minute_of_day;
use gp_feed::{Date, Event};
use gp_quant::time::{minutes_to_expiry, t_years};
use gp_quant::{ChainInput, FilterConfig, QuantError, Weights, analyze};

use crate::book::Book;
use crate::health::{Gaps, LagWindow, Nodes};
use crate::snapshot::{Accuracy, Health, SCHEMA_VERSION, Snapshot, conventions};

#[derive(Debug, Clone)]
pub struct EngineConfig {
    pub underlying: String,
    /// RBI 91-day T-bill, decimal.
    pub r: f64,
    /// C2 convention names and their weights; S0 first.
    pub conventions: Vec<(String, Weights)>,
    /// Exchange holidays between today and the expiry shown (weekends are always skipped).
    pub holidays: Vec<Date>,
    /// Capture nodes in `src_mask` bit order.
    pub nodes: Vec<String>,
    pub h1_accuracy: Option<BTreeMap<String, Option<Accuracy>>>,
}

impl Default for EngineConfig {
    fn default() -> Self {
        Self {
            underlying: "NIFTY".into(),
            r: 0.065,
            conventions: vec![("S0".into(), Weights::S0)],
            holidays: Vec::new(),
            nodes: vec!["A".into(), "B".into()],
            h1_accuracy: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SnapshotError {
    /// No index price yet.
    NoSpot,
    /// No option chain on or after today.
    NoChain,
    Quant(QuantError),
}

impl std::fmt::Display for SnapshotError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoSpot => f.write_str("no index price yet"),
            Self::NoChain => f.write_str("no option chain for today or later"),
            Self::Quant(e) => write!(f, "analyze: {e}"),
        }
    }
}

impl std::error::Error for SnapshotError {}

/// Trading sessions after `today` up to and including `expiry`.
pub fn sessions_after(today: Date, expiry: Date, holidays: &[Date]) -> u32 {
    let mut n = 0;
    let mut d = today.add_days(1);
    while d <= expiry {
        if d.weekday() < 5 && !holidays.contains(&d) {
            n += 1;
        }
        d = d.add_days(1);
    }
    n
}

pub struct Engine {
    cfg: EngineConfig,
    book: Book,
    /// `ltt_ms` of updates taken since the last snapshot.
    pending: Vec<i64>,
    lags: LagWindow,
    nodes: Nodes,
    gaps: Gaps,
    events: u64,
}

impl Engine {
    pub fn new(cfg: EngineConfig) -> Self {
        Self {
            nodes: Nodes::new(&cfg.nodes),
            cfg,
            book: Book::new(),
            pending: Vec::new(),
            lags: LagWindow::default(),
            gaps: Gaps::default(),
            events: 0,
        }
    }

    pub fn on_event(&mut self, e: &Event) {
        self.events += 1;
        self.nodes.seen(e.src_mask, e.recv_ns.div_euclid(1_000_000));
        if e.underlying == self.cfg.underlying && self.book.apply(e) {
            self.pending.push(e.ltt_ms);
        }
    }

    /// A `GAP_START` on any node.
    pub fn on_gap(&mut self, start_ms: i64) {
        self.gaps.add(start_ms);
    }

    pub fn events(&self) -> u64 {
        self.events
    }

    pub fn book(&self) -> &Book {
        &self.book
    }

    /// The C2 snapshot as of `ts_ms`. Lag samples for the updates taken since the last
    /// snapshot are counted even when this fails.
    pub fn snapshot(&mut self, ts_ms: i64, source: &str) -> Result<Snapshot, SnapshotError> {
        for ltt in self.pending.drain(..) {
            self.lags.record(ts_ms, (ts_ms - ltt) as f64);
        }
        let (spot, _) = self.book.spot().ok_or(SnapshotError::NoSpot)?;
        let today = Date::ist_from_unix_ms(ts_ms);
        let expiry = self
            .book
            .front_expiry(today)
            .ok_or(SnapshotError::NoChain)?;
        let rows = self.book.rows(expiry);
        let minutes = minutes_to_expiry(
            ist_minute_of_day(ts_ms),
            sessions_after(today, expiry, &self.cfg.holidays),
        );
        let t = t_years(minutes);
        let weights: Vec<Weights> = self.cfg.conventions.iter().map(|c| c.1).collect();
        let names: Vec<String> = self.cfg.conventions.iter().map(|c| c.0.clone()).collect();
        let a = analyze(
            &ChainInput {
                rows: &rows,
                r: self.cfg.r,
                t_years: t,
                lot_size: f64::from(self.book.lot_size(expiry).unwrap_or(0)),
                filter: FilterConfig::LIVE,
            },
            &weights,
        )
        .map_err(SnapshotError::Quant)?;
        let (p50, p99) = self.lags.quantiles(ts_ms);
        Ok(Snapshot {
            v: SCHEMA_VERSION,
            ts_ms,
            source: source.to_string(),
            underlying: self.cfg.underlying.clone(),
            expiry: expiry.to_string(),
            spot,
            forward: a.forward,
            t_years: t,
            conventions: conventions(&names, &a),
            strikes: rows.iter().map(|r| r.strike).collect(),
            h1_accuracy: self.cfg.h1_accuracy.clone(),
            health: Health {
                lag_ms_p50: p50,
                lag_ms_p99: p99,
                gaps_today: self.gaps.until(ts_ms),
                nodes: self.nodes.states(ts_ms),
                quotes_used: a.quotes_used,
                quotes_total: a.quotes_total,
            },
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::snapshot::{NodeState, Regime};
    use gp_feed::Master;
    use gp_feed::synth::{Market, MarketConfig, universe};
    use gp_feed::{Normalizer, updates};

    #[test]
    fn trading_sessions() {
        let fri = Date::from_ymd(2026, 10, 16);
        let tue = Date::from_ymd(2026, 10, 20);
        assert_eq!(sessions_after(fri, tue, &[]), 2);
        assert_eq!(sessions_after(fri, tue, &[tue]), 1);
        assert_eq!(sessions_after(tue, tue, &[]), 0);
    }

    /// Events from the synthetic market, as the merge would produce them.
    pub fn market_events(date: Date, frames: usize) -> Vec<Event> {
        let u = universe(date, 25_000.0, 2, 5.0, 50.0);
        let master = Master::new(u.clone());
        let mut m = Market::new(
            MarketConfig {
                per_frame: u.len(),
                ..MarketConfig::new(date)
            },
            u,
        );
        let norm = Normalizer::default();
        let mut out = Vec::new();
        for _ in 0..frames {
            let f = m.next_frame();
            let recv = (m.now_ms() + 30) * 1_000_000;
            for up in updates(&f) {
                let ins = master.get(&up.instrument_key).unwrap();
                out.push(norm.event(&up, ins, recv, 0b11));
            }
        }
        out
    }

    #[test]
    fn snapshot_from_a_synthetic_chain() {
        let date = Date::from_ymd(2026, 10, 19);
        let evs = market_events(date, 20);
        let mut eng = Engine::new(EngineConfig {
            conventions: vec![
                ("S0".into(), Weights::S0),
                (
                    "S1".into(),
                    Weights {
                        w_c: -0.3,
                        w_p: 0.6,
                    },
                ),
            ],
            ..EngineConfig::default()
        });
        assert_eq!(eng.snapshot(0, "replay"), Err(SnapshotError::NoSpot));
        for e in &evs {
            eng.on_event(e);
        }
        eng.on_gap(evs[0].ltt_ms - 1);
        let ts = evs.last().unwrap().ltt_ms + 1_000;
        let s = eng.snapshot(ts, "replay").unwrap();
        assert_eq!(s.expiry, "2026-10-20");
        assert!((s.spot - 25_000.0).abs() < 200.0);
        assert!(
            (s.forward - s.spot).abs() < 50.0,
            "{} {}",
            s.forward,
            s.spot
        );
        // Monday 09:20 to Tuesday 15:30: ~2 sessions
        assert!((s.t_years * 252.0 - 1.98).abs() < 0.05, "{}", s.t_years);
        assert_eq!(s.conventions.len(), 2);
        let s0 = &s.conventions["S0"];
        assert_eq!(s0.gex.len(), s.strikes.len());
        assert!((s0.gex.iter().sum::<f64>() - s0.net_gex).abs() < 1e-3 * s0.net_gex.abs());
        assert_eq!(s0.regime == Regime::LongGamma, s0.net_gex > 0.0);
        assert!(s.strikes.windows(2).all(|w| w[0] < w[1]));
        // one OTM quote per strike is used, out of both sides
        assert!(s.health.quotes_used > s.health.quotes_total / 4);
        assert_eq!(s.health.gaps_today, 1);
        assert_eq!(s.health.nodes["B"], NodeState::Up);
        assert!(s.health.lag_ms_p50 >= 1_000.0 && s.health.lag_ms_p99 <= 6_000.0);
        // a second snapshot with nothing new keeps the lag window
        let again = eng.snapshot(ts + 1_000, "replay").unwrap();
        assert_eq!(again.health.lag_ms_p99, s.health.lag_ms_p99);
        assert_eq!(eng.events(), evs.len() as u64);
    }
}
