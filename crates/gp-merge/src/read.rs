//! One node's log for one day: its frames, and its gaps as intervals.

use std::path::Path;

use gp_log::{LogError, RecordKind, is_torn, list_segments, scan, segment_bytes};

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
    /// Bytes of a torn tail dropped from the last segment.
    pub torn_bytes: u64,
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

/// Read every segment of `dir`. Earlier segments must be clean, since capture recovers torn
/// tails on restart. The last one may end in a torn record (capture was killed and not
/// restarted): its valid prefix is kept and the torn bytes are counted.
///
/// A gap still open when capture stopped is closed by the first market-data frame after it,
/// because a restarted capture starts without an open gap and never writes its `GAP_END`.
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
    let segments = list_segments(dir)?;
    for (i, seg) in segments.iter().enumerate() {
        log.segments += 1;
        let buf = segment_bytes(&seg.path)?;
        let s = scan(&buf);
        if let Some(reason) = s.tail {
            if i + 1 < segments.len() || !is_torn(reason) {
                return Err(LogError::Corrupt {
                    path: seg.path.clone(),
                    offset: s.valid_len as u64,
                    reason,
                });
            }
            log.torn_bytes += (buf.len() - s.valid_len) as u64;
        }
        for r in s.records {
            match r.kind {
                RecordKind::RawFrame => {
                    if open.is_some() && gp_feed::carries_data(&r.payload) {
                        let mut g = open.take().expect("open");
                        g.end_ns = Some(r.recv_ns);
                        log.gaps.push(g);
                    }
                    log.frames.push((r.recv_ns, r.payload));
                }
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
    fn restart_closes_an_open_gap_and_a_torn_tail_is_kept() {
        use gp_feed::synth::{Market, MarketConfig, universe};
        use prost::Message;
        let date = gp_feed::Date::from_ymd(2026, 10, 19);
        let mut m = Market::new(
            MarketConfig::new(date),
            universe(date, 25_000.0, 1, 2.0, 50.0),
        );
        let data = m.next_frame().encode_to_vec();
        let dir = tempfile::tempdir().unwrap();
        let (mut w, _) = Writer::open(dir.path(), WriterConfig::default()).unwrap();
        w.append(RecordKind::GapStart, 0, 20, 0, b"closed: eof")
            .unwrap();
        w.close().unwrap();
        // the restarted capture writes no GAP_END: its first data frame ends the gap
        let (mut w, _) = Writer::open(dir.path(), WriterConfig::default()).unwrap();
        w.append(RecordKind::RawFrame, 0, 90, 0, b"f").unwrap();
        w.append(RecordKind::RawFrame, 0, 100, 0, &data).unwrap();
        w.close().unwrap();
        let last = gp_log::list_segments(dir.path())
            .unwrap()
            .pop()
            .unwrap()
            .path;
        let mut bytes = std::fs::read(&last).unwrap();
        bytes.extend_from_slice(&[0x31, 0x4C, 0x50, 0x47, 9, 9, 9]);
        std::fs::write(&last, bytes).unwrap();

        let log = read_node(dir.path(), "A", 1).unwrap();
        assert_eq!(log.frames.len(), 2);
        assert_eq!(log.torn_bytes, 7);
        assert_eq!(
            log.gaps,
            [Gap {
                start_ns: 20,
                end_ns: Some(100),
                reason: "closed: eof".into()
            }]
        );
    }

    #[test]
    fn missing_node_is_absent_not_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let log = read_node(&dir.path().join("A/2026-10-19"), "A", 1).unwrap();
        assert!(!log.present);
        assert!(log.frames.is_empty());
    }
}
