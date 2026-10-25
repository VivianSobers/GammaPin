//! The live side of the unified stream: follow each node's log as capture writes it.
//!
//! Every node's events are passed on, each stamped with its own `src_mask` bit: the
//! engine's book ignores the second copy of an update, and seeing both copies is what tells
//! the health panel that both nodes are up.

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::Arc;

use gp_feed::{Event, Master, Normalizer, decode_frame, updates};
use gp_log::{LogError, RecordKind, Tail};
use gp_replay::{EventStream, Poll, Source, StreamError};

/// Records taken from one node per poll, at segment granularity: a monitor restarted in
/// the middle of the day works through the backlog in steps, both nodes in turn.
const POLL_BUDGET: usize = 20_000;

struct Node {
    dir: PathBuf,
    bit: u8,
    tail: Tail,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct LiveStats {
    pub frames: u64,
    pub bad_frames: u64,
    pub unknown_updates: u64,
    /// Bytes of torn segment tails skipped after a capture crash.
    pub torn_bytes: u64,
    /// Segments abandoned because of a bad record in the middle.
    pub skipped_segments: u64,
}

pub struct LiveStream {
    nodes: Vec<Node>,
    master: Arc<Master>,
    norm: Normalizer,
    queue: VecDeque<Event>,
    gaps: Vec<i64>,
    position_ms: Option<i64>,
    stats: LiveStats,
}

impl LiveStream {
    /// Follow `dirs` (one per node, in `src_mask` bit order).
    pub fn new(dirs: Vec<PathBuf>, master: Arc<Master>, norm: Normalizer) -> Self {
        let nodes = dirs
            .into_iter()
            .enumerate()
            .map(|(bit, dir)| Node {
                tail: Tail::new(&dir).with_budget(POLL_BUDGET),
                dir,
                bit: bit as u8,
            })
            .collect();
        Self {
            nodes,
            master,
            norm,
            queue: VecDeque::new(),
            gaps: Vec::new(),
            position_ms: None,
            stats: LiveStats::default(),
        }
    }

    pub fn stats(&self) -> LiveStats {
        self.stats
    }

    fn fill(&mut self) -> Result<(), StreamError> {
        for node in &mut self.nodes {
            let records = match node.tail.poll_skipping_torn() {
                Ok((r, torn)) => {
                    self.stats.torn_bytes += torn;
                    r
                }
                Err(LogError::Corrupt { path, .. }) => {
                    // Damage the writer can't have caused by stopping: give up on the segment.
                    let next = node.tail.segment_index().map_or(0, |i| i + 1);
                    tracing::warn!("skipping corrupt segment {}", path.display());
                    node.tail = Tail::starting_at(&node.dir, next).with_budget(POLL_BUDGET);
                    self.stats.skipped_segments += 1;
                    continue;
                }
                Err(e) => return Err(e.into()),
            };
            for r in records {
                match r.kind {
                    RecordKind::RawFrame => {
                        self.stats.frames += 1;
                        let Ok(resp) = decode_frame(&r.payload) else {
                            self.stats.bad_frames += 1;
                            continue;
                        };
                        for u in updates(&resp) {
                            match self.master.get(&u.instrument_key) {
                                Some(ins) => self.queue.push_back(self.norm.event(
                                    &u,
                                    ins,
                                    r.recv_ns,
                                    1 << node.bit,
                                )),
                                None => self.stats.unknown_updates += 1,
                            }
                        }
                    }
                    RecordKind::GapStart => self.gaps.push(r.recv_ns.div_euclid(1_000_000)),
                    _ => {}
                }
            }
        }
        Ok(())
    }
}

impl Drop for LiveStream {
    fn drop(&mut self) {
        tracing::info!("live stream: {:?}", self.stats);
    }
}

impl EventStream for LiveStream {
    fn source(&self) -> Source {
        Source::Live
    }

    fn poll_event(&mut self) -> Result<Poll, StreamError> {
        if self.queue.is_empty() {
            self.fill()?;
        }
        Ok(match self.queue.pop_front() {
            Some(e) => {
                self.position_ms = Some(self.position_ms.map_or(e.ltt_ms, |p| p.max(e.ltt_ms)));
                Poll::Ready(e)
            }
            None => Poll::Pending,
        })
    }

    fn position_ms(&self) -> Option<i64> {
        self.position_ms
    }

