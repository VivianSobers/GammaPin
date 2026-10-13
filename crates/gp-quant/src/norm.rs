//! Standard normal CDF and PDF.

use std::f64::consts::FRAC_1_SQRT_2;

/// 1/√(2π).
const INV_SQRT_2PI: f64 = 0.398_942_280_401_432_7;

/// N(x), via erfc so the tails keep full relative precision.
pub fn n_cdf(x: f64) -> f64 {
    0.5 * libm::erfc(-x * FRAC_1_SQRT_2)
}

/// φ(x). Uses libm's exp, which is faster than some platform C runtimes in the GEX hot loop.
pub fn n_pdf(x: f64) -> f64 {
    INV_SQRT_2PI * libm::exp(-0.5 * x * x)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_values() {
        assert!((n_cdf(0.0) - 0.5).abs() < 1e-16);
        assert!((n_cdf(1.959_963_984_540_054) - 0.975).abs() < 1e-15);
        assert!((n_cdf(-8.0) - 6.220_960_574_271_785e-16).abs() < 1e-28);
        assert!((n_pdf(0.0) - 0.398_942_280_401_432_7).abs() < 1e-16);
    }

    #[test]
    fn symmetry() {
        for x in [-6.0, -1.3, 0.2, 2.5, 7.0] {
            assert!((n_cdf(x) + n_cdf(-x) - 1.0).abs() < 1e-15);
            assert_eq!(n_pdf(x), n_pdf(-x));
        }
    }
}
