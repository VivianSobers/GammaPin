//! The replay log, `GPR1`: C1 events in replay order, laid out for memory-mapped reading.
//!
//! ```text
//! header   magic "GPR1" · version u32 = 1 · n_instruments u32 · n_events u64 · crc32c u32
//!          · table_len u32 (table plus its padding)          (28 bytes, little-endian)
//! table    per instrument: key (u16 len + UTF-8) · underlying (u16 len + UTF-8) · kind u8
//!          · expiry i32 (i32::MIN = none) · strike f64 (NaN = none) · lot_size i32
//! padding  zeros to an 8-byte boundary, counted in table_len
//! events   n_events fixed 216-byte records:
//!          instrument u32 · src_mask u8 · 3 zero bytes · ltt_ms i64 · recv_ns i64 · ltp f64
//!          · ltq i64 · cum_volume i64 · oi i64 · bid_px [f64; 5] · bid_qty [i64; 5]
//!          · ask_px [f64; 5] · ask_qty [i64; 5]
//! ```
//!
//! `crc32c` covers everything after the header. Floats are stored bit-exactly.

use std::collections::HashMap;
use std::io::Write;
use std::path::Path;

use gp_feed::{DEPTH, Date, Event, Kind};

pub const MAGIC: &[u8; 4] = b"GPR1";
pub const VERSION: u32 = 1;
pub const HEADER_LEN: usize = 28;
pub const EVENT_LEN: usize = 216;

/// Contract metadata shared by every event of one instrument.
#[derive(Debug, Clone, PartialEq)]
pub struct Meta {
    pub instrument_key: String,
    pub underlying: String,
    pub kind: Kind,
    pub expiry: Option<Date>,
    pub strike: Option<f64>,
    pub lot_size: i32,
}

