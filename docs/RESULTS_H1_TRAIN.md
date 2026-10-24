# H1 on the training set (G4.8)

Run on 15 Oct 2026, after the `prereg-v1` freeze, with `python research/h1_eod.py --split train`. Data: NSE bhavcopy at the close on the expiry eve, as `PREREGISTRATION.md` specifies. Full per-expiry outputs are written to `research/output/h1_eod_train.csv` and `h1_eod_train_summary.json` and are not committed.

**Sample:** 66 training expiries, 2 Jan 2025 to 30 Mar 2026, all with status `ok`. On one eve (5 Feb 2025) both S1 weights and both S2 weights are negative, so no strike has positive GEX and there is no pin; S1 and S2 have n = 65 and S0 has n = 66.

## Mean absolute error vs. settlement (index points)

| Prediction | MAE | n |
|---|---|---|
| pin S0 | 201.6 | 66 |
| pin S1 | 165.2 | 65 |
| pin S2 | 166.5 | 65 |
| max pain | 160.3 | 66 |
| nearest 100 | 146.2 | 66 |
| no move (eve forward) | **145.4** | 66 |

## Against the best baseline (no move)

MAE difference = MAE(GEX pin) − MAE(no move), with a 95% block-bootstrap CI (10,000 resamples, block length 3, seed 20261013) and a paired Wilcoxon p-value. The pre-registered bar is a CI entirely below 0 and p < 0.0167.

| Convention | n | Difference | 95% CI | Wilcoxon p | Supports H1 |
|---|---|---|---|---|---|
| S0 | 66 | +56.2 | [+28.1, +80.6] | 0.0009 | No (significantly worse) |
| S1 | 65 | +18.9 | [−1.6, +51.0] | 0.135 | No |
| S2 | 65 | +20.1 | [+0.4, +51.9] | 0.086 | No |

**Result:** H1 is not supported on the training set. No sign convention beats the best baseline. The naive S0 pin, which treats dealers as long calls and short puts (the US convention), is significantly worse than predicting no move.

This is a training-set result only. The test set was run once, on 17 Oct 2026 ([RESULTS_H1_TEST.md](RESULTS_H1_TEST.md)), and its result stands whatever this one shows.
