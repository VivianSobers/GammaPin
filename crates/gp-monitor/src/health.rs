//! Data health for C2: exchange-to-snapshot lag over the last 60 s, gaps, and node status.

use std::collections::{BTreeMap, VecDeque};

use crate::snapshot::NodeState;

/// Lag quantiles cover this much stream time.
pub const WINDOW_MS: i64 = 60_000;
/// A node is down when none of its events arrived for this long.
pub const NODE_TIMEOUT_MS: i64 = 5_000;

const FINE: usize = 1_000; // 1 ms buckets below 1 s
const COARSE: usize = 900; // 10 ms buckets from 1 s to 10 s
const BUCKETS: usize = FINE + COARSE + 1; // last one: 10 s and over

/// Latency histogram: 1 ms resolution to 1 s, 10 ms to 10 s, then one overflow bucket.
#[derive(Debug, Clone, PartialEq)]
pub struct Histogram {
    counts: Vec<u64>,
    total: u64,
}

impl Default for Histogram {
    fn default() -> Self {
        Self {
            counts: vec![0; BUCKETS],
            total: 0,
        }
    }
}

fn bucket(ms: f64) -> usize {
    let ms = if ms.is_finite() { ms.max(0.0) } else { 0.0 };
    if ms < FINE as f64 {
        ms as usize
    } else if ms < 10_000.0 {
        FINE + ((ms - 1_000.0) / 10.0) as usize
    } else {
        BUCKETS - 1
    }
}

/// Upper edge of a bucket, ms. The overflow bucket reports 10 s.
fn upper(b: usize) -> f64 {
    if b < FINE {
        (b + 1) as f64
    } else {
        (1_000 + (b - FINE + 1) * 10).min(10_000) as f64
    }
}

impl Histogram {
    pub fn record(&mut self, ms: f64) {
        self.counts[bucket(ms)] += 1;
        self.total += 1;
    }

    pub fn add(&mut self, other: &Self) {
        for (a, b) in self.counts.iter_mut().zip(&other.counts) {
            *a += b;
        }
        self.total += other.total;
    }

    pub fn count(&self) -> u64 {
        self.total
    }

    /// The bucket edge at or below which a fraction `q` of samples fall.
    pub fn quantile(&self, q: f64) -> Option<f64> {
        if self.total == 0 {
            return None;
        }
        let rank = ((q * self.total as f64).ceil() as u64).clamp(1, self.total);
        let mut seen = 0;
        for (b, &c) in self.counts.iter().enumerate() {
            seen += c;
            if seen >= rank {
                return Some(upper(b));
            }
        }
        Some(upper(BUCKETS - 1))
    }
}

/// One histogram per second of stream time, for the last [`WINDOW_MS`].
#[derive(Debug, Default)]
pub struct LagWindow {
    secs: VecDeque<(i64, Histogram)>,
}

impl LagWindow {
    pub fn record(&mut self, ts_ms: i64, lag_ms: f64) {
        let sec = ts_ms.div_euclid(1_000);
        match self.secs.back_mut() {
            Some((s, h)) if *s == sec => h.record(lag_ms),
            _ => {
                let mut h = Histogram::default();
                h.record(lag_ms);
                self.secs.push_back((sec, h));
            }
        }
    }

    /// p50 and p99 over the window ending at `now_ms`; zeros before any sample.
    pub fn quantiles(&mut self, now_ms: i64) -> (f64, f64) {
        let oldest = (now_ms - WINDOW_MS).div_euclid(1_000);
        while self.secs.front().is_some_and(|(s, _)| *s <= oldest) {
            self.secs.pop_front();
        }
        let mut all = Histogram::default();
        for (_, h) in &self.secs {
            all.add(h);
        }
        (
            all.quantile(0.50).unwrap_or(0.0),
            all.quantile(0.99).unwrap_or(0.0),
        )
    }
}

/// When each node was last heard from, by `src_mask` bit.
#[derive(Debug, Clone)]
pub struct Nodes {
    names: Vec<String>,
    last_ms: Vec<Option<i64>>,
}

impl Nodes {
    pub fn new(names: &[String]) -> Self {
        Self {
            names: names.to_vec(),
            last_ms: vec![None; names.len()],
        }
    }

    pub fn seen(&mut self, src_mask: u8, ts_ms: i64) {
        for (bit, last) in self.last_ms.iter_mut().enumerate() {
            if src_mask & (1 << bit) != 0 {
                *last = Some(last.map_or(ts_ms, |t| t.max(ts_ms)));
            }
        }
    }

    pub fn states(&self, now_ms: i64) -> BTreeMap<String, NodeState> {
        self.names
            .iter()
            .zip(&self.last_ms)
            .map(|(n, t)| {
                let up = t.is_some_and(|t| now_ms - t <= NODE_TIMEOUT_MS);
                (n.clone(), if up { NodeState::Up } else { NodeState::Down })
            })
            .collect()
    }
}

/// `GAP_START` times (Unix ms) across nodes; counted up to the snapshot time, so a replay
/// shows the gaps as they happened.
#[derive(Debug, Clone, Default)]
pub struct Gaps {
    starts: Vec<i64>,
}

impl Gaps {
    pub fn add(&mut self, start_ms: i64) {
        let at = self.starts.partition_point(|&t| t <= start_ms);
        self.starts.insert(at, start_ms);
    }

    pub fn until(&self, now_ms: i64) -> u64 {
        self.starts.partition_point(|&t| t <= now_ms) as u64
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn histogram_quantiles() {
        let mut h = Histogram::default();
        assert_eq!(h.quantile(0.5), None);
        for ms in 0..100 {
            h.record(ms as f64);
        }
        assert_eq!(h.quantile(0.5), Some(50.0));
        assert_eq!(h.quantile(0.99), Some(99.0));
        h.record(2_345.0);
        h.record(1e9);
        h.record(-5.0);
        assert_eq!(h.count(), 103);
        assert_eq!(h.quantile(1.0), Some(10_000.0));
        let mut g = Histogram::default();
        g.record(2_345.0);
        assert_eq!(g.quantile(0.5), Some(2_350.0));
    }

    #[test]
    fn window_forgets_old_seconds() {
        let mut w = LagWindow::default();
        for i in 0..10 {
            w.record(1_000 * i, 500.0);
        }
        w.record(100_000, 20.0);
        assert_eq!(w.quantiles(100_000), (21.0, 21.0));
        let mut w = LagWindow::default();
        w.record(0, 5.0);
        w.record(30_000, 900.0);
        assert_eq!(w.quantiles(30_000), (6.0, 901.0));
        assert_eq!(w.quantiles(500_000), (0.0, 0.0));
    }

    #[test]
    fn node_states_and_gaps() {
        let names = vec!["A".to_string(), "B".to_string()];
        let mut n = Nodes::new(&names);
        assert_eq!(n.states(0)["A"], NodeState::Down);
        n.seen(0b11, 1_000);
        n.seen(0b01, 9_000);
        let s = n.states(9_000);
        assert_eq!((s["A"], s["B"]), (NodeState::Up, NodeState::Down));

        let mut g = Gaps::default();
        g.add(50);
        g.add(10);
        g.add(30);
        assert_eq!((g.until(5), g.until(30), g.until(99)), (0, 2, 3));
    }
}
