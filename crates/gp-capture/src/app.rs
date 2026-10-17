//! The gp-capture process: config → instruments → one recorder → the capture loop, inside
//! the capture window.

use std::path::Path;
use std::time::Duration;

use anyhow::Context;
use gp_feed::date::ist_minute_of_day;
use gp_feed::{Date, Selection, read_master};
use tokio::sync::watch;

use crate::backoff::Backoff;
use crate::capture::{Summary, capture};
use crate::client::ConnOpts;
use crate::config::Config;
use crate::endpoint::{Endpoint, read_token};
use crate::recorder::{Recorder, realtime_ns};
use crate::subscriber::Subscriber;
use crate::token::TokenWatch;

pub fn now_ms() -> i64 {
    realtime_ns() / 1_000_000
}

/// Fetch the instrument master into `path` unless it is already there from today.
pub async fn ensure_master(cfg: &Config, today: Date) -> anyhow::Result<()> {
    let path = &cfg.capture.master_path;
    let fresh = std::fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .is_some_and(|d| Date::ist_from_unix_ms(d.as_millis() as i64) == today);
    if fresh || cfg.capture.feed_url.is_some() {
        return Ok(());
    }
    let bytes = reqwest::get(&cfg.upstox.instrument_master_url)
        .await?
        .error_for_status()?
        .bytes()
        .await?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(path, &bytes).with_context(|| format!("writing {}", path.display()))?;
    Ok(())
}

/// Exit status for the 09:05 token check: a token file written today (IST) must exist.
pub fn check_token(cfg: &Config, now_ms: i64) -> Result<(), String> {
    let path = &cfg.upstox.token_file;
    read_token(path)?;
    let modified = std::fs::metadata(path)
        .and_then(|m| m.modified())
        .map_err(|e| format!("{}: {e}", path.display()))?
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|e| e.to_string())?;
    let written = Date::ist_from_unix_ms(modified.as_millis() as i64);
    let today = Date::ist_from_unix_ms(now_ms);
    if written != today {
        return Err(format!(
            "{} is from {written}, not today ({today})",
            path.display()
        ));
    }
    Ok(())
}

pub struct RunOpts {
    /// Ignore the capture window: start now, run until stopped.
    pub now: bool,
}

/// Run one day's capture. Returns when the window closes or `stop` is raised.
pub async fn run(
    cfg: &Config,
    opts: RunOpts,
    mut stop: watch::Receiver<bool>,
) -> anyhow::Result<Summary> {
    let today = Date::ist_from_unix_ms(now_ms());
    let (open, close) = cfg.window()?;
    if !opts.now {
        let wait = open - ist_minute_of_day(now_ms());
        if wait > 0.0 {
            tracing::info!(minutes = wait, "waiting for the capture window");
            tokio::select! {
                _ = crate::client::stopped(&mut stop) => return Ok(Summary::default()),
                _ = tokio::time::sleep(Duration::from_secs_f64(wait * 60.0)) => {}
            }
        }
    }
    ensure_master(cfg, today).await?;
    let instruments = read_master(&cfg.capture.master_path)
        .with_context(|| format!("reading {}", cfg.capture.master_path.display()))?;
    let sel = Selection {
        expiries: cfg.capture.expiries,
        band_pct: cfg.capture.strike_band_pct,
        max: cfg.capture.max_instruments,
    };
    let mut sub = Subscriber::new(instruments, today, sel, &cfg.node.name);
    let dir = cfg.log_dir(today);
    let (mut rec, recovery) = Recorder::open(&dir, cfg.writer_config(), 0)?;
    if let Some(r) = recovery {
        tracing::warn!(?r, dir = %dir.display(), "recovered the previous segment");
    }
    let endpoint = match &cfg.capture.feed_url {
        Some(url) => Endpoint::Direct(url.clone()),
        None => Endpoint::Upstox {
            api_base: cfg.upstox.api_base.clone(),
            token_file: cfg.upstox.token_file.clone(),
        },
    };
    let watch = match &endpoint {
        Endpoint::Upstox { token_file, .. } => Some(TokenWatch::new(token_file)?),
        Endpoint::Direct(_) => None,
    };
    let (lo, hi) = cfg.backoff();
    let seed = u64::from(std::process::id()) ^ cfg.node.name.bytes().map(u64::from).sum::<u64>();
    let mut backoff = Backoff::new(lo, hi, seed);
    let conn = ConnOpts {
        flush: cfg.writer_config().flush_interval,
        ..ConnOpts::default()
    };

    // Stop at the end of the window as well as on the external signal.
    let (tx, mut rx) = watch::channel(false);
    let window_end = (!opts.now).then(|| close - ist_minute_of_day(now_ms()));
    tokio::spawn(async move {
        let end = async {
            match window_end {
                Some(m) => tokio::time::sleep(Duration::from_secs_f64(m.max(0.0) * 60.0)).await,
                None => std::future::pending().await,
            }
        };
        tokio::select! {
            _ = crate::client::stopped(&mut stop) => {}
            _ = end => {}
        }
        let _ = tx.send(true);
    });
    tracing::info!(dir = %dir.display(), "capturing");
    let summary = capture(
        &endpoint,
        &mut sub,
        &mut rec,
        &mut backoff,
        conn,
        watch.as_ref().map(TokenWatch::changed),
        &mut rx,
    )
    .await?;
    rec.close()?;
    tracing::info!(?summary, instruments = sub.keys().len(), "capture finished");
    Ok(summary)
}

