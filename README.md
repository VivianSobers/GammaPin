# GammaPin

The first open, pre-registered test of whether gamma exposure (GEX) predicts Nifty expiry-day moves, on free public data.

About 59% of Indian index-options turnover is in same-day-expiry contracts, and traders rely on GEX charts whose claims nobody has tested. GammaPin:

- tests the gamma pinning claim (H1) with hypotheses fixed before looking at the data, comparing the US sign convention with India's participant-wise positioning,
- forward-tests it on expiry day with a public, timestamped prediction,
- runs on free public data only: NSE and RBI files, no broker account.

**Results:** H1 is not supported on the training set (66 expiries) or the test set (28 expiries). The write-up is [docs/RESULTS.md](docs/RESULTS.md). H2 (volatility regime) is not tested: it needs intraday history that no free source provides.

## Data

| Data | Source | Used for |
|---|---|---|
| F&O daily bhavcopy | NSE archives | Option chain at the eve close: price and OI per strike |
| Participant OI | NSE | S1 and S2 sign conventions |
| Index close | NSE `ind_close_all` | Settlement (the target) |
| 91-day T-bill cut-off | RBI press releases | Risk-free rate |

## Live capture (dropped)

The original plan also captured the live options market on two redundant nodes. Every live NSE feed needs a broker or vendor account with SEBI KYC (Aadhaar, PAN, bank details), so live capture was dropped on 17 Oct 2026 ([CONTRIBUTING.md](CONTRIBUTING.md#scope-change-17-oct-2026-no-live-capture)). The finished `gp-log` crate stays; the other capture crates are stubs. The planned design was:

```
            Upstox Market Data Feed V3 (WebSocket, Protobuf)
                 │ conn 1                      │ conn 2
        ┌────────▼────────┐          ┌─────────▼────────┐
        │ Node A (4090)   │          │ Node B (4090)    │
        │ gp-capture      │          │ gp-capture       │   raw frames → GPL1 log
        └────────┬────────┘          └─────────┬────────┘
                 └──────────────┬──────────────┘  (LAN, chrony-synced clocks)
                       ┌────────▼─────────────────────────┐
                       │ Node C (RTX 4500 Ada)            │
                       │ gp-merge → Parquet + replay log  │
                       │ gp-monitor (live engine + UI)    │
                       │ research/ (Python)               │
                       └──────────────────────────────────┘
```

## Repository

| Path | What it is |
|---|---|
| `crates/gp-quant` | Black-76, IV, gamma, GEX (Rust + PyO3) |
| `research/` | Data fetchers, H1 and H2, forward test (Python 3.12) |
| `forward/` | Published forward-test predictions and scores |
| `PREREGISTRATION.md` | Hypotheses and metrics, frozen before the test set is opened |
| `crates/gp-log` | GPL1 append-only, CRC-checked segment log (built for the dropped live capture) |
| `crates/gp-capture`, `gp-merge`, `gp-replay` | Stubs (live capture dropped) |
| `crates/gp-monitor` | Dashboard page on its mock feed; the live server is not built |
| `ops/` | Runbook; systemd units and chrony config for the dropped capture nodes |

## Quick start

```bash
cargo test --workspace                         # Rust (stable, 2024 edition)
cd crates/gp-quant && maturin develop          # Python bindings
cd research && pip install -r requirements.txt && pytest
```

See [ops/RUNBOOK.md](ops/RUNBOOK.md) and [forward/README.md](forward/README.md) for the expiry-day forward test and [CONTRIBUTING.md](CONTRIBUTING.md) for the plan and who owns what.

## Notes

- **Analytics, not trading advice.**
- Raw NSE files are never committed (`research/cache/` is ignored); the fetchers download them from NSE. Code and statistics are published.

## License

[MIT](LICENSE)
