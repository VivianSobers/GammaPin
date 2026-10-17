//! Memory-mapped reading of a `GPR1` replay log. Everything is checked once at open
//! (length, checksum, instrument ids), so reading an event cannot fail afterwards.

use std::path::Path;

use gp_feed::{DEPTH, Date, Event, Kind};
use memmap2::Mmap;

use crate::format::{EVENT_LEN, FormatError, HEADER_LEN, MAGIC, Meta, VERSION};

/// The numeric part of an event; `instrument` indexes [`ReplayFile::metas`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Tick {
    pub instrument: u32,
    pub src_mask: u8,
    pub ltt_ms: i64,
    pub recv_ns: i64,
    pub ltp: f64,
    pub ltq: i64,
    pub cum_volume: i64,
    pub oi: i64,
    pub bid_px: [f64; DEPTH],
    pub bid_qty: [i64; DEPTH],
    pub ask_px: [f64; DEPTH],
    pub ask_qty: [i64; DEPTH],
}

enum Backing {
    Map(Mmap),
    Bytes(Vec<u8>),
}

impl Backing {
    fn bytes(&self) -> &[u8] {
        match self {
            Self::Map(m) => m,
            Self::Bytes(b) => b,
        }
    }
}

pub struct ReplayFile {
    backing: Backing,
    metas: Vec<Meta>,
    events_at: usize,
    len: u64,
}

fn kind_from(b: u8) -> Option<Kind> {
    Some(match b {
        0 => Kind::Ce,
        1 => Kind::Pe,
        2 => Kind::Fut,
        3 => Kind::Idx,
        _ => return None,
    })
}

struct Cursor<'a> {
    buf: &'a [u8],
    at: usize,
}

impl<'a> Cursor<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], FormatError> {
        let s = self
            .buf
            .get(self.at..self.at + n)
            .ok_or(FormatError::Table("truncated"))?;
        self.at += n;
        Ok(s)
    }
    fn u8(&mut self) -> Result<u8, FormatError> {
        Ok(self.take(1)?[0])
    }
    fn u16(&mut self) -> Result<u16, FormatError> {
        Ok(u16::from_le_bytes(self.take(2)?.try_into().expect("2")))
    }
    fn i32(&mut self) -> Result<i32, FormatError> {
        Ok(i32::from_le_bytes(self.take(4)?.try_into().expect("4")))
    }
    fn f64(&mut self) -> Result<f64, FormatError> {
        Ok(f64::from_le_bytes(self.take(8)?.try_into().expect("8")))
    }
    fn str(&mut self) -> Result<String, FormatError> {
        let n = self.u16()? as usize;
        String::from_utf8(self.take(n)?.to_vec()).map_err(|_| FormatError::Table("not UTF-8"))
    }
}

fn le_u32(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(b[at..at + 4].try_into().expect("4"))
}
fn le_i64(b: &[u8], at: usize) -> i64 {
    i64::from_le_bytes(b[at..at + 8].try_into().expect("8"))
}
fn le_f64(b: &[u8], at: usize) -> f64 {
    f64::from_le_bytes(b[at..at + 8].try_into().expect("8"))
}

impl ReplayFile {
    /// Map and validate a replay log.
    pub fn open(path: &Path) -> Result<Self, FormatError> {
        let file = std::fs::File::open(path)?;
        // SAFETY: replay logs are written once to a temporary name and renamed into place,
        // so a mapped file is never modified while it is read.
        let map = unsafe { Mmap::map(&file)? };
        Self::validate(Backing::Map(map))
    }

    /// Validate an in-memory replay log.
    pub fn from_bytes(bytes: Vec<u8>) -> Result<Self, FormatError> {
        Self::validate(Backing::Bytes(bytes))
    }

    fn validate(backing: Backing) -> Result<Self, FormatError> {
        let b = backing.bytes();
        if b.len() < HEADER_LEN || &b[..4] != MAGIC {
            return Err(FormatError::BadMagic);
        }
        let version = le_u32(b, 4);
        if version != VERSION {
            return Err(FormatError::Version(version));
        }
        let n_instruments = le_u32(b, 8);
        let len = u64::from_le_bytes(b[12..20].try_into().expect("8"));
        let crc = le_u32(b, 20);
        let table_len = le_u32(b, 24) as usize;
        let events_at = HEADER_LEN + table_len;
        let expected = (len as usize)
            .checked_mul(EVENT_LEN)
            .and_then(|n| n.checked_add(events_at))
            .ok_or(FormatError::Length)?;
        if b.len() != expected || !events_at.is_multiple_of(8) {
            return Err(FormatError::Length);
        }
        if crc32c::crc32c(&b[HEADER_LEN..]) != crc {
            return Err(FormatError::Crc);
        }
        let mut cur = Cursor {
            buf: &b[HEADER_LEN..events_at],
            at: 0,
        };
        let mut metas = Vec::with_capacity(n_instruments as usize);
        for _ in 0..n_instruments {
            let instrument_key = cur.str()?;
            let underlying = cur.str()?;
            let kind = kind_from(cur.u8()?).ok_or(FormatError::Table("unknown kind"))?;
            let expiry = cur.i32()?;
            let strike = cur.f64()?;
            metas.push(Meta {
                instrument_key,
                underlying,
                kind,
                expiry: (expiry != i32::MIN).then_some(Date(expiry)),
                strike: (!strike.is_nan()).then_some(strike),
                lot_size: cur.i32()?,
            });
        }
        for i in 0..len {
            let id = le_u32(b, events_at + i as usize * EVENT_LEN);
            if id >= n_instruments {
                return Err(FormatError::Instrument(i, id));
            }
        }
        Ok(Self {
            backing,
            metas,
            events_at,
            len,
        })
    }

