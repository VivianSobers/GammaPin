//! The Upstox instrument master: Nifty options, futures and the index.
//!
//! Source: `https://assets.upstox.com/market-quote/instruments/exchange/NSE.json.gz`, public and
//! without login. Units, as checked against the real file in `research/instruments.py`:
//! `expiry` is epoch ms at 23:59:59 IST of the expiry day, `tick_size` is in paise,
//! `strike_price` is in rupees and `lot_size` in index units per contract.

use std::collections::HashMap;
use std::io::Read;
use std::path::Path;

use serde::Deserialize;

use crate::Date;

pub const NIFTY_INDEX_KEY: &str = "NSE_INDEX|Nifty 50";
pub const NIFTY: &str = "NIFTY";
/// Rupees per `tick_size` unit.
const TICK_UNIT: f64 = 0.01;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Kind {
    Ce,
    Pe,
    Fut,
    Idx,
}

impl Kind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Ce => "CE",
            Self::Pe => "PE",
            Self::Fut => "FUT",
            Self::Idx => "IDX",
        }
    }

    pub fn is_option(self) -> bool {
        matches!(self, Self::Ce | Self::Pe)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Instrument {
    pub key: String,
    pub underlying: String,
    pub kind: Kind,
    /// None for the index.
    pub expiry: Option<Date>,
    /// Rupees; None for futures and the index.
    pub strike: Option<f64>,
    /// Index units per contract; 0 for the index.
    pub lot_size: i32,
    /// Rupees.
    pub tick_size: f64,
    pub weekly: bool,
}

#[derive(Deserialize)]
struct Raw {
    segment: Option<String>,
    name: Option<String>,
    instrument_type: Option<String>,
    instrument_key: String,
    underlying_key: Option<String>,
    expiry: Option<i64>,
    strike_price: Option<f64>,
    lot_size: Option<i64>,
    tick_size: Option<f64>,
    weekly: Option<bool>,
}

#[derive(Debug, thiserror::Error)]
pub enum MasterError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
    #[error("{0}: Nifty contract without expiry, lot size or (for an option) strike")]
    Incomplete(String),
}

fn convert(r: Raw) -> Result<Option<Instrument>, MasterError> {
    if r.instrument_key == NIFTY_INDEX_KEY {
        return Ok(Some(Instrument {
            key: r.instrument_key,
            underlying: NIFTY.into(),
            kind: Kind::Idx,
            expiry: None,
            strike: None,
            lot_size: 0,
            tick_size: r.tick_size.unwrap_or(0.0) * TICK_UNIT,
            weekly: false,
        }));
    }
    let nifty = r.segment.as_deref() == Some("NSE_FO")
        && r.name.as_deref() == Some(NIFTY)
        && r.underlying_key.as_deref() == Some(NIFTY_INDEX_KEY);
    let kind = match r.instrument_type.as_deref() {
        Some("CE") => Kind::Ce,
        Some("PE") => Kind::Pe,
        Some("FUT") => Kind::Fut,
        _ => return Ok(None),
    };
    if !nifty {
        return Ok(None);
    }
    let (Some(expiry), Some(lot)) = (r.expiry, r.lot_size) else {
        return Err(MasterError::Incomplete(r.instrument_key));
    };
    let strike = match (kind.is_option(), r.strike_price) {
        (false, _) => None,
        (true, Some(k)) => Some(k),
        (true, None) => return Err(MasterError::Incomplete(r.instrument_key)),
    };
    Ok(Some(Instrument {
        strike,
        key: r.instrument_key,
        underlying: NIFTY.into(),
        kind,
        expiry: Some(Date::ist_from_unix_ms(expiry)),
        lot_size: lot as i32,
        tick_size: (r.tick_size.unwrap_or(5.0) * TICK_UNIT * 1e4).round() / 1e4,
        weekly: r.weekly.unwrap_or(false),
    }))
}

