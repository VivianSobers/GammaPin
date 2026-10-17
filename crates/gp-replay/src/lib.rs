//! gp-replay: replay merged events ordered by (ltt_ms, recv_ns, node, seq)
//! at 1x, Nx or full speed. Same input, byte-identical output.

pub mod format;
pub mod order;
pub mod reader;
pub mod stream;

pub use format::{FormatError, Meta, write_replay};
pub use order::{is_replay_order, sort_for_replay};
pub use reader::{ReplayFile, Tick};
pub use stream::{EventStream, Poll, ReplayStream, Source, StreamError, VecStream, collect};

/// Replay speed.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Speed {
    RealTime,
    Multiple(f64),
    Max,
}
