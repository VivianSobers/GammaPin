# Does gamma exposure pin Nifty at expiry? (G7.5)

**Short answer: no.** On 94 Nifty weekly expiries, split 66 training and 28 test before any outcome was computed, the strike with the largest positive gamma exposure (GEX) at the expiry-eve close is never significantly better a guess of the settlement than the best simple baseline. Under the US sign convention it is significantly worse. In the first forward test (19 Oct 2026) S0 missed by 311 points, while S1/S2 missed by 111, closer than max pain (161), nearest 100 (211) and no move (215); one expiry cannot confirm or reject H1 (see [Forward test](#forward-test)).

## Question

GEX charts for Nifty are widely used to call a "pin": the strike the index is drawn to at expiry because option dealers hedge their gamma. We found no open test of it on NSE data ([NOVELTY.md](NOVELTY.md)). The charts also assume the US sign convention (dealers long calls, short puts), which NSE data cannot confirm. NSE does publish positioning by participant type, so we compared three conventions:

| | Who is assumed to be on the other side of customers |
|---|---|
| S0 | US convention: dealers long every call, short every put |
| S1 | Writers W = FII + Pro, signed by their net index-option position from NSE participant OI |
| S2 | The opposite of the Client category's net position |

## Design

Everything below was fixed in [PREREGISTRATION.md](../PREREGISTRATION.md) and tagged `prereg-v1` before the test set was opened. The test set was run once.

| Item | Value |
|---|---|
| Data | NSE F&O bhavcopy, participant OI and index close; RBI 91-day T-bill. All free and public |
| Sample | 94 nearest Nifty expiries, 2 Jan 2025 to 13 Oct 2026 |
| Split | Chronological: train to 30 Mar 2026 (66), test from 7 Apr 2026 (28) |
| Decision time | 15:30 IST close of the expiry eve (last session before expiry) |
| Prediction | Strike with the largest positive GEX, under S0, S1, S2 |
| Baselines | Max pain, nearest multiple of 100, no move (the eve's forward) |
| Target | Official Nifty 50 closing index value on expiry day ([SETTLEMENT.md](SETTLEMENT.md)) |
| Metric | Absolute error, index points |
| Bar | Against the split's best baseline: 95% block-bootstrap CI of the MAE difference entirely below 0 **and** paired Wilcoxon p < 0.05/3 |

## Results

### Mean absolute error (index points)

| Prediction | Train MAE | n | Test MAE | n |
|---|---|---|---|---|
| pin S0 | 201.6 | 66 | 227.8 | 28 |
| pin S1 | 165.2 | 65 | 153.5 | 28 |
| pin S2 | 166.5 | 65 | 153.5 | 28 |
| max pain | 160.3 | 66 | **144.8** | 28 |
| nearest 100 | 146.2 | 66 | 154.3 | 28 |
| no move | **145.4** | 66 | 154.7 | 28 |

Bold marks each split's best baseline: no move on the training set, max pain on the test set. On one training eve (5 Feb 2025) both S1 weights and both S2 weights are negative, so no strike has positive GEX and there is no pin; S1 and S2 have n = 65 there.

### Pre-registered test: GEX pin minus best baseline

Positive differences mean the pin is worse. CIs from 10,000 moving-block bootstrap resamples (block 3, seed 20261013).

| Convention | Split | Difference | 95% CI | Wilcoxon p | Supports H1 |
|---|---|---|---|---|---|
| S0 | train | +56.2 | [+28.1, +80.6] | 0.0009 | No, significantly worse |
| S1 | train | +18.9 | [−1.6, +51.0] | 0.135 | No |
| S2 | train | +20.1 | [+0.4, +51.9] | 0.086 | No |
| S0 | test | +83.0 | [+45.3, +127.0] | 0.0079 | No, significantly worse |
| S1 | test | +8.7 | [−41.3, +35.0] | 0.793 | No |
| S2 | test | +8.7 | [−41.3, +35.0] | 0.793 | No |

**H1 is not supported.** No convention meets the bar on either split, and every difference against the split's best baseline is positive. (On the test set S1/S2 are 0.8 and 1.2 points better than nearest 100 and no move, far from significant.)

Per-run detail: [RESULTS_H1_TRAIN.md](RESULTS_H1_TRAIN.md), [RESULTS_H1_TEST.md](RESULTS_H1_TEST.md), and `research/output/h1_eod_test.csv`.

## What the results say

1. **The US convention is the worst predictor tested.** The S0 pin is significantly worse than the best baseline on both splits, and than every baseline on the test set: on average 83 points further from settlement than max pain. A pin read off a Nifty GEX chart built on the US sign assumption does worse than assuming the index will not move.
2. **Signing by real positioning removes most of that damage, but does not add skill.** S1 and S2 sit about 19–20 points above the best baseline on the training set (S2's CI lies just above 0) and about 9 points above it on the test set, where the CIs straddle 0. They are as good as a round number, not better.
3. **The baselines are hard to beat and hard to separate.** No move, nearest 100 and max pain are within 15 points of each other on both splits, and which one is best changes between splits. A typical miss is 125–135 points for all of them (test-set medians), so a one-session-ahead pin call has a wide error whichever method makes it.

## Descriptive notes (not pre-registered)

These come from the committed test-set outputs and are for context only. None of them is a test, and none changes the result above.

- **S1 and S2 give the same pin on all 28 test expiries.** In both conventions the call weight is negative on 25 of 28 eves and the put weight positive on all 28, so the strike with the largest positive GEX is the same. Signing by FII + Pro writers or against clients made no practical difference in this sample.
- **S1 picks the max-pain strike on 11 of 28 test expiries; S0 never does.** Some of S1's competitiveness comes from agreeing with max pain.
- **Closing Auction Session.** From 3 Aug 2026 the settlement price comes from the closing auction ([SETTLEMENT.md](SETTLEMENT.md)). Test MAE before and after:

  | Prediction | Before 3 Aug (17) | From 3 Aug (11) |
  |---|---|---|
  | pin S0 | 231.9 | 221.5 |
  | pin S1 / S2 | 160.0 | 143.4 |
  | max pain | 125.8 | 174.2 |
  | nearest 100 | 150.3 | 160.6 |
  | no move | 143.7 | 171.7 |

  With 11 expiries after the change this is too small to read anything into. It is reported because the pre-registration lists the change as an open item.
- **Hits within 50 points (test):** max pain 7, S1/S2 6, nearest 100 6, no move 4, S0 2, out of 28.

## Not tested

**H2 (GEX predicts the volatility regime)** needs 5-minute intraday option history. No free source has it, and buying it needs a broker account. H2 is reported as untested, not as a null result; its pre-registered design is kept for a later study with intraday data.

## Forward test

The H1 predictions for **expiry 1 (Mon 19 Oct 2026)** and **expiry 2 (Tue 27 Oct 2026)** are published from the eve's bhavcopy before the session, with a SHA-256 in a public git commit, then scored against the settlement ([forward/README.md](../forward/README.md)). Expiry 1 was added after the freeze (PREREGISTRATION.md, "Deviations"). Two forward expiries cannot confirm or reject H1. They show the pipeline running live and in public, without hindsight.

| Expiry | Eve | Published (UTC) | SHA-256 | Pin S0 | Pin S1 | Pin S2 | Max pain | Nearest 100 | No move | Settlement | Errors |
|---|---|---|---|---|---|---|---|---|---|---|---|
| 19 Oct 2026 | 16 Oct | 18 Oct 17:16 | `c5827627…` | 23,200 | 23,000 | 23,000 | 23,050 | 23,100 | 23,103.5 | 22,888.95 | S0 311, S1 111, S2 111, MP 161, N100 211, NM 215 |
| 27 Oct 2026 | 26 Oct | pending | | | | | | | | | |

## Limitations

- **Eve close, not expiry-day 14:00.** The first design predicted from 14:00 on expiry day, closer to the event. Intraday data was not freely available, so the decision moved to the eve close, one full session before settlement ([PREREGISTRATION.md](../PREREGISTRATION.md), "Design change before the freeze"). Pinning that only shows up in the last hours of expiry day would not be seen here.
- **Close prices, not a snapshot.** `ClsPric` is NSE's closing price for each option, and untraded strikes give no price: they get no IV of their own, but their OI still counts, at an IV interpolated from the neighbouring strikes. The chain is a close-period approximation.
- **Participant OI is not per strike.** S1 and S2 apply one sign pair across all strikes, from positions in all NSE index options, not Nifty alone.
- **Sample size.** 28 test expiries give wide CIs: on S1 the test CI spans about 76 points. A small real effect could be missed. Every pre-registered comparison against the best baseline, though, points against H1.
- **No live capture.** The study uses no tick data and does not validate GEX from the end-of-day chain against intraday chains (G6.2 dropped).

## Reproduce

```bash
pip install maturin -r research/requirements.txt ./crates/gp-quant
cd research
python bhavcopy.py --start 2024-12-01 --end 2026-10-13
python participant_oi.py --start 2024-12-01 --end 2026-10-13
python settlements.py --start 2025-01-01 --end 2026-10-13
python h1_eod.py --split train
python h1_eod.py --split test
```

The T-bill series is rebuilt from RBI press releases with `python tbill_rbi.py --lo 58700 --hi 63900`, the prid range scanned for this study (to extend it, start one above the highest prid in `cache/tbill_rbi_pages.csv` and go about 60 higher per week of releases; see `rates.py`). Raw NSE files are not committed; the commands above download them.

**Analytics, not trading advice.**
