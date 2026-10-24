//! The one stream interface that both the live feed and replay implement, so the monitor
//! engine cannot tell them apart except by [`EventStream::source`].

use std::sync::Arc;

use gp_feed::Event;

use crate::ReplayFile;

/// C2 `source`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    Live,
    Replay,
}

impl Source {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Live => "live",
            Self::Replay => "replay",
        }
    }
}

// `Ready` is the common case; boxing it would allocate once per event.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, PartialEq)]
pub enum Poll {
    /// The next event.
    Ready(Event),
    /// Nothing yet; ask again later.
    Pending,
    /// The stream is over (replay finished, capture stopped).
    End,
}

pub type StreamError = Box<dyn std::error::Error + Send + Sync>;

pub trait EventStream: Send {
    fn source(&self) -> Source;
    /// The next event if one is available now. Never blocks.
    fn poll_event(&mut self) -> Result<Poll, StreamError>;
    /// Exchange time the stream has reached, Unix ms (the last event's `ltt_ms`).
    fn position_ms(&self) -> Option<i64>;
    /// `GAP_START` times (Unix ms) seen since the last call. A replay has none: its gaps
    /// are in the day's coverage report.
    fn take_gaps(&mut self) -> Vec<i64> {
        Vec::new()
    }
}

/// A replay log read front to back at full speed.
pub struct ReplayStream {
    file: Arc<ReplayFile>,
    next: u64,
    position_ms: Option<i64>,
}

impl ReplayStream {
    pub fn new(file: Arc<ReplayFile>) -> Self {
        Self {
            file,
            next: 0,
            position_ms: None,
        }
    }

    /// Events not yet returned.
    #[cfg(test)]
    pub(crate) fn remaining(&self) -> u64 {
        self.file.len() - self.next
    }
}

impl EventStream for ReplayStream {
    fn source(&self) -> Source {
        Source::Replay
    }

    fn poll_event(&mut self) -> Result<Poll, StreamError> {
        if self.next >= self.file.len() {
            return Ok(Poll::End);
        }
        let e = self.file.event(self.next);
        self.next += 1;
        self.position_ms = Some(e.ltt_ms);
        Ok(Poll::Ready(e))
    }

    fn position_ms(&self) -> Option<i64> {
        self.position_ms
    }
}

/// In-memory events, for tests and fixtures.
pub struct VecStream {
    events: std::collections::VecDeque<Event>,
    source: Source,
    position_ms: Option<i64>,
}

impl VecStream {
    pub fn new(events: Vec<Event>, source: Source) -> Self {
        Self {
            events: events.into(),
            source,
            position_ms: None,
        }
    }
}

impl EventStream for VecStream {
    fn source(&self) -> Source {
        self.source
    }

    fn poll_event(&mut self) -> Result<Poll, StreamError> {
        Ok(match self.events.pop_front() {
            Some(e) => {
                self.position_ms = Some(e.ltt_ms);
                Poll::Ready(e)
            }
            None => Poll::End,
        })
    }

    fn position_ms(&self) -> Option<i64> {
        self.position_ms
    }
}

/// Drain a stream to its end. Fails on `Pending`, which live streams return and so does a
/// [`crate::Paced`] stream while it holds an event back.
#[cfg(test)]
pub(crate) fn collect(stream: &mut dyn EventStream) -> Result<Vec<Event>, StreamError> {
    let mut out = Vec::new();
    loop {
        match stream.poll_event()? {
            Poll::Ready(e) => out.push(e),
            Poll::End => return Ok(out),
            Poll::Pending => return Err("stream is not finite".into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::format::encode;
    use crate::format::tests::event;
    use gp_feed::Kind;

    #[test]
    fn replay_and_vec_streams_agree() {
        let evs = vec![
            event("a", Kind::Ce, 1, 1, 1),
            event("b", Kind::Pe, 2, 2, 2),
            event("a", Kind::Ce, 3, 3, 3),
        ];
        let file = Arc::new(ReplayFile::from_bytes(encode(&evs).unwrap()).unwrap());
        let mut replay = ReplayStream::new(file);
        let mut vec = VecStream::new(evs.clone(), Source::Live);
        assert_eq!(replay.position_ms(), None);
        assert_eq!(replay.remaining(), 3);
        let streams: [&mut dyn EventStream; 2] = [&mut replay, &mut vec];
        let [r, v] = streams;
        assert_eq!(collect(r).unwrap(), evs);
        assert_eq!(collect(v).unwrap(), evs);
        assert_eq!((r.source(), v.source()), (Source::Replay, Source::Live));
        assert_eq!(r.position_ms(), Some(3));
        assert_eq!(r.poll_event().unwrap(), Poll::End);
        assert!(r.take_gaps().is_empty());
        assert_eq!(Source::Replay.as_str(), "replay");
    }
}
