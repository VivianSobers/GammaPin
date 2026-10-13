//! G3.7: full-chain recompute for 1,500 contracts (750 strikes × call/put) on one core.
//! Target: < 5 ms. Run with `cargo bench -p gp-quant`.

use criterion::{Criterion, criterion_group, criterion_main};
use gp_quant::black76::price;
use gp_quant::gex::GammaGrid;
use gp_quant::{ChainInput, FilterConfig, OptType, Quote, StrikeRow, Weights, analyze, smile};
use std::hint::black_box;

fn chain() -> Vec<StrikeRow> {
    let (f, t, r) = (25_012.5, 3.0 / 252.0, 0.065);
    (0..750)
        .map(|i| {
            let k = 15_650.0 + 25.0 * f64::from(i);
            let sigma = 0.12 + 0.6 * ((k - f) / f).powi(2);
            let q = |opt| {
                let p = price(f, k, t, r, sigma, opt);
                let half = (0.02 * p).max(0.05);
                Quote {
                    bid: (p - half).max(0.0),
                    ask: p + half,
                    oi: 50_000.0 / (1.0 + ((k - f) / 200.0).powi(2)),
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

fn bench(c: &mut Criterion) {
    let rows = chain();
    let input = ChainInput {
        rows: &rows,
        r: 0.065,
        t_years: 3.0 / 252.0,
        lot_size: 75.0,
        filter: FilterConfig::LIVE,
    };
    let weights = [
        Weights::S0,
        Weights {
            w_c: 0.036,
            w_p: 0.060,
        },
        Weights {
            w_c: 0.039,
            w_p: 0.067,
        },
    ];
    c.bench_function("analyze_1500_contracts_3_conventions", |b| {
        b.iter(|| analyze(black_box(&input), black_box(&weights)).unwrap())
    });
    c.bench_function("smile_750_strikes", |b| {
        b.iter(|| smile(black_box(&input)).unwrap())
    });
    let sm = smile(&input).unwrap();
    c.bench_function("gamma_grid_750_strikes", |b| {
        b.iter(|| {
            GammaGrid::new(
                black_box(&rows),
                &sm.sigma,
                sm.forward,
                3.0 / 252.0,
                0.065,
                75.0,
            )
        })
    });
}

criterion_group!(benches, bench);
criterion_main!(benches);
