# GammaPin Pre-Registration

> **Status: DRAFT, not frozen.**
> Freeze = commit this file with every `TBD` filled in, tag it `prereg-v1`, and only then open the test set.
> The test set is run once. Any change after the freeze goes in a dated "Deviations" section below.

## Data

| Item | Value |
|---|---|
| Sample | Nifty weekly expiries from the Upstox Expired Instruments API (~50) |
| Kept contracts | Only those passing the coverage check (`fetch_expired.py` coverage log) |
| Train / test split | Chronological 70 / 30. Last train expiry: `TBD`. First test expiry: `TBD` |
| Risk-free rate | RBI 91-day T-bill yield, constant within each day |
| Participant OI | NSE `fao_participant_oi_DDMMYYYY.csv`, day d used only from day d+1 |

## Sign conventions

| | w_c (calls) | w_p (puts) |
|---|---|---|
| S0: US convention | +1 | −1 |
| S1: participant-signed, writers W = FII + Pro (day d−1) | (CL_W − CS_W) / ΣCL | (PL_W − PS_W) / ΣPL |
| S2: opposite of clients | −(CL_Client − CS_Client) / ΣCL | −(PL_Client − PS_Client) / ΣPL |

Stated approximation: participant OI covers all NSE index options and is not per strike. S1 and S2 spread it evenly across strikes.

## H1: pinning

- **Prediction:** at 14:00 IST on expiry day, the strike with the largest positive GEX, under S0, S1 and S2.
- **Baselines:** max pain; nearest multiple of 100; F at 14:00.
- **Metric:** |settlement − prediction|.
- **Tests:** paired Wilcoxon signed-rank; 95% CI for the difference in mean absolute error from 10,000 block-bootstrap resamples (block length `TBD`).
- **Success criterion:** `TBD` (for example, the CI for MAE(S·) − MAE(best baseline) lies entirely below 0).

## H2: volatility regime

- **Model:** log RV(t+1) on standardized net GEX(t), 5-minute bars.
- **Controls:** HAR lags of RV, time-of-day dummies, days to expiry, distance to flip level.
- **Errors:** Newey-West, 6 lags.
- **Comparison:** out-of-sample R² gain over the HAR baseline for S0, S1, S2.
- **Success criterion:** `TBD`.

## Look-ahead guards

- Participant OI from d+1 only.
- OI as of each decision time.
- Settlement never a feature.
- Perturbation test: changing data after the decision time leaves features unchanged.

## Deviations

None yet.
