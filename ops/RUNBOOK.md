# GammaPin Runbook

All times IST.

Live capture was dropped on 17 Oct 2026 ([CONTRIBUTING.md](../CONTRIBUTING.md#scope-change-17-oct-2026-no-live-capture)): every live NSE feed needs a broker account with SEBI KYC. There is no daily capture, token or node routine.

The capture, merge, replay and monitor binaries are built and tested on a synthetic feed ([docs/SYSTEMS.md](../docs/SYSTEMS.md)). [Systems](#systems-built-not-deployed) below shows how to run them, on the mock feed today or on Upstox with an account. The systemd units and `chrony.conf` in this directory are for that deployment.

The routine in use is the H1 forward test on Nifty expiry days, from free NSE files only.

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

## Systems (built, not deployed)

### Dry run on the mock feed

No account needed. Everything runs on one machine against `gp-mockfeed`, a synthetic Nifty market on the Upstox V3 protocol.

```bash
cargo build --release
B=target/release; D=$(TZ=Asia/Kolkata date +%F)
mkdir -p dry && cd dry
../$B/gp-mockfeed --date $D --master NSE.json --now &  # ws://127.0.0.1:9100
```

Write `a.toml` (and `b.toml` with `name = "B"`):

```toml
[node]
name = "A"
data_dir = "inbox"                 # segments in inbox/A/{date}/
[capture]
feed_url = "ws://127.0.0.1:9100"
master_path = "NSE.json"
[merge]
inbox = "inbox"
out = "merged"
master_path = "NSE.json"
[monitor]
bind = "127.0.0.1:8080"
static_dir = "../crates/gp-monitor/static"
```

| Step | Command | Check |
|---|---|---|
| Capture on two nodes | `../$B/gp-capture --config a.toml --now &` and the same with `b.toml` | `inbox/A/$D/seg-000000.gpl` and `inbox/B/…` grow |
| Monitor, live | `../$B/gp-monitor --config a.toml live` | http://127.0.0.1:8080 shows `LIVE`, both nodes up |
| Cut a node | Stop B with Ctrl-C for a while, then start it again | Monitor shows B down after 5 s, then up |
| Merge | Stop the captures, then `../$B/gp-merge --config a.toml --date $D` | Exit 0; `merged/date=$D/coverage.md` shows B's outage as its longest silence, and no lost window; segments are now `.gpl.zst` |
| Replay | `../$B/gp-monitor --config a.toml replay --date $D --speed 10x` | The page shows `REPLAY` and the session again |
| Determinism | `../$B/gp-replay hash merged/date=$D/replay.gpr` twice | Same SHA-256 |

### Dashboard on live `/ws`

Open http://127.0.0.1:8080 (no `?mock`) while the monitor runs live. For S1, S2 and the H1 accuracy, copy the commented `weights` line and `[monitor.h1_accuracy]` from `config/gammapin.example.toml` into `a.toml`; without them the page shows S0 only and "no history yet".

| Check | Expect |
|---|---|
| Status line | `LIVE`, WebSocket `open`, updated < 3 s ago; no `MOCK DATA` flag or "(no /ws snapshot …)" reason |
| Validation | No `rejected: …` on the status line |
| Panels | Spot, F and t; GEX by strike for each convention with F, flip and pin markers; pin with H1 accuracy; regime; data health with both nodes up |
| Lag | p50 ≈ 0.6–0.75 s, p99 ≈ 1.1–1.25 s: C2 lag includes the wait for the next snapshot. After a monitor restart mid-day, the first 60 s read up to 10 s while it catches up on the day's logs |
| Cut a node | Node B turns `down` about 5 s after it stops, `up` within 2 s of its restart |
| Restart the monitor (e.g. live → replay) | `closed, retry in N s`, then `open` and `REPLAY`, with no fallback to mock |

### With an Upstox account (if one is ever opened)

| Time | Node | Step | Check |
|---|---|---|---|
| 08:45 | C | Log in, write the day's token to `/etc/gammapin/upstox.token` on A and B | `gp-capture check-token` exits 0 on both |
| 08:55 | A, B | `gp-capture.timer` starts capture (window 09:00–15:40) | `journalctl -u gp-capture` shows the subscription |
| 08:58 | C | `gp-monitor.timer` starts the live monitor | Page shows `LIVE`, nodes A and B up |
| 09:05 | A, B | `gp-token-check.timer` | Fails (and alerts) when no token from today exists |
| 15:40– | A, B | rsync the day's segments into Node C's inbox | Sizes match on both sides |
| 15:50 | C | `gp-merge.timer` | Exit 0; exit 3 means a window no node captured: read `coverage.md` |

| Symptom | Action |
|---|---|
| Monitor shows a node down | Check that node's capture: `journalctl -u gp-capture`. The other node keeps coverage; nothing is lost unless both are down |
| `gp-merge` exits 3 | Some window was missed by both nodes. It is listed under `lost` in `coverage.json`; note it in the results |
| `gp-merge` fails on a torn segment | The copy to the inbox is incomplete: rsync again. Capture itself recovers torn tails on restart |
| Token expired mid-day | Write the new token; capture reloads it on the next reconnect and logs `TOKEN_RELOAD` |
