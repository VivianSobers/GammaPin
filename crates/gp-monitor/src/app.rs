//! Wiring for the binary: pick the stream (live or replay), run the engine on its own
//! thread, and serve `/ws` until stopped.

use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use gp_feed::{Date, Master, Normalizer, read_master};
use gp_replay::{EventStream, Paced, ReplayFile, ReplayStream, Speed, SystemClock};
use tokio::net::TcpListener;
use tokio::sync::watch;

use crate::config::Config;
use crate::driver::{Driver, Summary, wall_ms};
use crate::engine::Engine;
use crate::live::LiveStream;
use crate::server::{AppState, router};

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Mode {
    /// Follow today's node logs under the inbox.
    Live { date: Date },
    /// Replay a merged day's `replay.gpr`.
    Replay { date: Date, speed: Speed },
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RunOpts {
    pub mode: Mode,
    /// Return when a replay ends instead of serving its last snapshot until stopped.
    pub exit_at_end: bool,
}

/// `GAP_START` times (Unix ms) from a merged day's coverage report, across nodes.
pub fn coverage_gaps(day_dir: &Path) -> anyhow::Result<Vec<i64>> {
    let path = day_dir.join(gp_merge::pipeline::COVERAGE_FILE);
    let text = match std::fs::read_to_string(&path) {
        Ok(t) => t,
        // a replay without its report shows no gaps
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => anyhow::bail!("{}: {e}", path.display()),
    };
    let v: serde_json::Value =
        serde_json::from_str(&text).map_err(|e| anyhow::anyhow!("{}: {e}", path.display()))?;
    let mut out = Vec::new();
    for node in v["nodes"].as_array().into_iter().flatten() {
        for g in node["gaps"].as_array().into_iter().flatten() {
            if let Some(ns) = g["start_ns"].as_i64() {
                out.push(ns.div_euclid(1_000_000));
            }
        }
    }
    Ok(out)
}

fn stream(cfg: &Config, mode: Mode, engine: &mut Engine) -> anyhow::Result<Box<dyn EventStream>> {
    let m = &cfg.merge;
    Ok(match mode {
        Mode::Replay { date, speed } => {
            let day = m.day_dir(date);
            let path = day.join(gp_merge::pipeline::REPLAY_FILE);
            let file =
                ReplayFile::open(&path).map_err(|e| anyhow::anyhow!("{}: {e}", path.display()))?;
            for g in coverage_gaps(&day)? {
                engine.on_gap(g);
            }
            let inner = ReplayStream::new(Arc::new(file));
            Box::new(Paced::new(inner, speed, Box::new(SystemClock::default())))
        }
        Mode::Live { date } => {
            let path = m.master_for(date);
            let master =
                read_master(&path).map_err(|e| anyhow::anyhow!("{}: {e}", path.display()))?;
            let dirs = m.nodes.iter().map(|n| m.node_dir(n, date)).collect();
            Box::new(LiveStream::new(
                dirs,
                Arc::new(Master::new(master)),
                Normalizer {
                    oi_in_units: m.oi_in_units,
                },
            ))
        }
    })
}

/// Serve on `listener` until `stop` becomes true (or, with `exit_at_end`, the replay ends).
pub async fn run(
    cfg: &Config,
    opts: RunOpts,
    listener: TcpListener,
    stop: watch::Receiver<bool>,
) -> anyhow::Result<Summary> {
    let mut engine = Engine::new(cfg.monitor.engine(&cfg.merge.nodes)?);
    let mut stream = stream(cfg, opts.mode, &mut engine)?;
    let (tx, rx) = watch::channel(None);
    let halt = Arc::new(AtomicBool::new(false));
    let every_ms = cfg.monitor.snapshot_ms.max(1) as i64;
    let halt2 = halt.clone();
    let (done_tx, done_rx) = tokio::sync::oneshot::channel();
    std::thread::Builder::new()
        .name("gp-engine".into())
        .spawn(move || {
            let drv = Driver {
                every_ms,
                clock: &wall_ms,
                stop: &halt2,
            };
            let out = drv.run(stream.as_mut(), &mut engine, &mut |s| {
                tx.send_replace(Some(Arc::from(s.to_json())));
            });
            let _ = done_tx.send(out.map_err(|e| anyhow::anyhow!("stream: {e}")));
        })?;

    let state = AppState {
        latest: rx,
        static_dir: cfg.monitor.static_dir.clone(),
    };
    let (shutdown_tx, shutdown) = watch::channel(false);
    let app = router(state);
    let server = tokio::spawn(async move {
        axum::serve(listener, app)
            .with_graceful_shutdown(until_true(shutdown))
            .await
    });
    let mut done_rx = done_rx;
    let finished = tokio::select! {
        _ = until_true(stop.clone()) => None,
        r = &mut done_rx => Some(r),
    };
    let failed = matches!(finished, Some(Ok(Err(_))) | Some(Err(_)));
    if let Some(Ok(Err(e))) = &finished {
        tracing::error!("engine stopped: {e:#}");
    }
    if finished.is_some() && !failed && !opts.exit_at_end {
        until_true(stop).await; // keep showing the last snapshot
    }
    halt.store(true, Ordering::Relaxed);
    let _ = shutdown_tx.send(true);
    server.await??;
    let result = match finished {
        Some(r) => r,
        None => done_rx.await,
    };
    result.map_err(|_| anyhow::anyhow!("engine thread panicked"))?
}

async fn until_true(mut rx: watch::Receiver<bool>) {
    let _ = rx.wait_for(|s| *s).await;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::tests::market_events;
    use crate::snapshot::Snapshot;
    use futures_util::StreamExt;
    use gp_merge::config::Merge;

    #[tokio::test(flavor = "multi_thread")]
    async fn replays_a_merged_day_over_ws() {
        let date = Date::from_ymd(2026, 10, 19);
        let root = tempfile::tempdir().unwrap();
        let merge = Merge {
            out: root.path().to_path_buf(),
            ..Merge::default()
        };
        let day = merge.day_dir(date);
        std::fs::create_dir_all(&day).unwrap();
        let mut evs = market_events(date, 40);
        gp_replay::sort_for_replay(&mut evs);
        gp_replay::write_replay(&day.join(gp_merge::pipeline::REPLAY_FILE), &evs).unwrap();
        let gap_ns = evs[0].ltt_ms * 1_000_000;
        std::fs::write(
            day.join(gp_merge::pipeline::COVERAGE_FILE),
            format!(r#"{{"nodes":[{{"gaps":[{{"start_ns":{gap_ns}}}]}},{{"gaps":[]}}]}}"#),
        )
        .unwrap();

        let mut cfg: Config = toml::from_str("[monitor]\nsnapshot_ms = 50").unwrap();
        cfg.merge = merge;
        cfg.monitor.static_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("static");
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let (stop_tx, stop) = watch::channel(false);
        let opts = RunOpts {
            mode: Mode::Replay {
                date,
                speed: Speed::Multiple(20.0),
            },
            exit_at_end: false,
        };
        let task = tokio::spawn(async move { run(&cfg, opts, listener, stop).await });

        let (mut ws, _) = tokio_tungstenite::connect_async(format!("ws://{addr}/ws"))
            .await
            .unwrap();
        let mut last: Option<Snapshot> = None;
        // 40 frames × 250 ms at 20× is 0.5 s: wait for the final snapshot
        while let Ok(Some(Ok(msg))) =
            tokio::time::timeout(std::time::Duration::from_secs(2), ws.next()).await
        {
            let s: Snapshot = serde_json::from_str(msg.to_text().unwrap()).unwrap();
            if let Some(prev) = &last {
                assert!(s.ts_ms >= prev.ts_ms);
            }
            last = Some(s);
            if last.as_ref().unwrap().ts_ms == evs.last().unwrap().ltt_ms {
                break;
            }
        }
        let s = last.expect("at least one snapshot");
        assert_eq!(s.source, "replay");
        assert_eq!(s.ts_ms, evs.last().unwrap().ltt_ms);
        assert_eq!(s.health.gaps_today, 1);
        stop_tx.send(true).unwrap();
        let sum = task.await.unwrap().unwrap();
        assert_eq!(sum.events, evs.len() as u64);
    }

    #[test]
    fn missing_coverage_means_no_gaps() {
        let dir = tempfile::tempdir().unwrap();
        assert!(coverage_gaps(dir.path()).unwrap().is_empty());
        let path = dir.path().join(gp_merge::pipeline::COVERAGE_FILE);
        std::fs::write(&path, "not json").unwrap();
        assert!(coverage_gaps(dir.path()).is_err());
        // unreadable is an error too, not an empty report
        std::fs::remove_file(&path).unwrap();
        std::fs::create_dir(&path).unwrap();
        assert!(coverage_gaps(dir.path()).is_err());
    }
}
