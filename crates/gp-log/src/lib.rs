//! GPL1 log format: 32-byte little-endian header followed by `len` payload bytes.
//!
//! ```text
//! magic:   u32  0x47504C31 "GPL1"
//! len:     u32
//! crc32c:  u32  over header[12..] + payload
//! kind:    u8
//! conn_id: u8
//! _pad:    u16
//! recv_ns: i64  CLOCK_REALTIME (chrony-synced)
//! mono_ns: u64  CLOCK_MONOTONIC
//! ```

pub const MAGIC: u32 = 0x4750_4C31;
pub const HEADER_LEN: usize = 32;
pub const SEGMENT_BYTES: u64 = 64 * 1024 * 1024;
pub const FLUSH_INTERVAL_MS: u64 = 100;

pub mod record;
pub mod segment;
pub mod writer;

pub use record::{Invalid, MAX_PAYLOAD, Record, RecordKind, decode, encode_header};
pub use segment::{
    Recovery, Scan, SegmentFile, ZSTD_LEVEL, compress_segment, list_segments, read_segment,
    recover_segment, scan, segment_bytes, segment_file_name,
};

pub use writer::{Writer, WriterConfig};

use std::path::PathBuf;

#[derive(Debug, thiserror::Error)]
pub enum LogError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("{path}: invalid record at byte {offset}: {reason:?}")]
    Corrupt {
        path: PathBuf,
        offset: u64,
        reason: Invalid,
    },
    #[error("record of {0} bytes does not fit in a segment")]
    PayloadTooLarge(usize),
    #[error("writer is poisoned by an earlier I/O error; reopen the log")]
    Poisoned,
}
