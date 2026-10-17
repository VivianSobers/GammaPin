# Forward test (H1, live expiry)

This directory holds the H1 forward test, published before settlement so that nobody can adjust it after the fact.

The primary design is end-of-day (`predict-eod`): the prediction is made from the expiry eve's NSE bhavcopy, the same design H1 was tested on, and needs no broker account. The 14:00 snapshot design (`predict`) needs the live monitor, which is not built since live capture was dropped (17 Oct 2026, [CONTRIBUTING.md](../CONTRIBUTING.md#scope-change-17-oct-2026-no-live-capture)).

| File | Written by | Contents |
|---|---|---|
| `{expiry}.json` | `forward_test.py predict-eod` | `payload` (eve, forward, pin, flip and net GEX for each convention, the `max_pain`, `nearest_100` and `no_move` baselines, the eve bhavcopy's file name and SHA-256, `created_at_utc`) and `sha256` of the payload |
| `{expiry}.json` | `forward_test.py predict` (14:00 design) | `payload` (pin, flip, regime and net GEX for each convention, plus the `no_move` and `nearest_100` baselines, source snapshot hash and time, `created_at_utc`) and `sha256` of the payload |
| `{expiry}_snapshot.json` | `forward_test.py predict` (14:00 design) | Byte-for-byte copy of the C2 snapshot the prediction came from |
| `{expiry}_score.json` | `forward_test.py score` | Settlement and \|settlement − prediction\| for each method |

`sha256` is the SHA-256 of the payload's canonical JSON: keys sorted, no whitespace, ASCII only. The 14:00 design leaves out max pain because a C2 snapshot has no OI per strike.

## Publish procedure, end-of-day design (primary)

Run from `research/`. The eve is the last trading day before expiry.

1. **Eve evening, after NSE publishes the day's files:** download the eve's bhavcopy and participant OI.
   ```
   python bhavcopy.py --start <eve> --end <eve>
   python participant_oi.py --start <eve> --end <eve>
   ```
   If RBI has held a 91-day T-bill auction since the last one in `cache/tbill_91d.csv`, refresh it with `tbill_rbi.py`.
2. **Before 09:15 IST on expiry day** (later works until 15:30, with a warning): run `python forward_test.py predict-eod --expiry YYYY-MM-DD`.
3. **Check that `eve` in `forward/{expiry}.json` is the real eve.** `predict-eod` uses the last cached bhavcopy before expiry, so a missed download in step 1 silently gives an older day. If it is wrong, download the file and rerun with `--force` (only before publishing).
4. Run the `git add`, `git commit` and `git push` commands it prints, **before 15:30 IST**. The pushed commit on GitHub is the public timestamp.
5. Post the printed hash somewhere public that records the time, as a second witness.
6. Never edit or replace a published `{expiry}.json`.

## Publish procedure, 14:00 snapshot design (not used: needs the live monitor)

1. Around 14:00 IST, save one C2 snapshot from the live monitor (`/ws`) to a file.
2. From `research/`, run `python forward_test.py predict --snapshot <file> --expiry YYYY-MM-DD`. The command refuses if the snapshot is not from expiry day, is after 15:00 IST, or shows a different expiry. It warns if the snapshot is more than 5 minutes from 14:00 or its source is not `live`.
3. Run the `git add`, `git commit` and `git push` commands it prints, **before 15:30 IST**. The pushed commit on GitHub is the public timestamp.
4. Post the printed hash somewhere public that records the time, as a second witness.
5. Never edit or replace a published `{expiry}.json`. `predict` will not overwrite one unless you pass `--force`, and `--force` is only for files that have not been published yet.

## After settlement

```
python forward_test.py verify ../forward/YYYY-MM-DD.json
python forward_test.py score --expiry YYYY-MM-DD [--settlement <Nifty 50 close>]
```

`score` refuses to run if the prediction fails verification. Without `--settlement` it reads the official close from the `settlements.py` cache. Commit `{expiry}_score.json` once it is written.
