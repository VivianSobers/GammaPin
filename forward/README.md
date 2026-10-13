# Forward test (H1, live expiry)

This directory holds the H1 forward test, published before settlement so that nobody can adjust it after the fact.

| File | Written by | Contents |
|---|---|---|
| `{expiry}.json` | `forward_test.py predict` | `payload` (pin, flip, regime and net GEX for each convention, plus the `no_move` and `nearest_100` baselines, source snapshot hash and time, `created_at_utc`) and `sha256` of the payload |
| `{expiry}_snapshot.json` | `forward_test.py predict` | Byte-for-byte copy of the C2 snapshot the prediction came from |
| `{expiry}_score.json` | `forward_test.py score` | Settlement and \|settlement − prediction\| for each method |

`sha256` is the SHA-256 of the payload's canonical JSON: keys sorted, no whitespace, ASCII only. Max pain is left out because a C2 snapshot has no OI per strike.

## Publish procedure (expiry day)

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
