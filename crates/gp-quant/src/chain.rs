//! Contract C3 (docs/CONTRACTS.md): the chain-level API used by gp-monitor and research.

use crate::filter::FilterConfig;

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Quote {
    pub bid: f64,
    pub ask: f64,
    /// Open interest in contracts; 0.0 when absent.
    pub oi: f64,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StrikeRow {
    pub strike: f64,
    pub call: Quote,
    pub put: Quote,
}

#[derive(Debug, Clone, Copy)]
pub struct ChainInput<'a> {
    /// One expiry, strikes ascending.
    pub rows: &'a [StrikeRow],
    pub r: f64,
    pub t_years: f64,
    pub lot_size: f64,
    pub filter: FilterConfig,
}

/// Dealer sign weights on call and put open interest.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Weights {
    pub w_c: f64,
    pub w_p: f64,
}

impl Weights {
    /// S0: US convention, dealers long calls and short puts.
    pub const S0: Self = Self {
        w_c: 1.0,
        w_p: -1.0,
    };
}

#[derive(Debug, Clone, PartialEq)]
pub struct ConventionResult {
    /// ₹ per 1% move, aligned with `ChainInput::rows`.
    pub gex: Vec<f64>,
    pub net_gex: f64,
    pub flip: Option<f64>,
    pub pin: Option<f64>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Analysis {
    pub forward: f64,
    /// Per row; NaN where no usable quote.
    pub iv: Vec<f64>,
    pub gamma: Vec<f64>,
    pub quotes_used: usize,
    pub quotes_total: usize,
    /// Same order as the `weights` passed to [`analyze`].
    pub conventions: Vec<ConventionResult>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuantError {
    /// Fewer than one strike with usable call and put mids.
    NoForward,
    /// No strike produced an implied vol.
    NoSmile,
    /// t_years, lot_size or rows are invalid.
    BadInput,
}

impl std::fmt::Display for QuantError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            Self::NoForward => "no strike with usable call and put mids",
            Self::NoSmile => "no strike produced an implied vol",
            Self::BadInput => "invalid chain input",
        };
        f.write_str(s)
    }
}

impl std::error::Error for QuantError {}

/// Forward, smile, gamma and GEX for each convention in `weights`.
pub fn analyze(input: &ChainInput, weights: &[Weights]) -> Result<Analysis, QuantError> {
    let _ = (input, weights);
    todo!("G3.5")
}
