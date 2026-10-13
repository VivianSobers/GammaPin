//! G1.5: cross-check against py_vollib reference values
//! (regenerate with scripts/gen_vollib_fixture.py).

use gp_quant::OptType;
use gp_quant::black76::{gamma, price};
use gp_quant::iv::implied_vol;

const FIXTURE: &str = include_str!("data/vollib_black76.csv");

#[test]
fn matches_py_vollib() {
    let mut n = 0;
    for line in FIXTURE.lines().skip(1) {
        let c: Vec<&str> = line.split(',').collect();
        let opt = if c[0] == "c" {
            OptType::Call
        } else {
            OptType::Put
        };
        let x: Vec<f64> = c[1..].iter().map(|s| s.parse().unwrap()).collect();
        let (f, k, t, r, sigma, ref_price, ref_gamma, ref_iv) =
            (x[0], x[1], x[2], x[3], x[4], x[5], x[6], x[7]);

        let p = price(f, k, t, r, sigma, opt);
        assert!(
            (p - ref_price).abs() < 1e-9 * ref_price.max(1.0),
            "price {line}: {p}"
        );
        let g = gamma(f, k, t, r, sigma);
        assert!(
            (g - ref_gamma).abs() < 1e-9 * ref_gamma,
            "gamma {line}: {g}"
        );
        let iv = implied_vol(ref_price, f, k, t, r, opt).unwrap();
        assert!((iv - ref_iv).abs() < 1e-6, "iv {line}: {iv}");
        n += 1;
    }
    assert_eq!(n, 300);
}
