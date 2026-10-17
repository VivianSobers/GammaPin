//! The capture loop: connect, record, and on any failure write `GAP_START`, back off,
//! reconnect and resubscribe. The first frame after a reconnect writes `GAP_END`.

use gp_log::LogError;
use tokio::sync::watch;

use crate::backoff::Backoff;
use crate::client::{ConnOpts, End, run_connection, stopped};
use crate::recorder::Recorder;
use crate::subscriber::Subscriber;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Summary {
    pub connections: u32,
    pub frames: u64,
    pub gaps: u64,
}

/// Capture from `url` until `stop` becomes true.
pub async fn capture(
    url: &str,
    sub: &mut Subscriber,
    rec: &mut Recorder,
    backoff: &mut Backoff,
    opts: ConnOpts,
    stop: &mut watch::Receiver<bool>,
) -> Result<Summary, LogError> {
    let mut summary = Summary::default();
    while !*stop.borrow() {
        let out = run_connection(url, sub, rec, opts, stop).await?;
        summary.connections += 1;
        summary.frames += out.frames;
        if out.frames > 0 {
            backoff.reset();
        }
        if out.end == End::Stopped {
            break;
        }
        rec.gap_start(&out.end.reason())?;
        rec.flush()?;
        let delay = backoff.next_delay();
        tokio::select! {
            _ = stopped(stop) => break,
            _ = tokio::time::sleep(delay) => {}
        }
    }
    summary.gaps = rec.stats().gaps;
    rec.flush()?;
    Ok(summary)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mock::{Fault, MockConfig, MockFeed};
    use gp_feed::synth::{MarketConfig, universe};
    use gp_feed::{Date, Selection, parse_request};
    use gp_log::{RecordKind, WriterConfig, read_log};
    use std::time::Duration;

    #[tokio::test]
    async fn survives_cuts_and_resubscribes() {
        let date = Date::from_ymd(2026, 10, 19);
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
        let sel = Selection {
            band_pct: 1.0,
            ..Selection::default()
        };
        let mut sub = Subscriber::new(u, date, sel, "B");
        let dir = tempfile::tempdir().unwrap();
        let (mut rec, _) = Recorder::open(dir.path(), WriterConfig::default(), 1).unwrap();
        let mut backoff = Backoff::new(Duration::from_millis(20), Duration::from_millis(100), 9);
        let opts = ConnOpts {
            stall: Duration::from_millis(250),
            flush: Duration::from_millis(20),
        };
        let (tx, mut rx) = watch::channel(false);
        let url = feed.url();
        let task = tokio::spawn(async move {
            let s = capture(&url, &mut sub, &mut rec, &mut backoff, opts, &mut rx)
                .await
                .unwrap();
            rec.close().unwrap();
            s
        });
        let pause = Duration::from_millis(300);
        tokio::time::sleep(pause).await;
        feed.cut(Fault::Drop);
        tokio::time::sleep(pause).await;
        feed.restore();
        tokio::time::sleep(pause).await;
        feed.cut(Fault::Silence);
        tokio::time::sleep(Duration::from_millis(500)).await;
        feed.restore();
        tokio::time::sleep(pause).await;
        tx.send(true).unwrap();
        let summary = task.await.unwrap();

        assert_eq!(summary.gaps, 2, "{summary:?}");
        assert!(summary.connections >= 3);
        let recs = read_log(dir.path()).unwrap();
        let kinds: Vec<RecordKind> = recs.iter().map(|r| r.kind).collect();
        let starts: Vec<usize> = (0..kinds.len())
            .filter(|&i| kinds[i] == RecordKind::GapStart)
            .collect();
        let ends: Vec<usize> = (0..kinds.len())
            .filter(|&i| kinds[i] == RecordKind::GapEnd)
            .collect();
        assert_eq!((starts.len(), ends.len()), (2, 2));
        assert!(starts[0] < ends[0] && ends[0] < starts[1] && starts[1] < ends[1]);
        assert!(String::from_utf8_lossy(&recs[starts[1]].payload).contains("stalled"));
        // each reconnect resubscribes the whole band in its first requests
        let first_sub_after_gap = recs[starts[0]..]
            .iter()
            .find(|r| r.kind == RecordKind::Subscribe)
            .unwrap();
        assert!(
            parse_request(&first_sub_after_gap.payload)
                .unwrap()
                .data
                .instrument_keys
                .len()
                > 1
        );
        // frames flowed in each of the three healthy windows
        assert!(kinds[..starts[0]].contains(&RecordKind::RawFrame));
        assert!(kinds[ends[0]..starts[1]].contains(&RecordKind::RawFrame));
        assert!(kinds[ends[1]..].contains(&RecordKind::RawFrame));
    }
}
