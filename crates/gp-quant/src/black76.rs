//! Black-76 prices on the forward F.
//!
//! C = e^{-rT}[F·N(d1) − K·N(d2)],  P = e^{-rT}[K·N(−d2) − F·N(−d1)]
//! d1 = [ln(F/K) + ½σ²T] / (σ√T),   d2 = d1 − σ√T

use crate::norm::{n_cdf, n_pdf};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OptType {
    Call,
    Put,
}

pub fn d1_d2(f: f64, k: f64, t: f64, sigma: f64) -> (f64, f64) {
    let v = sigma * t.sqrt();
    let d1 = ((f / k).ln() + 0.5 * v * v) / v;
    (d1, d1 - v)
}

pub fn price(f: f64, k: f64, t: f64, r: f64, sigma: f64, opt: OptType) -> f64 {
    let df = (-r * t).exp();
    let (d1, d2) = d1_d2(f, k, t, sigma);
    match opt {
        OptType::Call => df * (f * n_cdf(d1) - k * n_cdf(d2)),
        OptType::Put => df * (k * n_cdf(-d2) - f * n_cdf(-d1)),
    }
}

/// Discounted intrinsic value: e^{-rT}·max(±(F − K), 0).
pub fn intrinsic(f: f64, k: f64, t: f64, r: f64, opt: OptType) -> f64 {
    let df = (-r * t).exp();
    match opt {
        OptType::Call => df * (f - k).max(0.0),
        OptType::Put => df * (k - f).max(0.0),
    }
}

/// Γ = e^{-rT} φ(d1) / (F σ √T). Same for calls and puts.
pub fn gamma(f: f64, k: f64, t: f64, r: f64, sigma: f64) -> f64 {
    let (d1, _) = d1_d2(f, k, t, sigma);
    (-r * t).exp() * n_pdf(d1) / (f * sigma * t.sqrt())
}

/// ∂price/∂σ = e^{-rT} F φ(d1) √T. Same for calls and puts.
pub fn vega(f: f64, k: f64, t: f64, r: f64, sigma: f64) -> f64 {
    let (d1, _) = d1_d2(f, k, t, sigma);
    (-r * t).exp() * f * n_pdf(d1) * t.sqrt()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn atm_closed_form() {
        // F = K, r = 0: C = F(2N(σ√T/2) − 1)
        let c = price(100.0, 100.0, 1.0, 0.0, 0.2, OptType::Call);
        assert!((c - 7.965_567_455_405_804).abs() < 1e-12);
    }

    #[test]
    fn put_call_parity() {
        for &(f, k, t, r, s) in &[
            (25_000.0, 24_500.0, 0.01, 0.065, 0.12),
            (25_000.0, 26_000.0, 0.0004, 0.065, 0.4),
            (100.0, 80.0, 2.0, 0.03, 1.5),
        ] {
            let c = price(f, k, t, r, s, OptType::Call);
            let p = price(f, k, t, r, s, OptType::Put);
            let parity = (-r * t).exp() * (f - k);
            assert!((c - p - parity).abs() < 1e-9 * f, "{c} {p} {parity}");
        }
    }

    #[test]
    fn bounds() {
        let (f, k, t, r) = (25_000.0, 25_100.0, 0.002, 0.065);
        for s in [0.01, 0.1, 1.0, 3.0] {
            let c = price(f, k, t, r, s, OptType::Call);
            assert!(c >= intrinsic(f, k, t, r, OptType::Call));
            assert!(c <= (-r * t).exp() * f);
        }
    }

    #[test]
    fn gamma_matches_finite_difference() {
        let (k, t, r, s) = (25_000.0, 0.003, 0.065, 0.14);
        for f in [24_600.0, 25_000.0, 25_350.0] {
            let h = 0.5;
            let fd = (price(f + h, k, t, r, s, OptType::Call)
                - 2.0 * price(f, k, t, r, s, OptType::Call)
                + price(f - h, k, t, r, s, OptType::Call))
                / (h * h);
            let g = gamma(f, k, t, r, s);
            assert!((fd - g).abs() < 1e-6 * g.max(1e-9) + 1e-9, "{fd} vs {g}");
        }
    }

    #[test]
    fn vega_matches_finite_difference() {
        let (f, k, t, r) = (25_000.0, 25_200.0, 0.01, 0.065);
        for s in [0.05, 0.15, 0.8] {
            let h = 1e-5;
            let fd = (price(f, k, t, r, s + h, OptType::Put)
                - price(f, k, t, r, s - h, OptType::Put))
                / (2.0 * h);
            let v = vega(f, k, t, r, s);
            assert!((fd - v).abs() < 1e-5 * v, "{fd} vs {v}");
        }
    }
}
