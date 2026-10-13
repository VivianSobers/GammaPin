//! Gamma exposure in ₹ per 1% move:
//! GEX_K = (w_c·Γ_K·OI_K^C + w_p·Γ_K·OI_K^P) × lot × F² × 0.01.
//! Γ is the same for calls and puts at one strike under Black-76.

use crate::chain::{StrikeRow, Weights};

/// GEX per strike, aligned with `rows`.
pub fn gex_by_strike(rows: &[StrikeRow], gamma: &[f64], w: Weights, lot: f64, f: f64) -> Vec<f64> {
    let scale = lot * f * f * 0.01;
    rows.iter()
        .zip(gamma)
        .map(|(row, g)| (w.w_c * row.call.oi + w.w_p * row.put.oi) * g * scale)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chain::Quote;

    fn row(k: f64, call_oi: f64, put_oi: f64) -> StrikeRow {
        let q = |oi| Quote {
            bid: 1.0,
            ask: 1.1,
            oi,
        };
        StrikeRow {
            strike: k,
            call: q(call_oi),
            put: q(put_oi),
        }
    }

    #[test]
    fn hand_computed() {
        let rows = [
            row(24_900.0, 1_000.0, 3_000.0),
            row(25_000.0, 2_000.0, 500.0),
        ];
        let gamma = [0.0010, 0.0012];
        let g = gex_by_strike(&rows, &gamma, Weights::S0, 75.0, 25_000.0);
        let scale = 75.0 * 25_000.0 * 25_000.0 * 0.01;
        assert_eq!(g[0], (1_000.0 - 3_000.0) * 0.0010 * scale);
        assert_eq!(g[1], (2_000.0 - 500.0) * 0.0012 * scale);
    }

    #[test]
    fn weights_scale_linearly() {
        let rows = [row(25_000.0, 2_000.0, 500.0)];
        let w = Weights {
            w_c: 0.25,
            w_p: 0.5,
        };
        let g = gex_by_strike(&rows, &[0.001], w, 75.0, 25_000.0);
        let expected = (0.25 * 2_000.0 + 0.5 * 500.0) * 0.001 * 75.0 * 25_000.0 * 25_000.0 * 0.01;
        assert!((g[0] - expected).abs() < 1e-6);
    }
}
