//! Rotating GPL1 writer. Not thread-safe; the capture task owns one per connection.

use std::fs::{File, OpenOptions};
use std::io::{BufWriter, Write};
use std::path::PathBuf;
use std::time::{Duration, Instant};

use crate::{
    FLUSH_INTERVAL_MS, HEADER_LEN, LogError, MAX_PAYLOAD, RecordKind, Recovery, SEGMENT_BYTES,
    encode_header, list_segments, recover_segment, segment_file_name,
};

#[derive(Debug, Clone, Copy)]
pub struct WriterConfig {
    pub segment_bytes: u64,
    pub flush_interval: Duration,
}

impl Default for WriterConfig {
    fn default() -> Self {
        Self {
            segment_bytes: SEGMENT_BYTES,
            flush_interval: Duration::from_millis(FLUSH_INTERVAL_MS),
        }
    }
}

/// After any I/O error the writer is poisoned; drop it and call `Writer::open` again,
/// which recovers the torn tail and starts a new segment.
pub struct Writer {
    dir: PathBuf,
    cfg: WriterConfig,
    index: u32,
    /// None once poisoned: the buffer is discarded so nothing after the error reaches disk.
    file: Option<BufWriter<File>>,
    seg_len: u64,
    last_flush: Instant,
    poisoned: bool,
}

fn create_segment(dir: &std::path::Path, index: u32) -> Result<BufWriter<File>, LogError> {
    let file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(dir.join(segment_file_name(index)))?;
    File::open(dir)?.sync_all()?;
    Ok(BufWriter::with_capacity(1 << 20, file))
}

impl Writer {
    /// Open a log directory: recover the newest uncompressed segment, then start a new one after it.
    pub fn open(
        dir: impl Into<PathBuf>,
        cfg: WriterConfig,
    ) -> Result<(Self, Option<Recovery>), LogError> {
        let dir = dir.into();
        std::fs::create_dir_all(&dir)?;
        let segments = list_segments(&dir)?;
        let mut recovery = None;
        let index = match segments.last() {
            Some(last) => {
                if !last.compressed {
                    recovery = Some(recover_segment(&last.path)?);
                }
                last.index + 1
            }
            None => 0,
        };
        let file = create_segment(&dir, index)?;
        let writer = Self {
            dir,
            cfg,
            index,
            file: Some(file),
            seg_len: 0,
            last_flush: Instant::now(),
            poisoned: false,
        };
        Ok((writer, recovery))
    }

    pub fn append(
        &mut self,
        kind: RecordKind,
        conn_id: u8,
        recv_ns: i64,
        mono_ns: u64,
        payload: &[u8],
    ) -> Result<(), LogError> {
        let need = HEADER_LEN + payload.len();
        if payload.len() > MAX_PAYLOAD || need as u64 > self.cfg.segment_bytes {
            return Err(LogError::PayloadTooLarge(need));
        }
        if self.poisoned {
            return Err(LogError::Poisoned);
        }
        let result = (|| {
            if self.seg_len + need as u64 > self.cfg.segment_bytes {
                self.rotate()?;
            }
            let file = self.file()?;
            file.write_all(&encode_header(kind, conn_id, recv_ns, mono_ns, payload))?;
            file.write_all(payload)?;
            self.seg_len += need as u64;
            if self.last_flush.elapsed() >= self.cfg.flush_interval {
                self.flush()?;
            }
            Ok(())
        })();
        if result.is_err() {
            self.poison();
        }
        result
    }

    fn file(&mut self) -> Result<&mut BufWriter<File>, LogError> {
        self.file.as_mut().ok_or(LogError::Poisoned)
    }

    /// Mark the writer unusable and drop its buffer without writing it out.
    fn poison(&mut self) {
        self.poisoned = true;
        if let Some(f) = self.file.take() {
            let (_file, _unwritten) = f.into_parts();
        }
    }

    /// Push buffered records to the kernel and to disk.
    ///
    /// `append` flushes when `flush_interval` has passed, but only when records arrive;
    /// the owner should also call `flush` on a `flush_interval` timer so a quiet feed
    /// is still persisted.
    pub fn flush(&mut self) -> Result<(), LogError> {
        if self.poisoned {
            return Err(LogError::Poisoned);
        }
        let result = (|| {
            let file = self.file()?;
            file.flush()?;
            file.get_ref().sync_data()?;
            self.last_flush = Instant::now();
            Ok(())
        })();
        if result.is_err() {
            self.poison();
        }
        result
    }

    fn rotate(&mut self) -> Result<(), LogError> {
        self.flush()?;
        let next = self.index + 1;
        let file = create_segment(&self.dir, next)?;
        self.index = next;
        self.file = Some(file);
        self.seg_len = 0;
        Ok(())
    }

    pub fn segment_index(&self) -> u32 {
        self.index
    }

    pub fn close(mut self) -> Result<(), LogError> {
        self.flush()
    }
}

