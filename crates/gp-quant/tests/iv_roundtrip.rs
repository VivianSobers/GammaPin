//! G1.4: price(σ) → implied vol → σ within 1e-6,
//! for σ ∈ [0.01, 3] and T ∈ [1 trading minute, 30 trading days].

use gp_quant::OptType;
use gp_quant::black76::price;
use gp_quant::iv::implied_vol;
use proptest::prelude::*;

const MINUTE: f64 = 1.0 / (252.0 * 375.0);
const DAYS_30: f64 = 30.0 / 252.0;

proptest! {
    #![proptest_config(ProptestConfig::with_cases(20_000))]

    #[test]
    fn roundtrip(
        sigma in 0.01f64..=3.0,
        t in MINUTE..=DAYS_30,
        // strike placed z standard deviations from the forward, where vega is not vanishing
        z in -3.0f64..=3.0,
        f in 5_000.0f64..=60_000.0,
        r in 0.0f64..=0.10,
        call in any::<bool>(),
    ) {
        let k = f * (z * sigma * t.sqrt()).exp();
        let opt = if call { OptType::Call } else { OptType::Put };
        let p = price(f, k, t, r, sigma, opt);
        let iv = implied_vol(p, f, k, t, r, opt).map_err(|e| TestCaseError::fail(format!("{e:?}")))?;
        prop_assert!((iv - sigma).abs() < 1e-6, "sigma={sigma} iv={iv} t={t} k={k} f={f}");
    }
}
