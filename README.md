# GammaPin

An open tick-capture and replay system for NSE options, and the first open, pre-registered test of whether gamma exposure (GEX) predicts Nifty expiry-day moves.

About 59% of Indian index-options turnover is in same-day-expiry contracts, and traders rely on GEX charts whose claims nobody has tested. GammaPin:

- captures the live Nifty options market on two redundant nodes without losing data,
- replays any session deterministically,
- tests the gamma pinning (H1) and volatility regime (H2) claims with hypotheses fixed before looking at the data, comparing the US sign convention with India's participant-wise positioning,
- forward-tests live on expiry day with a public, timestamped prediction.

## Architecture

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
| `crates/gp-log` | GPL1 append-only, CRC-checked segment log |
| `crates/gp-capture` | Upstox V3 WebSocket capture |
| `crates/gp-merge` | Normalize, de-duplicate, write Parquet and replay log |
| `crates/gp-replay` | Deterministic replay, same interface as live |
| `crates/gp-quant` | Black-76, IV, gamma, GEX (Rust + PyO3) |
| `crates/gp-monitor` | Live engine and one-page dashboard |
| `research/` | Data fetchers, H1 and H2 (Python 3.12) |
| `ops/` | systemd units, chrony config, runbook |
| `PREREGISTRATION.md` | Hypotheses and metrics, frozen before the test set is opened |

## Quick start

```bash
cargo test --workspace                         # Rust (stable, 2024 edition)
cd crates/gp-quant && maturin develop          # Python bindings
cd research && pip install -r requirements.txt && pytest
```

Nodes run Ubuntu 24.04. See [ops/RUNBOOK.md](ops/RUNBOOK.md) for daily operation and [CONTRIBUTING.md](CONTRIBUTING.md) for the plan and who owns what.

## Notes

- **Analytics, not trading advice.**
- Raw tick data is never published (NSE licensing). Code and statistics are.

## License

[MIT](LICENSE)
