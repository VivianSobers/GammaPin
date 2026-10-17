//! Replay order: (ltt_ms, recv_ns, node, seq). `node` is the lowest node bit in `src_mask`
//! (A = 0, B = 1) and `seq` the position in the input, so ties always break the same way.

use gp_feed::Event;

/// The node that first carried an event: the lowest set bit of `src_mask` (8 when none).
pub fn node(src_mask: u8) -> u32 {
    src_mask.trailing_zeros()
}

fn key(e: &Event) -> (i64, i64, u32) {
    (e.ltt_ms, e.recv_ns, node(e.src_mask))
}

/// Sort into replay order. Stable, so equal keys keep their input (`seq`) order.
pub fn sort_for_replay(events: &mut [Event]) {
    events.sort_by_key(key);
}

pub fn is_replay_order(events: &[Event]) -> bool {
    events.windows(2).all(|w| key(&w[0]) <= key(&w[1]))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::format::tests::event;
    use gp_feed::Kind;

    #[test]
    fn orders_by_ltt_then_recv_then_node_then_seq() {
        let mut evs = vec![
            event("a", Kind::Ce, 5, 1, 1),
            event("b", Kind::Ce, 1, 9, 2),
            event("c", Kind::Ce, 1, 9, 1),
            event("d", Kind::Ce, 1, 3, 2),
            event("e", Kind::Ce, 1, 9, 3),
            event("f", Kind::Ce, 1, 9, 1),
        ];
        assert!(!is_replay_order(&evs));
        sort_for_replay(&mut evs);
        let keys: Vec<&str> = evs.iter().map(|e| e.instrument_key.as_str()).collect();
        // d: earlier recv · c, e, f: node A (e is A+B) in input order · b: node B · a: later ltt
        assert_eq!(keys, ["d", "c", "e", "f", "b", "a"]);
        assert!(is_replay_order(&evs));
    }

    #[test]
    fn node_bits() {
        assert_eq!(node(0b01), 0);
        assert_eq!(node(0b10), 1);
        assert_eq!(node(0b11), 0);
        assert_eq!(node(0), 8);
    }
}
