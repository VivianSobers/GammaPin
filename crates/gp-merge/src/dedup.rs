//! Content de-duplication across nodes (V2.3).
//!
//! The feed has no sequence numbers, so two updates are the same when their content key
//! matches: (instrument, `ltt_ms`, LTP, LTQ, cumulative volume, OI, all five depth levels).
//! The kept event has the earliest receive time over all copies and the OR of their
//! `src_mask` bits.

use std::collections::HashMap;

use gp_feed::{DEPTH, Event};

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct Key {
    instrument: String,
    ltt_ms: i64,
    ltp: u64,
    ltq: i64,
    cum_volume: i64,
    oi: i64,
    /// Bid price, bid quantity, ask price, ask quantity per level, as raw bits: compared
    /// exactly, with no hash to collide.
    depth: [[u64; 4]; DEPTH],
}

impl Key {
    fn of(e: &Event) -> Self {
        Self {
            instrument: e.instrument_key.clone(),
            ltt_ms: e.ltt_ms,
            ltp: e.ltp.to_bits(),
            ltq: e.ltq,
            cum_volume: e.cum_volume,
            oi: e.oi,
            depth: std::array::from_fn(|i| {
                [
                    e.bid_px[i].to_bits(),
                    e.bid_qty[i] as u64,
                    e.ask_px[i].to_bits(),
                    e.ask_qty[i] as u64,
                ]
            }),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize)]
pub struct DedupStats {
    pub input: u64,
    pub output: u64,
    /// Kept events carried by one node only, indexed by the node's `src_mask` bit.
    pub only: Vec<u64>,
    /// Kept events carried by more than one node.
    pub shared: u64,
}

/// De-duplicate events from `nodes` nodes; the result keeps first-seen order (sort it for
/// replay afterwards).
pub fn dedup(events: impl IntoIterator<Item = Event>, nodes: usize) -> (Vec<Event>, DedupStats) {
    let mut stats = DedupStats {
        only: vec![0; nodes],
        ..DedupStats::default()
    };
    let mut index: HashMap<Key, usize> = HashMap::new();
    let mut out: Vec<Event> = Vec::new();
    for e in events {
        stats.input += 1;
        match index.entry(Key::of(&e)) {
            std::collections::hash_map::Entry::Occupied(slot) => {
                let kept = &mut out[*slot.get()];
                kept.recv_ns = kept.recv_ns.min(e.recv_ns);
                kept.src_mask |= e.src_mask;
            }
            std::collections::hash_map::Entry::Vacant(slot) => {
                slot.insert(out.len());
                out.push(e);
            }
        }
    }
    stats.output = out.len() as u64;
    for e in &out {
        let bit = e.src_mask.trailing_zeros() as usize;
        if e.src_mask.is_power_of_two() && bit < nodes {
            stats.only[bit] += 1;
        } else {
            stats.shared += 1;
        }
    }
    (out, stats)
}

#[cfg(test)]
mod tests {
    use super::*;
    use gp_feed::{Date, Kind, NODE_A, NODE_B};

    fn ev(key: &str, ltt: i64, recv: i64, mask: u8) -> Event {
        Event {
            instrument_key: key.into(),
            underlying: "NIFTY".into(),
            kind: Kind::Ce,
            expiry: Some(Date::from_ymd(2026, 10, 20)),
            strike: Some(25_000.0),
            lot_size: 65,
            ltt_ms: ltt,
            recv_ns: recv,
            ltp: 100.0,
            ltq: 65,
            cum_volume: 6_500,
            oi: 10,
            bid_px: [99.95, 99.9, 0.0, 0.0, 0.0],
            bid_qty: [65, 130, 0, 0, 0],
            ask_px: [100.05, 0.0, 0.0, 0.0, 0.0],
            ask_qty: [65, 0, 0, 0, 0],
            src_mask: mask,
        }
    }

    #[test]
    fn merges_copies_from_both_nodes() {
        let a = vec![
            ev("x", 1, 500, NODE_A),
            ev("x", 2, 600, NODE_A),
            ev("y", 1, 700, NODE_A),
        ];
        let mut b = vec![
            ev("x", 1, 450, NODE_B),
            ev("x", 2, 650, NODE_B),
            ev("z", 3, 800, NODE_B),
        ];
        b[1].bid_qty[1] = 195; // depth changed: a different update
        let (out, stats) = dedup(a.into_iter().chain(b), 2);
        assert_eq!(out.len(), 5);
        assert_eq!((out[0].recv_ns, out[0].src_mask), (450, 0b11)); // earliest, both
        assert_eq!(
            stats,
            DedupStats {
                input: 6,
                output: 5,
                only: vec![2, 2],
                shared: 1
            }
        );
    }

    #[test]
    fn repeats_within_a_node_collapse() {
        let (out, stats) = dedup(vec![ev("x", 1, 10, NODE_A), ev("x", 1, 20, NODE_A)], 2);
        assert_eq!((out.len(), out[0].recv_ns, stats.only[0]), (1, 10, 1));
    }

    #[test]
    fn key_fields_matter() {
        let base = ev("x", 1, 10, NODE_A);
        let mut changed = Vec::new();
        for f in [
            |e: &mut Event| e.ltt_ms += 1,
            |e: &mut Event| e.cum_volume += 65,
            |e: &mut Event| e.oi += 1,
            |e: &mut Event| e.ask_px[4] = 100.25,
            |e: &mut Event| e.instrument_key.push('2'),
            |e: &mut Event| e.ltp += 0.05,
            |e: &mut Event| e.ltq += 65,
        ] {
            let mut e = base.clone();
            f(&mut e);
            changed.push(e);
        }
        let (out, _) = dedup(std::iter::once(base).chain(changed), 1);
        assert_eq!(out.len(), 8);
    }
}
