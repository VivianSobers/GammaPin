//! Implied volatility: Newton inside a shrinking bracket, Brent when Newton misbehaves.
//! Converged means |price(σ) − target| < 1e-8, within 50 price evaluations in total
//! (Brent reuses the bracket values Newton already computed).

use crate::black76::{OptType, intrinsic, price, vega};

pub const SIGMA_LO: f64 = 1e-4;
pub const SIGMA_HI: f64 = 5.0;
pub const PRICE_TOL: f64 = 1e-8;
pub const MAX_ITER: usize = 50;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IvError {
    /// Non-finite or non-positive F, K, T, or a non-finite price.
    BadInput,
    /// Target is at or below the price at SIGMA_LO (at or under intrinsic).
    BelowRange,
    /// Target is above the price at SIGMA_HI.
    AboveRange,
    NoConvergence,
}

pub fn implied_vol(
    target: f64,
    f: f64,
    k: f64,
    t: f64,
    r: f64,
    opt: OptType,
) -> Result<f64, IvError> {
    if !(target.is_finite()
        && f.is_finite()
        && k.is_finite()
        && t.is_finite()
        && f > 0.0
        && k > 0.0
        && t > 0.0
        && r.is_finite())
    {
        return Err(IvError::BadInput);
    }
    let g = |s: f64| price(f, k, t, r, s, opt) - target;
    let (mut lo, mut hi) = (SIGMA_LO, SIGMA_HI);
    let (mut g_lo, mut g_hi) = (g(lo), g(hi));
    // A target the floor vol already reproduces carries no measurable time value.
    if g_lo > -PRICE_TOL {
        return Err(IvError::BelowRange);
    }
    if g_hi < -PRICE_TOL {
        return Err(IvError::AboveRange);
    }
    if g_hi.abs() < PRICE_TOL {
        return Ok(hi);
    }

    // Brenner–Subrahmanyam start on the time value, clamped into the bracket.
    let df = (-r * t).exp();
    let tv = (target - intrinsic(f, k, t, r, opt)).max(0.0);
    let mut s = ((2.0 * std::f64::consts::PI / t).sqrt() * tv / (df * f)).clamp(0.01, 3.0);

    let mut evals = 2;
    while evals < MAX_ITER {
        let d = g(s);
        evals += 1;
        if d.abs() < PRICE_TOL {
            return Ok(s);
        }
        // price is increasing in σ, so the sign of d tells which side the root is on
        if d > 0.0 {
            (hi, g_hi) = (s, d);
        } else {
            (lo, g_lo) = (s, d);
        }
        let v = vega(f, k, t, r, s);
        let next = s - d / v;
        if !(v > 0.0 && next > lo && next < hi) {
            return brent(&g, (lo, g_lo), (hi, g_hi), MAX_ITER - evals);
        }
        s = next;
    }
    Err(IvError::NoConvergence)
}

/// Brent's method on a sign-changing bracket [a, b], given g(a) and g(b).
/// Evaluates g at most `max_iter` times.
fn brent(
    g: &dyn Fn(f64) -> f64,
    (a, fa): (f64, f64),
    (b, fb): (f64, f64),
    max_iter: usize,
) -> Result<f64, IvError> {
    let (mut a, mut b, mut fa, mut fb) = (a, b, fa, fb);
    if fa * fb > 0.0 {
        return Err(IvError::NoConvergence);
    }
    if fa.abs() < fb.abs() {
        std::mem::swap(&mut a, &mut b);
        std::mem::swap(&mut fa, &mut fb);
    }
    let (mut c, mut fc) = (a, fa);
    let mut d = c;
    let mut bisected = true;
    for _ in 0..max_iter {
        if fb.abs() < PRICE_TOL {
            return Ok(b);
        }
        let mut s = if fa != fc && fb != fc {
            a * fb * fc / ((fa - fb) * (fa - fc))
                + b * fa * fc / ((fb - fa) * (fb - fc))
                + c * fa * fb / ((fc - fa) * (fc - fb))
        } else {
            b - fb * (b - a) / (fb - fa)
        };
        let m = (3.0 * a + b) / 4.0;
        let tol = 1e-15;
        let outside = !((s > m.min(b)) && (s < m.max(b)));
        if outside
            || (bisected && (s - b).abs() >= (b - c).abs() / 2.0)
            || (!bisected && (s - b).abs() >= (c - d).abs() / 2.0)
            || (bisected && (b - c).abs() < tol)
            || (!bisected && (c - d).abs() < tol)
        {
            s = 0.5 * (a + b);
            bisected = true;
        } else {
            bisected = false;
        }
        let fs = g(s);
        d = c;
        c = b;
        fc = fb;
        if fa * fs < 0.0 {
            b = s;
            fb = fs;
        } else {
            a = s;
            fa = fs;
        }
        if fa.abs() < fb.abs() {
            std::mem::swap(&mut a, &mut b);
            std::mem::swap(&mut fa, &mut fb);
        }
    }
    if fb.abs() < PRICE_TOL {
        Ok(b)
    } else {
        Err(IvError::NoConvergence)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recovers_sigma() {
        let (f, t, r) = (25_000.0, 2.0 / 252.0, 0.065);
        for (k, opt) in [
            (24_500.0, OptType::Put),
            (25_000.0, OptType::Call),
            (25_800.0, OptType::Call),
        ] {
            for s in [0.08, 0.15, 0.6] {
                let p = price(f, k, t, r, s, opt);
                let iv = implied_vol(p, f, k, t, r, opt).unwrap();
                assert!((iv - s).abs() < 1e-7, "k={k} s={s} iv={iv}");
            }
        }
    }

    #[test]
    fn rejects_out_of_range() {
        let (f, k, t, r) = (25_000.0, 24_000.0, 0.01, 0.065);
        let below = intrinsic(f, k, t, r, OptType::Call) - 1.0;
        assert_eq!(
            implied_vol(below, f, k, t, r, OptType::Call),
            Err(IvError::BelowRange)
        );
        assert_eq!(
            implied_vol(30_000.0, f, k, t, r, OptType::Call),
            Err(IvError::AboveRange)
        );
        assert_eq!(
            implied_vol(f64::NAN, f, k, t, r, OptType::Call),
            Err(IvError::BadInput)
        );
        for (f, k, t) in [
            (f64::INFINITY, k, t),
            (f, f64::INFINITY, t),
            (f, k, f64::INFINITY),
        ] {
            assert_eq!(
                implied_vol(1.0, f, k, t, r, OptType::Call),
                Err(IvError::BadInput)
            );
        }
        // exactly intrinsic (the floor vol's price) is not a valid target either
        let at_floor = price(f, k, t, r, SIGMA_LO, OptType::Call);
        assert_eq!(
            implied_vol(at_floor, f, k, t, r, OptType::Call),
            Err(IvError::BelowRange)
        );
    }

    #[test]
    fn brent_path_converges() {
        // Far OTM with a tiny time value: Newton steps leave the bracket here.
        let (f, k, t, r) = (25_000.0, 27_500.0, 1.0 / 252.0, 0.065);
        let p = price(f, k, t, r, 0.9, OptType::Call);
        let iv = implied_vol(p, f, k, t, r, OptType::Call).unwrap();
        assert!((price(f, k, t, r, iv, OptType::Call) - p).abs() < PRICE_TOL);
    }
}
