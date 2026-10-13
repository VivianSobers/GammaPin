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

## C2: monitor snapshot (WebSocket JSON)

Producer: `gp-monitor` server (Vivian). Consumer: `static/index.html` (Guru).

`GET /ws` upgrades to a WebSocket that sends one JSON text message per second. Example: [`crates/gp-monitor/static/mock/snapshot.json`](../crates/gp-monitor/static/mock/snapshot.json).

| Field | Type | Notes |
|---|---|---|
| `v` | int | Schema version, currently `1` |
| `ts_ms` | int | Snapshot time, Unix ms |
| `source` | string | `"live"` or `"replay"` |
| `underlying` | string | `"NIFTY"` |
| `expiry` | string | `YYYY-MM-DD` of the chain shown |
| `spot` | number | Index LTP |
| `forward` | number | F from `gp_quant::analyze` |
| `t_years` | number | Time to expiry used |
| `conventions` | object | Keyed by `"S0"`, `"S1"` (and optionally `"S2"`), see below |
| `strikes` | number[] | Strikes in ascending order, shared by every `gex` array |
| `h1_accuracy` | object or null | Keyed by convention: `{ "mae": number, "n": int }` from training-set H1 |
| `health` | object | See below |

Each `conventions.<S>` object:

| Field | Type | Notes |
|---|---|---|
| `gex` | number[] | ₹ per 1% move, aligned with `strikes` |
| `net_gex` | number | Sum of `gex` |
| `flip` | number or null | Null when there is no sign change within F ± 3% |
| `pin` | number or null | Strike with the largest positive GEX, null when none is positive |
| `regime` | string | `"long_gamma"` when `net_gex > 0`, else `"short_gamma"` |

`health`:

| Field | Type | Notes |
|---|---|---|
| `lag_ms_p50` | number | Exchange `ltt_ms` to snapshot, last 60 s |
| `lag_ms_p99` | number | |
| `gaps_today` | int | `GAP_START` records today across nodes |
| `nodes` | object | `{ "A": "up" \| "down", "B": ... }` |
| `quotes_used` | int | Quotes that passed the filters |
| `quotes_total` | int | Quotes in the chain |

The page always shows "Analytics, not trading advice." regardless of the payload.

## C3: `gp-quant` API

Producer: `gp-quant` (Guru). Consumers: `gp-monitor` live engine (Vivian), `research/` via the Python bindings (Guru).

The types live in [`crates/gp-quant/src/chain.rs`](../crates/gp-quant/src/chain.rs). The signatures are fixed. `analyze` is implemented (G3.5), so `gp-monitor` can call it directly.

```rust
pub struct Quote { pub bid: f64, pub ask: f64, pub oi: f64 }      // oi in contracts; 0.0 when absent
pub struct StrikeRow { pub strike: f64, pub call: Quote, pub put: Quote }

pub struct ChainInput<'a> {
    pub rows: &'a [StrikeRow],   // one expiry, strikes ascending
    pub r: f64,                  // RBI 91-day T-bill, decimal
    pub t_years: f64,            // from gp_quant::time
    pub lot_size: f64,
    pub filter: FilterConfig,    // FilterConfig::LIVE for ticks, ::CANDLES for minute closes
}

pub struct Weights { pub w_c: f64, pub w_p: f64 }                 // Weights::S0 = { +1, -1 }

pub struct ConventionResult {
    pub gex: Vec<f64>,       // ₹ per 1% move, aligned with rows
    pub net_gex: f64,
    pub flip: Option<f64>,
    pub pin: Option<f64>,
}

pub struct Analysis {
    pub forward: f64,
    pub iv: Vec<f64>,        // per row; NaN where no usable quote
    pub gamma: Vec<f64>,
    pub quotes_used: usize,
    pub quotes_total: usize,
    pub conventions: Vec<ConventionResult>,   // same order as `weights`
}

pub fn analyze(input: &ChainInput, weights: &[Weights]) -> Result<Analysis, QuantError>;
```

- `gp-monitor` builds `rows` from the latest C1 event per instrument, calls `analyze` once per second per expiry, and maps the result to C2. `regime` in C2 is the sign of `net_gex`.
- When there is a quote, its mid is (bid + ask) / 2. In candle mode, pass `bid = ask = close`.
- `analyze` never panics on bad quotes. It drops them and counts them in `quotes_used`.
