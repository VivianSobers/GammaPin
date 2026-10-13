//! Quote filters applied before solving for implied vol.

/// Filter settings. `two_sided` is off for minute candles, where only a close exists.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FilterConfig {
    pub tick: f64,
    pub two_sided: bool,
}

impl FilterConfig {
    pub const LIVE: Self = Self {
        tick: 0.05,
        two_sided: true,
    };
    pub const CANDLES: Self = Self {
        tick: 0.05,
        two_sided: false,
    };
}