impl Meta {
    pub fn of(e: &Event) -> Self {
        Self {
            instrument_key: e.instrument_key.clone(),
            underlying: e.underlying.clone(),
            kind: e.kind,
            expiry: e.expiry,
            strike: e.strike,
            lot_size: e.lot_size,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum FormatError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("not a GPR1 replay log")]
    BadMagic,
    #[error("unsupported replay log version {0}")]
    Version(u32),
    #[error("replay log is truncated or has trailing bytes")]
    Length,
    #[error("replay log checksum mismatch")]
    Crc,
    #[error("bad instrument table: {0}")]
    Table(&'static str),
    #[error("event {0} refers to unknown instrument {1}")]
    Instrument(u64, u32),
    #[error("events are not in replay order (sort_for_replay)")]
    Order,
    #[error("instrument {0} has different contract metadata in different events")]
    Meta(String),
}

fn kind_code(k: Kind) -> u8 {
    match k {
        Kind::Ce => 0,
        Kind::Pe => 1,
        Kind::Fut => 2,
        Kind::Idx => 3,
    }
}

fn put_str(out: &mut Vec<u8>, s: &str) -> Result<(), FormatError> {
    let n = u16::try_from(s.len()).map_err(|_| FormatError::Table("string over 64 KiB"))?;
    out.extend_from_slice(&n.to_le_bytes());
    out.extend_from_slice(s.as_bytes());
    Ok(())
}

fn encode_event(out: &mut Vec<u8>, inst: u32, e: &Event) {
    let start = out.len();
    out.extend_from_slice(&inst.to_le_bytes());
    out.extend_from_slice(&[e.src_mask, 0, 0, 0]);
    for v in [e.ltt_ms, e.recv_ns] {
        out.extend_from_slice(&v.to_le_bytes());
    }
    out.extend_from_slice(&e.ltp.to_le_bytes());
    for v in [e.ltq, e.cum_volume, e.oi] {
        out.extend_from_slice(&v.to_le_bytes());
    }
    for i in 0..DEPTH {
        out.extend_from_slice(&e.bid_px[i].to_le_bytes());
    }
    for i in 0..DEPTH {
        out.extend_from_slice(&e.bid_qty[i].to_le_bytes());
    }
    for i in 0..DEPTH {
        out.extend_from_slice(&e.ask_px[i].to_le_bytes());
    }
    for i in 0..DEPTH {
        out.extend_from_slice(&e.ask_qty[i].to_le_bytes());
    }
    debug_assert_eq!(out.len() - start, EVENT_LEN);
}

/// Encode `events`, which must already be in replay order, as a complete replay log.
pub fn encode(events: &[Event]) -> Result<Vec<u8>, FormatError> {
    if !crate::is_replay_order(events) {
        return Err(FormatError::Order);
    }
    let mut index: HashMap<&str, (u32, Meta)> = HashMap::new();
    let mut table = Vec::new();
    let mut n_instruments = 0u32;
    let mut body = Vec::with_capacity(events.len() * EVENT_LEN);
    let mut ids = Vec::with_capacity(events.len());
    for e in events {
        let id = match index.get(e.instrument_key.as_str()) {
            Some((id, m)) => {
                if *m != Meta::of(e) {
                    return Err(FormatError::Meta(e.instrument_key.clone()));
                }
                *id
            }
            None => {
                let m = Meta::of(e);
                put_str(&mut table, &m.instrument_key)?;
                put_str(&mut table, &m.underlying)?;
                table.push(kind_code(m.kind));
                table.extend_from_slice(&m.expiry.map_or(i32::MIN, |d| d.0).to_le_bytes());
                table.extend_from_slice(&m.strike.unwrap_or(f64::NAN).to_le_bytes());
                table.extend_from_slice(&m.lot_size.to_le_bytes());
                index.insert(&e.instrument_key, (n_instruments, m));
                n_instruments += 1;
                n_instruments - 1
            }
        };
        ids.push(id);
    }
    while !(HEADER_LEN + table.len()).is_multiple_of(8) {
        table.push(0);
    }
    for (e, id) in events.iter().zip(ids) {
        encode_event(&mut body, id, e);
    }
    let table_len =
        u32::try_from(table.len()).map_err(|_| FormatError::Table("table over 4 GiB"))?;
    let crc = crc32c::crc32c_append(crc32c::crc32c(&table), &body);
    let mut out = Vec::with_capacity(HEADER_LEN + table.len() + body.len());
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&VERSION.to_le_bytes());
    out.extend_from_slice(&n_instruments.to_le_bytes());
    out.extend_from_slice(&(events.len() as u64).to_le_bytes());
    out.extend_from_slice(&crc.to_le_bytes());
    out.extend_from_slice(&table_len.to_le_bytes());
    out.extend_from_slice(&table);
    out.extend_from_slice(&body);
    Ok(out)
}

/// Write a replay log atomically (temporary file, fsync, rename).
pub fn write_replay(path: &Path, events: &[Event]) -> Result<(), FormatError> {
    let bytes = encode(events)?;
    let tmp = path.with_extension("gpr.tmp");
    {
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(&bytes)?;
        f.sync_all()?;
    }
    std::fs::rename(&tmp, path)?;
    Ok(())
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub fn event(key: &str, kind: Kind, ltt_ms: i64, recv_ns: i64, src_mask: u8) -> Event {
        Event {
            instrument_key: key.into(),
            underlying: "NIFTY".into(),
            kind,
            expiry: (kind != Kind::Idx).then(|| Date::from_ymd(2026, 10, 20)),
            strike: kind.is_option().then_some(25_000.0),
            lot_size: if kind == Kind::Idx { 0 } else { 65 },
            ltt_ms,
            recv_ns,
            ltp: 101.25 + ltt_ms as f64,
            ltq: 65,
            cum_volume: 1_000 + ltt_ms,
            oi: 77,
            bid_px: [101.2, 101.15, 101.1, 101.05, 101.0],
            bid_qty: [65, 130, 195, 260, 325],
            ask_px: [101.3, 101.35, 101.4, 101.45, 101.5],
            ask_qty: [65, 65, 65, 65, -1],
            src_mask,
        }
    }

    #[test]
    fn layout() {
        let evs = vec![
            event("NSE_FO|1", Kind::Ce, 1, 10, 1),
            event("NSE_INDEX|Nifty 50", Kind::Idx, 2, 20, 3),
            event("NSE_FO|1", Kind::Ce, 3, 30, 2),
        ];
        let b = encode(&evs).unwrap();
        assert_eq!(&b[..4], MAGIC);
        assert_eq!(u32::from_le_bytes(b[8..12].try_into().unwrap()), 2);
        assert_eq!(u64::from_le_bytes(b[12..20].try_into().unwrap()), 3);
        let table_len = u32::from_le_bytes(b[24..28].try_into().unwrap()) as usize;
        assert_eq!((HEADER_LEN + table_len) % 8, 0);
        assert_eq!(b.len(), HEADER_LEN + table_len + 3 * EVENT_LEN);
        let first = HEADER_LEN + table_len;
        assert_eq!(
            u32::from_le_bytes(b[first..first + 4].try_into().unwrap()),
            0
        );
        let third = first + 2 * EVENT_LEN;
        assert_eq!(
            u32::from_le_bytes(b[third..third + 4].try_into().unwrap()),
            0
        );
        assert_eq!(b[third + 4], 2);
    }

    #[test]
    fn refuses_unordered_events() {
        let evs = vec![event("a", Kind::Ce, 2, 0, 1), event("b", Kind::Ce, 1, 0, 1)];
        assert!(matches!(encode(&evs), Err(FormatError::Order)));
    }

    #[test]
    fn refuses_changing_metadata() {
        let mut later = event("a", Kind::Ce, 2, 0, 1);
        later.lot_size = 75;
        let evs = vec![event("a", Kind::Ce, 1, 0, 1), later];
        assert!(matches!(encode(&evs), Err(FormatError::Meta(k)) if k == "a"));
    }

    #[test]
    fn empty_log() {
        let b = encode(&[]).unwrap();
        assert_eq!(b.len(), HEADER_LEN + 4); // 28 + padding to 32
    }
}
