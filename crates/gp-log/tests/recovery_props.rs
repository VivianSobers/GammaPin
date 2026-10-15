use gp_log::{
    HEADER_LEN, RecordKind, Writer, WriterConfig, read_segment, recover_segment, segment_file_name,
};
use proptest::prelude::*;
use proptest::sample::Index;
use std::time::Duration;

fn write_all(dir: &std::path::Path, payloads: &[Vec<u8>]) -> std::path::PathBuf {
    let cfg = WriterConfig {
        segment_bytes: 1 << 24,
        flush_interval: Duration::ZERO,
    };
    let (mut w, _) = Writer::open(dir, cfg).unwrap();
    for (i, p) in payloads.iter().enumerate() {
        w.append(RecordKind::RawFrame, 1, i as i64, i as u64, p)
            .unwrap();
    }
    w.close().unwrap();
    dir.join(segment_file_name(0))
}

/// Byte offset where each record ends.
fn ends(payloads: &[Vec<u8>]) -> Vec<usize> {
    payloads
        .iter()
        .scan(0, |off, p| {
            *off += HEADER_LEN + p.len();
            Some(*off)
        })
        .collect()
}

proptest! {
    #[test]
    fn truncation_keeps_every_complete_record(
        payloads in prop::collection::vec(prop::collection::vec(any::<u8>(), 0..300), 1..40),
        cut in any::<Index>(),
    ) {
        let dir = tempfile::tempdir().unwrap();
        let path = write_all(dir.path(), &payloads);
        let full = std::fs::read(&path).unwrap();
        let cut_at = cut.index(full.len() + 1);
        std::fs::write(&path, &full[..cut_at]).unwrap();

        let expected = ends(&payloads).iter().filter(|&&e| e <= cut_at).count();
        let rec = recover_segment(&path).unwrap();
        prop_assert_eq!(rec.records, expected);
        let got = read_segment(&path).unwrap();
        prop_assert_eq!(got.len(), expected);
        for (r, p) in got.iter().zip(&payloads) {
            prop_assert_eq!(&r.payload, p);
        }
    }

    #[test]
    fn corrupt_byte_drops_its_record_and_everything_after(
        payloads in prop::collection::vec(prop::collection::vec(any::<u8>(), 0..300), 1..40),
        pos in any::<Index>(),
        flip in 1u8..=255,
    ) {
        let dir = tempfile::tempdir().unwrap();
        let path = write_all(dir.path(), &payloads);
        let mut bytes = std::fs::read(&path).unwrap();
        let p = pos.index(bytes.len());
        bytes[p] ^= flip;
        std::fs::write(&path, &bytes).unwrap();

        let hit = ends(&payloads).iter().position(|&e| p < e).unwrap();
        recover_segment(&path).unwrap();
        prop_assert_eq!(read_segment(&path).unwrap().len(), hit);
    }
}
