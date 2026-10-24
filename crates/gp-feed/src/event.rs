//! The C1 merged event (docs/CONTRACTS.md): one de-duplicated feed update with its contract
//! metadata. Field order matches the Parquet columns and the replay record.

use crate::{DEPTH, Date, Instrument, Kind, Update};

/// `src_mask` bit for each capture node.
pub const NODE_A: u8 = 0b01;
pub const NODE_B: u8 = 0b10;

#[derive(Debug, Clone, PartialEq)]
pub struct Event {
    pub instrument_key: String,
    pub underlying: String,
    pub kind: Kind,
    /// None for the index.
    pub expiry: Option<Date>,
    /// Rupees; None for futures and the index.
    pub strike: Option<f64>,
    pub lot_size: i32,
    /// Exchange last-trade time, Unix ms.
    pub ltt_ms: i64,
    /// Earliest receive time across nodes, Unix ns.
    pub recv_ns: i64,
    pub ltp: f64,
    pub ltq: i64,
    pub cum_volume: i64,
    /// Open interest in contracts.
    pub oi: i64,
    pub bid_px: [f64; DEPTH],
    pub bid_qty: [i64; DEPTH],
    pub ask_px: [f64; DEPTH],
    pub ask_qty: [i64; DEPTH],
    /// Bit 0 = Node A, bit 1 = Node B.
    pub src_mask: u8,
}

/// Turns feed updates into C1 events.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Normalizer {
    /// The feed's OI is in index units (lot size × contracts) and is divided by the lot
    /// size. Unverified for the live feed; set by `[merge] oi_in_units`, see the
    /// "OI units" row in docs/SYSTEMS.md.
    pub oi_in_units: bool,
}

impl Default for Normalizer {
    fn default() -> Self {
        Self { oi_in_units: true }
    }
}

impl Normalizer {
    /// OI in contracts, rounded to the nearest whole contract.
    pub fn oi_contracts(&self, oi: f64, lot_size: i32) -> i64 {
        let oi = if oi.is_finite() { oi.max(0.0) } else { 0.0 };
        if self.oi_in_units && lot_size > 0 {
            (oi / f64::from(lot_size)).round() as i64
        } else {
            oi.round() as i64
        }
    }

    /// The C1 event for `update`, received at `recv_ns` by the nodes in `src_mask`.
    pub fn event(&self, update: &Update, ins: &Instrument, recv_ns: i64, src_mask: u8) -> Event {
        Event {
            instrument_key: update.instrument_key.clone(),
            underlying: ins.underlying.clone(),
            kind: ins.kind,
            expiry: ins.expiry,
            strike: ins.strike,
            lot_size: ins.lot_size,
            ltt_ms: update.ltt_ms,
            recv_ns,
            ltp: update.ltp,
            ltq: update.ltq,
            cum_volume: update.cum_volume,
            oi: self.oi_contracts(update.oi, ins.lot_size),
            bid_px: update.bid_px,
            bid_qty: update.bid_qty,
            ask_px: update.ask_px,
            ask_qty: update.ask_qty,
            src_mask,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ins() -> Instrument {
        Instrument {
            key: "NSE_FO|1".into(),
            underlying: "NIFTY".into(),
            kind: Kind::Pe,
            expiry: Some(Date::from_ymd(2026, 10, 20)),
            strike: Some(24_900.0),
            lot_size: 65,
            tick_size: 0.05,
            weekly: true,
        }
    }

    fn update() -> Update {
        Update {
            instrument_key: "NSE_FO|1".into(),
            ltt_ms: 10,
            ltp: 101.5,
            ltq: 65,
            cum_volume: 6_500,
            oi: 65.0 * 1_000.0 + 20.0,
            bid_px: [101.4, 101.35, 0.0, 0.0, 0.0],
            bid_qty: [130, 65, 0, 0, 0],
            ask_px: [101.6, 0.0, 0.0, 0.0, 0.0],
            ask_qty: [195, 0, 0, 0, 0],
            current_ts: 11,
        }
    }

    #[test]
    fn event_carries_metadata_and_contracts() {
        let e = Normalizer::default().event(&update(), &ins(), 123, NODE_B);
        assert_eq!(
            (e.kind, e.strike, e.lot_size),
            (Kind::Pe, Some(24_900.0), 65)
        );
        assert_eq!(e.expiry, Some(Date::from_ymd(2026, 10, 20)));
        assert_eq!((e.ltt_ms, e.recv_ns, e.src_mask), (10, 123, NODE_B));
        assert_eq!(e.oi, 1_000);
        assert_eq!((e.bid_qty[0], e.ask_px[0]), (130, 101.6));
    }

    #[test]
    fn oi_conversion() {
        let units = Normalizer { oi_in_units: true };
        let contracts = Normalizer { oi_in_units: false };
        assert_eq!(units.oi_contracts(650.0, 65), 10);
        assert_eq!(units.oi_contracts(650.0, 0), 650); // index: no lot
        assert_eq!(contracts.oi_contracts(650.4, 65), 650);
        assert_eq!(units.oi_contracts(f64::NAN, 65), 0);
        assert_eq!(units.oi_contracts(-5.0, 65), 0);
    }
}
