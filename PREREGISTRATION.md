# GammaPin Pre-Registration

> **Status: DRAFT, not frozen.**
> Freeze = commit this file with every `TBD` filled in, tag it `prereg-v1`, and only then open the test set.
> The test set is run once. Any change after the freeze goes in a dated "Deviations" section below.

## Data

| Item | Value |
|---|---|
| Source | NSE F&O daily bhavcopy (UDiFF), free from nsearchives.nseindia.com (`research/bhavcopy.py`). No broker data. |
| Sample | Every Nifty option expiry dated 1 Jan 2025 to 13 Oct 2026 that is the nearest listed expiry in some cached bhavcopy and whose eve bhavcopy is cached (`h1_eod.sample`). Chosen from the expiry listings only, before any outcome was computed. |
| Train / test split | Chronological 70 / 30. Last train expiry: `TBD`. First test expiry: `TBD` |
| Risk-free rate | RBI 91-day T-bill yield, latest auction strictly before the eve |
| Participant OI | NSE `fao_participant_oi_DDMMYYYY.csv`, day d used only from day d+1 |

## Sign conventions

| | w_c (calls) | w_p (puts) |
|---|---|---|
| S0: US convention | +1 | −1 |
| S1: participant-signed, writers W = FII + Pro (day d−1) | (CL_W − CS_W) / ΣCL | (PL_W − PS_W) / ΣPL |
| S2: opposite of clients | −(CL_Client − CS_Client) / ΣCL | −(PL_Client − PS_Client) / ΣPL |

Stated approximation: participant OI covers all NSE index options and is not per strike. S1 and S2 spread it evenly across strikes.

## Design change before the freeze (15 Oct 2026)

The first draft used 1-minute option candles from the Upstox Expired Instruments API, which needs the paid Upstox Plus plan. The project runs on free data only, so before any data were downloaded or any result computed:

- **H1** moves to end-of-day chains from the NSE bhavcopy, with the decision at the close of the expiry eve instead of 14:00 on expiry day. The 14:00 design stays in code (`h1_pinning.py`) and is not run.
- **H2** needs intraday history, which no free source provides, so it is not tested (see H2).

## H1: pinning

- **Prediction:** at the close (15:30 IST) of the last trading day before expiry (the eve), the strike with the largest positive GEX, under S0, S1 and S2.
- **Baselines:** max pain; nearest multiple of 100; F at the eve close.
- **Metric:** |settlement − prediction|.
- **Tests:** paired Wilcoxon signed-rank; 95% CI for the difference in mean absolute error from 10,000 block-bootstrap resamples (block length 3).
- **Success criterion:** for each convention S0, S1, S2 separately, compare against the best baseline, meaning the baseline with the lowest MAE on the same split and the same expiries. Convention S· supports H1 when both hold:
  1. the 95% block-bootstrap CI for MAE(S·) − MAE(best baseline) lies entirely below 0, and
  2. the paired Wilcoxon p-value is below 0.05 / 3 (Bonferroni over the three conventions).

  H1 is supported if at least one convention meets both. Comparisons against the other baselines are reported but are not part of the criterion.

## H2: volatility regime

**Not tested in this study.** It needs 5-minute history that only the paid Upstox data provide. The design and criterion below are kept so that a later study with intraday data starts from a fixed plan; running it here would be a dated deviation.

- **Model:** log RV(t+1) on standardized net GEX(t), 5-minute bars.
- **Controls:** HAR lags of RV, time-of-day dummies, days to expiry, distance to flip level.
- **Errors:** Newey-West, 6 lags.
- **Comparison:** out-of-sample R² gain over the HAR baseline for S0, S1, S2.
- **Success criterion:** convention S· supports H2 when both hold:
  1. the gex_z_S coefficient in the training fit is negative (higher positive GEX, lower next-bar RV) with Newey-West two-sided p < 0.05, and
  2. the test-split R²_oos against the primary baseline is above 0.

  H2 is supported if at least one convention meets both.

## Look-ahead guards

- Participant OI from d+1 only.
- OI as of each decision time.
- Settlement never a feature.
- Perturbation test: changing data after the decision time leaves features unchanged.