/// Nifty instruments in a master JSON array: the index first, then by (expiry, strike, kind).
pub fn parse_master(json: &[u8]) -> Result<Vec<Instrument>, MasterError> {
    let raw: Vec<Raw> = serde_json::from_slice(json)?;
    let mut out = Vec::new();
    for r in raw {
        out.extend(convert(r)?);
    }
    out.sort_by(|a, b| {
        (a.expiry, a.strike.unwrap_or(0.0), a.kind)
            .partial_cmp(&(b.expiry, b.strike.unwrap_or(0.0), b.kind))
            .expect("finite strikes")
    });
    Ok(out)
}

/// Read a master file, gzip-compressed when it ends in `.gz`.
pub fn read_master(path: &Path) -> Result<Vec<Instrument>, MasterError> {
    let raw = std::fs::read(path)?;
    if path.extension().is_some_and(|e| e == "gz") {
        let mut json = Vec::new();
        flate2::read::GzDecoder::new(&raw[..]).read_to_end(&mut json)?;
        parse_master(&json)
    } else {
        parse_master(&raw)
    }
}

/// Instruments by key, for normalizing feed updates.
#[derive(Debug, Clone, Default)]
pub struct Master {
    by_key: HashMap<String, Instrument>,
}

impl Master {
    pub fn new(instruments: impl IntoIterator<Item = Instrument>) -> Self {
        Self {
            by_key: instruments
                .into_iter()
                .map(|i| (i.key.clone(), i))
                .collect(),
        }
    }

    pub fn get(&self, key: &str) -> Option<&Instrument> {
        self.by_key.get(key)
    }

    pub fn len(&self) -> usize {
        self.by_key.len()
    }

    pub fn is_empty(&self) -> bool {
        self.by_key.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> Vec<u8> {
        std::fs::read(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/instruments_nse.json"
        ))
        .unwrap()
    }

    #[test]
    fn keeps_nifty_options_futures_and_index() {
        let ins = parse_master(&fixture()).unwrap();
        let keys: Vec<_> = ins.iter().map(|i| i.key.as_str()).collect();
        assert_eq!(
            keys,
            [
                "NSE_INDEX|Nifty 50",
                "NSE_FO|90001",
                "NSE_FO|90002",
                "NSE_FO|90003",
                "NSE_FO|90006",
                "NSE_FO|90004",
                "NSE_FO|90005",
            ]
        );
        let c = &ins[3];
        assert_eq!(c.kind, Kind::Ce);
        assert_eq!(c.expiry, Some(Date::from_ymd(2026, 10, 20)));
        assert_eq!(
            (c.strike, c.lot_size, c.tick_size),
            (Some(25050.0), 65, 0.05)
        );
        assert!(c.weekly);
        let fut = &ins[4];
        assert_eq!((fut.kind, fut.strike), (Kind::Fut, None));
        assert_eq!(fut.expiry, Some(Date::from_ymd(2026, 10, 27)));
        let idx = &ins[0];
        assert_eq!((idx.kind, idx.expiry, idx.lot_size), (Kind::Idx, None, 0));
    }

    #[test]
    fn gzip_and_plain_agree() {
        let dir = tempfile::tempdir().unwrap();
        let plain = dir.path().join("NSE.json");
        let gz = dir.path().join("NSE.json.gz");
        std::fs::write(&plain, fixture()).unwrap();
        let mut enc = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        std::io::Write::write_all(&mut enc, &fixture()).unwrap();
        std::fs::write(&gz, enc.finish().unwrap()).unwrap();
        assert_eq!(read_master(&plain).unwrap(), read_master(&gz).unwrap());
    }

    #[test]
    fn master_lookup_and_bad_rows() {
        let m = Master::new(parse_master(&fixture()).unwrap());
        assert_eq!(m.len(), 7);
        assert_eq!(m.get("NSE_FO|90005").unwrap().kind, Kind::Pe);
        assert!(m.get("NSE_FO|90007").is_none()); // BANKNIFTY

        let missing = br#"[{"segment":"NSE_FO","name":"NIFTY","instrument_type":"CE",
            "instrument_key":"NSE_FO|1","underlying_key":"NSE_INDEX|Nifty 50"}]"#;
        assert!(matches!(
            parse_master(missing),
            Err(MasterError::Incomplete(k)) if k == "NSE_FO|1"
        ));
        assert!(parse_master(b"{").is_err());
    }
}
