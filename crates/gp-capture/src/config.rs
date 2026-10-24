//! The `[node]`, `[upstox]` and `[capture]` sections of `gammapin.toml`.

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    pub node: Node,
    #[serde(default)]
    pub upstox: Upstox,
    #[serde(default)]
    pub capture: Capture,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Node {
    /// `A` or `B` for capture nodes.
    pub name: String,
    /// Segments go to `{data_dir}/{name}/{YYYY-MM-DD}/`.
    pub data_dir: PathBuf,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct Upstox {
    pub token_file: PathBuf,
    pub instrument_master_url: String,
    pub api_base: String,
}

impl Default for Upstox {
    fn default() -> Self {
        Self {
            token_file: "/etc/gammapin/upstox.token".into(),
            instrument_master_url:
                "https://assets.upstox.com/market-quote/instruments/exchange/NSE.json.gz".into(),
            api_base: "https://api.upstox.com".into(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct Capture {
    /// `["HH:MM", "HH:MM"]` IST: start and stop of the capture window.
    pub window_ist: [String; 2],
    pub segment_mib: u64,
    pub flush_ms: u64,
    /// `[first, max]` reconnect delay.
    pub backoff_ms: [u64; 2],
    pub strike_band_pct: f64,
    pub expiries: usize,
    pub max_instruments: usize,
    /// Cached instrument master (`.json` or `.json.gz`).
    pub master_path: PathBuf,
    /// Connect straight to this WebSocket URL instead of asking Upstox to authorize one
    /// (the mock feed, dry runs).
    pub feed_url: Option<String>,
}

impl Default for Capture {
    fn default() -> Self {
        Self {
            window_ist: ["09:00".into(), "15:40".into()],
            segment_mib: 64,
            flush_ms: 100,
            backoff_ms: [100, 5_000],
            strike_band_pct: 10.0,
            expiries: 2,
            max_instruments: 1_500,
            master_path: "data/NSE.json.gz".into(),
            feed_url: None,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("{0}: {1}")]
    Read(PathBuf, std::io::Error),
    #[error("{0}: {1}")]
    Parse(PathBuf, toml::de::Error),
    #[error("bad time {0:?}, expected HH:MM")]
    Time(String),
    #[error("capture window {0}-{1} does not open before it closes")]
    Window(String, String),
}

/// `HH:MM` to minutes after midnight.
pub fn parse_hhmm(s: &str) -> Result<f64, ConfigError> {
    let bad = || ConfigError::Time(s.to_string());
    let (h, m) = s.split_once(':').ok_or_else(bad)?;
    let two_digits = |p: &str| p.len() == 2 && p.bytes().all(|b| b.is_ascii_digit());
    if !(two_digits(h) && two_digits(m)) {
        return Err(bad());
    }
    let (h, m): (u32, u32) = (h.parse().map_err(|_| bad())?, m.parse().map_err(|_| bad())?);
    if h > 23 || m > 59 {
        return Err(bad());
    }
    Ok(f64::from(h * 60 + m))
}

impl Config {
    pub fn load(path: &Path) -> Result<Self, ConfigError> {
        let text = std::fs::read_to_string(path).map_err(|e| ConfigError::Read(path.into(), e))?;
        toml::from_str(&text).map_err(|e| ConfigError::Parse(path.into(), e))
    }

    /// Capture window as minutes after IST midnight; it must open before it closes.
    pub fn window(&self) -> Result<(f64, f64), ConfigError> {
        let [open, close] = &self.capture.window_ist;
        let (o, c) = (parse_hhmm(open)?, parse_hhmm(close)?);
        if o >= c {
            return Err(ConfigError::Window(open.clone(), close.clone()));
        }
        Ok((o, c))
    }

    pub fn writer_config(&self) -> gp_log::WriterConfig {
        gp_log::WriterConfig {
            segment_bytes: self.capture.segment_mib * 1024 * 1024,
            flush_interval: Duration::from_millis(self.capture.flush_ms),
        }
    }

    pub fn backoff(&self) -> (Duration, Duration) {
        let [lo, hi] = self.capture.backoff_ms;
        (Duration::from_millis(lo), Duration::from_millis(hi))
    }

    /// Log directory for one IST date.
    pub fn log_dir(&self, date: gp_feed::Date) -> PathBuf {
        self.node
            .data_dir
            .join(&self.node.name)
            .join(date.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn example_config_loads() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../config/gammapin.example.toml");
        let cfg = Config::load(&path).unwrap();
        assert_eq!(cfg.node.name, "A");
        assert_eq!(cfg.window().unwrap(), (540.0, 940.0));
        assert_eq!(cfg.writer_config().segment_bytes, 64 << 20);
        assert_eq!(cfg.backoff().1, Duration::from_secs(5));
        assert_eq!(cfg.capture.expiries, 2);
        assert_eq!(cfg.capture.feed_url, None);
        assert_eq!(
            cfg.log_dir(gp_feed::Date::from_ymd(2026, 10, 19)),
            PathBuf::from("data/A/2026-10-19")
        );
        let mut reversed = cfg.clone();
        reversed.capture.window_ist = ["15:40".into(), "09:00".into()];
        assert!(matches!(reversed.window(), Err(ConfigError::Window(..))));
    }

    #[test]
    fn minimal_config_takes_defaults() {
        let cfg: Config = toml::from_str(
            "[node]\nname = \"B\"\ndata_dir = \"/srv\"\n[capture]\nfeed_url = \"ws://127.0.0.1:9\"\n",
        )
        .unwrap();
        assert_eq!(cfg.capture.max_instruments, 1_500);
        assert_eq!(cfg.capture.feed_url.as_deref(), Some("ws://127.0.0.1:9"));
        assert_eq!(cfg.upstox.api_base, "https://api.upstox.com");
    }

    #[test]
    fn hhmm() {
        assert_eq!(parse_hhmm("09:15").unwrap(), 555.0);
        for bad in ["9:15", "24:00", "09:60", "0915", "aa:bb", "+9:15", "09:+5"] {
            assert!(parse_hhmm(bad).is_err(), "{bad}");
        }
    }
}
