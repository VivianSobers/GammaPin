//! One WebSocket connection: connect, subscribe, write every frame, report how it ended.

use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use gp_feed::carries_data;
use gp_log::LogError;
use tokio::sync::watch;
use tokio_tungstenite::tungstenite::Message;

use crate::recorder::Recorder;
use crate::subscriber::Subscriber;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum End {
    /// The stop signal was raised.
    Stopped,
    /// The server closed the connection.
    Closed(String),
    /// Connect, TLS, WebSocket or send error.
    Failed(String),
    /// Nothing at all arrived for the stall timeout.
    Stalled,
}

impl End {
    /// Text for the `GAP_START` payload.
    pub fn reason(&self) -> String {
        match self {
            Self::Stopped => "stopped".into(),
            Self::Closed(r) => format!("closed: {r}"),
            Self::Failed(e) => format!("failed: {e}"),
            Self::Stalled => "stalled".into(),
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct ConnOpts {
    /// Give up on a connection that delivers nothing (not even a ping) for this long.
    pub stall: Duration,
    /// Flush the log at least this often while frames are quiet.
    pub flush: Duration,
}

impl Default for ConnOpts {
    fn default() -> Self {
        Self {
            stall: Duration::from_secs(10),
            flush: Duration::from_millis(gp_log::FLUSH_INTERVAL_MS),
        }
    }
}

/// Resolves once `stop` is true (or its sender is gone). Holds no borrow across awaits.
pub async fn stopped(stop: &mut watch::Receiver<bool>) {
    let _ = stop.wait_for(|s| *s).await;
}

/// How a connection went.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Outcome {
    pub end: End,
    /// Frames written on this connection, the `market_info` greeting included.
    pub frames: u64,
    /// Of those, frames that carry market data ([`gp_feed::carries_data`]).
    pub data_frames: u64,
}

/// Run one connection to `url` until it ends or `stop` becomes true.
///
/// Frames are written before they are looked at. The first market-data frame closes an open
/// gap.
pub async fn run_connection(
    url: &str,
    sub: &mut Subscriber,
    rec: &mut Recorder,
    opts: ConnOpts,
    stop: &mut watch::Receiver<bool>,
) -> Result<Outcome, LogError> {
    let (mut frames, mut data_frames) = (0, 0);
    let done = |end| {
        Ok(Outcome {
            end,
            frames: 0,
            data_frames: 0,
        })
    };
    let ws = tokio::select! {
        _ = stopped(stop) => return done(End::Stopped),
        r = tokio::time::timeout(opts.stall, tokio_tungstenite::connect_async(url)) => match r {
            Err(_) => return done(End::Stalled),
            Ok(Err(e)) => return done(End::Failed(e.to_string())),
            Ok(Ok((ws, _))) => ws,
        },
    };
    let (mut sink, mut stream) = ws.split();
    for req in sub.connect_requests() {
        rec.subscribe(&req)?;
        if let Err(e) = sink.send(Message::Binary(req.into())).await {
            return done(End::Failed(e.to_string()));
        }
    }
    let mut flush = tokio::time::interval(opts.flush);
    let mut deadline = tokio::time::Instant::now() + opts.stall;
    let end = loop {
        tokio::select! {
            _ = stopped(stop) => {
                // a dead peer must not hold up shutdown
                let _ = tokio::time::timeout(opts.stall, sink.close()).await;
                break End::Stopped;
            }
            _ = flush.tick() => rec.flush()?,
            _ = tokio::time::sleep_until(deadline) => break End::Stalled,
            msg = stream.next() => {
                deadline = tokio::time::Instant::now() + opts.stall;
                let payload = match msg {
                    None => break End::Closed("eof".into()),
                    Some(Err(e)) => break End::Failed(e.to_string()),
                    Some(Ok(Message::Close(c))) => {
                        break End::Closed(c.map(|c| c.reason.to_string()).unwrap_or_default())
                    }
                    Some(Ok(Message::Binary(b))) => b,
                    Some(Ok(Message::Text(t))) => t.as_bytes().to_vec().into(),
                    Some(Ok(_)) => continue, // ping/pong: alive, nothing to record
                };
                rec.raw_frame(&payload)?;
                frames += 1;
                if carries_data(&payload) {
                    data_frames += 1;
                    if rec.in_gap() {
                        rec.gap_end()?;
                    }
                }
                for req in sub.on_frame(&payload) {
                    rec.subscribe(&req)?;
                    if let Err(e) = sink.send(Message::Binary(req.into())).await {
                        return Ok(Outcome { end: End::Failed(e.to_string()), frames, data_frames });
                    }
                }
            }
        }
    };
    rec.flush()?;
    Ok(Outcome {
        end,
        frames,
        data_frames,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mock::{Fault, MockConfig, MockFeed};
    use gp_feed::synth::{MarketConfig, universe};
    use gp_feed::{Date, Selection};
    use gp_log::{RecordKind, WriterConfig, read_log};

    fn setup(interval_ms: u64) -> (MockConfig, Subscriber) {
        let date = Date::from_ymd(2026, 10, 19);
        let u = universe(date, 25_000.0, 2, 3.0, 50.0);
        let sel = Selection {
            band_pct: 1.0,
            ..Selection::default()
        };
        (
            MockConfig {
                market: MarketConfig::new(date),
                instruments: u.clone(),
                interval: Duration::from_millis(interval_ms),
            },
            Subscriber::new(u, date, sel, "A"),
        )
    }

    fn opts(stall_ms: u64) -> ConnOpts {
        ConnOpts {
            stall: Duration::from_millis(stall_ms),
            flush: Duration::from_millis(20),
        }
    }

    #[tokio::test]
    async fn records_until_stopped() {
        let (mcfg, mut sub) = setup(5);
        let feed = MockFeed::start(mcfg, "127.0.0.1:0").await.unwrap();
        let dir = tempfile::tempdir().unwrap();
        let (mut rec, _) = Recorder::open(dir.path(), WriterConfig::default(), 0).unwrap();
        let (tx, mut rx) = watch::channel(false);
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(400)).await;
            tx.send(true).unwrap();
        });
        let out = run_connection(&feed.url(), &mut sub, &mut rec, opts(2_000), &mut rx)
            .await
            .unwrap();
        assert_eq!(out.end, End::Stopped);
        assert!(out.frames > 20, "{}", out.frames);
        assert!(sub.banded());
        rec.close().unwrap();

        let recs = read_log(dir.path()).unwrap();
        assert_eq!(recs[0].kind, RecordKind::Subscribe);
        let n_raw = recs
            .iter()
            .filter(|r| r.kind == RecordKind::RawFrame)
            .count();
        assert_eq!(n_raw as u64, out.frames);
        let n_sub = recs
            .iter()
            .filter(|r| r.kind == RecordKind::Subscribe)
            .count();
        assert!(n_sub >= 2, "index request plus band request(s)");
        // after the band request, frames carry options too
        let last = gp_feed::decode_frame(&recs.last().unwrap().payload).unwrap();
        assert!(last.feeds.len() > 1);
    }

    #[tokio::test]
    async fn reports_stall_and_drop() {
        let (mcfg, mut sub) = setup(5);
        let feed = MockFeed::start(mcfg, "127.0.0.1:0").await.unwrap();
        let dir = tempfile::tempdir().unwrap();
        let (mut rec, _) = Recorder::open(dir.path(), WriterConfig::default(), 0).unwrap();
        let (_tx, mut rx) = watch::channel(false);

        feed.cut(Fault::Silence);
        let out = run_connection(&feed.url(), &mut sub, &mut rec, opts(300), &mut rx)
            .await
            .unwrap();
        // only the market_info greeting could arrive before the silence
        assert_eq!(out.end, End::Stalled);
        assert!(out.frames <= 1);
        assert_eq!(out.data_frames, 0);

        feed.cut(Fault::Drop);
        let out = run_connection(&feed.url(), &mut sub, &mut rec, opts(300), &mut rx)
            .await
            .unwrap();
        assert!(
            matches!(out.end, End::Failed(_) | End::Closed(_)),
            "{:?}",
            out.end
        );
        assert!(out.end.reason().len() > 6);

        let refused = run_connection("ws://127.0.0.1:1", &mut sub, &mut rec, opts(300), &mut rx)
            .await
            .unwrap();
        assert!(matches!(refused.end, End::Failed(_)));
    }

    #[test]
    fn only_data_frames_close_a_gap() {
        use gp_feed::proto::{FeedResponse, Type};
        use prost::Message as _;
        let info = FeedResponse {
            r#type: Type::MarketInfo as i32,
            ..Default::default()
        };
        assert!(!carries_data(&info.encode_to_vec()));
        assert!(!carries_data(&[0xFF; 4]));
        let (mcfg, _) = setup(5);
        let mut m = gp_feed::synth::Market::new(mcfg.market, mcfg.instruments);
        assert!(carries_data(&m.next_frame().encode_to_vec()));
    }
}
