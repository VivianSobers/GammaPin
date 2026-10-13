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

#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecordKind {
    RawFrame = 1,
    GapStart = 2,
    GapEnd = 3,
    Subscribe = 4,
    TokenReload = 5,
}
