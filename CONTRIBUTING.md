# Contributing to GammaPin

Two people, three machines, 14 days, two Nifty expiries. This file is the plan and the tracker. Update the status column in the same PR that finishes a task.

**Legend:** ✅ done · ❌ not done · 🚩 checkpoint (both people sign off before the next phase depends on it)

## Team and hardware

| | Guru | Vivian |
|---|---|---|
| GitHub | [@guru-bharadwaj20](https://github.com/guru-bharadwaj20) | TBD |
| Hardware | 1× RTX 4500 Ada | 2× RTX 4090 (two machines) |
| Nodes | **Node C**: merge, monitor, research | **Node A** and **Node B**: redundant capture |
| Role | Quant: `gp-quant` + PyO3, `research/`, H1, H2, dashboard UI, pre-registration | Rust systems: `gp-log`, `gp-capture`, `gp-merge`, `gp-replay`, monitor server, ops, chaos and performance tests |
| Upstox connection | none | conn 1 (Node A), conn 2 (Node B) |

Why this split: the two capture nodes sit on Vivian's machines, so he owns everything that touches the feed and the log format and can run the crash and chaos tests on his own hardware. Node C runs merge, the monitor and research, so Guru's quant work runs where the merged data lands. `gp-merge` is written by Vivian and deployed on Node C. The GPUs are only an optional speed-up for the 10,000-resample bootstrap.

**Shared, every market day:** the 08:45 `gp auth` login and capture check (alternate days, see [ops/RUNBOOK.md](ops/RUNBOOK.md)).

## Phase overview

| Phase | Name | Days | Owner | Checkpoint | Status |
|---|---|---|---|---|---|
| 0 | Setup | Before day 1 | Both | 🚩 CP0: machines synced, token works, CI green | ❌ |
| 1 | Capture foundation | 1–3 | Vivian | 🚩 CP1: both nodes capture a full session, crash test passes | ❌ |
| 2 | Pricing core | 2–3 | Guru | 🚩 CP2: IV round-trip property tests pass, bindings import | ❌ |
| 3 | Historical data | 1–5 | Guru | 🚩 CP3: ~50 expiries fetched with a coverage report | ❌ |
| 4 | Merge and replay | 4–6 | Vivian | 🚩 CP4: deterministic replay, SHA-256 identical in CI | ❌ |
| 5 | GEX and pre-registration | 4–6 | Guru | 🚩 CP5: `PREREGISTRATION.md` frozen and tagged | ❌ |
| 6 | Expiry 1 (live) | 7–9 | Both | 🚩 CP6: zero-loss chaos test, ticks vs candles validated | ❌ |
| 7 | Hardening and freeze | 10–13 | Both | 🚩 CP7: code frozen, test set run once | ❌ |
| 8 | Expiry 2 (live) | 14 | Both | 🚩 CP8: forward test published before settlement | ❌ |
| 9 | Release | After | Both | 🚩 CP9: public repo, write-up, demo replay | ❌ |

Start on a Wednesday so day 7 and day 14 fall on Nifty expiry Tuesdays (Sensex expires Thursdays). If an expiry Tuesday is an NSE holiday, expiry moves to the previous trading day: check the calendar in phase 0.

---

## Phase 0: Setup (before day 1)

| # | Task | Owner | Status |
|---|---|---|---|
| 0.1 | Repo scaffold: Cargo workspace, crate stubs, `research/`, `ops/`, CI | Guru | ✅ |
| 0.2 | `README.md` | Guru | ✅ |
| 0.3 | `CONTRIBUTING.md` (this plan) | Guru | ✅ |
| 0.4 | `PREREGISTRATION.md` draft (not frozen) | Guru | ✅ |
| 0.5 | `ops/RUNBOOK.md` first draft | Guru | ✅ |
| 0.6 | Upstox account, Plus plan (check price), API app created | Guru | ❌ |
| 0.7 | Ubuntu 24.04 on Node A and Node B | Vivian | ❌ |
| 0.8 | Ubuntu 24.04 on Node C | Guru | ❌ |
| 0.9 | chrony installed from `ops/chrony.conf` on all 3 nodes, offset < 5 ms | Both | ❌ |
| 0.10 | LAN between the nodes; Node A/B can rsync segments to Node C | Both | ❌ |
| 0.11 | NSE holiday calendar checked for the 14-day window | Guru | ❌ |
| 0.12 | `MarketDataFeedV3.proto` downloaded into `proto/` | Vivian | ❌ |
| 0.13 | Vivian added as collaborator, branch protection on `main` | Guru | ❌ |

**🚩 CP0:** all three nodes on chrony, one manual Upstox token obtained, CI green on `main`. ❌

## Phase 1: Capture foundation (days 1–3) · Vivian · Nodes A, B

| # | Task | Day | Status |
|---|---|---|---|
| 1.1 | `gp-log` writer: 32-byte GPL1 header, CRC32C, 64 MiB segments, 100 ms flush | 1 | ❌ |
| 1.2 | `gp-log` reader and crash recovery: truncate last segment at last valid CRC | 1 | ❌ |
| 1.3 | Basic capture running the same day (raw frames only) | 1 | ❌ |
| 1.4 | Instrument list from the instrument master: current + next Nifty weekly, ±10% of forward, ≤ 1,500 | 2 | ❌ |
| 1.5 | First Rust `gp-capture` live on Node A (tokio-tungstenite + rustls) | 2 | ❌ |
| 1.6 | Reconnect: exponential backoff with jitter (100 ms → 5 s), resubscribe, `GAP_START`/`GAP_END` | 3 | ❌ |
| 1.7 | Node B running a second, redundant connection | 3 | ❌ |
| 1.8 | `gp auth` CLI writes the token file; `notify` hot-reload, `TOKEN_RELOAD` record | 3 | ❌ |
| 1.9 | 09:05 alert when no valid token | 3 | ❌ |
| 1.10 | zstd compression of segments after close | 3 | ❌ |
| 1.11 | Crash test: kill capture mid-write, confirm recovery (`proptest` on truncation) | 3 | ❌ |
| 1.12 | systemd units from `ops/systemd/` installed on A and B | 3 | ❌ |

**🚩 CP1:** Node A and Node B each capture a full 09:00–15:40 session, and the crash test passes. ❌

## Phase 2: Pricing core (days 2–3) · Guru · Node C

| # | Task | Day | Status |
|---|---|---|---|
| 2.1 | Black-76 call and put prices (`libm::erfc` for N(x)) | 2 | ❌ |
| 2.2 | Gamma: Γ = e^{−rT} φ(d1) / (F σ √T) | 2 | ❌ |
| 2.3 | IV solver: bracketed Newton, Brent fallback, \|error\| < 1e-8, ≤ 50 iterations | 2 | ❌ |
| 2.4 | Property test: price(σ) → IV → σ within 1e-6, σ ∈ [0.01, 3], T ∈ [1 min, 30 days] | 2 | ❌ |
| 2.5 | Cross-check against `py_vollib` reference values | 2 | ❌ |
| 2.6 | Time to expiry: trading minutes to 15:30 / (252 × 375), floor 1 minute | 3 | ❌ |
| 2.7 | Forward: median of K + e^{rT}(C_mid − P_mid) over 3 nearest strikes | 3 | ❌ |
| 2.8 | Quote filters (spread, min mid, intrinsic, OTM only, σ range) | 3 | ❌ |
| 2.9 | PyO3 + numpy bindings, `maturin develop` works from `research/` | 3 | ❌ |
| 2.10 | `criterion` bench: full chain (≤ 1,500 contracts) < 5 ms on one core | 3 | ❌ |

**🚩 CP2:** property and cross-check tests pass in CI, and `import gp_quant` works in Python. ❌

## Phase 3: Historical data (days 1–5) · Guru · Node C

| # | Task | Day | Status |
|---|---|---|---|
| 3.1 | `participant_oi.py`: 12 months of `fao_participant_oi_DDMMYYYY.csv`, column names confirmed from a real file | 1 | ❌ |
| 3.2 | `fetch_expired.py`: start the long-running expired-data fetch (≤ 1 month per request) | 1 | ❌ |
| 3.3 | Coverage log per contract; keep only complete contracts | 2–5 | ❌ |
| 3.4 | RBI 91-day T-bill series for r | 2 | ❌ |
| 3.5 | Lot sizes and tick size from the instrument master and NSE contract specs | 2 | ❌ |
| 3.6 | Confirm NSE settlement method (last-30-minute average) from the contract specs | 2 | ❌ |
| 3.7 | Novelty check: SSRN, arXiv, GitHub for "Nifty gamma exposure" (1 hour) | 1 | ❌ |

**🚩 CP3:** about 50 expiries on disk with a coverage report, and participant OI loaded in DuckDB/Polars. ❌

## Phase 4: Merge and replay (days 4–6) · Vivian · runs on Node C

| # | Task | Day | Status |
|---|---|---|---|
| 4.1 | Protobuf decoder (`prost-build` on `proto/`) with golden-file tests | 4 | ❌ |
| 4.2 | Normalize to events: key, expiry, strike, type, ltt_ms, recv_ns, ltp, ltq, volume, OI, 5-level book, src_mask | 4 | ❌ |
| 4.3 | Content de-duplication: (instrument, ltt_ms, cum_volume, oi, hash(L1–L5)), keep earliest recv | 4 | ❌ |
| 4.4 | Parquet output partitioned by date and instrument | 5 | ❌ |
| 4.5 | Sorted replay log and per-node coverage report (gaps filled by the other node) | 5 | ❌ |
| 4.6 | `gp-replay`: order by (ltt_ms, recv_ns, node, seq); 1×, N×, max speed | 6 | ❌ |
| 4.7 | One stream trait shared by live and replay | 6 | ❌ |
| 4.8 | Determinism test in CI: same input, identical SHA-256 output | 6 | ❌ |
| 4.9 | Memory-mapped replay ≥ 1M events/s | 6 | ❌ |

**🚩 CP4:** one captured session merges cleanly and replays with an identical SHA-256 across runs in CI. ❌

## Phase 5: GEX and pre-registration (days 4–6) · Guru · Node C

| # | Task | Day | Status |
|---|---|---|---|
| 5.1 | GEX per strike (₹ per 1% move) in `gp-quant` | 4 | ❌ |
| 5.2 | Flip level on a grid of F ± 3% in 0.05% steps | 4 | ❌ |
| 5.3 | Sign conventions S0, S1, S2 from participant OI (day d−1) | 5 | ❌ |
| 5.4 | Max-pain, nearest-100 and F-at-14:00 baselines | 5 | ❌ |
| 5.5 | Fill every `TBD` in `PREREGISTRATION.md`, fix the 70/30 chronological split | 5 | ❌ |
| 5.6 | **Freeze:** commit and tag `prereg-v1` before touching the test set | 5 | ❌ |
| 5.7 | H1 pipeline (`h1_pinning.py`) on the training set only | 6 | ❌ |
| 5.8 | H2 pipeline (`h2_vol.py`) on the training set only | 6 | ❌ |
| 5.9 | Look-ahead guards and perturbation test (`hypothesis`) | 6 | ❌ |

**🚩 CP5:** `prereg-v1` tag exists on GitHub before any test-set expiry is loaded. Vivian reviews and signs off. ❌

## Phase 6: Expiry 1, live (days 7–9) · Both

| # | Task | Owner | Day | Status |
|---|---|---|---|---|
| 6.1 | Full redundant capture on A and B through expiry 1 | Vivian | 7 | ❌ |
| 6.2 | Live chaos test: cut one node's network for 60 s, merge shows 0 lost frames | Vivian | 7 | ❌ |
| 6.3 | Monitor running on replayed data | Guru | 7 | ❌ |
| 6.4 | `gp-monitor` axum server and WebSocket feed on the unified stream | Vivian | 8 | ❌ |
| 6.5 | Validate ticks vs minute candles on expiry 1 (`validate_ticks.py`) | Guru | 8 | ❌ |
| 6.6 | Internal live dry run of the monitor (Sensex expiry day) | Both | 9 | ❌ |
| 6.7 | Benchmarks: ≥ 50k msg/s per node, chain recompute < 5 ms | Vivian | 9 | ❌ |
| 6.8 | Fix issues found in the dry run | Guru | 9 | ❌ |

**🚩 CP6:** chaos test shows zero lost frames, and the tick-vs-candle validation report is committed. ❌

## Phase 7: Hardening and freeze (days 10–13) · Both

| # | Task | Owner | Day | Status |
|---|---|---|---|---|
| 7.1 | Feed-to-dashboard latency histograms, p99 < 250 ms | Vivian | 10–11 | ❌ |
| 7.2 | Finalize `ops/RUNBOOK.md` | Vivian | 10–11 | ❌ |
| 7.3 | Invite finance club users | Both | 10–11 | ❌ |
| 7.4 | Finalize the models (training set only) | Guru | 10–11 | ❌ |
| 7.5 | Dashboard: F, flip, GEX by strike (S0/S1), pin, regime, H1 accuracy, data health, banner | Guru | 10–13 | ❌ |
| 7.6 | **Code freeze**, tag `freeze-v1` | Both | 12 | ❌ |
| 7.7 | Run H1 and H2 on the test set, **once**, commit the outputs | Guru | 12 | ❌ |
| 7.8 | Full dry run on replay | Vivian | 13 | ❌ |

**🚩 CP7:** `freeze-v1` tag, and test-set results committed unchanged. ❌

## Phase 8: Expiry 2, live (day 14) · Both

| # | Task | Owner | Status |
|---|---|---|---|
| 8.1 | Redundant capture through expiry 2 | Vivian | ❌ |
| 8.2 | Live monitor with finance club users | Guru | ❌ |
| 8.3 | Publish the 14:00 prediction with a timestamp, before settlement | Guru | ❌ |
| 8.4 | Record the settlement and score the forward test | Guru | ❌ |

**🚩 CP8:** timestamped prediction is public before 15:30, and the scored result is committed. ❌

## Phase 9: Release (after day 14) · Both

| # | Task | Owner | Status |
|---|---|---|---|
| 9.1 | Results write-up with confidence intervals, including null results | Guru | ❌ |
| 9.2 | Demo replay of expiry 2 | Vivian | ❌ |
| 9.3 | Systems write-up: capture, merge, replay, chaos and benchmark numbers | Vivian | ❌ |
| 9.4 | Open-source release: code and statistics only, no raw ticks | Both | ❌ |

**🚩 CP9:** public repo with the write-up, the pre-registration and the demo replay. ❌

---

## Performance targets

| Metric | Target | Owner | Status |
|---|---|---|---|
| Data loss when one node fails | 0 frames (60 s network cut) | Vivian | ❌ |
| Capture throughput per node | ≥ 50k msg/s sustained | Vivian | ❌ |
| Full-chain recompute (≤ 1,500 contracts) | < 5 ms on one core | Guru | ❌ |
| Feed-to-dashboard latency | p99 < 250 ms | Vivian | ❌ |
| Replay speed | ≥ 1M events/s | Vivian | ❌ |
| Replay determinism | SHA-256 identical across runs | Vivian | ❌ |

## How we work

| Rule | Detail |
|---|---|
| Branches | `guru/<topic>` or `vivian/<topic>`, PR into `main`, no direct pushes |
| Review | The other person reviews every PR. Anything touching the GPL1 format or `gp-quant` maths needs both |
| CI | `cargo fmt --check`, `cargo clippy -D warnings`, `cargo test`, `pytest` must pass |
| Commits | Short imperative subject (`gp-log: add CRC recovery`), one logical change each |
| Tracker | Flip ❌ to ✅ in this file in the PR that completes the task |
| Data | Never commit raw ticks, Parquet or tokens (`.gitignore` covers them) |
| Pre-registration | After `prereg-v1`, changes go in its "Deviations" section with a date and reason |
| Test set | Opened once, on day 12, by Guru, with Vivian watching. No re-runs |

## Local setup

```bash
rustup show                                     # picks up rust-toolchain.toml
cargo test --workspace
python -m venv .venv && source .venv/bin/activate
pip install maturin -r research/requirements.txt
cd crates/gp-quant && maturin develop && cd ../..
cd research && pytest
cp config/gammapin.example.toml config/gammapin.toml   # set node name, paths
```
