//! Pump a stream into the engine and publish a snapshot every `every_ms` of wall time.
//!
//! Runs on its own thread: the stream never blocks, and between events this loop sleeps
//! briefly. Live snapshots are stamped with wall time, replay snapshots with the stream's
//! exchange time, so a replay looks as it did on the day.

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use gp_replay::{EventStream, Poll, Source, StreamError};

use crate::engine::Engine;
use crate::health::Histogram;
use crate::snapshot::Snapshot;

/// Events taken per pass before checking the clock.
const BATCH: usize = 4_096;
/// Sleep while the stream has nothing new.
const IDLE: Duration = Duration::from_millis(2);

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Summary {
    pub events: u64,
    pub snapshots: u64,
    /// Snapshots that could not be built yet (no spot or chain).
    pub skipped: u64,
    /// Live only: age of the newest index tick when each snapshot was built, ms. This is
    /// the feed-to-engine latency the dashboard sees, without the snapshot cadence.
    pub staleness: Histogram,
}

pub struct Driver<'a> {
    pub every_ms: i64,
    /// Wall clock, Unix ms.
    pub clock: &'a dyn Fn() -> i64,
    pub stop: &'a AtomicBool,
}

pub fn wall_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as i64)
}

impl Driver<'_> {
    /// Run until the stream ends or `stop` is set. One last snapshot is published at the end.
    pub fn run(
        &self,
        stream: &mut dyn EventStream,
        engine: &mut Engine,
        publish: &mut dyn FnMut(Snapshot),
    ) -> Result<Summary, StreamError> {
        let source = stream.source();
        let mut sum = Summary::default();
        let mut next = (self.clock)();
        let mut last_err: Option<String> = None;
        let mut snap = |stream: &dyn EventStream, engine: &mut Engine, sum: &mut Summary| {
            let now = (self.clock)();
            let ts = match source {
                Source::Live => now,
                Source::Replay => stream.position_ms().unwrap_or(now),
            };
            match engine.snapshot(ts, source.as_str()) {
                Ok(s) => {
                    sum.snapshots += 1;
                    if source == Source::Live
                        && let Some((_, ltt)) = engine.book().spot()
                    {
                        sum.staleness.record((now - ltt) as f64);
                    }
                    if last_err.take().is_some() {
                        tracing::info!("snapshots resumed");
                    }
                    publish(s);
                }
                Err(e) => {
                    sum.skipped += 1;
                    // once per change of reason, not once per period
                    let why = e.to_string();
                    if last_err.as_deref() != Some(why.as_str()) {
                        tracing::warn!("no snapshot: {why}");
                        last_err = Some(why);
                    }
                }
            }
        };
        loop {
            if self.stop.load(Ordering::Relaxed) {
                break;
            }
            let mut idle = false;
            let mut ended = false;
            for _ in 0..BATCH {
                match stream.poll_event()? {
                    Poll::Ready(e) => {
                        engine.on_event(&e);
                        sum.events += 1;
                    }
                    Poll::Pending => {
                        idle = true;
                        break;
                    }
                    Poll::End => {
                        ended = true;
                        break;
                    }
                }
            }
            for g in stream.take_gaps() {
                engine.on_gap(g);
            }
            if ended {
                break;
            }
            if (self.clock)() >= next {
                snap(stream, engine, &mut sum);
                next += self.every_ms;
                next = next.max((self.clock)());
            }
            if idle {
                std::thread::sleep(IDLE);
            }
        }
        snap(stream, engine, &mut sum);
        Ok(sum)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::EngineConfig;
    use crate::engine::tests::market_events;
    use gp_feed::Date;
    use gp_replay::{ManualClock, Paced, Speed, VecStream};
    use std::cell::Cell;

    #[test]
    fn replay_snapshots_follow_the_stream_clock() {
        let date = Date::from_ymd(2026, 10, 19);
        let evs = market_events(date, 40);
        let n = evs.len() as u64;
        let last_ltt = evs.last().unwrap().ltt_ms;
        let mut stream = VecStream::new(evs, Source::Replay);
        let mut engine = Engine::new(EngineConfig::default());
        // every clock read advances 1 ms, so a 50 ms period gives several snapshots
        let t = Cell::new(0);
        let clock = || {
            t.set(t.get() + 1);
            t.get()
        };
        let stop = AtomicBool::new(false);
        let drv = Driver {
            every_ms: 50,
            clock: &clock,
            stop: &stop,
        };
        let mut got = Vec::new();
        let sum = drv
            .run(&mut stream, &mut engine, &mut |s| got.push(s))
            .unwrap();
        assert_eq!(sum.events, n);
        assert_eq!(sum.snapshots as usize, got.len());
        assert!(!got.is_empty());
        let last = got.last().unwrap();
        assert_eq!(last.ts_ms, last_ltt);
        assert_eq!(last.source, "replay");
        assert!(got.windows(2).all(|w| w[0].ts_ms <= w[1].ts_ms));
        assert_eq!(sum.staleness.count(), 0); // replay: not measured
    }

    #[test]
    fn live_snapshots_measure_staleness() {
        let date = Date::from_ymd(2026, 10, 19);
        let evs = market_events(date, 8);
        let newest = evs
            .iter()
            .filter(|e| e.kind == gp_feed::Kind::Idx)
            .map(|e| e.ltt_ms)
            .max()
            .unwrap();
        let mut stream = VecStream::new(evs, Source::Live);
        let mut engine = Engine::new(EngineConfig::default());
        let stop = AtomicBool::new(false);
        let clock = || newest + 120;
        let drv = Driver {
            every_ms: 1_000,
            clock: &clock,
            stop: &stop,
        };
        let sum = drv.run(&mut stream, &mut engine, &mut |_| {}).unwrap();
        assert_eq!(sum.staleness.count(), sum.snapshots);
        assert_eq!(sum.staleness.quantile(0.5), Some(121.0));
    }

    #[test]
    fn paced_replay_waits_and_stops() {
        let date = Date::from_ymd(2026, 10, 19);
        let evs = market_events(date, 10);
        let clock = ManualClock::default();
        // 1×: an event is due once the clock has moved as far past the first event as its
        // ltt_ms has; this clock never moves
        let mut stream = Paced::new(
            VecStream::new(evs, Source::Replay),
            Speed::parse("1x").unwrap(),
            Box::new(clock.clone()),
        );
        let mut engine = Engine::new(EngineConfig::default());
        let stop = AtomicBool::new(false);
        let polls = Cell::new(0);
        let wall = || {
            polls.set(polls.get() + 1);
            if polls.get() > 50 {
                stop.store(true, Ordering::Relaxed);
            }
            0
        };
        let drv = Driver {
            every_ms: 1_000,
            clock: &wall,
            stop: &stop,
        };
        let sum = drv.run(&mut stream, &mut engine, &mut |_| {}).unwrap();
        // only the events with the first event's ltt_ms were released
        assert!(sum.events > 0 && sum.events < 100, "{sum:?}");
        assert!(stop.load(Ordering::Relaxed));
    }
}
