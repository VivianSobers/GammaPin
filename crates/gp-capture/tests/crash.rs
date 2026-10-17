//! V1.11: kill gp-capture with SIGKILL mid-write, restart it, and check that the log
//! recovers to whole records and keeps going in a new segment.

use std::process::{Child, Command, Stdio};
use std::time::Duration;

use gp_capture::mock::{MockConfig, MockFeed};
use gp_feed::Date;
use gp_feed::synth::{MarketConfig, master_json, universe};
use gp_log::{RecordKind, list_segments, read_log, segment_bytes};

fn spawn_capture(config: &std::path::Path) -> Child {
    Command::new(env!("CARGO_BIN_EXE_gp-capture"))
        .arg("--config")
        .arg(config)
        .arg("--now")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap()
}

fn raw_frames(dir: &std::path::Path) -> usize {
    read_log(dir)
        .unwrap()
        .iter()
        .filter(|r| r.kind == RecordKind::RawFrame)
        .count()
}

#[tokio::test(flavor = "multi_thread")]
async fn sigkill_mid_write_recovers() {
    let today = Date::ist_from_unix_ms(gp_capture::app::now_ms());
    let u = universe(today, 25_000.0, 2, 10.0, 50.0);
    let feed = MockFeed::start(
        MockConfig {
            market: MarketConfig {
                per_frame: 200,
                ..MarketConfig::new(today)
            },
            instruments: u.clone(),
            interval: Duration::from_millis(1),
        },
        "127.0.0.1:0",
    )
    .await
    .unwrap();
    let dir = tempfile::tempdir().unwrap();
    let master = dir.path().join("master.json");
    std::fs::write(&master, master_json(&u)).unwrap();
    let config = dir.path().join("gammapin.toml");
    // 1 MiB segments so rotation happens several times per run; flush every 10 ms.
    std::fs::write(
        &config,
        format!(
            "[node]\nname = \"A\"\ndata_dir = \"{}\"\n[capture]\nsegment_mib = 1\nflush_ms = 10\n\
             feed_url = \"{}\"\nmaster_path = \"{}\"\n",
            dir.path().join("data").display(),
            feed.url(),
            master.display()
        ),
    )
    .unwrap();
    let logs = dir.path().join("data/A").join(today.to_string());

    let mut torn_tails = 0;
    for round in 0..3 {
        let mut child = spawn_capture(&config);
        tokio::time::sleep(Duration::from_millis(1_500)).await;
        child.kill().unwrap(); // SIGKILL: no flush, no close
        child.wait().unwrap();

        // The newest segment may end in a torn record; everything before it must be whole.
        let segs = list_segments(&logs).unwrap();
        assert!(
            segs.len() >= 2,
            "round {round}: expected rotation, got {segs:?}"
        );
        let last = segs.last().unwrap();
        let scan = gp_log::scan(&segment_bytes(&last.path).unwrap());
        if scan.tail.is_some() {
            torn_tails += 1;
        }
        for s in &segs[..segs.len() - 1] {
            gp_log::read_segment(&s.path).unwrap();
        }
    }

    // A final clean run recovers the last torn tail on start and exits cleanly on SIGTERM.
    let child = spawn_capture(&config);
    tokio::time::sleep(Duration::from_millis(800)).await;
    // SAFETY: plain kill(2) on our own child.
    unsafe { libc::kill(child.id() as i32, libc::SIGTERM) };
    let status = wait_with_timeout(child, Duration::from_secs(10));
    assert!(status.success(), "{status:?}");

    let frames = raw_frames(&logs);
    assert!(frames > 500, "only {frames} frames survived");
    let recs = read_log(&logs).unwrap(); // every segment now reads clean
    assert!(recs.iter().any(|r| r.kind == RecordKind::Subscribe));
    eprintln!("frames kept: {frames}, torn tails recovered: {torn_tails}");
}

fn wait_with_timeout(mut child: Child, limit: Duration) -> std::process::ExitStatus {
    let start = std::time::Instant::now();
    loop {
        if let Some(s) = child.try_wait().unwrap() {
            return s;
        }
        assert!(
            start.elapsed() < limit,
            "gp-capture did not stop on SIGTERM"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}
