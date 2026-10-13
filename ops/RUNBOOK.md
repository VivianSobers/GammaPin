# GammaPin Runbook

All times IST. Node A and Node B are the capture nodes (Vivian). Node C runs merge, monitor and research (Guru).

## Every market day

| Time | Step | Who | Check |
|---|---|---|---|
| 08:30–08:45 | Run `gp auth`, complete the Upstox login, token file written | Rotating | Token file timestamp is today |
| 08:45 | Confirm `chronyc tracking` offset < 5 ms on all 3 nodes | Both | `System time` line |
| 08:55 | `gp-capture.timer` starts capture on A and B | Auto | `systemctl status gp-capture` is active on both |
| 09:00 | First RAW_FRAME records arriving | Both | Segment file growing on A and B |
| 09:05 | Token alert fires if no valid token | Auto | Alert silent |
| 15:40 | Capture stops | Auto | Last segment closed cleanly |
| 15:50 | `gp-merge.timer` merges, writes Parquet, compresses segments | Auto | Coverage report written, zero unexplained gaps |
| Evening | Download that day's participant OI CSV | Guru | File present in `research/cache/` |

## Expiry day (Nifty, Tuesday)

1. Start `gp-monitor` on Node C by 09:15 and open the dashboard.
2. At 14:00 publish the H1 prediction with a timestamp (forward test, expiry 2 only).
3. After 15:30 record the settlement price once NSE publishes it.

## When something breaks

| Symptom | Action |
|---|---|
| No token by 09:05 | Run `gp auth` again; capture picks up the new token without a restart |
| One node down | Leave it; the other node keeps capturing. Note the window for the coverage report |
| Capture crashed | systemd restarts it; on start it truncates the last segment at the last valid CRC |
| Clock offset > 5 ms | `sudo chronyc makestep`, then note it in the day's log |
| Disk > 80% | Compress old segments with zstd, move merged days off the node |
