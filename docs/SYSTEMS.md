# Systems: capture, merge, replay and monitor

Track V of [CONTRIBUTING.md](../CONTRIBUTING.md) built a redundant capture pipeline for the Upstox Market Data Feed V3: two nodes record every WebSocket frame, a merge turns both logs into one de-duplicated record, and a monitor serves the live (or replayed) GEX chain to the dashboard.

Live capture was dropped on 17 Oct 2026 because every NSE feed needs a broker account with KYC. Everything below was therefore built and measured against **`gp-mockfeed`**, a synthetic Nifty market spoken over the same V3 protobuf protocol. No real tick was ever captured, and no test uses real market data. What is real: the protocol code, the failure handling, the file formats and the timings on this hardware.

## Pipeline

```
 Upstox V3 (or gp-mockfeed)
   │ WebSocket, protobuf
   ├────────────────────────┐
 gp-capture (Node A)     gp-capture (Node B)      raw frames → GPL1 segments, GAP records
   │ rsync                  │
   └──────────┬─────────────┘
          gp-merge (Node C)                        decode → C1 events → de-duplicate
           ├─ Parquet  date=…/instrument=…/        (C1, for research)
           ├─ replay.gpr                           (sorted, memory-mapped)
           └─ coverage.json / coverage.md          (gaps, silences, lost windows)
          gp-monitor (Node C)                      live: tail both nodes' segments
           └─ /ws  C2 snapshot every second        replay: replay.gpr at 1×, N× or max
```

| Crate | Role |
|---|---|
| `gp-log` | GPL1 segment log: 32-byte record header, CRC32C, 64 MiB segments, 100 ms flush, crash recovery, zstd after merge, tailing |
| `gp-feed` | V3 protobuf decoder (`prost-build` + `protox`, golden-file tests), instrument master, strike-band selection, C1 event, synthetic market |
| `gp-capture` | Subscribe (index first, then the band), record before decoding, reconnect with jittered backoff, stall detection, token hot-reload, 09:05 token check, mock feed |
| `gp-merge` | Read both nodes, normalize, de-duplicate, write Parquet, the replay log and the coverage report; compress the node segments |
| `gp-replay` | GPR1 replay log, replay order, the `EventStream` trait shared with live, pacing, determinism digest |
| `gp-monitor` | Chain book, `gp_quant::analyze` per snapshot (C3), C2 snapshot with health, axum `/ws` and the dashboard page |

## Design decisions

- **Record first, decode later.** Capture writes each frame's raw bytes before looking at them. A decoder bug can be fixed and the day re-merged; a dropped frame cannot be recovered.
- **Two nodes, de-duplicated by content.** Both nodes subscribe to the same instruments. An update is identified by (instrument, `ltt_ms`, cumulative volume, OI, hash of the five depth levels). The merge keeps the earliest receive time and ORs the nodes into `src_mask`. A gap on one node costs nothing while the other is up. Data is lost only where both nodes are uncovered at once: inside a gap, or before a node's first frame or after its last, since a late start or a crash leaves no gap record. The coverage report lists those windows within the trading session and the span either node recorded (`gp-merge` exits 3 if there are any).
- **Gaps are explicit.** On a disconnect or stall capture writes `GAP_START`, and `GAP_END` on the first market-data frame after the resubscription. The `market_info` greeting does not close a gap: it arrives before the subscription takes effect (found by the chaos test, below). A restart without a gap record still shows as the node's longest silence in the coverage report.
- **Crash safety.** Records carry a CRC. A writer killed mid-record leaves a torn tail; on restart it is truncated at the last valid record and writing continues in a new segment (`SIGKILL` test, three kills). The merge and the live tail treat the torn bytes as the end of that segment, and the merge reports how many it dropped. A gap left open by the crash is closed by the restarted node's first market-data frame.
- **One stream interface.** Live tailing and replay both implement `EventStream`, so the monitor engine cannot tell them apart. Only the snapshot clock differs: wall time live, the stream's exchange time in replay, so a replay looks as it did on the day.
- **Replay determinism.** Events are ordered by (`ltt_ms`, `recv_ns`, node, sequence) with a stable sort. `gp-replay hash` and the CI determinism test produce identical SHA-256 digests across runs.

## Results

Measured on a 4-core Intel Xeon at 2.8 GHz with 15 GB RAM (a cloud VM, not the planned 4090 nodes), release build.

