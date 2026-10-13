# Contributing to GammaPin

Two people, three machines, 14 days, two Nifty expiries. This file is the plan and the tracker. Flip a status in the same commit that finishes the task.

**Legend:** ✅ done · ❌ not done · 🚩 checkpoint

## Team and hardware

| | Guru | Vivian |
|---|---|---|
| GitHub | [@guru-bharadwaj20](https://github.com/guru-bharadwaj20) | TBD |
| Hardware | 1× RTX 4500 Ada | 2× RTX 4090 (two machines) |
| Nodes | **Node C**: merge target, monitor host, research | **Node A** and **Node B**: redundant capture |
| Owns | `gp-quant` (+ PyO3), `research/`, dashboard page, `PREREGISTRATION.md`, `docs/` | `gp-log`, `gp-capture`, `gp-merge`, `gp-replay`, `gp-monitor` server, `ops/` |
| Track | **G** (quant and research) | **V** (systems and capture) |

## How the split works

The project runs as **two independent tracks**. Every phase belongs to one person, who works through it in one stretch. Nobody works one subtask and then waits for the other person's subtask.

1. **Contracts first.** The three interfaces between the tracks are fixed in [docs/CONTRACTS.md](docs/CONTRACTS.md) before day 1: the event schema (C1), the snapshot JSON (C2) and the `gp-quant` API (C3). Each side builds and tests against the contract with its own fixtures, never against the other person's unfinished code.
2. **Handoffs have slack.** Each handoff below is ready days before the receiver needs it, and the receiver has other work queued until then.
3. **A contract change** needs a PR that both approve. Everything else is reviewed by the other person but never blocks your next task.

### Handoffs

| # | What | From → To | Ready by | Needed by | Until then, the receiver uses |
|---|---|---|---|---|---|
| H1 | Contracts C1, C2, C3 | Guru → Vivian | Day 0 | Day 4 | n/a |
| H2 | `gp_quant::analyze` (C3) implemented | Guru → Vivian | Day 4 | Day 8 | Signature from C3 |
| H3 | Merged expiry-1 Parquet (C1) | Vivian → Guru | Day 7 evening | Day 10 | Synthetic C1 fixtures in tests |
| H4 | `gp-monitor` serving `/ws` (C2) | Vivian → Guru | Day 9 | Day 13 | `static/mock/snapshot.json` |
| H5 | Daily Upstox token | Guru (account holder) → Nodes A, B | 08:45 daily | 09:00 daily | Token sync built by Vivian in V1 |

## Timeline

Start on a Wednesday so day 7 and day 14 are Nifty expiry Tuesdays. See [docs/CALENDAR.md](docs/CALENDAR.md): if you start on Wed 14 Oct 2026, Tue 20 Oct is Dussehra, so expiry 1 moves to **Mon 19 Oct (day 6)**.

| Day | Market | Track G (Guru, Node C) | Track V (Vivian, Nodes A, B) |
|---|---|---|---|
| 0 | | G0 setup and contracts | V0 setup |
| 1 Wed | | G1 pricing core · G2 starts long downloads | V1 capture foundation |
| 2 Thu | Sensex expiry | G1 pricing core | V1 capture foundation |
| 3 Fri | | G1 → G3 GEX engine | V1 capture foundation |
| 4–5 | Weekend | G3 GEX engine · G2 finishes | V2 merge |
| 6 Mon | | G4 research pipelines | V3 replay |
| 7 Tue | ⚡ Nifty expiry 1 | G4 research pipelines | V4 expiry-1 capture and chaos test |
| 8 Wed | | G5 dashboard (mock data) | V5 live engine and monitor server |
| 9 Thu | Sensex expiry | G5 dashboard (mock data) | V5 live engine and monitor server |
| 10–11 | | G6 tick validation and model finalization | V6 hardening |
| 12 Sun | | G7 freeze, test set run once | V6 hardening, code freeze |
| 13 Mon | | G5.3 connect dashboard to `/ws` | V6 full dry run |
| 14 Tue | ⚡ Nifty expiry 2 | G7 live forward test | V7 capture |
| After | | G7 results write-up | V7 demo replay, systems write-up |

**Joint, but not blocking:** the code freeze tag `freeze-v1` (day 12), inviting finance club users (day 10–11), and the open-source release.

---

# Track G: Guru (quant and research, Node C)

## G0: Setup and contracts (day 0)

| # | Task | Status |
|---|---|---|
| G0.1 | Repo scaffold: Cargo workspace, crate stubs, `research/`, `ops/`, CI | ✅ |
| G0.2 | `README.md` | ✅ |
| G0.3 | `CONTRIBUTING.md`: two independent tracks | ✅ |
| G0.4 | `PREREGISTRATION.md` draft | ✅ |
| G0.5 | `ops/RUNBOOK.md` first draft | ✅ |
| G0.6 | Contract C1: merged event schema | ✅ |
| G0.7 | Contract C2: snapshot JSON and mock file | ✅ |
| G0.8 | Contract C3: `gp-quant` API used by `gp-monitor` | ✅ |
| G0.9 | NSE holiday calendar for the window | ✅ |
| G0.10 | Novelty check: SSRN, arXiv, GitHub for "Nifty gamma exposure" | ✅ |
| G0.11 | Upstox account, Plus plan (check price), API app | ❌ |
| G0.12 | Node C: Ubuntu 24.04, chrony from `ops/chrony.conf`, inbox for A/B segments | ❌ |
| G0.13 | Add Vivian as collaborator, branch protection on `main` | ❌ |

**🚩 CP-G0:** contracts merged, one manual Upstox token obtained. ❌

## G1: Pricing core (days 1–3) · `crates/gp-quant`

| # | Task | Status |
|---|---|---|
| G1.1 | Normal CDF/PDF (`libm::erfc`) and Black-76 call/put prices | ✅ |
| G1.2 | Gamma and vega | ✅ |
| G1.3 | IV solver: bracketed Newton, Brent fallback, \|price error\| < 1e-8, ≤ 50 iterations | ✅ |
| G1.4 | Property test: price(σ) → IV → σ within 1e-6, σ ∈ [0.01, 3], T ∈ [1 min, 30 days] | ✅ |
| G1.5 | Cross-check against `py_vollib` reference values | ✅ |
| G1.6 | Time to expiry: trading minutes to 15:30 / (252 × 375), floor 1 minute | ✅ |
| G1.7 | Forward: median of K + e^{rT}(C_mid − P_mid) over the 3 strikes nearest the money | ❌ |
| G1.8 | Quote filters: spread, min mid, discounted intrinsic, OTM only, σ range | ❌ |
| G1.9 | PyO3 + numpy bindings (`maturin develop`) | ❌ |

**🚩 CP-G1:** property and cross-check tests green in CI, `import gp_quant` works. ❌

## G2: Historical data (days 1–5, downloads run in the background) · `research/`

| # | Task | Status |
|---|---|---|
| G2.1 | `participant_oi.py`: download and parse NSE participant OI (columns confirmed from a real file) | ❌ |
| G2.2 | Download 12 months of participant OI | ❌ |
| G2.3 | `instruments.py`: Nifty option contracts, lot size, tick size from the Upstox instrument master | ❌ |
| G2.4 | `fetch_expired.py`: expired option and index candles, ≤ 1 month per request, coverage log | ❌ |
| G2.5 | Run the expired fetch (needs Plus plan and a token) | ❌ |
| G2.6 | `rates.py`: RBI 91-day T-bill loader | ❌ |
| G2.7 | Download the T-bill series from RBI DBIE into `research/cache/` | ❌ |
| G2.8 | Confirm the settlement method from NSE's official contract specs | ❌ |

**🚩 CP-G2:** expiries on disk with a coverage report; participant OI and rates load. ❌

## G3: GEX engine (days 3–5) · `crates/gp-quant`

| # | Task | Status |
|---|---|---|
| G3.1 | Chain smile: IV per strike from the OTM quote, interpolated across rejected strikes | ❌ |
| G3.2 | GEX per strike (₹ per 1% move) | ❌ |
| G3.3 | Flip level on a grid of F ± 3% in 0.05% steps | ❌ |
| G3.4 | Sign conventions S0, S1, S2 from participant OI | ❌ |
| G3.5 | `analyze()`: forward, GEX, flip, pin and regime per convention (C3) | ❌ |
| G3.6 | Python bindings for `analyze()` and the S1/S2 weights | ❌ |
| G3.7 | `criterion` bench: 1,500-contract chain < 5 ms on one core | ❌ |

**🚩 CP-G3:** `analyze()` matches C3 and the bench is under 5 ms (handoff H2). ❌

## G4: Research pipelines (days 6–7, training data only) · `research/`

| # | Task | Status |
|---|---|---|
| G4.1 | Baselines: max pain, nearest multiple of 100, F at 14:00 | ❌ |
| G4.2 | Statistics: paired Wilcoxon, block-bootstrap CI (10,000 resamples) | ❌ |
| G4.3 | H1 pipeline (`h1_pinning.py`) | ❌ |
| G4.4 | H2 pipeline (`h2_vol.py`): HAR controls, Newey-West (6 lags), out-of-sample R² | ❌ |
| G4.5 | Look-ahead guards and perturbation test | ❌ |
| G4.6 | Fill every `TBD` in `PREREGISTRATION.md` (split dates, block length, success criteria) | ❌ |
| G4.7 | **Freeze:** commit and tag `prereg-v1` before touching the test set | ❌ |
| G4.8 | Run H1 and H2 on the training set | ❌ |

**🚩 CP-G4:** `prereg-v1` tag pushed before any test-set expiry is loaded. ❌

## G5: Dashboard (days 8–9, then day 13) · `crates/gp-monitor/static/`

| # | Task | Status |
|---|---|---|
| G5.1 | Page shell: banner, WebSocket client, mock mode from `static/mock/snapshot.json` | ❌ |
| G5.2 | Panels: F and flip, GEX by strike (S0/S1) with uPlot, pin, regime, H1 accuracy, data health | ❌ |
| G5.3 | Point the page at Vivian's live `/ws` (after handoff H4) | ❌ |

**🚩 CP-G5:** dashboard renders every C2 field from the mock feed. ❌

## G6: Validation and model finalization (days 10–11)

| # | Task | Status |
|---|---|---|
| G6.1 | `validate_ticks.py`: tick-derived vs candle-derived GEX, flip and pin | ❌ |
| G6.2 | Run the validation on expiry 1 (after handoff H3), commit the report | ❌ |
| G6.3 | Finalize the models (training set only) | ❌ |

**🚩 CP-G6:** validation report committed. ❌

## G7: Test set, forward test, write-up (day 12 onward)

| # | Task | Status |
|---|---|---|
| G7.1 | Run H1 and H2 on the test set, **once**, commit the outputs (day 12) | ❌ |
| G7.2 | `forward_test.py`: write the 14:00 prediction with a SHA-256, and score it after settlement | ❌ |
| G7.3 | Publish the 14:00 prediction on expiry 2, before settlement | ❌ |
| G7.4 | Score the forward test against the settlement | ❌ |
| G7.5 | Results write-up with confidence intervals, including null results | ❌ |

**🚩 CP-G7:** test-set results and the scored forward test committed. ❌

---

# Track V: Vivian (systems and capture, Nodes A and B)

## V0: Setup (day 0)

| # | Task | Status |
|---|---|---|
| V0.1 | Ubuntu 24.04 on Node A and Node B | ❌ |
| V0.2 | chrony from `ops/chrony.conf` on A and B, offset < 5 ms | ❌ |
| V0.3 | LAN to Node C; A and B can rsync segments into Node C's inbox | ❌ |
| V0.4 | `MarketDataFeedV3.proto` downloaded into `proto/` | ❌ |

**🚩 CP-V0:** both nodes synced and able to reach Node C. ❌

## V1: Capture foundation (days 1–3) · `gp-log`, `gp-capture`

| # | Task | Status |
|---|---|---|
| V1.1 | `gp-log` writer: 32-byte GPL1 header, CRC32C, 64 MiB segments, 100 ms flush | ❌ |
| V1.2 | `gp-log` reader and crash recovery: truncate at the last valid CRC | ❌ |
| V1.3 | Basic capture running the same day (raw frames only) | ❌ |
| V1.4 | Instrument list from the instrument master: current + next Nifty weekly, ±10% of forward, ≤ 1,500 | ❌ |
| V1.5 | `gp-capture` live on Node A (tokio-tungstenite + rustls) | ❌ |
| V1.6 | Reconnect: backoff with jitter (100 ms → 5 s), resubscribe, `GAP_START`/`GAP_END` | ❌ |
| V1.7 | Node B running the second connection | ❌ |
| V1.8 | `gp auth` CLI and token sync from Node C to A/B; `notify` hot-reload, `TOKEN_RELOAD` record | ❌ |
| V1.9 | 09:05 alert when no valid token | ❌ |
| V1.10 | zstd compression of segments after close | ❌ |
| V1.11 | Crash test: kill capture mid-write, confirm recovery | ❌ |
| V1.12 | systemd units from `ops/systemd/` installed on A and B | ❌ |

**🚩 CP-V1:** both nodes capture a full 09:00–15:40 session, and the crash test passes. ❌

## V2: Merge (days 4–5) · `gp-merge`, deployed on Node C

| # | Task | Status |
|---|---|---|
| V2.1 | Protobuf decoder (`prost-build`) with golden-file tests | ❌ |
| V2.2 | Normalize records to the C1 event schema | ❌ |
| V2.3 | Content de-duplication: (instrument, ltt_ms, cum_volume, oi, hash(L1–L5)), earliest recv, `src_mask` | ❌ |
| V2.4 | Parquet output partitioned by date and instrument | ❌ |
| V2.5 | Sorted replay log and per-node coverage report | ❌ |

**🚩 CP-V2:** one captured session merges into C1 Parquet with a coverage report. ❌

## V3: Replay (day 6) · `gp-replay`

| # | Task | Status |
|---|---|---|
| V3.1 | Order by (ltt_ms, recv_ns, node, seq); 1×, N×, max speed | ❌ |
| V3.2 | One stream trait shared by live and replay | ❌ |
| V3.3 | Determinism test in CI: identical SHA-256 across runs | ❌ |
| V3.4 | Memory-mapped replay ≥ 1M events/s | ❌ |

**🚩 CP-V3:** replay determinism test green in CI. ❌

## V4: Expiry 1 (day 7) · Nodes A, B

| # | Task | Status |
|---|---|---|
| V4.1 | Full redundant capture through expiry 1 | ❌ |
| V4.2 | Live chaos test: cut one node's network for 60 s, merge shows 0 lost frames | ❌ |
| V4.3 | Merge expiry 1 and hand the Parquet to Guru (handoff H3) | ❌ |

**🚩 CP-V4:** chaos test passes with zero lost frames. ❌

## V5: Live engine and monitor server (days 8–9) · `gp-monitor`

| # | Task | Status |
|---|---|---|
| V5.1 | Live engine on the unified stream: build the chain per expiry, call `gp_quant::analyze` (C3) | ❌ |
| V5.2 | axum server: `/ws` pushes a C2 snapshot every second, serves `static/` | ❌ |
| V5.3 | Data health in the snapshot: gaps, lag, node status | ❌ |
| V5.4 | Benchmarks: ≥ 50k msg/s capture per node | ❌ |

**🚩 CP-V5:** monitor serves C2 snapshots from replayed expiry 1 (handoff H4). ❌

## V6: Hardening (days 10–13)

| # | Task | Status |
|---|---|---|
| V6.1 | Feed-to-dashboard latency histograms, p99 < 250 ms | ❌ |
| V6.2 | Finalize `ops/RUNBOOK.md` | ❌ |
| V6.3 | Code freeze, tag `freeze-v1` (day 12) | ❌ |
| V6.4 | Full dry run on replay (day 13) | ❌ |

**🚩 CP-V6:** `freeze-v1` tagged, dry run clean. ❌

## V7: Expiry 2 and release (day 14 onward)

| # | Task | Status |
|---|---|---|
| V7.1 | Redundant capture through expiry 2, monitor up for users | ❌ |
| V7.2 | Demo replay of expiry 2 | ❌ |
| V7.3 | Systems write-up: capture, merge, replay, chaos and benchmark numbers | ❌ |

**🚩 CP-V7:** demo replay published, no raw ticks in the repo. ❌

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

## Known risks

| Risk | Plan |
|---|---|
| Upstox's Expiries endpoint lists only about **6 months** of past expiries (~26 weeklies, not ~50) | Fetch as early as possible; state the real sample size in the pre-registration |
| Expiry 1 falls on a holiday | Expiry moves to the previous trading day ([docs/CALENDAR.md](docs/CALENDAR.md)) |
| Participant OI covers all index options and is not per strike | S1/S2 spread it evenly across strikes; stated in the pre-registration |
| Intraday OI updates in batches | Treat OI as a step function, record arrival times |
| NSE data licensing | Publish code and statistics only, never raw ticks |

## How we work

| Rule | Detail |
|---|---|
| Branches | `guru/<topic>` or `vivian/<topic>`, PR into `main` |
| Review | The other person reviews, but never blocks your next task. Contract changes need both approvals |
| CI | `cargo fmt --check`, `cargo clippy -D warnings`, `cargo test`, `pytest` must pass |
| Commits | One subtask per commit, subject starts with its ID: `G1.3: add IV solver` |
| Tracker | Flip ❌ to ✅ in the commit that completes the task |
| Data | Never commit raw ticks, Parquet or tokens (`.gitignore` covers them) |
| Pre-registration | After `prereg-v1`, changes go in its "Deviations" section with a date and reason |
| Test set | Opened once, on day 12. No re-runs |

## Local setup

```bash
rustup show                                     # picks up rust-toolchain.toml
cargo test --workspace
python -m venv .venv && source .venv/bin/activate
pip install maturin -r research/requirements.txt
pip install ./crates/gp-quant                   # or: cd crates/gp-quant && maturin develop
cd research && pytest
cp config/gammapin.example.toml config/gammapin.toml   # set node name, paths
```
