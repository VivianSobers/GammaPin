//! Reconnect delays: exponential from `first` to `max`, with jitter so that two nodes that
//! lose the feed together do not reconnect in lockstep.

use std::time::Duration;

use gp_feed::synth::Rng;

#[derive(Debug, Clone)]
pub struct Backoff {
    first: Duration,
    max: Duration,
    attempt: u32,
    rng: Rng,
}

impl Backoff {
    pub fn new(first: Duration, max: Duration, seed: u64) -> Self {
        Self {
            first,
            max: max.max(first),
            attempt: 0,
            rng: Rng::new(seed),
        }
    }

    /// Delay before the next attempt: uniform in [d/2, d], d = min(first × 2^n, max).
    pub fn next_delay(&mut self) -> Duration {
        let cap = self
            .first
            .saturating_mul(1u32 << self.attempt.min(20))
            .min(self.max);
        self.attempt = self.attempt.saturating_add(1);
        cap.mul_f64(0.5 + 0.5 * self.rng.uniform())
    }

    /// Call after a connection has delivered data.
    pub fn reset(&mut self) {
        self.attempt = 0;
    }

    #[cfg(test)]
    pub(crate) fn attempt(&self) -> u32 {
        self.attempt
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grows_to_cap_with_jitter() {
        let mut b = Backoff::new(Duration::from_millis(100), Duration::from_secs(5), 7);
        let caps = [100, 200, 400, 800, 1_600, 3_200, 5_000, 5_000, 5_000];
        for cap in caps {
            let d = b.next_delay().as_millis() as u64;
            assert!(d >= cap / 2 && d <= cap, "{d} not in [{}, {cap}]", cap / 2);
        }
        assert_eq!(b.attempt(), caps.len() as u32);
        b.reset();
        assert!(b.next_delay() <= Duration::from_millis(100));
    }

    #[test]
    fn jitter_differs_between_nodes() {
        let mut a = Backoff::new(Duration::from_millis(100), Duration::from_secs(5), 1);
        let mut b = Backoff::new(Duration::from_millis(100), Duration::from_secs(5), 2);
        let da: Vec<_> = (0..8).map(|_| a.next_delay()).collect();
        let db: Vec<_> = (0..8).map(|_| b.next_delay()).collect();
        assert_ne!(da, db);
    }

    #[test]
    fn many_attempts_do_not_overflow() {
        let mut b = Backoff::new(Duration::from_millis(100), Duration::from_secs(5), 3);
        for _ in 0..1_000 {
            assert!(b.next_delay() <= Duration::from_secs(5));
        }
    }
}
