//! One GPL1 record: the 32-byte header plus payload, and its validation.

use crate::{HEADER_LEN, MAGIC, SEGMENT_BYTES};

#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecordKind {
    RawFrame = 1,
    GapStart = 2,
    GapEnd = 3,
    Subscribe = 4,
    TokenReload = 5,
}

impl RecordKind {
    pub fn from_u8(b: u8) -> Option<Self> {
        match b {
            1 => Some(Self::RawFrame),
            2 => Some(Self::GapStart),
            3 => Some(Self::GapEnd),
            4 => Some(Self::Subscribe),
            5 => Some(Self::TokenReload),
            _ => None,
        }
    }
}

/// Largest payload a record may carry: one record must fit in an empty default segment.
pub const MAX_PAYLOAD: usize = SEGMENT_BYTES as usize - HEADER_LEN;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Record {
    pub kind: RecordKind,
    pub conn_id: u8,
    pub recv_ns: i64,
    pub mono_ns: u64,
    pub payload: Vec<u8>,
}

/// Why the bytes at some offset are not a valid record.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Invalid {
    Truncated,
    BadMagic,
    TooLong,
    BadCrc,
    UnknownKind(u8),
}

fn checksum(header_tail: &[u8], payload: &[u8]) -> u32 {
    crc32c::crc32c_append(crc32c::crc32c(header_tail), payload)
}

/// Build the header for `payload`. The caller guarantees `payload.len() <= MAX_PAYLOAD`.
pub fn encode_header(
    kind: RecordKind,
    conn_id: u8,
    recv_ns: i64,
    mono_ns: u64,
    payload: &[u8],
) -> [u8; HEADER_LEN] {
    let mut h = [0u8; HEADER_LEN];
    h[0..4].copy_from_slice(&MAGIC.to_le_bytes());
    h[4..8].copy_from_slice(&(payload.len() as u32).to_le_bytes());
    h[12] = kind as u8;
    h[13] = conn_id;
    h[16..24].copy_from_slice(&recv_ns.to_le_bytes());
    h[24..32].copy_from_slice(&mono_ns.to_le_bytes());
    let crc = checksum(&h[12..], payload);
    h[8..12].copy_from_slice(&crc.to_le_bytes());
    h
}

impl Record {
    pub fn encode_into(&self, out: &mut Vec<u8>) {
        out.extend_from_slice(&encode_header(
            self.kind,
            self.conn_id,
            self.recv_ns,
            self.mono_ns,
            &self.payload,
        ));
        out.extend_from_slice(&self.payload);
    }
}

fn le_u32(b: &[u8]) -> u32 {
    u32::from_le_bytes(b.try_into().expect("4 bytes"))
}

/// Decode the record at the start of `buf`; returns it and its encoded length.
pub fn decode(buf: &[u8]) -> Result<(Record, usize), Invalid> {
    if buf.len() < HEADER_LEN {
        return Err(Invalid::Truncated);
    }
    if le_u32(&buf[0..4]) != MAGIC {
        return Err(Invalid::BadMagic);
    }
    let len = le_u32(&buf[4..8]) as usize;
    if len > MAX_PAYLOAD {
        return Err(Invalid::TooLong);
    }
    let total = HEADER_LEN + len;
    if buf.len() < total {
        return Err(Invalid::Truncated);
    }
    if le_u32(&buf[8..12]) != checksum(&buf[12..HEADER_LEN], &buf[HEADER_LEN..total]) {
        return Err(Invalid::BadCrc);
    }
    let kind = RecordKind::from_u8(buf[12]).ok_or(Invalid::UnknownKind(buf[12]))?;
    let record = Record {
        kind,
        conn_id: buf[13],
        recv_ns: i64::from_le_bytes(buf[16..24].try_into().expect("8 bytes")),
        mono_ns: u64::from_le_bytes(buf[24..32].try_into().expect("8 bytes")),
        payload: buf[HEADER_LEN..total].to_vec(),
    };
    Ok((record, total))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rec(payload: &[u8]) -> Record {
        Record {
            kind: RecordKind::RawFrame,
            conn_id: 2,
            recv_ns: -5,
            mono_ns: 7,
            payload: payload.to_vec(),
        }
    }

    #[test]
    fn round_trip() {
        let r = rec(b"hello");
        let mut buf = Vec::new();
        r.encode_into(&mut buf);
        assert_eq!(buf.len(), HEADER_LEN + 5);
        assert_eq!(&buf[0..4], &MAGIC.to_le_bytes());
        assert_eq!(decode(&buf), Ok((r, HEADER_LEN + 5)));
    }

    #[test]
    fn header_layout() {
        let h = encode_header(RecordKind::GapEnd, 9, 0x0102, 0x0304, b"ab");
        assert_eq!(u32::from_le_bytes(h[4..8].try_into().unwrap()), 2);
        assert_eq!(h[12], 3);
        assert_eq!(h[13], 9);
        assert_eq!(&h[14..16], &[0, 0]);
        assert_eq!(i64::from_le_bytes(h[16..24].try_into().unwrap()), 0x0102);
        assert_eq!(u64::from_le_bytes(h[24..32].try_into().unwrap()), 0x0304);
    }

    #[test]
    fn empty_payload_is_valid() {
        let mut buf = Vec::new();
        rec(b"").encode_into(&mut buf);
        assert_eq!(decode(&buf).map(|(_, n)| n), Ok(HEADER_LEN));
    }

    #[test]
    fn rejects_bad_input() {
        let mut buf = Vec::new();
        rec(b"payload").encode_into(&mut buf);
        assert_eq!(decode(&buf[..HEADER_LEN - 1]), Err(Invalid::Truncated));
        assert_eq!(decode(&buf[..buf.len() - 1]), Err(Invalid::Truncated));

        let mut bad = buf.clone();
        bad[0] ^= 0xFF;
        assert_eq!(decode(&bad), Err(Invalid::BadMagic));

        let mut bad = buf.clone();
        bad[4..8].copy_from_slice(&u32::MAX.to_le_bytes());
        assert_eq!(decode(&bad), Err(Invalid::TooLong));

        let mut bad = buf.clone();
        *bad.last_mut().unwrap() ^= 1;
        assert_eq!(decode(&bad), Err(Invalid::BadCrc));

        let mut bad = buf.clone();
        bad[15] = 1; // pad byte is covered by the CRC
        assert_eq!(decode(&bad), Err(Invalid::BadCrc));
    }

    #[test]
    fn unknown_kind_with_valid_crc() {
        let mut buf = Vec::new();
        rec(b"x").encode_into(&mut buf);
        buf[12] = 200;
        let crc = crc32c::crc32c_append(crc32c::crc32c(&buf[12..HEADER_LEN]), &buf[HEADER_LEN..]);
        buf[8..12].copy_from_slice(&crc.to_le_bytes());
        assert_eq!(decode(&buf), Err(Invalid::UnknownKind(200)));
    }
}
