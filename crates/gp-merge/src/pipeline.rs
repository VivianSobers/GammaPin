//! One day's merge: read every node, decode, de-duplicate, sort for replay, and write the
//! C1 Parquet partitions, `replay.gpr` and the coverage report.
//!
//! Output goes to `date=YYYY-MM-DD.tmp/` and replaces `date=YYYY-MM-DD/` only when complete,
//! so a rerun after a failure never leaves a half-written day.

use std::path::Path;

use gp_feed::{Date, Master, Normalizer};
use gp_replay::{sort_for_replay, write_replay};

use crate::config::Merge;
use crate::coverage::{Coverage, lost_windows, node_report};
use crate::dedup::dedup;
use crate::normalize::node_events;
use crate::parquet::write_partitions;
use crate::read::read_node;

pub const REPLAY_FILE: &str = "replay.gpr";

pub fn merge_day(cfg: &Merge, date: Date, master: &Master) -> anyhow::Result<Coverage> {
    anyhow::ensure!(cfg.nodes.len() <= 8, "src_mask has room for 8 nodes");
    let norm = Normalizer {
        oi_in_units: cfg.oi_in_units,
    };
    let mut all = Vec::new();
    let mut reports = Vec::new();
    for (bit, node) in cfg.nodes.iter().enumerate() {
        let log = read_node(&cfg.node_dir(node, date), node, bit as u8)?;
        let (events, stats) = node_events(&log, master, norm);
        all.extend(events);
        reports.push(node_report(&log, stats));
    }
    anyhow::ensure!(
        reports.iter().any(|r| r.present),
        "no node has a log for {date} under {}",
        cfg.inbox.display()
    );
    let (mut events, dedup_stats) = dedup(all);
    sort_for_replay(&mut events);

    let final_dir = cfg.day_dir(date);
    let tmp = final_dir.with_file_name(format!("date={date}.tmp"));
    if tmp.exists() {
        std::fs::remove_dir_all(&tmp)?;
    }
    std::fs::create_dir_all(&tmp)?;
    let partitions = write_partitions(&tmp, &events)?;
    write_replay(&tmp.join(REPLAY_FILE), &events)?;
    let coverage = Coverage {
        date: date.to_string(),
        lost: lost_windows(&reports),
        nodes: reports,
        dedup: dedup_stats,
        partitions,
        replay_events: events.len() as u64,
    };
    std::fs::write(
        tmp.join("coverage.json"),
        serde_json::to_vec_pretty(&coverage)?,
    )?;
    std::fs::write(tmp.join("coverage.md"), coverage.markdown())?;
    replace_dir(&tmp, &final_dir)?;
    Ok(coverage)
}

fn replace_dir(from: &Path, to: &Path) -> std::io::Result<()> {
    if to.exists() {
        let old = to.with_file_name(format!(
            "{}.old",
            to.file_name().unwrap_or_default().to_string_lossy()
        ));
        if old.exists() {
            std::fs::remove_dir_all(&old)?;
        }
        std::fs::rename(to, &old)?;
        std::fs::rename(from, to)?;
        std::fs::remove_dir_all(&old)
    } else {
        std::fs::rename(from, to)
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use gp_feed::synth::{Market, MarketConfig, universe};
    use gp_log::{RecordKind, Writer, WriterConfig};
    use gp_replay::ReplayFile;
    use prost::Message;

    /// Two node logs from one synthetic market: B misses frames `b_gap`, A misses `a_gap`.
    pub fn two_nodes(
        inbox: &Path,
        date: Date,
        frames: usize,
        a_gap: std::ops::Range<usize>,
        b_gap: std::ops::Range<usize>,
    ) -> Master {
        let u = universe(date, 25_000.0, 1, 2.0, 50.0);
        let mut market = Market::new(MarketConfig::new(date), u.clone());
        let mut writers: Vec<Writer> = ["A", "B"]
            .iter()
            .map(|n| {
                Writer::open(
                    inbox.join(n).join(date.to_string()),
                    WriterConfig::default(),
                )
                .unwrap()
                .0
            })
            .collect();
        for i in 0..frames {
            let bytes = market.next_frame().encode_to_vec();
            let t = market.now_ms() * 1_000_000;
            for (n, (w, gap)) in writers.iter_mut().zip([&a_gap, &b_gap]).enumerate() {
                let lag = n as i64 * 3_000;
                if i == gap.start && !gap.is_empty() {
                    w.append(RecordKind::GapStart, 0, t + lag, 0, b"stalled")
                        .unwrap();
                }
                if gap.contains(&i) {
                    continue;
                }
                if i == gap.end && !gap.is_empty() {
                    w.append(RecordKind::GapEnd, 0, t + lag, 0, b"").unwrap();
                }
                w.append(RecordKind::RawFrame, 0, t + lag, 0, &bytes)
                    .unwrap();
            }
        }
        for w in writers {
            w.close().unwrap();
        }
        Master::new(u)
    }

    fn cfg(root: &Path) -> Merge {
        Merge {
            inbox: root.join("inbox"),
            out: root.join("merged"),
            ..Merge::default()
        }
    }

    #[test]
    fn merges_two_nodes_with_gaps() {
        let root = tempfile::tempdir().unwrap();
        let c = cfg(root.path());
        let date = Date::from_ymd(2026, 10, 19);
        let master = two_nodes(&c.inbox, date, 300, 50..120, 200..230);
        let cov = merge_day(&c, date, &master).unwrap();

        // Every frame was captured by at least one node: 300 frames × 40 updates.
        assert_eq!(cov.dedup.output, 300 * 40);
        assert_eq!(cov.dedup.only_b, 70 * 40);
        assert_eq!(cov.dedup.only_a, 30 * 40);
        assert_eq!(cov.dedup.both, 200 * 40);
        assert!(cov.lost.is_empty());
        assert_eq!(cov.nodes[0].gaps.len(), 1);
        assert_eq!(cov.replay_events, 300 * 40);

        let day = c.day_dir(date);
        let replay = ReplayFile::open(&day.join(REPLAY_FILE)).unwrap();
        assert_eq!(replay.len(), 12_000);
        let evs: Vec<_> = replay.events().collect();
        assert!(gp_replay::is_replay_order(&evs));
        // events from both nodes take A's (earlier) receive time
        assert!(
            evs.iter()
                .filter(|e| e.src_mask == 3)
                .all(|e| e.recv_ns % 1_000_000 == 0)
        );
        assert!(day.join("coverage.json").exists() && day.join("coverage.md").exists());
        assert_eq!(cov.partitions.files, master.len() as u64); // one partition per instrument
    }

    #[test]
    fn rerun_replaces_the_day_and_one_node_suffices() {
        let root = tempfile::tempdir().unwrap();
        let c = cfg(root.path());
        let date = Date::from_ymd(2026, 10, 19);
        let master = two_nodes(&c.inbox, date, 50, 0..0, 0..0);
        let first = merge_day(&c, date, &master).unwrap();
        std::fs::remove_dir_all(c.node_dir("B", date)).unwrap();
        let second = merge_day(&c, date, &master).unwrap();
        assert_eq!(first.dedup.output, second.dedup.output);
        assert!(!second.nodes[1].present);
        assert_eq!(second.dedup.only_a, second.dedup.output);
        assert!(!c.out.join(format!("date={date}.tmp")).exists());

        std::fs::remove_dir_all(c.node_dir("A", date)).unwrap();
        assert!(merge_day(&c, date, &master).is_err());
    }
}
