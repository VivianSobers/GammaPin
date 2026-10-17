//! SHA-256 over a stream's events, for checking that replay is deterministic.

use gp_feed::Event;
use sha2::{Digest, Sha256};

use crate::{EventStream, Poll, StreamError};

/// Canonical bytes of one event: key and metadata, then every field little-endian, bit-exact.
pub fn canonical(e: &Event, out: &mut Vec<u8>) {
    for s in [&e.instrument_key, &e.underlying] {
        out.extend_from_slice(&(s.len() as u32).to_le_bytes());
        out.extend_from_slice(s.as_bytes());
    }
    out.extend_from_slice(e.kind.as_str().as_bytes());
    out.extend_from_slice(&e.expiry.map_or(i32::MIN, |d| d.0).to_le_bytes());
    out.extend_from_slice(&e.strike.unwrap_or(f64::NAN).to_bits().to_le_bytes());
    out.extend_from_slice(&e.lot_size.to_le_bytes());
    for v in [e.ltt_ms, e.recv_ns] {
        out.extend_from_slice(&v.to_le_bytes());
    }
    out.extend_from_slice(&e.ltp.to_bits().to_le_bytes());
    for v in [e.ltq, e.cum_volume, e.oi] {
        out.extend_from_slice(&v.to_le_bytes());
    }
    for px in [&e.bid_px, &e.ask_px] {
        for p in px {
            out.extend_from_slice(&p.to_bits().to_le_bytes());
        }
    }
    for q in [&e.bid_qty, &e.ask_qty] {
        for v in q {
            out.extend_from_slice(&v.to_le_bytes());
        }
    }
    out.push(e.src_mask);
}

pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Digest and count of every event until the stream ends.
pub fn stream_digest(stream: &mut dyn EventStream) -> Result<(String, u64), StreamError> {
    let mut h = Sha256::new();
    let mut buf = Vec::with_capacity(512);
    let mut n = 0;
    loop {
        match stream.poll_event()? {
            Poll::Ready(e) => {
                buf.clear();
                canonical(&e, &mut buf);
                h.update(&buf);
                n += 1;
            }
            Poll::Pending => std::thread::sleep(std::time::Duration::from_millis(1)),
            Poll::End => return Ok((hex(&h.finalize()), n)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::format::tests::event;
    use crate::{Source, VecStream};
    use gp_feed::Kind;

    #[test]
    fn empty_and_sensitive() {
        let (empty, n) = stream_digest(&mut VecStream::new(vec![], Source::Replay)).unwrap();
        assert_eq!(n, 0);
        assert_eq!(
            empty,
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        let evs = vec![event("a", Kind::Ce, 1, 1, 1), event("b", Kind::Pe, 2, 2, 2)];
        let (a, _) = stream_digest(&mut VecStream::new(evs.clone(), Source::Replay)).unwrap();
        let (b, _) = stream_digest(&mut VecStream::new(evs.clone(), Source::Live)).unwrap();
        assert_eq!(a, b);
        let mut changed = evs.clone();
        changed[1].ltp = -changed[1].ltp;
        let (c, _) = stream_digest(&mut VecStream::new(changed, Source::Replay)).unwrap();
        assert_ne!(a, c);
        let mut zero = evs;
        zero[0].ltp = 0.0;
        let mut neg_zero = zero.clone();
        neg_zero[0].ltp = -0.0;
        let (z, _) = stream_digest(&mut VecStream::new(zero, Source::Replay)).unwrap();
        let (nz, _) = stream_digest(&mut VecStream::new(neg_zero, Source::Replay)).unwrap();
        assert_ne!(z, nz, "bit-exact, not value-equal");
    }
}
