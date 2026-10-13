//! Forward from put-call parity: F = K + e^{rT}(C_mid − P_mid),
//! median over the 3 strikes nearest the money (smallest |C_mid − P_mid|).

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ParityPoint {
    pub strike: f64,
    pub call_mid: f64,
    pub put_mid: f64,
}

pub fn forward(points: &[ParityPoint], r: f64, t: f64) -> Option<f64> {
    let growth = (r * t).exp();
    let mut near: Vec<&ParityPoint> = points
        .iter()
        .filter(|p| {
            p.call_mid > 0.0 && p.put_mid > 0.0 && p.call_mid.is_finite() && p.put_mid.is_finite()
        })
        .collect();
    near.sort_by(|a, b| {
        (a.call_mid - a.put_mid)
            .abs()
            .total_cmp(&(b.call_mid - b.put_mid).abs())
            .then(a.strike.total_cmp(&b.strike))
    });
    near.truncate(3);
    let mut est: Vec<f64> = near
        .iter()
        .map(|p| p.strike + growth * (p.call_mid - p.put_mid))
        .collect();
    est.sort_by(f64::total_cmp);
    match est.len() {
        0 => None,
        n if n % 2 == 1 => Some(est[n / 2]),
        n => Some(0.5 * (est[n / 2 - 1] + est[n / 2])),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::black76::{OptType, price};

    fn chain(f: f64, r: f64, t: f64) -> Vec<ParityPoint> {
        (0..21)
            .map(|i| {
                let k = 24_500.0 + 50.0 * f64::from(i);
                let s = 0.12 + 0.0000002 * (k - f).powi(2);
                ParityPoint {
                    strike: k,
                    call_mid: price(f, k, t, r, s, OptType::Call),
                    put_mid: price(f, k, t, r, s, OptType::Put),
                }
            })
            .collect()
    }

    #[test]
    fn recovers_forward_from_parity() {
        let (f, r, t) = (25_013.7, 0.065, 3.0 / 252.0);
        let got = forward(&chain(f, r, t), r, t).unwrap();
        assert!((got - f).abs() < 1e-6, "{got}");
    }

    #[test]
    fn median_resists_one_bad_quote() {
        let (f, r, t) = (25_013.7, 0.065, 3.0 / 252.0);
        let mut pts = chain(f, r, t);
        let atm = pts.iter_mut().find(|p| p.strike == 25_000.0).unwrap();
        atm.call_mid += 9.0;
        let got = forward(&pts, r, t).unwrap();
        assert!((got - f).abs() < 1e-6, "{got}");
    }

    #[test]
    fn none_without_two_sided_points() {
        let pts = [ParityPoint {
            strike: 25_000.0,
            call_mid: 0.0,
            put_mid: 10.0,
        }];
        assert_eq!(forward(&pts, 0.06, 0.01), None);
    }
}