## Analysis details (draft, not frozen)

These are the implementation decisions already in code (`research/`, `crates/gp-quant`). They become pre-registered at the freeze. Values marked `TBD` above are not set here.

### Data and look-ahead

| Item | Rule |
|---|---|
| Candle timing | A 1-minute candle stamped `ts` (its start, naive IST) is known only from `ts + 1 min`. |
| Chain as of t | Per contract, the last known close and the last known OI. A close is used only if it is less than 30 minutes old at t (candle start > t − 31 min); otherwise its price counts as missing (0, rejected by the quote filters) while its OI is kept. A side with no known candle has price 0 and OI 0. |
| Participant OI | Latest NSE file dated strictly before the trading day, index-option columns, in contracts. Error if that file is more than 7 calendar days older than the day. |
| Risk-free rate | Cut-off yield y (decimal) of the latest RBI 91-day T-bill auction dated strictly before the day (a same-day result may not be public by 14:00 IST), constant within the day, converted to r = (365/91)·ln(1 + y·91/365). Source: hand-built CSV `research/cache/tbill_91d.csv`. |
| Settlement | Official Nifty 50 `Closing Index Value` on the expiry date, from NSE `ind_close_all_DDMMYYYY.csv`. Target only. Equals NCL's option settlement price; rule and the 3 Aug 2026 change in [docs/SETTLEMENT.md](docs/SETTLEMENT.md). |
| Option OI units | Upstox candle OI assumed in units (lot size × contracts) and divided by the lot size (`OI_IN_UNITS = True`). |
| Fetch window | Expiry − 6 calendar days to expiry, every listed strike (the default; see Open items for `--band-pct`). |
| Coverage | An expiry is `complete` when both the index candle count and the number of distinct option-candle minutes (union over contracts, on index dates) reach ≥ 95% of 375 × sessions in the index data. |
| Time to expiry | T = trading minutes to 15:30 on expiry / (252 × 375): today's minutes from max(t, 09:15), plus 375 per trading session after today up to expiry. Floored at one trading minute. Trading sessions = dates in the cached index candles (H1 also adds the expiry dates). |

### Pricing and GEX (gp-quant, candle mode)

| Step | Rule |
|---|---|
| Quotes | The close is used as bid = ask. The two-sided checks (bid > 0, ask > bid, spread ≤ max(25% of mid, 4 ticks of 0.05)) are skipped; mid ≥ 0.10 is kept. |
| Forward | F_K = K + e^{rT}(C_mid − P_mid) at strikes where both mids pass. F = median of F_K over the 3 strikes with the smallest \|C_mid − P_mid\| (ties → lower strike; with fewer than 3, the median of those available). None → no chain. |
| IV | Black-76 on each strike's OTM quote (call if K ≥ F, else put). Rejected if mid < discounted intrinsic, the solver fails, or σ ∉ [0.01, 3.0]. Solver: Newton in the bracket [1e-4, 5] with Brent fallback, \|price error\| < 1e-8. No IV at any strike → no chain. |
| Smile gaps | Rejected strikes get σ by linear interpolation in strike between the nearest accepted strikes, flat beyond the ends. |
| Gamma | Black-76 Γ at F with that σ (same for calls and puts). |
| GEX | GEX_K = (w_c·OI_K^C + w_p·OI_K^P)·Γ_K·lot·F²·0.01 (₹ per 1% move, OI in contracts). Net GEX = Σ_K GEX_K over all listed strikes. |
| Flip | Net GEX recomputed at levels F·(1 + 0.0005·i), i = −60…60 (F ± 3% in 0.05% steps), each strike's σ, T and r held fixed (sticky strike). Sign changes between adjacent levels located by linear interpolation (an exact zero at a level also counts); the one nearest F is kept. None if net GEX keeps one sign. |
| Pin | Strike with the largest positive GEX_K; ties → higher strike. None if no GEX_K > 0. |

### H1 implementation (end-of-day, primary)

