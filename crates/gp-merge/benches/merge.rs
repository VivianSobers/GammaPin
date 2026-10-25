//! Merge throughput: two node logs of a synthetic session (node B missing a stretch),
//! merged into C1 Parquet, the replay log and the coverage report.
//!
//!     cargo bench -p gp-merge --bench merge

use std::time::Instant;

use gp_feed::synth::{Market, MarketConfig, universe};
use gp_feed::{Date, Master};
use gp_log::{RecordKind, Writer, WriterConfig};
use gp_merge::config::Merge;
use gp_merge::pipeline::merge_day;
use prost::Message;

const FRAMES: usize = 20_000;

fn main() {
    let date = Date::from_ymd(2026, 10, 19);
    let u = universe(date, 25_000.0, 2, 10.0, 50.0);
    let master = Master::new(u.clone());
    let mut market = Market::new(MarketConfig::new(date), u);
    let root = tempfile::tempdir().unwrap();
    let cfg = Merge {
        inbox: root.path().join("inbox"),
        out: root.path().join("merged"),
        compress: false,
        ..Merge::default()
    };
    let mut writers: Vec<Writer> = ["A", "B"]
        .iter()
        .map(|n| {
            Writer::open(cfg.node_dir(n, date), WriterConfig::default())
                .unwrap()
                .0
        })
        .collect();
    let mut bytes = 0;
    for i in 0..FRAMES {
        let frame = market.next_frame().encode_to_vec();
        let t = market.now_ms() * 1_000_000;
        bytes += frame.len();
        for (n, w) in writers.iter_mut().enumerate() {
            if n == 1 && (5_000..6_000).contains(&i) {
                continue; // B misses a stretch; A covers it
            }
            w.append(RecordKind::RawFrame, 0, t + n as i64 * 2_000_000, 0, &frame)
                .unwrap();
        }
    }
    for w in writers {
        w.close().unwrap();
    }

    let start = Instant::now();
    let cov = merge_day(&cfg, date, &master).unwrap();
    let secs = start.elapsed().as_secs_f64();
    println!(
        "{} frames/node ({:.0} MB), {} events in, {} out, {} partitions: {:.2} s, {:.0} events/s in",
        FRAMES,
        bytes as f64 / 1e6,
        cov.dedup.input,
        cov.dedup.output,
        cov.partitions.files,
        secs,
        cov.dedup.input as f64 / secs
    );
}
