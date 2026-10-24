//! Whole-segment operations: scan, read, recover and list.

use std::fs;
use std::path::{Path, PathBuf};

use crate::{Invalid, LogError, Record, decode};

pub struct Scan {
    pub records: Vec<Record>,
    /// Length of the valid prefix in bytes.
    pub valid_len: usize,
    /// Why scanning stopped before the end, if it did.
    pub tail: Option<Invalid>,
}

/// Decode records from the start of `buf` until the end or the first invalid record.
pub fn scan(buf: &[u8]) -> Scan {
    let mut records = Vec::new();
    let mut off = 0;
    while off < buf.len() {
        match decode(&buf[off..]) {
            Ok((record, n)) => {
                records.push(record);
                off += n;
            }
            Err(e) => {
                return Scan {
                    records,
                    valid_len: off,
                    tail: Some(e),
                };
            }
        }
    }
    Scan {
        records,
        valid_len: off,
        tail: None,
    }
}

/// zstd level for closed segments: fast to write, about 4x smaller on feed frames.
pub const ZSTD_LEVEL: i32 = 3;

/// Whether a scan stopping at `reason` looks like a torn write (a crash mid-record) rather
/// than a record this version cannot read. Recovery truncates torn tails; readers treat
/// them as the end of the data.
pub(crate) fn is_torn(reason: Invalid) -> bool {
    !matches!(reason, Invalid::UnknownKind(_))
}

fn is_compressed(path: &Path) -> bool {
    path.to_str().is_some_and(|p| p.ends_with(".gpl.zst"))
}

/// The raw bytes of a segment, decompressing a `.gpl.zst` file.
pub fn segment_bytes(path: &Path) -> Result<Vec<u8>, LogError> {
    let buf = fs::read(path)?;
    if is_compressed(path) {
        Ok(zstd::decode_all(&buf[..])?)
    } else {
        Ok(buf)
    }
}