| Item | Rule |
|---|---|
| Sample | As in Data: nearest listed expiries dated 2025-01-01 to 2026-10-13 with a cached eve bhavcopy (`prereg.toml [h1_eod]`). The split is `prereg.expiries_for` over this sample. |
| Eve | The last cached bhavcopy day before the expiry. Decision time 15:30 IST on the eve. |
| Chain | Nifty index-option rows of that expiry in the eve's bhavcopy. Price = `ClsPric` when `TtlTradgVol` > 0, else 0 (rejected by the quote filters). OI in contracts = `OpnIntrst` / `NewBrdLotQty` (bhavcopy OI is in units). Lot = the modal lot size of the expiry's rows. Rows dated after the eve are refused. |
| Time to expiry | T = trading minutes from 15:30 on the eve to 15:30 on expiry / (252 × 375), i.e. 375 × trading sessions after the eve up to expiry (375 when the expiry is the next session). Sessions = cached bhavcopy days plus the expiry. |
| Inputs | r and S1/S2 weights for the eve date (files strictly before the eve). |
| Pricing and GEX | gp-quant candle mode as above, on this chain. |
| Max pain, nearest 100, no move | As in the intraday table below, from the eve-close chain and F at the eve close. |
| Status | `no_bhavcopy`, `no_rate`, `no_chain` (no forward or smile), else `ok`. Missing weights keep S0 only (`weights_ok` false). |
| Outputs | `research/output/h1_eod_{split}.csv` and `_summary.json`. Tests and success criterion as in H1. |

### H1 implementation (intraday 14:00, not run)

| Item | Rule |
|---|---|
| Sample | Cached expiries flagged `complete` in `coverage_summary.csv` (rule above); the split is computed over these. Without the file the run stops. Excluded counts are reported in the summary JSON (`coverage`) and the CSV has a `coverage_complete` column. `--include-incomplete` keeps every cached expiry, for train diagnostics only; it is recorded in the JSON and refused for the test split. |
| Decision time | 14:00 IST on the expiry day; the chain uses candles stamped ≤ 13:59. |
| Inputs | r and S1/S2 weights for the expiry date (rules above). |
| Max pain | Listed strike in the 14:00 chain minimising Σ_j OI_j^C·max(K − K_j, 0) + OI_j^P·max(K_j − K, 0). Ties → lower strike. Non-finite or negative OI counts as 0. NaN if total OI is 0. |
| Nearest 100 | F at 14:00 rounded to the nearest multiple of 100, halves up. |
| No move | F at 14:00. |
| No options file | Status `no_options`; every prediction NaN. |
| No rate | Status `no_rate`; every prediction NaN, baselines included. |
| No participant OI | `weights_ok = false`; S1 and S2 NaN; S0 and baselines kept. |
| No forward or no smile | Status `no_chain`; GEX pins, nearest 100 and no move NaN; max pain still computed from the chain. |
| Pin None | NaN for that convention. |
| No settlement | Errors NaN; status unchanged. |
| NaN errors | Excluded from that method's MAE and from every paired comparison it enters. Each MAE reports its n; each comparison reports n and n_excluded. |

Tests, per (S·, baseline) pair (3 × 3), on the expiries where both errors are finite, in chronological order, with d = \|err_S\| − \|err_baseline\|:

- **Wilcoxon:** two-sided signed-rank on d; zero differences dropped (`zero_method="wilcox"`); all d = 0 gives statistic 0, p = 1.
- **Bootstrap:** moving-block bootstrap of mean(d): ⌈n/b⌉ blocks of length b with uniform starts in [0, n − b] (no wrap-around), concatenated and cut to n; 10,000 resamples; 95% percentile CI; seed 20261013 for every comparison. Skipped, with a note, when n < b.
- **Block length b:** 3 (≈ n^{1/3} for about 30 training expiries; allows dependence between neighbouring weeks).
- **Split while `last_train_expiry` is `TBD`:** train = first ⌈0.7·n⌉ complete cached expiries (provisional, with a warning). The test split refuses to run, and also requires the git tag `prereg-v1`.

### H2 implementation