| Metric | Target | Measured | How |
|---|---|---|---|
| Data loss when one node fails | 0 frames (60 s network cut) | **0 frames lost** | `gp-merge` chaos test: two captures behind separately cuttable TCP proxies; node A cut for 60 s. The union of both logs has no missing frame and `lost` is empty. CI runs a short version (two 400 ms cuts, one per node): 0 failures in 120 runs under 4× parallel load |
| Capture throughput per node | ≥ 50k msg/s | **291k msg/s** at 782 B, **1.25M msg/s** at 57 B; 21k msg/s at 7 KB (disk-bound, 151 MB/s) | `cargo bench -p gp-capture --bench throughput`: real client path, a local WebSocket server pushing pre-encoded frames |
| Replay speed | ≥ 1M events/s | **11.0M events/s** through `EventStream`; 12.2M decoding C1 events; 27M for prices only | `cargo bench -p gp-replay --bench replay`, 2M-event, 432 MB log |
| Replay determinism | SHA-256 identical across runs | **Identical** | CI test plus `gp-replay hash` twice on a dry-run day |
| Feed-to-dashboard latency | p99 < 250 ms | **p99 156 ms** (staleness p99 145 ms + delivery p99 11 ms) | `gp-monitor` dry-run test over 60 s, below |
| Merge throughput | (none set) | **143k events/s** in: 1.56M events from two nodes, 143 MB of frames, in 10.9 s | `cargo bench -p gp-merge --bench merge` |
| Full-chain recompute | < 5 ms | 0.74 ms on the dev machine (an earlier run gave 2.4 ms) | `gp-quant` bench (Track G) |

### Latency

The dry-run test (`cargo test --release -p gp-monitor --test dry_run -- --ignored`) runs the mock feed on wall-clock exchange time, 50 frames per second. Two in-process captures write their logs with the production 100 ms flush. The monitor follows both logs live and a WebSocket client plays the dashboard. Over 60 snapshots:

| Stage | p50 | p99 |
|---|---|---|
| Staleness: newest index tick to the snapshot built from it | 118 ms | 145 ms |
| Delivery: snapshot to the WebSocket client | 2 ms | 11 ms |

Staleness is dominated by the log's 100 ms flush interval: the monitor reads what capture has flushed to disk. The 1-second snapshot cadence is not included, since a tick that arrives just after a snapshot waits for the next one. The C2 `lag_ms_*` fields do include that cadence wait, by their contract definition (exchange time to snapshot).

### Chaos drill

| Version | Cuts | Result |
|---|---|---|
| CI | Node A 400 ms, then node B 400 ms | Each node has exactly one gap, no lost window, union contiguous |
| `sixty_second_cut` (ignored) | Node A cut for 60 s | Passed in 74 s |
| `ten_alternating_cuts` (ignored) | Ten 3 s cuts, alternating nodes | Passed |

Under a loaded CPU the first version of the test failed about 1 run in 50. The cause was two real issues and one test issue:
- `GAP_END` was written on the `market_info` greeting, before data resumed, so a short window looked covered when it was not (fixed in capture).
- The test proxy could miss a connection accepted during a cut (fixed in the test).
- With a 500 ms stall timeout a starved process could time out spuriously. The test now uses 2 s; production uses 10 s.

## Dry run with the real binaries

The [runbook](../ops/RUNBOOK.md#dry-run-on-the-mock-feed) dry run was executed end to end on 17 Oct 2026:
- `gp-mockfeed --now`, two `gp-capture --now` nodes and `gp-monitor live`.
- Node B was stopped for 15 s and restarted; the monitor showed B down, then up.
- `gp-merge` exited 0 with no lost window and B's 15.1 s silence in `coverage.md`.
- Both nodes' segments were compressed.
- `gp-monitor replay` served the day again, and two `gp-replay hash` runs agreed.

The merged Parquet loaded with `research/validate_ticks.py` (`load_events`, `tick_chain_at`) with the C1 schema: fixed-size `Array(Float64, 5)` depth, `Date` expiry, `UInt8` `src_mask`.

## Not done

| Item | Why |
|---|---|
| Capture of a real session, expiry 1 and expiry 2 (V1.3, V1.7, V4.1, V4.3, V7.1, V7.2; `gp-capture` itself, V1.5, is built and tested on the mock feed) | No Upstox account (no KYC) |
| Upstox login and token sync (`gp auth`, V1.8) | Needs the account; capture already re-reads the token file on each reconnect and logs `TOKEN_RELOAD` |
| Node setup, chrony, LAN and systemd install (V0.1–V0.3, V1.12) | No capture nodes; units are in `ops/systemd/` |
| Live chaos test (V4.2 as written) | Simulated on the mock feed instead (above) |
| OI units | Whether V3 sends OI in units or contracts can only be checked on the live feed; `oi_in_units` in `[merge]` sets it |