impl Drop for Writer {
    fn drop(&mut self) {
        if let Some(f) = &mut self.file {
            let _ = f.flush();
            let _ = f.get_ref().sync_data();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{read_segment, segment_file_name};
    use std::time::Duration;

    fn cfg(segment_bytes: u64, flush_interval: Duration) -> WriterConfig {
        WriterConfig {
            segment_bytes,
            flush_interval,
        }
    }

    fn put(w: &mut Writer, i: u8, len: usize) {
        w.append(RecordKind::RawFrame, 1, i as i64, i as u64, &vec![i; len])
            .unwrap();
    }

    #[test]
    fn drop_without_close_persists_records() {
        let dir = tempfile::tempdir().unwrap();
        let (mut w, _) = Writer::open(dir.path(), cfg(1 << 20, Duration::from_secs(3600))).unwrap();
        put(&mut w, 1, 10);
        drop(w);
        assert_eq!(
            read_segment(&dir.path().join(segment_file_name(0)))
                .unwrap()
                .len(),
            1
        );
    }

    #[test]
    fn failed_rotation_poisons_and_keeps_index() {
        let dir = tempfile::tempdir().unwrap();
        let rec_len = (HEADER_LEN + 10) as u64;
        let (mut w, _) = Writer::open(dir.path(), cfg(rec_len, Duration::ZERO)).unwrap();
        put(&mut w, 1, 10);
        // A directory squatting on the next segment name makes create_new fail.
        std::fs::create_dir(dir.path().join(segment_file_name(1))).unwrap();
        assert!(matches!(
            w.append(RecordKind::RawFrame, 1, 0, 0, &[0; 10]),
            Err(LogError::Io(_))
        ));
        assert_eq!(w.segment_index(), 0);
        assert!(matches!(
            w.append(RecordKind::RawFrame, 1, 0, 0, &[0; 1]),
            Err(LogError::Poisoned)
        ));
        assert!(matches!(w.flush(), Err(LogError::Poisoned)));
        assert_eq!(
            read_segment(&dir.path().join(segment_file_name(0)))
                .unwrap()
                .len(),
            1
        );
    }

    #[test]
    fn oversized_payload_does_not_poison() {
        let dir = tempfile::tempdir().unwrap();
        let (mut w, _) = Writer::open(dir.path(), cfg(100, Duration::ZERO)).unwrap();
        assert!(w.append(RecordKind::RawFrame, 1, 0, 0, &[0; 69]).is_err());
        w.append(RecordKind::RawFrame, 1, 0, 0, &[0; 68]).unwrap();
    }

    #[test]
    fn rotates_without_splitting_records() {
        let dir = tempfile::tempdir().unwrap();
        let rec_len = (HEADER_LEN + 10) as u64;
        let (mut w, rec) = Writer::open(dir.path(), cfg(2 * rec_len, Duration::ZERO)).unwrap();
        assert!(rec.is_none());
        for i in 0..5 {
            put(&mut w, i, 10);
        }
        w.close().unwrap();
        let counts: Vec<usize> = (0..3)
            .map(|i| {
                read_segment(&dir.path().join(segment_file_name(i)))
                    .unwrap()
                    .len()
            })
            .collect();
        assert_eq!(counts, vec![2, 2, 1]); // exact fit stays in one segment
        assert!(!dir.path().join(segment_file_name(3)).exists());
    }

    #[test]
    fn rejects_record_larger_than_segment() {
        let dir = tempfile::tempdir().unwrap();
        let (mut w, _) = Writer::open(dir.path(), cfg(100, Duration::ZERO)).unwrap();
        let err = w
            .append(RecordKind::RawFrame, 1, 0, 0, &[0; 69])
            .unwrap_err();
        assert!(matches!(err, LogError::PayloadTooLarge(_)));
        w.append(RecordKind::RawFrame, 1, 0, 0, &[0; 68]).unwrap();
        assert_eq!(w.segment_index(), 0);
    }

    #[test]
    fn flush_interval_controls_visibility() {
        let dir = tempfile::tempdir().unwrap();
        let seg0 = dir.path().join(segment_file_name(0));
        let (mut w, _) = Writer::open(dir.path(), cfg(1 << 20, Duration::from_secs(3600))).unwrap();
        put(&mut w, 1, 10);
        assert_eq!(std::fs::metadata(&seg0).unwrap().len(), 0);
        w.flush().unwrap();
        assert_eq!(
            std::fs::metadata(&seg0).unwrap().len(),
            (HEADER_LEN + 10) as u64
        );

        let dir2 = tempfile::tempdir().unwrap();
        let (mut w2, _) = Writer::open(dir2.path(), cfg(1 << 20, Duration::ZERO)).unwrap();
        put(&mut w2, 1, 10);
        assert_eq!(
            std::fs::metadata(dir2.path().join(segment_file_name(0)))
                .unwrap()
                .len(),
            (HEADER_LEN + 10) as u64
        );
    }

    #[test]
    fn reopen_after_crash_recovers_and_starts_next_segment() {
        let dir = tempfile::tempdir().unwrap();
        let (mut w, _) = Writer::open(dir.path(), cfg(1 << 20, Duration::ZERO)).unwrap();
        for i in 0..3 {
            put(&mut w, i, 8);
        }
        drop(w);
        let seg0 = dir.path().join(segment_file_name(0));
        let mut f = std::fs::OpenOptions::new()
            .append(true)
            .open(&seg0)
            .unwrap();
        std::io::Write::write_all(&mut f, &[0x47; 11]).unwrap(); // torn tail

        let (mut w, rec) = Writer::open(dir.path(), cfg(1 << 20, Duration::ZERO)).unwrap();
        let rec = rec.expect("recovered the last segment");
        assert_eq!((rec.records, rec.dropped_bytes), (3, 11));
        assert_eq!(w.segment_index(), 1);
        put(&mut w, 9, 8);
        w.close().unwrap();
        assert_eq!(read_segment(&seg0).unwrap().len(), 3);
        assert_eq!(
            read_segment(&dir.path().join(segment_file_name(1)))
                .unwrap()
                .len(),
            1
        );
    }
}