| Item | Rule |
|---|---|
| Sample | As H1: complete cached expiries only, split over these with the same rules (`prereg.expiries_for`). Coverage counts in the report JSON (`coverage`), `coverage_complete` in the features file; `--include-incomplete` for train diagnostics only, refused for the test split. |
| Bars | 5-minute bars aligned to 09:15 (75 per session). Candles off the minute grid or outside 09:15–15:30 are dropped. |
| RV | sqrt(Σ squared 1-minute log returns of the candles stamped in the bar). Return = log(close / previous close of the same session); the session's first candle uses log(close / open), so no overnight return. RV is null with fewer than 3 returns. |
| Target | y = log RV of bar t+1, where t+1 is the next bar index of the same session; otherwise null. Rows with null y are not used for fitting or scoring. |
| HAR | har_5m = log RV_t; har_1h = log of the mean RV over bars t−11…t; har_1d = log of the mean RV over bars t−74…t. Windows run over consecutive available bars in time order, across sessions (overnight gaps skipped). Rows without a full window (any null RV) are dropped. |
| Time of day | Bucket k = bar // 6 (30 minutes), k = 0…12; dummies for k = 1…12; 09:15–09:45 is the reference. Bucket 12 is 15:15–15:30 (3 bars). |
| dte | Trading sessions d with date(t) < d ≤ expiry (0 on expiry day). |
| GEX inputs | Chain of that expiry as of bar_end(t), with r and S1/S2 weights of date(t). Bars where the chain gives no forward or smile are dropped. |
| Flip terms | flip_dist_S = (F − flip_S)/F at bar_end(t); 0 and flip_none_S = 1 when there is no flip. |
| Standardization | gex_z_S = (net GEX_S − mean)/std, mean and sample std (ddof 1) from the training rows with non-null y only; the same constants are applied to evaluation rows. |
| Models, per S | Model: const + 3 HAR + 12 tod + dte + flip_dist_S + flip_none_S + gex_z_S. Primary baseline: the same without gex_z_S. Secondary baseline `har_only`: const + HAR + tod + dte. Columns with no variation in the training rows are dropped (if gex_z_S is dropped the run fails). |
| Estimation | OLS on training rows pooled over expiries and sorted by bar_end; Newey-West HAC (Bartlett), 6 lags. |
| Score | R²_oos = 1 − SSE_model/SSE_baseline on the evaluation rows. Also reported: the same against `har_only`, and 1 − SSE/SST for each model. |
| Expiry week | Trading days d with max(expiry − 6 days, previous cached expiry + 1 day) ≤ d ≤ expiry. The previous expiry may be incomplete. |
| Missing inputs | A day without a rate or participant OI is dropped for all conventions (listed in `skipped_days`). |
| Train split | In-sample fit on all training rows, plus pseudo-OOS: the training expiries with usable rows, in order; the first max(1, ⌊0.7·n⌋) are standardized and fitted, the rest evaluated. Skipped with fewer than 2 expiries. |
| Test split | Standardize and fit on all training rows, score on the test rows. As H1: refuses while `last_train_expiry` is `TBD` and requires the git tag `prereg-v1`. While `TBD`, the train split is the provisional first ⌈0.7·n⌉ with a warning. |

### Open items

- **Option OI units (intraday design only).** `OI_IN_UNITS` is unverified for Upstox candles. Bhavcopy OI is in units: about 97% of Nifty option rows on 13 Oct 2026 divide by the lot size, and the rest are long-dated contracts under an older lot size.
- **Close vs decision.** `ClsPric` for options is NSE's closing price (equal to `SttlmPric` on non-expiry days), not the last trade, so the eve chain is a close-period price, not a 15:30 snapshot.
- **Closing Auction Session.** From 3 Aug 2026 the settlement close is formed by the CAS auction price (15:15–15:35) instead of the 15:00–15:30 VWAP, and the sample spans that date ([docs/SETTLEMENT.md](docs/SETTLEMENT.md)). Possible effects: H1 may need a pre/post split or control; T is still measured to 15:30 while F&O trades until 15:40; H2's 15:15–15:30 bucket may show near-zero RV after 3 Aug.
- **Strike band.** `fetch_expired.py --band-pct` picks strikes with the expiry-eve close, which is after the earlier days H2 uses. Data for H2 must be fetched without it.
- **Per-contract coverage.** Coverage filters whole expiries; no rule drops individual contracts using `coverage.csv`.

## Deviations

None yet.
