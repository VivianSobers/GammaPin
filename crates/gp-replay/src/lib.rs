//! gp-replay: replay merged events ordered by (ltt_ms, recv_ns, node, seq)
//! at 1x, Nx or full speed. Same input, byte-identical output.

/// Replay speed.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Speed {
    RealTime,
    Multiple(f64),
    Max,
}
