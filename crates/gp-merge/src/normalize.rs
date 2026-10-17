//! A node's frames → C1 events, with counts of what could not be used.

use std::collections::BTreeMap;

use gp_feed::{Event, Master, Normalizer, decode_frame, updates};

use crate::read::NodeLog;

#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize)]
pub struct DecodeStats {
    pub frames: u64,
    /// Frames that are not a valid `FeedResponse`.
    pub bad_frames: u64,
    pub updates: u64,
    /// Updates for keys missing from the instrument master, by key.
    pub unknown_keys: BTreeMap<String, u64>,
}

/// Every update in the node's frames as a C1 event stamped with this node's bit.
pub fn node_events(log: &NodeLog, master: &Master, norm: Normalizer) -> (Vec<Event>, DecodeStats) {
    let mut stats = DecodeStats::default();
    let mut out = Vec::new();
    for (recv_ns, payload) in &log.frames {
        stats.frames += 1;
        let Ok(resp) = decode_frame(payload) else {
            stats.bad_frames += 1;
            continue;
        };
        for u in updates(&resp) {
            stats.updates += 1;
            match master.get(&u.instrument_key) {
                Some(ins) => out.push(norm.event(&u, ins, *recv_ns, 1 << log.bit)),
                None => *stats.unknown_keys.entry(u.instrument_key).or_default() += 1,
            }
        }
    }
    (out, stats)
}

#[cfg(test)]
mod tests {
    use super::*;
    use gp_feed::synth::{Market, MarketConfig, universe};
    use gp_feed::{Date, NIFTY_INDEX_KEY};
    use prost::Message;

    #[test]
    fn decodes_counts_and_stamps() {
        let date = Date::from_ymd(2026, 10, 19);
        let u = universe(date, 25_000.0, 1, 2.0, 50.0);
        let mut m = Market::new(MarketConfig::new(date), u.clone());
        // a master without the index: its updates become unknown keys
        let master = Master::new(u.into_iter().filter(|i| i.key != NIFTY_INDEX_KEY));
        let log = NodeLog {
            bit: 1,
            frames: vec![
                (100, m.next_frame().encode_to_vec()),
                (200, vec![0xFF, 0xFF]),
                (300, m.next_frame().encode_to_vec()),
            ],
            ..NodeLog::default()
        };
        let (events, stats) = node_events(&log, &master, Normalizer::default());
        assert_eq!((stats.frames, stats.bad_frames, stats.updates), (3, 1, 80));
        assert_eq!(stats.unknown_keys[NIFTY_INDEX_KEY], 2);
        assert_eq!(events.len(), 78);
        assert!(events.iter().all(|e| e.src_mask == 0b10));
        assert_eq!(events[0].recv_ns, 100);
        assert_eq!(events.last().unwrap().recv_ns, 300);
    }
}
