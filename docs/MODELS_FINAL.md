# Final models (G6.3)

Fixed on 17 Oct 2026, from the training set only, before the test split is opened. Nothing below was tuned after the training result in [RESULTS_H1_TRAIN.md](RESULTS_H1_TRAIN.md); every choice is the one frozen in `PREREGISTRATION.md` at tag `prereg-v1`.

## H1: pinning (`research/h1_eod.py`)

| Item | Final value |
|---|---|
| Data | NSE bhavcopy at the expiry-eve close |
| Predictions | GEX pin under S0, S1, S2 |
| Baselines | max pain, nearest 100, no move (eve forward) |
| Comparison | the lowest-MAE baseline on each split, on the same expiries (PREREGISTRATION H1): **no move** on training (145.4 pts); the test split picks its own |
| Statistics | paired Wilcoxon, block bootstrap (10,000 resamples, block 3, seed 20261013) |
| Success bar | 95% CI of the MAE difference entirely below 0 and p < 0.0167, per convention |
| Split | train to 30 Mar 2026 (66 expiries), test from 7 Apr 2026 (28 expiries) |

No parameters are fitted for H1: the pin is a deterministic function of the chain, so finalizing means fixing the conventions, the baseline and the test, all listed above.

## H2: volatility (`research/h2_vol.py`)

Specification as pre-registered (HAR controls, 12 time-of-day dummies, dte, flip distance, gex_z_S; OLS with Newey-West, 6 lags). Not estimated: there is no free intraday option data for the sample, so H2 is not tested on either split. This is reported as untested, not as a null result.

## Deviations

None.
