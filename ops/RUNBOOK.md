# GammaPin Runbook

All times IST.

Live capture was dropped on 17 Oct 2026 ([CONTRIBUTING.md](../CONTRIBUTING.md#scope-change-17-oct-2026-no-live-capture)): every live NSE feed needs a broker account with SEBI KYC. There is no daily capture, token or node routine. The systemd units and `chrony.conf` in this directory belong to that dropped design; the earlier capture runbook is in git history.

What remains is the H1 forward test on Nifty expiry days, from free NSE files only.

## Expiry eve (the trading day before a Nifty expiry)

| Time | Step | Check |
|---|---|---|
| Evening, after NSE publishes | From `research/`: `python bhavcopy.py --start <eve> --end <eve>` and `python participant_oi.py --start <eve> --end <eve>` | Both files present in `research/cache/` |
| Same evening | Refresh `research/cache/tbill_91d.csv` with `tbill_rbi.py` if RBI held a 91-day auction since its last row | Latest row is the latest auction before the eve |

## Expiry day (Nifty, Tuesday; previous trading day on a holiday, see [docs/CALENDAR.md](../docs/CALENDAR.md))

| Time | Step | Check |
|---|---|---|
| Before 09:15 | `python forward_test.py predict-eod --expiry YYYY-MM-DD` | `eve` in `forward/{expiry}.json` is the real eve, not an older day |
| Before 15:30 | Commit and push the prediction with the commands it prints; post the hash publicly | Commit visible on GitHub, timestamp before 15:30 |
| Evening, after NSE publishes | `python settlements.py --start <expiry> --end <expiry>`, then `python forward_test.py score --expiry YYYY-MM-DD` | `forward/{expiry}_score.json` written; commit it |

Full procedure: [forward/README.md](../forward/README.md).

## When something breaks

| Symptom | Action |
|---|---|
| NSE file not published yet, or 404 | Wait and retry; NSE sometimes publishes late. A 404 on a weekday can mean a holiday: check the calendar |
| NSE blocks the download (403) | Retry later with the default `--pause`; do not hammer the site |
| `predict-eod` picked an older eve | Download the missing bhavcopy, rerun with `--force` (only before publishing) |
| Prediction not pushed by 15:30 | Do not publish it late. Record the miss in the results write-up |
