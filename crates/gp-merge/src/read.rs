//! One node's log for one day: its frames, and its gaps as intervals.

use std::path::Path;

use gp_log::{LogError, RecordKind, list_segments, read_segment};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Gap {
    pub start_ns: i64,
    /// None when the log ends inside the gap.
    pub end_ns: Option<i64>,
    pub reason: String,
}

#[derive(Debug, Clone, Default)]
pub struct NodeLog {
    pub node: String,
    /// This node's `src_mask` bit.
    pub bit: u8,
    /// False when the node has no directory for the day (down all day, or not synced).
    pub present: bool,
    pub segments: usize,
    /// (receive time, frame payload) in log order.
    pub frames: Vec<(i64, Vec<u8>)>,
    pub gaps: Vec<Gap>,
    pub subscribes: usize,
    pub token_reloads: usize,
}

impl NodeLog {
    pub fn frame_bytes(&self) -> u64 {
        self.frames.iter().map(|(_, p)| p.len() as u64).sum()
    }

    /// Longest time between consecutive frames, ns (catches restarts that left no gap record).
    pub fn longest_silence_ns(&self) -> i64 {
        self.frames
            .windows(2)
            .map(|w| w[1].0 - w[0].0)
            .max()
            .unwrap_or(0)
    }
}

/// Read every segment of `dir`. Each must be clean: capture recovers torn tails on restart,
/// so a torn segment here means the copy to the inbox is incomplete.
pub fn read_node(dir: &Path, node: &str, bit: u8) -> Result<NodeLog, LogError> {
    let mut log = NodeLog {
        node: node.to_string(),
        bit,
        ..NodeLog::default()
    };
    if !dir.is_dir() {
        return Ok(log);
    }
    log.present = true;
    let mut open: Option<Gap> = None;
    for seg in list_segments(dir)? {
        log.segments += 1;
        for r in read_segment(&seg.path)? {
            match r.kind {
                RecordKind::RawFrame => log.frames.push((r.recv_ns, r.payload)),
                RecordKind::GapStart => {
                    if open.is_none() {
                        open = Some(Gap {
                            start_ns: r.recv_ns,
                            end_ns: None,
                            reason: String::from_utf8_lossy(&r.payload).into_owned(),
                        });
                    }
                }
                RecordKind::GapEnd => {
                    if let Some(mut g) = open.take() {
                        g.end_ns = Some(r.recv_ns);
                        log.gaps.push(g);
                    }
                }
                RecordKind::Subscribe => log.subscribes += 1,
                RecordKind::TokenReload => log.token_reloads += 1,
            }
        }
    }
    log.gaps.extend(open);
    Ok(log)
}

#[cfg(test)]
mod tests {
    use super::*;
    use gp_log::{Writer, WriterConfig};

    #[test]
    fn frames_and_gap_intervals() {
        let dir = tempfile::tempdir().unwrap();
        let (mut w, _) = Writer::open(
            dir.path(),
            WriterConfig {
                segment_bytes: 120,
                ..WriterConfig::default()
            },
        )
        .unwrap();
        let mut put = |k, t, p: &[u8]| w.append(k, 0, t, t as u64, p).unwrap();
        put(RecordKind::Subscribe, 1, b"sub");
        put(RecordKind::RawFrame, 10, b"f1");
        put(RecordKind::GapStart, 20, b"stalled");
        put(RecordKind::GapEnd, 50, b"");
        put(RecordKind::RawFrame, 60, b"f2");
        put(RecordKind::TokenReload, 61, b"token");
        put(RecordKind::RawFrame, 200, b"f3");
        put(RecordKind::GapStart, 210, b"closed: eof");
        w.close().unwrap();

        let log = read_node(dir.path(), "B", 2).unwrap();
        assert!(log.present && log.segments > 1);
        assert_eq!(
            log.frames.iter().map(|f| f.0).collect::<Vec<_>>(),
            [10, 60, 200]
        );
        assert_eq!(log.frame_bytes(), 6);
        assert_eq!(log.longest_silence_ns(), 140);
        assert_eq!(
            log.gaps,
            [
                Gap {
                    start_ns: 20,
                    end_ns: Some(50),
                    reason: "stalled".into()
                },
                Gap {
                    start_ns: 210,
                    end_ns: None,
                    reason: "closed: eof".into()
                },
            ]
        );
        assert_eq!((log.subscribes, log.token_reloads), (1, 1));
    }

    #[test]
    fn missing_node_is_absent_not_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let log = read_node(&dir.path().join("A/2026-10-19"), "A", 1).unwrap();
        assert!(!log.present);
        assert!(log.frames.is_empty());
    }
}