    pub fn len(&self) -> u64 {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub fn metas(&self) -> &[Meta] {
        &self.metas
    }

    /// The raw bytes of event `i` (for hashing).
    pub fn event_bytes(&self, i: u64) -> &[u8] {
        let at = self.events_at + i as usize * EVENT_LEN;
        &self.backing.bytes()[at..at + EVENT_LEN]
    }

    /// Event `i`, numbers only. Panics when `i >= len()`.
    pub fn tick(&self, i: u64) -> Tick {
        assert!(i < self.len, "event {i} of {}", self.len);
        let b = self.backing.bytes();
        let at = self.events_at + i as usize * EVENT_LEN;
        let arr_f = |off: usize| std::array::from_fn(|k| le_f64(b, at + off + 8 * k));
        let arr_i = |off: usize| std::array::from_fn(|k| le_i64(b, at + off + 8 * k));
        Tick {
            instrument: le_u32(b, at),
            src_mask: b[at + 4],
            ltt_ms: le_i64(b, at + 8),
            recv_ns: le_i64(b, at + 16),
            ltp: le_f64(b, at + 24),
            ltq: le_i64(b, at + 32),
            cum_volume: le_i64(b, at + 40),
            oi: le_i64(b, at + 48),
            bid_px: arr_f(56),
            bid_qty: arr_i(96),
            ask_px: arr_f(136),
            ask_qty: arr_i(176),
        }
    }

    /// Event `i` with its contract metadata.
    pub fn event(&self, i: u64) -> Event {
        let t = self.tick(i);
        let m = &self.metas[t.instrument as usize];
        Event {
            instrument_key: m.instrument_key.clone(),
            underlying: m.underlying.clone(),
            kind: m.kind,
            expiry: m.expiry,
            strike: m.strike,
            lot_size: m.lot_size,
            ltt_ms: t.ltt_ms,
            recv_ns: t.recv_ns,
            ltp: t.ltp,
            ltq: t.ltq,
            cum_volume: t.cum_volume,
            oi: t.oi,
            bid_px: t.bid_px,
            bid_qty: t.bid_qty,
            ask_px: t.ask_px,
            ask_qty: t.ask_qty,
            src_mask: t.src_mask,
        }
    }

    pub fn events(&self) -> impl Iterator<Item = Event> + '_ {
        (0..self.len).map(|i| self.event(i))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::format::tests::event;
    use crate::format::{encode, write_replay};

    fn sample() -> Vec<Event> {
        let mut evs = vec![
            event("NSE_FO|1", Kind::Ce, 1, 10, 1),
            event("NSE_INDEX|Nifty 50", Kind::Idx, 2, 20, 3),
            event("NSE_FO|2", Kind::Fut, 3, 30, 2),
            event("NSE_FO|1", Kind::Ce, 4, 40, 3),
        ];
        evs[3].ltp = -0.0;
        evs[2].bid_px[4] = f64::MIN_POSITIVE;
        evs
    }

    #[test]
    fn round_trip_through_mmap() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("day.gpr");
        write_replay(&path, &sample()).unwrap();
        assert!(!dir.path().join("day.gpr.tmp").exists());
        let f = ReplayFile::open(&path).unwrap();
        assert_eq!(f.len(), 4);
        assert_eq!(f.metas().len(), 3);
        let back: Vec<Event> = f.events().collect();
        assert_eq!(back, sample());
        assert!(back[3].ltp.is_sign_negative()); // bit-exact
        assert_eq!(f.tick(1).instrument, 1);
        assert_eq!(f.metas()[1].expiry, None);
        assert_eq!(f.metas()[2].strike, None);
    }

    #[test]
    fn rejects_damage() {
        let good = encode(&sample()).unwrap();
        assert!(ReplayFile::from_bytes(good.clone()).is_ok());

        let mut b = good.clone();
        b[0] = b'X';
        assert!(matches!(
            ReplayFile::from_bytes(b),
            Err(FormatError::BadMagic)
        ));
        let mut b = good.clone();
        b[4] = 9;
        assert!(matches!(
            ReplayFile::from_bytes(b),
            Err(FormatError::Version(9))
        ));
        let b = good[..good.len() - 1].to_vec();
        assert!(matches!(
            ReplayFile::from_bytes(b),
            Err(FormatError::Length)
        ));
        let mut b = good.clone();
        *b.last_mut().unwrap() ^= 1;
        assert!(matches!(ReplayFile::from_bytes(b), Err(FormatError::Crc)));

        // a bad instrument id with a matching checksum
        let mut b = good.clone();
        let table_len = u32::from_le_bytes(b[24..28].try_into().unwrap()) as usize;
        let first = HEADER_LEN + table_len;
        b[first..first + 4].copy_from_slice(&7u32.to_le_bytes());
        let crc = crc32c::crc32c(&b[HEADER_LEN..]);
        b[20..24].copy_from_slice(&crc.to_le_bytes());
        assert!(matches!(
            ReplayFile::from_bytes(b),
            Err(FormatError::Instrument(0, 7))
        ));
    }

    #[test]
    fn empty_file() {
        let f = ReplayFile::from_bytes(encode(&[]).unwrap()).unwrap();
        assert!(f.is_empty());
        assert_eq!(f.events().count(), 0);
    }
}
