//! The per-node coverage report written next to each merged day.
//!
//! A gap on one node costs nothing while the other node is up; data is lost only where
//! both nodes are in a gap at once. Those windows are listed as `lost`.

use std::fmt::Write;

use serde::Serialize;

use crate::dedup::DedupStats;
use crate::normalize::DecodeStats;
use crate::parquet::PartitionStats;
use crate::read::NodeLog;

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct GapReport {
    pub start_ns: i64,
    /// Equal to the node's last record time when the log ended inside the gap.
    pub end_ns: i64,
    pub open_at_end: bool,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct NodeReport {
    pub node: String,
    pub present: bool,
    pub segments: usize,
    pub frames: u64,
    pub frame_bytes: u64,
    pub first_recv_ns: Option<i64>,
    pub last_recv_ns: Option<i64>,
    pub gaps: Vec<GapReport>,
    pub gap_ms: f64,
    pub longest_silence_ms: f64,
    pub subscribes: usize,
    pub token_reloads: usize,
    pub decode: DecodeStats,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Coverage {
    pub date: String,
    pub nodes: Vec<NodeReport>,
    pub dedup: DedupStats,
    pub partitions: PartitionStats,
    pub replay_events: u64,
    /// Windows (ns) where every present node was in a gap: data that no node captured.
    pub lost: Vec<(i64, i64)>,
}

pub fn node_report(log: &NodeLog, decode: DecodeStats) -> NodeReport {
    let last = log.frames.last().map(|f| f.0);
    let gaps: Vec<GapReport> = log
        .gaps
        .iter()
        .map(|g| GapReport {
            start_ns: g.start_ns,
            end_ns: g
                .end_ns
                .unwrap_or(last.unwrap_or(g.start_ns).max(g.start_ns)),
            open_at_end: g.end_ns.is_none(),
            reason: g.reason.clone(),
        })
        .collect();
    NodeReport {
        node: log.node.clone(),
        present: log.present,
        segments: log.segments,
        frames: log.frames.len() as u64,
        frame_bytes: log.frame_bytes(),
        first_recv_ns: log.frames.first().map(|f| f.0),
        last_recv_ns: last,
        // fold from +0.0: an empty f64 sum is -0.0, which prints as "-0.0"
        gap_ms: gaps
            .iter()
            .fold(0.0, |acc, g| acc + (g.end_ns - g.start_ns) as f64 / 1e6),
        gaps,
        longest_silence_ms: log.longest_silence_ns() as f64 / 1e6,
        subscribes: log.subscribes,
        token_reloads: log.token_reloads,
        decode,
    }
}

/// Intersection of all present nodes' gap windows. Gaps still open at a node's end run to
/// `i64::MAX`, because nothing more arrived from it.
pub fn lost_windows(nodes: &[NodeReport]) -> Vec<(i64, i64)> {
    let present: Vec<&NodeReport> = nodes.iter().filter(|n| n.present).collect();
    let Some((first, rest)) = present.split_first() else {
        return Vec::new();
    };
    let windows = |n: &NodeReport| -> Vec<(i64, i64)> {
        n.gaps
            .iter()
            .map(|g| (g.start_ns, if g.open_at_end { i64::MAX } else { g.end_ns }))
            .collect()
    };
    let mut acc = windows(first);
    for n in rest {
        let other = windows(n);
        let mut next = Vec::new();
        for &(a0, a1) in &acc {
            for &(b0, b1) in &other {
                let (s, e) = (a0.max(b0), a1.min(b1));
                if s < e {
                    next.push((s, e));
                }
            }
        }
        acc = next;
    }
    acc.sort_unstable();
    acc
}

fn ms(ns: i64) -> String {
    let m = gp_feed::date::ist_minute_of_day(ns / 1_000_000);
    format!(
        "{:02}:{:02}:{:06.3}",
        (m / 60.0) as u32,
        (m % 60.0) as u32,
        (m * 60.0) % 60.0
    )
}

impl Coverage {
    /// A short Markdown summary for people.
    pub fn markdown(&self) -> String {
        let mut s = format!("# Coverage {}\n\n", self.date);
        s += "| Node | Frames | MB | Gaps | Gap time (s) | Longest silence (s) | Bad frames | Unknown keys |\n";
        s += "|---|---|---|---|---|---|---|---|\n";
        for n in &self.nodes {
            if !n.present {
                writeln!(s, "| {} | absent | | | | | | |", n.node).unwrap();
                continue;
            }
            writeln!(
                s,
                "| {} | {} | {:.1} | {} | {:.1} | {:.1} | {} | {} |",
                n.node,
                n.frames,
                n.frame_bytes as f64 / 1e6,
                n.gaps.len(),
                n.gap_ms / 1e3,
                n.longest_silence_ms / 1e3,
                n.decode.bad_frames,
                n.decode.unknown_keys.values().sum::<u64>()
            )
            .unwrap();
        }
        let d = &self.dedup;
        writeln!(
            s,
            "\nEvents: {} in, {} out ({} on both nodes, {} only A, {} only B). {} Parquet files, {} replay events.",
            d.input, d.output, d.both, d.only_a, d.only_b, self.partitions.files, self.replay_events
        )
        .unwrap();
        if self.lost.is_empty() {
            s += "\nNo window with every node in a gap: nothing was lost.\n";
        } else {
            s += "\n**Lost windows (every node in a gap), IST:**\n\n";
            for (a, b) in &self.lost {
                let end = if *b == i64::MAX {
                    "end of day".into()
                } else {
                    ms(*b)
                };
                writeln!(s, "- {} – {}", ms(*a), end).unwrap();
            }
        }
        for n in self.nodes.iter().filter(|n| !n.gaps.is_empty()) {
            writeln!(s, "\nNode {} gaps:\n", n.node).unwrap();
            for g in &n.gaps {
                writeln!(
                    s,
                    "- {} – {} ({:.1} s{}): {}",
                    ms(g.start_ns),
                    ms(g.end_ns),
                    (g.end_ns - g.start_ns) as f64 / 1e9,
                    if g.open_at_end { ", open at end" } else { "" },
                    g.reason
                )
                .unwrap();
            }
        }
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::read::Gap;

    fn node(name: &str, gaps: Vec<Gap>, frames: Vec<i64>) -> NodeReport {
        let log = NodeLog {
            node: name.into(),
            present: true,
            frames: frames.into_iter().map(|t| (t, vec![0; 10])).collect(),
            gaps,
            ..NodeLog::default()
        };
        node_report(&log, DecodeStats::default())
    }

    fn gap(s: i64, e: Option<i64>) -> Gap {
        Gap {
            start_ns: s,
            end_ns: e,
            reason: "stalled".into(),
        }
    }

    #[test]
    fn single_node_gaps_lose_nothing() {
        let a = node("A", vec![gap(100, Some(200))], vec![50, 250]);
        let b = node("B", vec![gap(300, Some(400))], vec![50, 450]);
        assert!(lost_windows(&[a.clone(), b]).is_empty());
        assert_eq!(a.gap_ms, 100.0 / 1e6);
        assert_eq!(a.longest_silence_ms, 200.0 / 1e6);
    }

    #[test]
    fn overlapping_gaps_are_lost() {
        let a = node(
            "A",
            vec![gap(100, Some(300)), gap(900, None)],
            vec![50, 950],
        );
        let b = node(
            "B",
            vec![gap(250, Some(500)), gap(1_000, None)],
            vec![50, 990],
        );
        assert_eq!(
            lost_windows(&[a.clone(), b]),
            [(250, 300), (1_000, i64::MAX)]
        );
        assert!(a.gaps[1].open_at_end);
        assert_eq!(a.gaps[1].end_ns, 950);
    }

    #[test]
    fn absent_node_is_ignored_and_markdown_renders() {
        let a = node("A", vec![gap(100, Some(300))], vec![50, 400]);
        let mut b = node("B", vec![], vec![]);
        b.present = false;
        let lost = lost_windows(&[a.clone(), b.clone()]);
        assert_eq!(lost, [(100, 300)]);
        let c = Coverage {
            date: "2026-10-19".into(),
            nodes: vec![a, b],
            dedup: DedupStats::default(),
            partitions: PartitionStats::default(),
            replay_events: 0,
            lost,
        };
        let md = c.markdown();
        assert!(md.contains("| B | absent |"));
        assert!(md.contains("Lost windows"));
        assert!(md.contains("Node A gaps"));
    }

    #[test]
    fn no_gaps_is_plain_zero() {
        let a = node("A", vec![], vec![50, 60]);
        assert!(a.gap_ms.is_sign_positive());
        let c = Coverage {
            date: "2026-10-19".into(),
            nodes: vec![a],
            dedup: DedupStats::default(),
            partitions: PartitionStats::default(),
            replay_events: 0,
            lost: vec![],
        };
        assert!(!c.markdown().contains("-0.0"));
    }
}
