//! Contract C3 (docs/CONTRACTS.md): the chain-level API used by gp-monitor and research.

use crate::black76::OptType;
use crate::filter::FilterConfig;
use crate::forward::{ParityPoint, forward};

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

/// Forward and per-strike vol for one expiry.
#[derive(Debug, Clone, PartialEq)]
pub struct Smile {
    pub forward: f64,
    /// IV from the strike's OTM quote; NaN where it failed the filters.
    pub iv: Vec<f64>,
    /// `iv` with gaps filled by linear interpolation in strike, flat beyond the ends.
    pub sigma: Vec<f64>,
    pub quotes_used: usize,
    pub quotes_total: usize,
}

fn validate(input: &ChainInput) -> Result<(), QuantError> {
    let ok_t = input.t_years.is_finite() && input.t_years > 0.0;
    let ok_lot = input.lot_size.is_finite() && input.lot_size > 0.0;
    let ok_rows = !input.rows.is_empty()
        && input
            .rows
            .iter()
            .all(|r| r.strike.is_finite() && r.strike > 0.0)
        && input.rows.windows(2).all(|w| w[0].strike < w[1].strike);
    if ok_t && ok_lot && ok_rows && input.r.is_finite() {
        Ok(())
    } else {
        Err(QuantError::BadInput)
    }
}

/// Forward from parity, then one IV per strike from its out-of-the-money quote.
pub fn smile(input: &ChainInput) -> Result<Smile, QuantError> {
    validate(input)?;
    let (rows, cfg, t, r) = (input.rows, input.filter, input.t_years, input.r);

    let points: Vec<ParityPoint> = rows
        .iter()
        .filter_map(|row| {
            let call_mid = cfg.mid(&row.call).ok()?;
            let put_mid = cfg.mid(&row.put).ok()?;
            Some(ParityPoint {
                strike: row.strike,
                call_mid,
                put_mid,
            })
        })
        .collect();
    let f = forward(&points, r, t).ok_or(QuantError::NoForward)?;

    let iv: Vec<f64> = rows
        .iter()
        .map(|row| {
            let (q, opt) = if row.strike >= f {
                (&row.call, OptType::Call)
            } else {
                (&row.put, OptType::Put)
            };
            cfg.filtered_iv(q, f, row.strike, t, r, opt)
                .unwrap_or(f64::NAN)
        })
        .collect();
    let quotes_used = iv.iter().filter(|s| !s.is_nan()).count();
    if quotes_used == 0 {
        return Err(QuantError::NoSmile);
    }
    let quotes_total = rows
        .iter()
        .flat_map(|row| [&row.call, &row.put])
        .filter(|q| q.bid > 0.0 || q.ask > 0.0)
        .count();
    let strikes: Vec<f64> = rows.iter().map(|row| row.strike).collect();
    let sigma = fill_gaps(&strikes, &iv);
    Ok(Smile {
        forward: f,
        iv,
        sigma,
        quotes_used,
        quotes_total,
    })
}

/// Linear interpolation in strike across NaN gaps, flat extrapolation at the ends.
/// Needs at least one finite value.
fn fill_gaps(x: &[f64], y: &[f64]) -> Vec<f64> {
    let known: Vec<usize> = (0..y.len()).filter(|&i| !y[i].is_nan()).collect();
    let (first, last) = (known[0], known[known.len() - 1]);
    let mut out = y.to_vec();
    for i in 0..y.len() {
        if !out[i].is_nan() {
            continue;
        }
        out[i] = if i < first {
            y[first]
        } else if i > last {
            y[last]
        } else {
            let hi = known.partition_point(|&j| j < i);
            let (a, b) = (known[hi - 1], known[hi]);
            let w = (x[i] - x[a]) / (x[b] - x[a]);
            y[a] + w * (y[b] - y[a])
        };
    }
    out
}

/// Forward, smile, gamma and GEX for each convention in `weights`.
pub fn analyze(input: &ChainInput, weights: &[Weights]) -> Result<Analysis, QuantError> {
    let _ = (input, weights);
    todo!("G3.5")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::black76::price;

    pub(crate) const F: f64 = 25_012.5;
    pub(crate) const T: f64 = 1.0 / 252.0;
    pub(crate) const R: f64 = 0.065;

    pub(crate) fn vol(k: f64) -> f64 {
        0.12 + 0.6 * ((k - F) / F).powi(2)
    }

    /// Two-sided Black-76 chain, 24,000–26,000 in 50s, 1-rupee wide quotes.
    pub(crate) fn synthetic_rows() -> Vec<StrikeRow> {
        (0..41)
            .map(|i| {
                let k = 24_000.0 + 50.0 * f64::from(i);
                let q = |opt| {
                    let p = price(F, k, T, R, vol(k), opt);
                    Quote {
                        bid: (p - 0.5).max(0.0),
                        ask: p + 0.5,
                        oi: 1_000.0,
                    }
                };
                StrikeRow {
                    strike: k,
                    call: q(OptType::Call),
                    put: q(OptType::Put),
                }
            })
            .collect()
    }

    pub(crate) fn input(rows: &[StrikeRow]) -> ChainInput<'_> {
        ChainInput {
            rows,
            r: R,
            t_years: T,
            lot_size: 75.0,
            filter: FilterConfig::LIVE,
        }
    }

    #[test]
    fn smile_recovers_forward_and_vols() {
        let rows = synthetic_rows();
        let s = smile(&input(&rows)).unwrap();
        assert!((s.forward - F).abs() < 1e-6, "{}", s.forward);
        for (row, iv) in rows.iter().zip(&s.iv) {
            if !iv.is_nan() {
                assert!((iv - vol(row.strike)).abs() < 1e-6);
            }
        }
        // 1-rupee spreads pass the 25% rule only when the mid is at least 4
        let expected = rows
            .iter()
            .filter(|row| {
                let opt = if row.strike >= F {
                    OptType::Call
                } else {
                    OptType::Put
                };
                price(F, row.strike, T, R, vol(row.strike), opt) >= 4.0
            })
            .count();
        assert_eq!(s.quotes_used, expected);
        assert_eq!(s.quotes_total, 82);
        assert!(s.sigma.iter().all(|v| v.is_finite()));
    }

    #[test]
    fn gaps_are_interpolated() {
        let x = [1.0, 2.0, 3.0, 4.0, 5.0];
        let y = [f64::NAN, 0.2, f64::NAN, 0.4, f64::NAN];
        assert_eq!(
            fill_gaps(&x, &y),
            vec![0.2, 0.2, 0.30000000000000004, 0.4, 0.4]
        );
    }

    #[test]
    fn rejects_bad_input() {
        let rows = synthetic_rows();
        let mut bad = input(&rows);
        bad.t_years = 0.0;
        assert_eq!(smile(&bad), Err(QuantError::BadInput));
        let empty: [StrikeRow; 0] = [];
        assert_eq!(smile(&input(&empty)), Err(QuantError::BadInput));
    }

    #[test]
    fn no_forward_without_two_sided_quotes() {
        let mut rows = synthetic_rows();
        rows.iter_mut().for_each(|r| r.put = Quote::default());
        assert_eq!(smile(&input(&rows)), Err(QuantError::NoForward));
    }
}
