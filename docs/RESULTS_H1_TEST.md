# H1 on the test set (G7.1)

Run once on 17 Oct 2026, after the `prereg-v1` freeze and the final models ([MODELS_FINAL.md](MODELS_FINAL.md)), with `python research/h1_eod.py --split test`. Outputs are committed: `research/output/h1_eod_test.csv` and `h1_eod_test_summary.json`.

**Sample:** 28 test expiries, 7 Apr 2026 to 13 Oct 2026, all with status `ok`, n = 28 for every prediction.

## Mean absolute error vs. settlement (index points)

| Prediction | MAE | n |
|---|---|---|
| pin S0 | 227.8 | 28 |
| pin S1 | 153.5 | 28 |
| pin S2 | 153.5 | 28 |
| max pain | **144.8** | 28 |
| nearest 100 | 154.3 | 28 |
| no move (eve forward) | 154.7 | 28 |

## Against the best baseline (max pain)

As pre-registered, the best baseline is the one with the lowest MAE on the same split: on the test set that is max pain, not no move as on the training set. 95% block-bootstrap CI (10,000 resamples, block length 3, seed 20261013), paired Wilcoxon p. Bar: CI entirely below 0 and p < 0.0167.

| Convention | n | Difference | 95% CI | Wilcoxon p | Supports H1 |
|---|---|---|---|---|---|
| S0 | 28 | +83.0 | [+45.3, +127.0] | 0.0079 | No (significantly worse) |
| S1 | 28 | +8.7 | [−41.3, +35.0] | 0.793 | No |
| S2 | 28 | +8.7 | [−41.3, +35.0] | 0.793 | No |

S1 and S2 give the same pin on every test expiry, so their rows are identical.

**Result:** H1 is not supported on the test set, matching the training set. No sign convention beats the best baseline, and the naive S0 pin is again significantly worse.

**H2:** not tested (no free intraday option data for the sample), as stated in the final models.