    fn take_gaps(&mut self) -> Vec<i64> {
        std::mem::take(&mut self.gaps)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gp_feed::Date;
    use gp_feed::synth::{Market, MarketConfig, universe};
    use gp_log::{Writer, WriterConfig};
    use prost::Message;
    use std::time::Duration;

    #[test]
    fn follows_two_writers() {
        let date = Date::from_ymd(2026, 10, 19);
        let u = universe(date, 25_000.0, 1, 2.0, 50.0);
        let master = Arc::new(Master::new(u.clone()));
        let mut m = Market::new(MarketConfig::new(date), u);
        let root = tempfile::tempdir().unwrap();
        let dirs = vec![root.path().join("A"), root.path().join("B")];
        let mut live = LiveStream::new(dirs.clone(), master, Normalizer::default());
        assert_eq!(live.poll_event().unwrap(), Poll::Pending);

        let cfg = WriterConfig {
            segment_bytes: 64 * 1024,
            flush_interval: Duration::ZERO,
        };
        let (mut a, _) = Writer::open(&dirs[0], cfg).unwrap();
        let (mut b, _) = Writer::open(&dirs[1], cfg).unwrap();
        let mut sent = 0;
        for i in 0..30 {
            let f = m.next_frame();
            sent += f.feeds.len();
            let bytes = f.encode_to_vec();
            a.append(RecordKind::RawFrame, 0, i, 0, &bytes).unwrap();
            if i == 10 {
                b.append(RecordKind::GapStart, 0, 5_000_000, 0, b"stalled")
                    .unwrap();
            }
            if i >= 10 {
                b.append(RecordKind::RawFrame, 0, i, 0, &bytes).unwrap();
            }
        }
        a.flush().unwrap();
        b.flush().unwrap();

        let mut got = Vec::new();
        while let Poll::Ready(e) = live.poll_event().unwrap() {
            got.push(e);
        }
        let from_a = got.iter().filter(|e| e.src_mask == 1).count();
        let from_b = got.iter().filter(|e| e.src_mask == 2).count();
        assert_eq!(from_a, sent);
        assert!(from_b > 0 && from_b < sent);
        assert_eq!(live.take_gaps(), [5]);
        assert!(live.take_gaps().is_empty());
        assert_eq!(live.stats().frames, 50);
        assert_eq!(live.position_ms(), got.iter().map(|e| e.ltt_ms).max());
        // the writers rotated through several segments meanwhile
        assert!(gp_log::list_segments(&dirs[0]).unwrap().len() > 2);
    }

    #[test]
    fn skips_a_torn_segment() {
        let root = tempfile::tempdir().unwrap();
        let dir = root.path().join("A");
        std::fs::create_dir_all(&dir).unwrap();
        let cfg = WriterConfig {
            segment_bytes: 1 << 20,
            flush_interval: Duration::ZERO,
        };
        let (mut w, _) = Writer::open(&dir, cfg).unwrap();
        w.append(RecordKind::GapStart, 0, 1_000_000, 0, b"x")
            .unwrap();
        w.close().unwrap();
        // tear the first segment, then let a restarted writer continue in the next one
        let seg0 = dir.join(gp_log::segment_file_name(0));
        let mut bytes = std::fs::read(&seg0).unwrap();
        bytes.extend_from_slice(&[0x31, 0x4C, 0x50, 0x47, 9, 9, 9]);
        let mut live = LiveStream::new(
            vec![dir.clone()],
            Arc::new(Master::new(Vec::new())),
            Normalizer::default(),
        );
        std::fs::write(&seg0, &bytes).unwrap();
        std::fs::write(dir.join(gp_log::segment_file_name(1)), b"").unwrap();
        assert_eq!(live.poll_event().unwrap(), Poll::Pending);
        assert_eq!(live.stats().torn_bytes, 7);
        assert_eq!(live.take_gaps(), [1]);

        // a bad CRC at the end of a closed segment is a torn write too: skipped, next followed
        let seg1 = dir.join(gp_log::segment_file_name(1));
        let mut bad = bytes.clone();
        bad.truncate(bytes.len() - 7);
        let last = bad.len() - 1;
        bad[last] ^= 0xFF;
        std::fs::write(&seg1, &bad).unwrap();
        std::fs::write(dir.join(gp_log::segment_file_name(2)), b"").unwrap();
        assert_eq!(live.poll_event().unwrap(), Poll::Pending);
        assert_eq!(live.stats().torn_bytes, 7 + bad.len() as u64);
        assert_eq!(live.stats().skipped_segments, 0);
        assert_eq!(live.nodes[0].tail.segment_index(), Some(2));
    }
}
