# Contracts between the tracks

These interfaces let Track G (Guru) and Track V (Vivian) work without waiting on each other.
Each side builds against the contract with its own fixtures. A change here needs a PR approved by both.

## C1: merged event schema

Producer: `gp-merge` (Vivian). Consumers: `gp-replay`, `research/validate_ticks.py` (Guru).

One row per de-duplicated feed update. The same fields, in the same order, are the replay record.

| Column | Arrow type | Notes |
|---|---|---|
| `instrument_key` | `utf8` | Upstox key, e.g. `NSE_FO\|43567`, `NSE_INDEX\|Nifty 50` |
| `underlying` | `utf8` | `NIFTY` (or `SENSEX`) |
| `kind` | `utf8` | `CE`, `PE`, `FUT` or `IDX` |
| `expiry` | `date32` | Null for `IDX` |
| `strike` | `float64` | Rupees. Null for `FUT` and `IDX` |
| `lot_size` | `int32` | From the instrument master on that date |
| `ltt_ms` | `int64` | Exchange last-trade time, Unix ms |
| `recv_ns` | `int64` | Earliest receive time across nodes, Unix ns (`CLOCK_REALTIME`) |
| `ltp` | `float64` | Last traded price |
| `ltq` | `int64` | Last traded quantity |
| `cum_volume` | `int64` | Cumulative day volume |
| `oi` | `int64` | Open interest **in contracts** (convert if the feed sends units) |
| `bid_px` | `fixed_size_list<float64, 5>` | L1 first. Missing levels are `0.0` |
| `bid_qty` | `fixed_size_list<int64, 5>` | |
| `ask_px` | `fixed_size_list<float64, 5>` | |
| `ask_qty` | `fixed_size_list<int64, 5>` | |
| `src_mask` | `uint8` | bit 0 = Node A, bit 1 = Node B |

**Layout:** `data/merged/date=YYYY-MM-DD/instrument=<key with | replaced by _>/part-0.parquet`, rows sorted by (`ltt_ms`, `recv_ns`).

**Fixture:** each side may generate synthetic C1 rows for tests. No real ticks are committed.
