//! Gamma exposure in ₹ per 1% move:
//! GEX_K = (w_c·Γ_K·OI_K^C + w_p·Γ_K·OI_K^P) × lot × F² × 0.01.
//! Γ is the same for calls and puts at one strike under Black-76.

use crate::chain::{StrikeRow, Weights};
use crate::norm::n_pdf;

/// Flip search covers F ± 3% ...
pub const FLIP_HALF_WIDTH: f64 = 0.03;
/// ... in 0.05% steps.
pub const FLIP_STEP: f64 = 0.0005;

/// GEX per strike, aligned with `rows`.
pub fn gex_by_strike(rows: &[StrikeRow], gamma: &[f64], w: Weights, lot: f64, f: f64) -> Vec<f64> {
    let scale = lot * f * f * 0.01;
    rows.iter()
        .zip(gamma)
        .map(|(row, g)| (w.w_c * row.call.oi + w.w_p * row.put.oi) * g * scale)
        .collect()
}

/// Call-side and put-side gamma notional at each hypothetical forward level,
/// before sign weights. Vols are held fixed per strike (sticky strike).
#[derive(Debug, Clone, PartialEq)]
pub struct GammaGrid {
    pub levels: Vec<f64>,
    /// Σ_K Γ_K(level)·OI_K^C × lot × level² × 0.01
    pub call: Vec<f64>,
    /// Σ_K Γ_K(level)·OI_K^P × lot × level² × 0.01
    pub put: Vec<f64>,
}

impl GammaGrid {
    pub fn new(rows: &[StrikeRow], sigma: &[f64], f: f64, t: f64, r: f64, lot: f64) -> Self {
        let n = (FLIP_HALF_WIDTH / FLIP_STEP).round() as i32;
        let levels: Vec<f64> = (-n..=n)
            .map(|i| f * (1.0 + f64::from(i) * FLIP_STEP))
            .collect();
        let df = (-r * t).exp();
        let sqrt_t = t.sqrt();
        // per strike: ln K, σ√T, ½σ²T
        let pre: Vec<(f64, f64, f64)> = rows
            .iter()
            .zip(sigma)
            .map(|(row, s)| {
                let v = s * sqrt_t;
                (row.strike.ln(), v, 0.5 * v * v)
            })
            .collect();
        let (mut call, mut put) = (
            Vec::with_capacity(levels.len()),
            Vec::with_capacity(levels.len()),
        );
        for &x in &levels {
            let ln_x = x.ln();
            let (mut c, mut p) = (0.0, 0.0);
            for (row, &(ln_k, v, half_v2)) in rows.iter().zip(&pre) {
                let g = n_pdf((ln_x - ln_k + half_v2) / v) / v;
                c += g * row.call.oi;
                p += g * row.put.oi;
            }
            // Γ = e^{-rT} φ(d1) / (x v); times lot·x²·0.01 leaves one x
            let scale = df * lot * x * 0.01;
            call.push(c * scale);
            put.push(p * scale);
        }
        Self { levels, call, put }
    }

    /// Net GEX at each level for one convention.
    pub fn net(&self, w: Weights) -> impl Iterator<Item = f64> + '_ {
        self.call
            .iter()
            .zip(&self.put)
            .map(move |(c, p)| w.w_c * c + w.w_p * p)
    }

    /// Level where net GEX changes sign, nearest to `f`, linearly interpolated.
    /// None when net GEX keeps one sign over the whole grid.
    pub fn flip(&self, w: Weights, f: f64) -> Option<f64> {
        let net: Vec<f64> = self.net(w).collect();
        let mut best: Option<f64> = None;
        for i in 0..net.len() {
            let crossing = if net[i] == 0.0 {
                Some(self.levels[i])
            } else if i + 1 < net.len() && net[i] * net[i + 1] < 0.0 {
                let (x0, x1) = (self.levels[i], self.levels[i + 1]);
                Some(x0 + (x1 - x0) * net[i] / (net[i] - net[i + 1]))
            } else {
                None
            };
            if let Some(x) = crossing
                && best.is_none_or(|b| (x - f).abs() < (b - f).abs())
            {
                best = Some(x);
            }
        }
        best
    }
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

    #[test]
    fn grid_matches_gex_at_center() {
        use crate::black76::gamma;
        let (f, t, r, lot) = (25_000.0, 1.0 / 252.0, 0.065, 75.0);
        let rows: Vec<StrikeRow> = (0..9)
            .map(|i| row(24_800.0 + 50.0 * f64::from(i), 800.0, 1_200.0))
            .collect();
        let sigma = vec![0.13; rows.len()];
        let grid = GammaGrid::new(&rows, &sigma, f, t, r, lot);
        let gam: Vec<f64> = rows
            .iter()
            .map(|row| gamma(f, row.strike, t, r, 0.13))
            .collect();
        let direct: f64 = gex_by_strike(&rows, &gam, Weights::S0, lot, f).iter().sum();
        let mid = grid.levels.len() / 2;
        assert_eq!(grid.levels.len(), 121);
        assert!((grid.levels[mid] - f).abs() < 1e-9);
        let on_grid = grid.net(Weights::S0).nth(mid).unwrap();
        assert!(
            (on_grid - direct).abs() < 1e-9 * direct.abs(),
            "{on_grid} vs {direct}"
        );
    }

    #[test]
    fn flip_between_put_wall_and_call_wall() {
        // puts below 24,800, calls above 25,200: net S0 GEX turns positive between them
        let (f, t, r, lot) = (25_000.0, 2.0 / 252.0, 0.065, 75.0);
        let rows = [row(24_800.0, 0.0, 10_000.0), row(25_200.0, 10_000.0, 0.0)];
        let grid = GammaGrid::new(&rows, &[0.12, 0.12], f, t, r, lot);
        let flip = grid.flip(Weights::S0, f).unwrap();
        assert!((flip - 25_000.0).abs() < 15.0, "{flip}");
        let net: Vec<f64> = grid.net(Weights::S0).collect();
        assert!(net[0] < 0.0 && net[net.len() - 1] > 0.0);
    }

    #[test]
    fn no_flip_when_one_signed() {
        let (f, t, r, lot) = (25_000.0, 2.0 / 252.0, 0.065, 75.0);
        let rows = [row(24_900.0, 5_000.0, 0.0), row(25_100.0, 5_000.0, 0.0)];
        let grid = GammaGrid::new(&rows, &[0.12, 0.12], f, t, r, lot);
        assert_eq!(grid.flip(Weights::S0, f), None);
    }
}
