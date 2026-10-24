//! V5.4: capture throughput. A bare WebSocket server pushes pre-encoded synthetic frames as
//! fast as the socket takes them; gp-capture's real client path writes them to a GPL1 log.
//!
//!     cargo bench -p gp-capture --bench throughput

use std::time::Instant;

use futures_util::{SinkExt, StreamExt};
use gp_capture::client::{ConnOpts, End, run_connection};
use gp_capture::recorder::Recorder;
use gp_capture::subscriber::Subscriber;
use gp_feed::synth::{Market, MarketConfig, universe};
use gp_feed::{Date, Selection};
use gp_log::WriterConfig;
use prost::Message as _;
use tokio::sync::watch;
use tokio_tungstenite::tungstenite::Message;

async fn serve(frames: Vec<Vec<u8>>, rounds: usize) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let (tcp, _) = listener.accept().await.unwrap();
        let ws = tokio_tungstenite::accept_async(tcp).await.unwrap();
        let (mut sink, mut stream) = ws.split();
        let _ = stream.next().await; // the first subscription request
        // Keep reading (later requests, the close reply) so the socket never resets.
        tokio::spawn(async move { while let Some(Ok(_)) = stream.next().await {} });
        for _ in 0..rounds {
            for f in &frames {
                sink.feed(Message::Binary(f.clone().into())).await.unwrap();
            }
        }
        sink.flush().await.unwrap();
        sink.close().await.unwrap();
    });
    format!("ws://{addr}")
}

async fn run(per_frame: usize) {
    let date = Date::from_ymd(2026, 10, 19);
    let u = universe(date, 25_000.0, 2, 10.0, 50.0);
    let mut market = Market::new(
        MarketConfig {
            per_frame,
            ..MarketConfig::new(date)
        },
        u.clone(),
    );
    let frames: Vec<Vec<u8>> = (0..2_000)
        .map(|_| market.next_frame().encode_to_vec())
        .collect();
    let avg = frames.iter().map(Vec::len).sum::<usize>() / frames.len();
    let rounds = 100;
    let url = serve(frames, rounds).await;

    let dir = tempfile::tempdir().unwrap();
    let (mut rec, _) = Recorder::open(dir.path(), WriterConfig::default(), 0).unwrap();
    let mut sub = Subscriber::new(u, date, Selection::default(), "bench");
    let (_tx, mut rx) = watch::channel(false);
    let start = Instant::now();
    let out = run_connection(&url, &mut sub, &mut rec, ConnOpts::default(), &mut rx)
        .await
        .unwrap();
    rec.close().unwrap();
    let secs = start.elapsed().as_secs_f64();
    assert!(matches!(out.end, End::Closed(_)), "{:?}", out.end);
    assert_eq!(out.frames, 2_000 * rounds as u64);
    println!(
        "{per_frame:>3} instruments/frame, {avg:>5} B/frame: {:>9.0} frames/s, {:>6.1} MB/s ({} frames in {secs:.2} s)",
        out.frames as f64 / secs,
        out.frames as f64 * avg as f64 / secs / 1e6,
        out.frames,
    );
}

fn main() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    for per_frame in [1, 5, 40] {
        rt.block_on(run(per_frame));
    }
}
