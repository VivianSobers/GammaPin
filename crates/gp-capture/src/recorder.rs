//! The capture process's GPL1 log: every frame of every connection is written as received,
//! with both clocks.

use std::path::PathBuf;

use gp_log::{LogError, RecordKind, Recovery, Writer, WriterConfig};

/// `CLOCK_REALTIME` in ns (chrony keeps the nodes within a few ms of each other).
pub fn realtime_ns() -> i64 {
    let mut ts = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    // SAFETY: valid clock id and a valid out-pointer.
    unsafe { libc::clock_gettime(libc::CLOCK_REALTIME, &mut ts) };
    ts.tv_sec * 1_000_000_000 + ts.tv_nsec
}

/// `CLOCK_MONOTONIC` in ns: never steps, so intervals survive clock corrections.
pub fn monotonic_ns() -> u64 {
    let mut ts = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    // SAFETY: valid clock id and a valid out-pointer.
    unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC, &mut ts) };
    ts.tv_sec as u64 * 1_000_000_000 + ts.tv_nsec as u64
}

/// Record `conn_id` for a capture node: A → 1, B → 2, anything else 0. It tags which node
/// wrote a log; readers do not depend on it.
pub fn conn_id_for(node: &str) -> u8 {
    match node {
        "A" => 1,
        "B" => 2,
        _ => 0,
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Stats {
    pub frames: u64,
    pub frame_bytes: u64,
    pub gaps: u64,
}

pub struct Recorder {
    writer: Writer,
    conn_id: u8,
    in_gap: bool,
    stats: Stats,
}

impl Recorder {
    /// Open (and recover) the log in `dir`. `conn_id` is stamped on every record; the
    /// capture binary passes [`conn_id_for`] of its node name.
    pub fn open(
        dir: impl Into<PathBuf>,
        cfg: WriterConfig,
        conn_id: u8,
    ) -> Result<(Self, Option<Recovery>), LogError> {
        let (writer, recovery) = Writer::open(dir, cfg)?;
        Ok((
            Self {
                writer,
                conn_id,
                in_gap: false,
                stats: Stats::default(),
            },
            recovery,
        ))
    }

    fn put(&mut self, kind: RecordKind, payload: &[u8]) -> Result<i64, LogError> {
        let recv_ns = realtime_ns();
        self.writer
            .append(kind, self.conn_id, recv_ns, monotonic_ns(), payload)?;
        Ok(recv_ns)
    }

    /// Write one WebSocket frame exactly as received; returns its receive time.
    pub fn raw_frame(&mut self, payload: &[u8]) -> Result<i64, LogError> {
        let t = self.put(RecordKind::RawFrame, payload)?;
        self.stats.frames += 1;
        self.stats.frame_bytes += payload.len() as u64;
        Ok(t)
    }

    /// The feed is gone (disconnect, error, stall). Only the first call of a gap is written.
    pub fn gap_start(&mut self, reason: &str) -> Result<(), LogError> {
        if !self.in_gap {
            self.put(RecordKind::GapStart, reason.as_bytes())?;
            self.in_gap = true;
            self.stats.gaps += 1;
        }
        Ok(())
    }

    /// Data flows again. A no-op outside a gap.
    pub fn gap_end(&mut self) -> Result<(), LogError> {
        if self.in_gap {
            self.put(RecordKind::GapEnd, b"")?;
            self.in_gap = false;
        }
        Ok(())
    }

    pub fn in_gap(&self) -> bool {
        self.in_gap
    }

    /// A subscription request as sent.
    pub fn subscribe(&mut self, request: &[u8]) -> Result<(), LogError> {
        self.put(RecordKind::Subscribe, request).map(drop)
    }

    /// A new access token was picked up; `note` must not contain the token itself.
    pub fn token_reload(&mut self, note: &str) -> Result<(), LogError> {
        self.put(RecordKind::TokenReload, note.as_bytes()).map(drop)
    }

    pub fn flush(&mut self) -> Result<(), LogError> {
        self.writer.flush()
    }

    pub fn stats(&self) -> Stats {
        self.stats
    }

    pub fn close(self) -> Result<(), LogError> {
        self.writer.close()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gp_log::read_log;

    #[test]
    fn records_frames_and_gap_transitions() {
        let dir = tempfile::tempdir().unwrap();
        let (mut r, rec) = Recorder::open(dir.path(), WriterConfig::default(), 2).unwrap();
        assert!(rec.is_none());
        r.subscribe(b"{\"method\":\"sub\"}").unwrap();
        let t0 = r.raw_frame(b"one").unwrap();
        r.gap_end().unwrap(); // not in a gap: nothing written
        r.gap_start("closed by peer").unwrap();
        r.gap_start("connect refused").unwrap(); // same gap
        assert!(r.in_gap());
        r.gap_end().unwrap();
        r.raw_frame(b"two").unwrap();
        r.token_reload("token file changed").unwrap();
        assert_eq!(
            r.stats(),
            Stats {
                frames: 2,
                frame_bytes: 6,
                gaps: 1
            }
        );
        r.close().unwrap();

        let recs = read_log(dir.path()).unwrap();
        let kinds: Vec<_> = recs.iter().map(|r| r.kind).collect();
        use RecordKind::*;
        assert_eq!(
            kinds,
            [Subscribe, RawFrame, GapStart, GapEnd, RawFrame, TokenReload]
        );
        assert_eq!(recs[2].payload, b"closed by peer");
        assert!(recs.iter().all(|r| r.conn_id == 2));
        assert_eq!(recs[1].recv_ns, t0);
        assert!(recs.windows(2).all(|w| w[0].mono_ns <= w[1].mono_ns));
    }

    #[test]
    fn clocks_are_sane() {
        let now = realtime_ns();
        assert!(now > 1_700_000_000_000_000_000); // after Nov 2023
        let (a, b) = (monotonic_ns(), monotonic_ns());
        assert!(b >= a);
    }
}
