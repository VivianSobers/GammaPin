//! Reading a log directory: all closed segments at once, or following a live writer.

use std::fs;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use crate::segment::is_torn;
use crate::{Invalid, LogError, Record, list_segments, read_segment, scan, segment_bytes};

/// Every record in `dir`, in segment order. Each segment must be clean.
pub fn read_log(dir: &Path) -> Result<Vec<Record>, LogError> {
    let mut out = Vec::new();
    for seg in list_segments(dir)? {
        out.extend(read_segment(&seg.path)?);
    }
    Ok(out)
}

/// Follows a directory that a [`crate::Writer`] is appending to.
///
/// Each [`Tail::poll`] returns the records completed since the last call. A record the
/// writer has only partly flushed is held back until the rest arrives. The tail moves to the
/// next segment once that segment exists and the current one has no bytes left.
///
/// When an error stops a poll after some records were already read, those records are
/// returned and the error is raised by the next poll.
pub struct Tail {
    dir: PathBuf,
    index: Option<u32>,
    offset: u64,
    pending: Vec<u8>,
    deferred: Option<LogError>,
}

impl Tail {
    /// Start at the first segment in `dir` (which may not exist yet).
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self {
            dir: dir.into(),
            index: None,
            offset: 0,
            pending: Vec::new(),
            deferred: None,
        }
    }

    /// Start at segment `index`, waiting for it if it does not exist yet. Used to move past
    /// a segment whose tail was torn by a writer crash.
    pub fn starting_at(dir: impl Into<PathBuf>, index: u32) -> Self {
        Self {
            index: Some(index),
            ..Self::new(dir)
        }
    }

    /// Segment index being read, once one exists.
    pub fn segment_index(&self) -> Option<u32> {
        self.index
    }

    fn read_new_bytes(&mut self, path: &Path, compressed: bool) -> Result<(), LogError> {
        if compressed {
            let all = segment_bytes(path)?;
            let start = (self.offset as usize).min(all.len());
            self.pending.extend_from_slice(&all[start..]);
            self.offset = all.len() as u64;
        } else {
            let mut f = fs::File::open(path)?;
            f.seek(SeekFrom::Start(self.offset))?;
            let n = f.read_to_end(&mut self.pending)?;
            self.offset += n as u64;
        }
        Ok(())
    }

    fn take_records(&mut self, path: &Path, out: &mut Vec<Record>) -> Result<(), LogError> {
        let s = scan(&self.pending);
        out.extend(s.records);
        match s.tail {
            // A partial record, or the bytes of a torn write: hold them back. They become
            // a torn tail once the writer moves to the next segment.
            None => {
                self.pending.drain(..s.valid_len);
                Ok(())
            }
            Some(reason) if is_torn(reason) => {
                self.pending.drain(..s.valid_len);
                Ok(())
            }
            Some(reason) => Err(LogError::Corrupt {
                path: path.to_path_buf(),
                offset: self.offset - (self.pending.len() - s.valid_len) as u64,
                reason,
            }),
        }
    }

    /// Records completed since the last call, possibly across several segments.
    pub fn poll(&mut self) -> Result<Vec<Record>, LogError> {
        self.poll_inner(false).map(|(r, _)| r)
    }

    /// Like [`Tail::poll`], but a segment left with a torn tail by a writer crash is skipped
    /// past instead of failing, keeping the records before the tear. Returns the number of
    /// bytes skipped.
    pub fn poll_skipping_torn(&mut self) -> Result<(Vec<Record>, u64), LogError> {
        self.poll_inner(true)
    }

    fn poll_inner(&mut self, skip_torn: bool) -> Result<(Vec<Record>, u64), LogError> {
        if let Some(e) = self.deferred.take() {
            return Err(e);
        }
        let mut out = Vec::new();
        let mut skipped = 0;
        match self.advance(skip_torn, &mut out, &mut skipped) {
            Err(e) if !out.is_empty() => {
                self.deferred = Some(e);
                Ok((out, skipped))
            }
            r => r.map(|()| (out, skipped)),
        }
    }

    fn advance(
        &mut self,
        skip_torn: bool,
        out: &mut Vec<Record>,
        skipped: &mut u64,
    ) -> Result<(), LogError> {
        // A listed `.gpl` can be replaced by its `.gpl.zst` before we open it.
        let mut relists = 0;
        loop {
            let segments = list_segments(&self.dir).or_else(|e| match e {
                LogError::Io(io) if io.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
                e => Err(e),
            })?;
            let Some(pos) = (match self.index {
                None => (!segments.is_empty()).then_some(0),
                Some(i) => segments.iter().position(|s| s.index == i),
            }) else {
                return Ok(());
            };
            let seg = &segments[pos];
            self.index = Some(seg.index);
            match self.read_new_bytes(&seg.path, seg.compressed) {
                Err(LogError::Io(io))
                    if io.kind() == std::io::ErrorKind::NotFound && relists < 3 =>
                {
                    relists += 1;
                    continue;
                }
                r => r?,
            }
            self.take_records(&seg.path, out)?;
            match segments.get(pos + 1) {
                // The writer only starts a new segment after the old one is complete,
                // so anything left over here is a torn tail that will never be finished.
                Some(next) => {
                    if !self.pending.is_empty() {
                        if !skip_torn {
                            return Err(LogError::Corrupt {
                                path: seg.path.clone(),
                                offset: self.offset - self.pending.len() as u64,
                                reason: scan(&self.pending).tail.unwrap_or(Invalid::Truncated),
                            });
                        }
                        *skipped += self.pending.len() as u64;
                        self.pending.clear();
                    }
                    self.index = Some(next.index);
                    self.offset = 0;
                }
                None => return Ok(()),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{RecordKind, Writer, WriterConfig, segment_file_name};
    use std::time::Duration;

    fn put(w: &mut Writer, i: u8, len: usize) {
        w.append(RecordKind::RawFrame, 1, i as i64, i as u64, &vec![i; len])
            .unwrap();
    }

    fn cfg(segment_bytes: u64) -> WriterConfig {
        WriterConfig {
            segment_bytes,
            flush_interval: Duration::ZERO,
        }
    }

    #[test]
    fn read_log_spans_segments() {
        let dir = tempfile::tempdir().unwrap();
        let (mut w, _) = Writer::open(dir.path(), cfg(200)).unwrap();
        for i in 0..10 {
            put(&mut w, i, 40);
        }
        w.close().unwrap();
        assert!(list_segments(dir.path()).unwrap().len() > 1);
        let recs = read_log(dir.path()).unwrap();
        assert_eq!(
            recs.iter().map(|r| r.recv_ns).collect::<Vec<_>>(),
            (0..10).collect::<Vec<_>>()
        );
    }

    #[test]
    fn tail_follows_writer_across_rotation() {
        let dir = tempfile::tempdir().unwrap();
        let mut tail = Tail::new(dir.path().join("not-yet"));
        assert!(tail.poll().unwrap().is_empty());

        let mut tail = Tail::new(dir.path());
        assert!(tail.poll().unwrap().is_empty());
        let (mut w, _) = Writer::open(dir.path(), cfg(200)).unwrap();
        let mut seen = Vec::new();
        for i in 0..12 {
            put(&mut w, i, 40);
            w.flush().unwrap();
            seen.extend(tail.poll().unwrap().into_iter().map(|r| r.recv_ns));
        }
        w.close().unwrap();
        seen.extend(tail.poll().unwrap().into_iter().map(|r| r.recv_ns));
        assert_eq!(seen, (0..12).collect::<Vec<_>>());
        assert!(tail.segment_index().unwrap() > 0);
    }

    #[test]
    fn tail_holds_back_partial_record() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(segment_file_name(0));
        let mut buf = Vec::new();
        for i in 0..2u8 {
            crate::Record {
                kind: RecordKind::RawFrame,
                conn_id: 0,
                recv_ns: i as i64,
                mono_ns: 0,
                payload: vec![i; 10],
            }
            .encode_into(&mut buf);
        }
        let split = buf.len() - 5;
        fs::write(&path, &buf[..split]).unwrap();
        let mut tail = Tail::new(dir.path());
        assert_eq!(tail.poll().unwrap().len(), 1);
        fs::write(&path, &buf).unwrap();
        let rest = tail.poll().unwrap();
        assert_eq!(rest.len(), 1);
        assert_eq!(rest[0].recv_ns, 1);
    }

    #[test]
    fn tail_rejects_torn_closed_segment() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join(segment_file_name(0)), [0x47; 9]).unwrap();
        fs::write(dir.path().join(segment_file_name(1)), []).unwrap();
        assert!(matches!(
            Tail::new(dir.path()).poll(),
            Err(LogError::Corrupt {
                reason: Invalid::Truncated,
                ..
            })
        ));
    }

    #[test]
    fn tail_reads_compressed_segment() {
        let dir = tempfile::tempdir().unwrap();
        let (mut w, _) = Writer::open(dir.path(), cfg(1 << 20)).unwrap();
        for i in 0..3 {
            put(&mut w, i, 100);
        }
        w.close().unwrap();
        crate::compress_segment(&dir.path().join(segment_file_name(0))).unwrap();
        assert_eq!(Tail::new(dir.path()).poll().unwrap().len(), 3);
    }

    #[test]
    fn starts_at_a_later_segment() {
        let dir = tempfile::tempdir().unwrap();
        let mut t = Tail::starting_at(dir.path(), 1);
        assert!(t.poll().unwrap().is_empty());
        let (mut w, _) = Writer::open(dir.path(), cfg(200)).unwrap();
        for i in 0..6 {
            put(&mut w, i, 40);
        }
        w.close().unwrap();
        let all = read_log(dir.path()).unwrap();
        let first_seg = read_segment(&dir.path().join(segment_file_name(0))).unwrap();
        let got = t.poll().unwrap();
        assert_eq!(got.len(), all.len() - first_seg.len());
        assert_eq!(got[..], all[first_seg.len()..]);
        assert_eq!(t.segment_index(), Some(w_last(dir.path())));
    }

    fn w_last(dir: &Path) -> u32 {
        list_segments(dir).unwrap().last().unwrap().index
    }

    #[test]
    fn records_before_an_error_come_first() {
        let dir = tempfile::tempdir().unwrap();
        let (mut w, _) = Writer::open(dir.path(), cfg(1 << 20)).unwrap();
        put(&mut w, 1, 10);
        w.close().unwrap();
        let seg0 = dir.path().join(segment_file_name(0));
        let mut bytes = fs::read(&seg0).unwrap();
        bytes.extend_from_slice(&[0x31, 0x4C, 0x50]);
        fs::write(&seg0, &bytes).unwrap();
        fs::write(dir.path().join(segment_file_name(1)), b"").unwrap();

        let mut t = Tail::new(dir.path());
        assert_eq!(t.poll().unwrap().len(), 1);
        assert!(matches!(t.poll(), Err(LogError::Corrupt { .. })));
    }

    #[test]
    fn torn_bad_crc_is_a_torn_tail() {
        let dir = tempfile::tempdir().unwrap();
        let (mut w, _) = Writer::open(dir.path(), cfg(1 << 20)).unwrap();
        put(&mut w, 1, 10);
        put(&mut w, 2, 10);
        w.close().unwrap();
        let seg0 = dir.path().join(segment_file_name(0));
        let mut bytes = fs::read(&seg0).unwrap();
        let last = bytes.len() - 1;
        bytes[last] ^= 0xFF; // second record's CRC no longer matches
        fs::write(&seg0, &bytes).unwrap();
        // still the live segment: held back, not an error
        let mut t = Tail::new(dir.path());
        assert_eq!(t.poll().unwrap().len(), 1);
        // once the writer has moved on, it is skipped like any torn tail
        fs::write(dir.path().join(segment_file_name(1)), b"").unwrap();
        let (recs, skipped) = t.poll_skipping_torn().unwrap();
        assert!(recs.is_empty());
        assert_eq!(skipped, (crate::HEADER_LEN + 10) as u64);
    }

    #[test]
    fn lenient_poll_keeps_records_before_a_tear() {
        let dir = tempfile::tempdir().unwrap();
        let (mut w, _) = Writer::open(dir.path(), cfg(1 << 20)).unwrap();
        put(&mut w, 1, 10);
        put(&mut w, 2, 10);
        w.close().unwrap();
        let seg0 = dir.path().join(segment_file_name(0));
        let mut bytes = fs::read(&seg0).unwrap();
        bytes.extend_from_slice(&[0x31, 0x4C, 0x50]);
        fs::write(&seg0, &bytes).unwrap();
        fs::write(dir.path().join(segment_file_name(1)), b"").unwrap();

        let mut strict = Tail::new(dir.path());
        assert_eq!(strict.poll().unwrap().len(), 2);
        assert!(strict.poll().is_err());
        let mut t = Tail::new(dir.path());
        let (recs, skipped) = t.poll_skipping_torn().unwrap();
        assert_eq!(recs.len(), 2);
        assert_eq!(skipped, 3);
        assert_eq!(t.segment_index(), Some(1));
    }
}
