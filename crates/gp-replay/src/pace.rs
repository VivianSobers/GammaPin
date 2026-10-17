//! Replay at 1×, N× or full speed: an event is released once the wall clock has advanced as
//! far past the first event as its `ltt_ms` has, divided by the speed.

use std::sync::{Arc, Mutex};
use std::time::Instant;

use gp_feed::Event;

use crate::{EventStream, Poll, Source, Speed, StreamError};

pub trait Clock: Send {
    /// Milliseconds since some fixed point.
    fn now_ms(&self) -> f64;
}

pub struct SystemClock(Instant);

impl Default for SystemClock {
    fn default() -> Self {
        Self(Instant::now())
    }
}

impl Clock for SystemClock {
    fn now_ms(&self) -> f64 {
        self.0.elapsed().as_secs_f64() * 1e3
    }
}

/// A clock moved by hand, for tests.
#[derive(Clone, Default)]
pub struct ManualClock(Arc<Mutex<f64>>);

impl ManualClock {
    pub fn advance(&self, ms: f64) {
        *self.0.lock().expect("clock") += ms;
    }
}

impl Clock for ManualClock {
    fn now_ms(&self) -> f64 {
        *self.0.lock().expect("clock")
    }
}

impl Speed {
    fn rate(self) -> Option<f64> {
        match self {
            Self::RealTime => Some(1.0),
            Self::Multiple(x) if x > 0.0 && x.is_finite() => Some(x),
            Self::Multiple(_) | Self::Max => None,
        }
    }

    /// Parse `1x`, `10x`, `max` (case-insensitive).
    pub fn parse(s: &str) -> Option<Self> {
        let s = s.trim().to_ascii_lowercase();
        if s == "max" {
            return Some(Self::Max);
        }
        let x: f64 = s.strip_suffix('x')?.parse().ok()?;
        match x {
            1.0 => Some(Self::RealTime),
            x if x > 0.0 && x.is_finite() => Some(Self::Multiple(x)),
            _ => None,
        }
    }
}

pub struct Paced<S> {
    inner: S,
    rate: Option<f64>,
    clock: Box<dyn Clock>,
    held: Option<Event>,
    /// (wall ms, ltt_ms) of the first event.
    anchor: Option<(f64, i64)>,
}

impl<S: EventStream> Paced<S> {
    pub fn new(inner: S, speed: Speed, clock: Box<dyn Clock>) -> Self {
        Self {
            inner,
            rate: speed.rate(),
            clock,
            held: None,
            anchor: None,
        }
    }

    /// Milliseconds until the held event is due (0 when nothing is held or it is due).
    pub fn wait_ms(&self) -> f64 {
        match (&self.held, self.rate, self.anchor) {
            (Some(e), Some(rate), Some((wall, ltt))) => {
                (wall + (e.ltt_ms - ltt) as f64 / rate - self.clock.now_ms()).max(0.0)
            }
            _ => 0.0,
        }
    }
}

impl<S: EventStream> EventStream for Paced<S> {
    fn source(&self) -> Source {
        self.inner.source()
    }

    fn poll_event(&mut self) -> Result<Poll, StreamError> {
        let Some(rate) = self.rate else {
            return self.inner.poll_event();
        };
        if self.held.is_none() {
            match self.inner.poll_event()? {
                Poll::Ready(e) => {
                    self.anchor.get_or_insert((self.clock.now_ms(), e.ltt_ms));
                    self.held = Some(e);
                }
                other => return Ok(other),
            }
        }
        let (wall, ltt) = self.anchor.expect("set with the first event");
        let e = self.held.as_ref().expect("held");
        if self.clock.now_ms() >= wall + (e.ltt_ms - ltt) as f64 / rate {
            Ok(Poll::Ready(self.held.take().expect("held")))
        } else {
            Ok(Poll::Pending)
        }
    }

    fn position_ms(&self) -> Option<i64> {
        self.inner.position_ms()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::VecStream;
    use crate::format::tests::event;
    use gp_feed::Kind;

    fn stream() -> VecStream {
        VecStream::new(
            [1_000, 1_000, 1_500, 3_000]
                .iter()
                .map(|&t| event("a", Kind::Ce, t, 0, 1))
                .collect(),
            Source::Replay,
        )
    }

    fn drain_ready(p: &mut impl EventStream) -> Vec<i64> {
        let mut out = Vec::new();
        while let Poll::Ready(e) = p.poll_event().unwrap() {
            out.push(e.ltt_ms);
        }
        out
    }

    #[test]
    fn real_time_and_multiple() {
        for (speed, step) in [(Speed::RealTime, 500.0), (Speed::Multiple(10.0), 50.0)] {
            let clock = ManualClock::default();
            let mut p = Paced::new(stream(), speed, Box::new(clock.clone()));
            assert_eq!(drain_ready(&mut p), [1_000, 1_000]); // both at the anchor
            assert_eq!(p.wait_ms(), step);
            clock.advance(step - 1.0);
            assert!(drain_ready(&mut p).is_empty());
            clock.advance(1.0);
            assert_eq!(drain_ready(&mut p), [1_500]);
            clock.advance(3.0 * step);
            assert_eq!(drain_ready(&mut p), [3_000]);
            assert_eq!(p.poll_event().unwrap(), Poll::End);
        }
    }

    #[test]
    fn max_speed_passes_through() {
        let mut p = Paced::new(stream(), Speed::Max, Box::new(ManualClock::default()));
        assert_eq!(drain_ready(&mut p), [1_000, 1_000, 1_500, 3_000]);
        assert_eq!(p.wait_ms(), 0.0);
    }

    #[test]
    fn parse_speeds() {
        assert_eq!(Speed::parse("1x"), Some(Speed::RealTime));
        assert_eq!(Speed::parse("20X"), Some(Speed::Multiple(20.0)));
        assert_eq!(Speed::parse("0.5x"), Some(Speed::Multiple(0.5)));
        assert_eq!(Speed::parse("max"), Some(Speed::Max));
        for bad in ["0x", "-2x", "fast", "10", "infx"] {
            assert_eq!(Speed::parse(bad), None, "{bad}");
        }
    }
}
