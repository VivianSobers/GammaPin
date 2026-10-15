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

/// Read every record of a closed segment. Any invalid byte is an error.
pub fn read_segment(path: &Path) -> Result<Vec<Record>, LogError> {
    let buf = fs::read(path)?;
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

pub fn segment_file_name(index: u32) -> String {
    format!("seg-{index:06}.gpl")
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
    out.sort_by_key(|s| s.index);
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
    fn scan_garbage_len_does_not_allocate() {
        let mut buf = encode(&[rec(1, 3)]);
        let mut second = encode(&[rec(2, 3)]);
        second[4..8].copy_from_slice(&u32::MAX.to_le_bytes());
        buf.extend_from_slice(&second);
        let s = scan(&buf);
        assert_eq!(s.records.len(), 1);
        assert_eq!(s.tail, Some(Invalid::TooLong));
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
}