/// Read every record of a closed segment, compressed or not. Any invalid byte is an error.
pub fn read_segment(path: &Path) -> Result<Vec<Record>, LogError> {
    let buf = segment_bytes(path)?;
    let s = scan(&buf);
    match s.tail {
        None => Ok(s.records),
        Some(reason) => Err(LogError::Corrupt {
            path: path.to_path_buf(),
            offset: s.valid_len as u64,
            reason,
        }),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Recovery {
    pub records: usize,
    pub kept_bytes: u64,
    pub dropped_bytes: u64,
}

/// Truncate a segment at its first invalid byte. The valid prefix is kept; everything
/// after it is discarded, including any records that would decode further on.
///
/// Only for uncompressed segments: a `.gpl.zst` file is refused, since truncating its
/// compressed bytes would destroy it.
pub fn recover_segment(path: &Path) -> Result<Recovery, LogError> {
    if is_compressed(path) {
        return Err(LogError::Io(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!("{}: cannot recover a compressed segment", path.display()),
        )));
    }
    let buf = fs::read(path)?;
    let s = scan(&buf);
    if let Some(reason) = s.tail
        && !is_torn(reason)
    {
        return Err(LogError::Corrupt {
            path: path.to_path_buf(),
            offset: s.valid_len as u64,
            reason,
        });
    }
    let dropped = (buf.len() - s.valid_len) as u64;
    if dropped > 0 {
        let file = fs::OpenOptions::new().write(true).open(path)?;
        file.set_len(s.valid_len as u64)?;
        file.sync_all()?;
    }
    Ok(Recovery {
        records: s.records.len(),
        kept_bytes: s.valid_len as u64,
        dropped_bytes: dropped,
    })
}

pub fn segment_file_name(index: u32) -> String {
    format!("seg-{index:06}.gpl")
}

fn sync_dir(dir: &Path) -> Result<(), LogError> {
    fs::File::open(dir)?.sync_all()?;
    Ok(())
}

/// Compress a closed segment to `seg-NNNNNN.gpl.zst` and remove the original.
///
/// The segment is validated first, so a torn file is never compressed. The new file is
/// written under a temporary name and renamed, so a crash leaves either the original or
/// both files, never a partial `.gpl.zst`. Leftovers are harmless but not removed here:
/// a stale `.gpl.zst.tmp` is ignored by [`list_segments`] and overwritten by the next
/// attempt, and an original `.gpl` left beside its `.gpl.zst` is hidden by
/// [`list_segments`] and can be deleted by hand.
pub fn compress_segment(path: &Path) -> Result<PathBuf, LogError> {
    let buf = fs::read(path)?;
    let s = scan(&buf);
    if let Some(reason) = s.tail {
        return Err(LogError::Corrupt {
            path: path.to_path_buf(),
            offset: s.valid_len as u64,
            reason,
        });
    }
    let dir = path.parent().unwrap_or(Path::new("."));
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(".zst");
    let out = dir.join(&name);
    let mut tmp_name = name;
    tmp_name.push(".tmp");
    let tmp = dir.join(tmp_name);
    let compressed = zstd::encode_all(&buf[..], ZSTD_LEVEL)?;
    {
        let mut f = fs::File::create(&tmp)?;
        std::io::Write::write_all(&mut f, &compressed)?;
        f.sync_all()?;
    }
    fs::rename(&tmp, &out)?;
    sync_dir(dir)?;
    fs::remove_file(path)?;
    sync_dir(dir)?;
    Ok(out)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SegmentFile {
    pub index: u32,
    pub path: PathBuf,
    pub compressed: bool,
}

fn parse_name(name: &str) -> Option<(u32, bool)> {
    let rest = name.strip_prefix("seg-")?;
    let (digits, compressed) = match rest.strip_suffix(".gpl.zst") {
        Some(d) => (d, true),
        None => (rest.strip_suffix(".gpl")?, false),
    };
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    Some((digits.parse().ok()?, compressed))
}

/// Segment files in `dir`, sorted by index.
///
/// When both forms of one index exist (a crash inside [`compress_segment`] after the
/// rename), only the `.gpl.zst` is listed: it was complete before the original was removed.
pub fn list_segments(dir: &Path) -> Result<Vec<SegmentFile>, LogError> {
    let mut out = Vec::new();
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        if let Some((index, compressed)) = entry.file_name().to_str().and_then(parse_name) {
            out.push(SegmentFile {
                index,
                path: entry.path(),
                compressed,
            });
        }
    }
    out.sort_by_key(|s| (s.index, !s.compressed));
    out.dedup_by_key(|s| s.index);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::RecordKind;

    fn rec(i: u8, len: usize) -> Record {
        Record {
            kind: RecordKind::RawFrame,
            conn_id: 1,
            recv_ns: i as i64,
            mono_ns: i as u64,
            payload: vec![i; len],
        }
    }

    fn encode(records: &[Record]) -> Vec<u8> {
        let mut buf = Vec::new();
        for r in records {
            r.encode_into(&mut buf);
        }
        buf
    }

    #[test]
    fn scan_clean_and_torn() {
        let recs = vec![rec(1, 3), rec(2, 0), rec(3, 10)];
        let buf = encode(&recs);
        let s = scan(&buf);
        assert_eq!(
            (s.records.clone(), s.valid_len, s.tail),
            (recs.clone(), buf.len(), None)
        );

        let s = scan(&buf[..buf.len() - 4]);
        assert_eq!(s.records, recs[..2].to_vec());
        assert_eq!(s.tail, Some(Invalid::Truncated));

        let empty = scan(&[]);
        assert_eq!(
            (empty.records.len(), empty.valid_len, empty.tail),
            (0, 0, None)
        );
    }

    #[test]
    fn scan_rejects_garbage_len() {
        let mut buf = encode(&[rec(1, 3)]);
        let mut second = encode(&[rec(2, 3)]);
        second[4..8].copy_from_slice(&u32::MAX.to_le_bytes());
        buf.extend_from_slice(&second);
        let s = scan(&buf);
        assert_eq!(s.records.len(), 1);
        assert_eq!(s.tail, Some(Invalid::TooLong));
    }

    #[test]
    fn compress_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(segment_file_name(4));
        let recs = vec![rec(1, 300), rec(2, 0), rec(3, 1000)];
        std::fs::write(&path, encode(&recs)).unwrap();

        let out = compress_segment(&path).unwrap();
        assert_eq!(out, dir.path().join("seg-000004.gpl.zst"));
        assert!(!path.exists());
        assert!(std::fs::metadata(&out).unwrap().len() < encode(&recs).len() as u64);
        assert_eq!(read_segment(&out).unwrap(), recs);

        let listed = list_segments(dir.path()).unwrap();
        assert_eq!(listed.len(), 1);
        assert!(listed[0].compressed);
        assert_eq!(listed[0].index, 4);
    }

    #[test]
    fn list_prefers_compressed_duplicate() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(segment_file_name(2));
        let recs = vec![rec(1, 8)];
        std::fs::write(&path, encode(&recs)).unwrap();
        let out = compress_segment(&path).unwrap();
        std::fs::write(&path, encode(&recs)).unwrap(); // as if the remove never happened
        std::fs::write(dir.path().join(segment_file_name(3)), encode(&recs)).unwrap();

        let listed = list_segments(dir.path()).unwrap();
        assert_eq!(
            listed
                .iter()
                .map(|s| (s.index, s.compressed))
                .collect::<Vec<_>>(),
            vec![(2, true), (3, false)]
        );
        assert_eq!(listed[0].path, out);
    }

    #[test]
    fn compress_refuses_torn_segment() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(segment_file_name(0));
        let mut buf = encode(&[rec(1, 3)]);
        buf.extend_from_slice(&[0x47; 7]);
        std::fs::write(&path, &buf).unwrap();
        assert!(matches!(
            compress_segment(&path),
            Err(LogError::Corrupt { offset, .. }) if offset == (buf.len() - 7) as u64
        ));
        assert!(path.exists());
        assert_eq!(list_segments(dir.path()).unwrap().len(), 1);
    }

    #[test]
    fn read_segment_reports_corruption() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(segment_file_name(0));
        let mut buf = encode(&[rec(1, 3), rec(2, 3)]);
        std::fs::write(&path, &buf).unwrap();
        assert_eq!(read_segment(&path).unwrap().len(), 2);

        buf.extend_from_slice(&[0xAB; 5]);
        std::fs::write(&path, &buf).unwrap();
        match read_segment(&path) {
            Err(LogError::Corrupt { offset, reason, .. }) => {
                assert_eq!(offset, (buf.len() - 5) as u64);
                assert_eq!(reason, Invalid::Truncated);
            }
            other => panic!("expected Corrupt, got {other:?}"),
        }
    }

    #[test]
    fn lists_segments_in_index_order() {
        let dir = tempfile::tempdir().unwrap();
        for name in [
            "seg-000010.gpl",
            "seg-000002.gpl.zst",
            "seg-000001.gpl",
            "notes.txt",
            "seg-x.gpl",
        ] {
            std::fs::write(dir.path().join(name), b"").unwrap();
        }
        let segs = list_segments(dir.path()).unwrap();
        let got: Vec<(u32, bool)> = segs.iter().map(|s| (s.index, s.compressed)).collect();
        assert_eq!(got, vec![(1, false), (2, true), (10, false)]);
        assert_eq!(segment_file_name(7), "seg-000007.gpl");
    }

    #[test]
    fn recovery_truncates_torn_header() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(segment_file_name(0));
        let good = encode(&[rec(1, 4), rec(2, 4)]);
        let mut buf = good.clone();
        buf.extend_from_slice(&encode(&[rec(3, 4)])[..20]); // crash mid-header
        std::fs::write(&path, &buf).unwrap();

        let r = recover_segment(&path).unwrap();
        assert_eq!(
            (r.records, r.kept_bytes, r.dropped_bytes),
            (2, good.len() as u64, 20)
        );
        assert_eq!(std::fs::read(&path).unwrap(), good);
        assert_eq!(read_segment(&path).unwrap().len(), 2);
    }

    #[test]
    fn recovery_refuses_to_truncate_unknown_kind() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(segment_file_name(0));
        let mut buf = encode(&[rec(1, 4)]);
        let mut unknown = encode(&[rec(2, 4)]);
        unknown[12] = 200;
        let crc = crc32c::crc32c_append(
            crc32c::crc32c(&unknown[12..crate::HEADER_LEN]),
            &unknown[crate::HEADER_LEN..],
        );
        unknown[8..12].copy_from_slice(&crc.to_le_bytes());
        buf.extend_from_slice(&unknown);
        std::fs::write(&path, &buf).unwrap();

        match recover_segment(&path) {
            Err(LogError::Corrupt {
                reason: Invalid::UnknownKind(200),
                ..
            }) => {}
            other => panic!("expected Corrupt(UnknownKind), got {other:?}"),
        }
        assert_eq!(std::fs::read(&path).unwrap(), buf);
    }

    #[test]
    fn recovery_refuses_compressed_segment() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(segment_file_name(0));
        std::fs::write(&path, encode(&[rec(1, 4)])).unwrap();
        let zst = compress_segment(&path).unwrap();
        let before = std::fs::read(&zst).unwrap();
        assert!(matches!(recover_segment(&zst), Err(LogError::Io(_))));
        assert_eq!(std::fs::read(&zst).unwrap(), before);
    }

    #[test]
    fn recovery_of_clean_and_empty_files() {
        let dir = tempfile::tempdir().unwrap();
        let clean = dir.path().join(segment_file_name(0));
        let buf = encode(&[rec(1, 4)]);
        std::fs::write(&clean, &buf).unwrap();
        let r = recover_segment(&clean).unwrap();
        assert_eq!((r.records, r.dropped_bytes), (1, 0));
        assert_eq!(std::fs::read(&clean).unwrap(), buf);

        let empty = dir.path().join(segment_file_name(1));
        std::fs::write(&empty, b"").unwrap();
        let r = recover_segment(&empty).unwrap();
        assert_eq!((r.records, r.kept_bytes, r.dropped_bytes), (0, 0, 0));
    }
}
