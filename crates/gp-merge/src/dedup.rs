//! Content de-duplication across nodes (V2.3).
//!
//! The feed has no sequence numbers, so two updates are the same when their content key
//! matches: (instrument, ltt_ms, cum_volume, oi, hash of L1–L5). The kept event has the
//! earliest receive time over all copies and the OR of their `src_mask` bits.

use std::collections::HashMap;

use gp_feed::Event;

/// FNV-1a over the bits of all five levels: stable across runs, machines and Rust versions.
pub fn depth_hash(e: &Event) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    let mut eat = |v: u64| {
        for b in v.to_le_bytes() {
            h ^= u64::from(b);
            h = h.wrapping_mul(0x0000_0100_0000_01b3);
        }
    };
    for i in 0..gp_feed::DEPTH {
        eat(e.bid_px[i].to_bits());
        eat(e.bid_qty[i] as u64);
        eat(e.ask_px[i].to_bits());
        eat(e.ask_qty[i] as u64);
    }
    h
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct Key {
    instrument: String,
    ltt_ms: i64,
    cum_volume: i64,
    oi: i64,
    depth: u64,
}

impl Key {
    fn of(e: &Event) -> Self {
        Self {
            instrument: e.instrument_key.clone(),
            ltt_ms: e.ltt_ms,
            cum_volume: e.cum_volume,
            oi: e.oi,
            depth: depth_hash(e),
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize)]
pub struct DedupStats {
    pub input: u64,
    pub output: u64,
    /// Kept events by the nodes that carried them.
    pub only_a: u64,
    pub only_b: u64,
    pub both: u64,
}

/// De-duplicate events; the result keeps first-seen order (sort it for replay afterwards).
pub fn dedup(events: impl IntoIterator<Item = Event>) -> (Vec<Event>, DedupStats) {
    let mut stats = DedupStats::default();
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
        match e.src_mask {
            0b01 => stats.only_a += 1,
            0b10 => stats.only_b += 1,
            _ => stats.both += 1,
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
        let (out, stats) = dedup(a.into_iter().chain(b));
        assert_eq!(out.len(), 5);
        assert_eq!((out[0].recv_ns, out[0].src_mask), (450, 0b11)); // earliest, both
        assert_eq!(
            stats,
            DedupStats {
                input: 6,
                output: 5,
                only_a: 2,
                only_b: 2,
                both: 1
            }
        );
    }

    #[test]
    fn repeats_within_a_node_collapse() {
        let (out, stats) = dedup(vec![ev("x", 1, 10, NODE_A), ev("x", 1, 20, NODE_A)]);
        assert_eq!((out.len(), out[0].recv_ns, stats.only_a), (1, 10, 1));
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
        ] {
            let mut e = base.clone();
            f(&mut e);
            changed.push(e);
        }
        let (out, _) = dedup(std::iter::once(base).chain(changed));
        assert_eq!(out.len(), 6);
        assert_ne!(depth_hash(&out[0]), depth_hash(&out[4]));
    }
}