/// Load the config named on the command line.
pub fn load(path: &Path) -> anyhow::Result<Config> {
    Ok(Config::load(path)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(dir: &Path) -> Config {
        toml::from_str(&format!(
            "[node]\nname = \"A\"\ndata_dir = \"{}\"\n[upstox]\ntoken_file = \"{}\"\n",
            dir.display(),
            dir.join("upstox.token").display()
        ))
        .unwrap()
    }

    #[test]
    fn token_check() {
        let dir = tempfile::tempdir().unwrap();
        let c = cfg(dir.path());
        assert!(
            check_token(&c, now_ms())
                .unwrap_err()
                .contains("upstox.token")
        );
        std::fs::write(dir.path().join("upstox.token"), "t\n").unwrap();
        check_token(&c, now_ms()).unwrap();
        let tomorrow = now_ms() + 86_400_000;
        assert!(check_token(&c, tomorrow).unwrap_err().contains("not today"));
    }

    #[tokio::test]
    async fn runs_against_the_mock_feed() {
        use crate::mock::{MockConfig, MockFeed};
        use gp_feed::synth::{MarketConfig, master_json, universe};
        let date = Date::ist_from_unix_ms(now_ms());
        let u = universe(date, 25_000.0, 2, 3.0, 50.0);
        let feed = MockFeed::start(
            MockConfig {
                market: MarketConfig::new(date),
                instruments: u.clone(),
                interval: Duration::from_millis(5),
            },
            "127.0.0.1:0",
        )
        .await
        .unwrap();
        let dir = tempfile::tempdir().unwrap();
        let master = dir.path().join("master.json");
        std::fs::write(&master, master_json(&u)).unwrap();
        let mut c = cfg(dir.path());
        c.capture.feed_url = Some(feed.url());
        c.capture.master_path = master;
        c.capture.strike_band_pct = 1.0;
        let (tx, rx) = watch::channel(false);
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(400)).await;
            tx.send(true).unwrap();
        });
        let s = run(&c, RunOpts { now: true }, rx).await.unwrap();
        assert!(s.frames > 20 && s.gaps == 0, "{s:?}");
        let recs = gp_log::read_log(&c.log_dir(date)).unwrap();
        assert_eq!(
            recs.iter()
                .filter(|r| r.kind == gp_log::RecordKind::RawFrame)
                .count() as u64,
            s.frames
        );
    }
}
