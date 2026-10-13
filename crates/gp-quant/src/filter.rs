//! Quote filters applied before solving for implied vol:
//! bid > 0, ask > bid, spread ≤ max(25% of mid, 4 ticks), mid ≥ 0.10,
//! mid ≥ discounted intrinsic, out-of-the-money only, σ ∈ [0.01, 3.0].

use crate::black76::{OptType, intrinsic};
use crate::chain::Quote;
use crate::iv::implied_vol;

pub const MIN_MID: f64 = 0.10;
pub const MAX_SPREAD_FRAC: f64 = 0.25;
pub const MAX_SPREAD_TICKS: f64 = 4.0;
pub const SIGMA_MIN: f64 = 0.01;
pub const SIGMA_MAX: f64 = 3.0;

/// Filter settings. `two_sided` is off for minute candles, where only a close exists.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FilterConfig {
    pub tick: f64,
    pub two_sided: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reject {
    NoBid,
    Crossed,
    WideSpread,
    TooCheap,
    BelowIntrinsic,
    NotOtm,
    IvFailed,
    IvOutOfRange,
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

    /// Mid of a quote that passes the spread and minimum-price checks.
    pub fn mid(&self, q: &Quote) -> Result<f64, Reject> {
        if self.two_sided {
            if q.bid.is_nan() || q.bid <= 0.0 {
                return Err(Reject::NoBid);
            }
            if q.ask.is_nan() || q.ask <= q.bid {
                return Err(Reject::Crossed);
            }
        }
        let mid = 0.5 * (q.bid + q.ask);
        if mid.is_nan() || mid < MIN_MID {
            return Err(Reject::TooCheap);
        }
        if self.two_sided
            && q.ask - q.bid > (MAX_SPREAD_FRAC * mid).max(MAX_SPREAD_TICKS * self.tick)
        {
            return Err(Reject::WideSpread);
        }
        Ok(mid)
    }

    /// Implied vol of an out-of-the-money quote that passes every filter.
    pub fn filtered_iv(
        &self,
        q: &Quote,
        f: f64,
        k: f64,
        t: f64,
        r: f64,
        opt: OptType,
    ) -> Result<f64, Reject> {
        if !is_otm(f, k, opt) {
            return Err(Reject::NotOtm);
        }
        let mid = self.mid(q)?;
        if mid < intrinsic(f, k, t, r, opt) {
            return Err(Reject::BelowIntrinsic);
        }
        let sigma = implied_vol(mid, f, k, t, r, opt).map_err(|_| Reject::IvFailed)?;
        if !(SIGMA_MIN..=SIGMA_MAX).contains(&sigma) {
            return Err(Reject::IvOutOfRange);
        }
        Ok(sigma)
    }
}

/// Calls at or above the forward, puts below it.
pub fn is_otm(f: f64, k: f64, opt: OptType) -> bool {
    match opt {
        OptType::Call => k >= f,
        OptType::Put => k < f,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::black76::price;

    const F: f64 = 25_000.0;
    const T: f64 = 2.0 / 252.0;
    const R: f64 = 0.065;

    fn q(bid: f64, ask: f64) -> Quote {
        Quote { bid, ask, oi: 0.0 }
    }

    #[test]
    fn price_checks() {
        let c = FilterConfig::LIVE;
        assert_eq!(c.mid(&q(0.0, 1.0)), Err(Reject::NoBid));
        assert_eq!(c.mid(&q(5.0, 5.0)), Err(Reject::Crossed));
        assert_eq!(c.mid(&q(0.05, 0.10)), Err(Reject::TooCheap));
        assert_eq!(c.mid(&q(10.0, 14.0)), Err(Reject::WideSpread));
        // 4 ticks allowed even when 25% of mid is smaller
        assert_eq!(c.mid(&q(0.40, 0.60)), Ok(0.5));
        assert_eq!(c.mid(&q(100.0, 101.0)), Ok(100.5));
    }

    #[test]
    fn candles_skip_two_sided_checks() {
        assert_eq!(FilterConfig::CANDLES.mid(&q(42.0, 42.0)), Ok(42.0));
        assert_eq!(
            FilterConfig::CANDLES.mid(&q(0.05, 0.05)),
            Err(Reject::TooCheap)
        );
    }

    #[test]
    fn otm_only_and_sigma_recovered() {
        let c = FilterConfig::LIVE;
        let k = 25_300.0;
        let p = price(F, k, T, R, 0.13, OptType::Call);
        let sigma = c
            .filtered_iv(&q(p - 0.5, p + 0.5), F, k, T, R, OptType::Call)
            .unwrap();
        assert!((sigma - 0.13).abs() < 1e-6);
        assert_eq!(
            c.filtered_iv(&q(p - 0.5, p + 0.5), F, k, T, R, OptType::Put),
            Err(Reject::NotOtm)
        );
    }

    #[test]
    fn sigma_range() {
        let c = FilterConfig::CANDLES;
        let k = 24_000.0;
        let p = price(F, k, T, R, 3.5, OptType::Put);
        assert_eq!(
            c.filtered_iv(&q(p, p), F, k, T, R, OptType::Put),
            Err(Reject::IvOutOfRange)
        );
    }
}
