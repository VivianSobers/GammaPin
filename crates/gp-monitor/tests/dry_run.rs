//! V6.4 and V6.1 on the mock feed: two capture nodes write their logs, the monitor follows
//! them live and serves `/ws`, and a client plays the dashboard. The mock's exchange clock
//! runs on wall time, so latencies are real.

use std::path::Path;
use std::time::Duration;

use futures_util::StreamExt;
use gp_capture::backoff::Backoff;
use gp_capture::capture::capture;
use gp_capture::client::ConnOpts;
use gp_capture::endpoint::Endpoint;
use gp_capture::mock::{MockConfig, MockFeed};
use gp_capture::recorder::Recorder;
use gp_capture::subscriber::Subscriber;
use gp_feed::date::ist_minute_of_day;
use gp_feed::synth::{MarketConfig, master_json, universe};
use gp_feed::{Date, Selection};
use gp_log::WriterConfig;
use gp_monitor::app::{Mode, RunOpts, run};
use gp_monitor::config::Config;
use gp_monitor::driver::wall_ms;
use gp_monitor::health::Histogram;
use gp_monitor::snapshot::{NodeState, Snapshot};
use tokio::sync::watch;

/// Frames every 20 ms, as Upstox sends a busy chain.
const STEP_MS: i64 = 20;

struct Report {
    snapshots: Vec<Snapshot>,
    staleness: Histogram,
    delivery: Histogram,
}

async fn dry_run(secs: u64) -> Report {
    let now = wall_ms();
    let today = Date::ist_from_unix_ms(now);
    let u = universe(today, 25_000.0, 2, 3.0, 50.0);
    // exchange time of frame i ≈ the wall time it is sent
    let market = MarketConfig {
        start_minute: ist_minute_of_day(now - STEP_MS),
        step_ms: STEP_MS,
        per_frame: u.len(),
        ..MarketConfig::new(today)
    };
    let feed = MockFeed::start(
        MockConfig {
            market,
            instruments: u.clone(),
            interval: Duration::from_millis(STEP_MS as u64),
        },
        "127.0.0.1:0",
    )
    .await
    .unwrap();

    let root = tempfile::tempdir().unwrap();
    let master = root.path().join("master.json");
    std::fs::write(&master, master_json(&u)).unwrap();
    let inbox = root.path().join("inbox");
    let mut cfg: Config = toml::from_str(&format!(
        "[merge]\ninbox = {:?}\nmaster_path = {:?}\n[monitor]\nstatic_dir = {:?}\n",
        inbox.display().to_string(),
        master.display().to_string(),
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("static")
            .display()
            .to_string(),
    ))
    .unwrap();
    cfg.monitor.snapshot_ms = 1_000;

    let (cap_tx, cap_rx) = watch::channel(false);
    let mut nodes = Vec::new();
    for (bit, name) in cfg.merge.nodes.clone().iter().enumerate() {
        let sel = Selection {
            band_pct: 50.0,
            ..Selection::default()
        };
        let mut sub = Subscriber::new(u.clone(), today, sel, name);
        let (mut rec, _) = Recorder::open(
            cfg.merge.node_dir(name, today),
            WriterConfig::default(),
            bit as u8,
        )
        .unwrap();
        let mut backoff = Backoff::new(Duration::from_millis(100), Duration::from_secs(5), 7);
        let (url, mut stop) = (Endpoint::Direct(feed.url()), cap_rx.clone());
        nodes.push(tokio::spawn(async move {
            capture(
                &url,
                &mut sub,
                &mut rec,
                &mut backoff,
                ConnOpts::default(),
                None,
                &mut stop,
            )
            .await
            .unwrap();
            rec.close().unwrap();
        }));
    }

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let (stop_tx, stop) = watch::channel(false);
    let opts = RunOpts {
        mode: Mode::Live { date: today },
        exit_at_end: false,
    };
    let monitor = tokio::spawn(async move { run(&cfg, opts, listener, stop).await });

    let (mut ws, _) = tokio_tungstenite::connect_async(format!("ws://{addr}/ws"))
        .await
        .unwrap();
    let mut snapshots = Vec::new();
    let mut delivery = Histogram::default();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(secs);
    while let Ok(Some(Ok(msg))) = tokio::time::timeout_at(deadline, ws.next()).await {
        let s: Snapshot = serde_json::from_str(msg.to_text().unwrap()).unwrap();
        delivery.record((wall_ms() - s.ts_ms) as f64);
        snapshots.push(s);
    }
    stop_tx.send(true).unwrap();
    let sum = monitor.await.unwrap().unwrap();
    cap_tx.send(true).unwrap();
    for n in nodes {
        n.await.unwrap();
    }
    Report {
        snapshots,
        staleness: sum.staleness,
        delivery,
    }
}

fn print(r: &Report) {
    let q = |h: &Histogram, p| h.quantile(p).unwrap_or(f64::NAN);
    eprintln!(
        "snapshots {}  staleness p50 {} p99 {} ms  delivery p50 {} p99 {} ms",
        r.snapshots.len(),
        q(&r.staleness, 0.5),
        q(&r.staleness, 0.99),
        q(&r.delivery, 0.5),
        q(&r.delivery, 0.99),
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn live_dry_run_on_the_mock_feed() {
    let r = dry_run(5).await;
    print(&r);
    assert!(r.snapshots.len() >= 3, "{}", r.snapshots.len());
    let last = r.snapshots.last().unwrap();
    assert_eq!(last.source, "live");
    assert_eq!(last.health.gaps_today, 0);
    assert!(last.health.nodes.values().all(|s| *s == NodeState::Up));
    assert!(last.health.quotes_used > 10);
    assert!(last.conventions.contains_key("S0"));
    assert!((last.spot - 25_000.0).abs() < 500.0);
    // loose bounds for a busy CI runner; the measured numbers come from the ignored test
    assert!(r.staleness.quantile(0.99).unwrap() < 1_000.0);
    assert!(r.delivery.quantile(0.99).unwrap() < 500.0);
}

/// V6.1: one minute of live monitoring. Feed-to-dashboard latency is staleness (newest
/// index tick to snapshot) plus delivery (snapshot to the client); p99 of each must sum
/// below 250 ms. Run with `cargo test --release -p gp-monitor --test dry_run -- --ignored`.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "takes a minute"]
async fn latency_over_one_minute() {
    let r = dry_run(60).await;
    print(&r);
    let p99 = r.staleness.quantile(0.99).unwrap() + r.delivery.quantile(0.99).unwrap();
    assert!(p99 < 250.0, "{p99}");
}
