"""Compare GEX and flip levels from live ticks with those from minute candles on captured expiries.

The historical sample is minute candles, so H1 and H2 rest on candles approximating what the
tick feed would have shown. On expiries captured live, this script builds both chains at the
same instants and reports how far apart the forward, flip, pin and GEX profile are.

Tick side: C1 merged events (docs/CONTRACTS.md), `data/merged/date=YYYY-MM-DD/instrument=*/part-0.parquet`.
As of IST time `ts`, the last event per (strike, kind) with `ltt_ms <= ts` gives the L1 bid, ask
and OI, analysed in LIVE mode. Candle side: `gpdata.analyze_at` on the expired-options parquet.
"""

from __future__ import annotations

import argparse
import datetime as dt
from pathlib import Path

import numpy as np
import polars as pl

import gp_quant as gp
import gpdata

ROOT = Path(__file__).resolve().parent.parent
MERGED = ROOT / "data" / "merged"
OUTPUT = Path(__file__).resolve().parent / "output"
IST = dt.timezone(dt.timedelta(hours=5, minutes=30))
DECISION = dt.time(14, 0)


def load_events(merged_dir: Path, day: dt.date) -> pl.DataFrame:
    """Every C1 event for one date, sorted by (ltt_ms, recv_ns)."""
    files = sorted((merged_dir / f"date={day.isoformat()}").glob("instrument=*/part-0.parquet"))
    if not files:
        raise FileNotFoundError(f"no C1 partitions under {merged_dir / f'date={day.isoformat()}'}")
    return pl.concat([pl.read_parquet(f) for f in files], how="vertical_relaxed").sort(["ltt_ms", "recv_ns"])


def ist_ms(ts: dt.datetime) -> int:
    """Unix ms of a naive IST datetime."""
    return int(ts.replace(tzinfo=IST).timestamp() * 1000)


def _level1(col: str, dtype: pl.DataType) -> pl.Expr:
    e = pl.col(col)
    return (e.arr.first() if isinstance(dtype, pl.Array) else e.list.first()).fill_null(0.0)


def tick_chain_at(events: pl.DataFrame, expiry: dt.date, ts: dt.datetime) -> dict:
    """One row per strike as of exchange time `ts` (naive IST): last L1 bid, ask and OI per side."""
    opts = events.filter(
        pl.col("kind").is_in(["CE", "PE"]) & (pl.col("expiry") == expiry) & (pl.col("ltt_ms") <= ist_ms(ts))
    )
    last = opts.group_by(["strike", "kind"], maintain_order=True).last().with_columns(
        _level1("bid_px", opts.schema["bid_px"]).alias("bid"),
        _level1("ask_px", opts.schema["ask_px"]).alias("ask"),
        pl.col("oi").cast(pl.Float64),
    )

    def side(kind: str, prefix: str) -> pl.DataFrame:
        return last.filter(pl.col("kind") == kind).select(
            "strike",
            pl.col("bid").alias(f"{prefix}_bid"),
            pl.col("ask").alias(f"{prefix}_ask"),
            pl.col("oi").alias(f"{prefix}_oi"),
        )

    chain = side("CE", "call").join(side("PE", "put"), on="strike", how="full", coalesce=True).fill_null(0.0).sort("strike")
    out = {c: chain[c].to_numpy().astype(np.float64) for c in chain.columns}
    out["lot"] = float(opts["lot_size"][0]) if opts.height else float("nan")
    return out


def analyze_ticks_at(
    events: pl.DataFrame,
    ts: dt.datetime,
    expiry: dt.date,
    r: float,
    weights: list[tuple[float, float]],
    trading_days: list[dt.date],
) -> dict | None:
    """gp_quant.analyze in LIVE mode on the tick chain as of `ts`. None when no forward or smile exists."""
    ch = tick_chain_at(events, expiry, ts)
    if len(ch["strike"]) == 0:
        return None
    t = gpdata.t_years_at(ts, expiry, trading_days)
    try:
        out = gp.analyze(
            ch["strike"], ch["call_bid"], ch["call_ask"], ch["call_oi"],
            ch["put_bid"], ch["put_ask"], ch["put_oi"], r, t, ch["lot"], weights, candles=False,
        )
    except ValueError:
        return None
    out["strikes"] = ch["strike"]
    return out


def gex_agreement(k_tick: np.ndarray, g_tick: np.ndarray, k_candle: np.ndarray, g_candle: np.ndarray) -> dict:
    """Pearson correlation and relative L1 of the two GEX vectors on their common strikes."""
    common, it, ic = np.intersect1d(k_tick, k_candle, return_indices=True)
    a, b = np.asarray(g_tick)[it], np.asarray(g_candle)[ic]
    ok = np.isfinite(a) & np.isfinite(b)
    a, b = a[ok], b[ok]
    corr = float(np.corrcoef(a, b)[0, 1]) if len(a) >= 2 and a.std() > 0 and b.std() > 0 else None
    denom = float(np.abs(a).sum())
    rel_l1 = float(np.abs(a - b).sum() / denom) if denom > 0 else None
    return {"n_common": int(len(a)), "gex_corr": corr, "gex_rel_l1": rel_l1}


def _diff(a: float | None, b: float | None) -> float | None:
    return None if a is None or b is None else float(a) - float(b)


def compare(tick: dict | None, candle: dict | None, names: list[str]) -> list[dict]:
    """One row per convention comparing the tick and candle analyses at one instant."""
    rows = []
    for i, name in enumerate(names):
        row: dict = {"convention": name, "tick_ok": tick is not None, "candle_ok": candle is not None}
        if tick is not None:
            row |= {"quotes_used_tick": tick["quotes_used"], "quotes_total_tick": tick["quotes_total"]}
        if candle is not None:
            row |= {"quotes_used_candle": candle["quotes_used"], "quotes_total_candle": candle["quotes_total"]}
        if tick is None or candle is None:
            rows.append(row)
            continue
        ct, cc = tick["conventions"][i], candle["conventions"][i]
        pin_t, pin_c = ct["pin"], cc["pin"]
        row |= {
            "forward_tick": tick["forward"],
            "forward_candle": candle["forward"],
            "forward_diff": tick["forward"] - candle["forward"],
            "flip_tick": ct["flip"],
            "flip_candle": cc["flip"],
            "flip_diff": _diff(ct["flip"], cc["flip"]),
            "pin_tick": pin_t,
            "pin_candle": pin_c,
            "pin_equal": pin_t == pin_c,
            "pin_dist": None if pin_t is None or pin_c is None else abs(pin_t - pin_c),
            "net_gex_tick": ct["net_gex"],
            "net_gex_candle": cc["net_gex"],
            "net_gex_ratio": cc["net_gex"] / ct["net_gex"] if ct["net_gex"] != 0 else None,
        }
        row |= gex_agreement(tick["strikes"], ct["gex"], candle["strikes"], cc["gex"])
        rows.append(row)
    return rows


def time_grid(day: dt.date, start: dt.time, end: dt.time, step_min: int) -> list[dt.datetime]:
    t, stop, out = dt.datetime.combine(day, start), dt.datetime.combine(day, end), []
    while t <= stop:
        out.append(t)
        t += dt.timedelta(minutes=step_min)
    return out


def weekdays(start: dt.date, end: dt.date, holidays: set[dt.date] = frozenset()) -> list[dt.date]:
    """Weekdays in [start, end] that are not holidays: the trading calendar for t_years_at."""
    n = (end - start).days
    days = (start + dt.timedelta(days=i) for i in range(n + 1))
    return [d for d in days if d.weekday() < 5 and d not in holidays]


COLUMNS = [
    "ts", "convention", "tick_ok", "candle_ok",
    "forward_tick", "forward_candle", "forward_diff",
    "flip_tick", "flip_candle", "flip_diff",
    "pin_tick", "pin_candle", "pin_equal", "pin_dist",
    "n_common", "gex_corr", "gex_rel_l1",
    "net_gex_tick", "net_gex_candle", "net_gex_ratio",
    "quotes_used_tick", "quotes_total_tick", "quotes_used_candle", "quotes_total_candle",
]


def validate(
    events: pl.DataFrame,
    opts: pl.DataFrame,
    expiry: dt.date,
    times: list[dt.datetime],
    r: float,
    weights: dict[str, tuple[float, float]],
    trading_days: list[dt.date],
) -> pl.DataFrame:
    """Tick-vs-candle comparison at every time in `times`, one row per (time, convention)."""
    names, w = list(weights), list(weights.values())
    rows = []
    for ts in times:
        tick = analyze_ticks_at(events, ts, expiry, r, w, trading_days)
        candle = gpdata.analyze_at(opts, ts, expiry, r, w, trading_days)
        rows += [{"ts": ts} | row for row in compare(tick, candle, names)]
    df = pl.DataFrame(rows, infer_schema_length=None)
    return df.select([pl.col(c) if c in df.columns else pl.lit(None).alias(c) for c in COLUMNS])


def _fmt(x: float | None, nd: int = 2) -> str:
    return "n/a" if x is None or (isinstance(x, float) and np.isnan(x)) else f"{x:,.{nd}f}"


def summarize(df: pl.DataFrame, day: dt.date, expiry: dt.date, decision: dt.time = DECISION) -> str:
    """Markdown report: medians and worst cases per convention, the decision-time row highlighted."""
    lines = [
        f"# Tick vs candle validation, {day.isoformat()} (expiry {expiry.isoformat()})",
        "",
        "Tick chain: last C1 event per contract with `ltt_ms <= ts`, LIVE filters. "
        "Candle chain: `gpdata.analyze_at`, CANDLES filters. Differences are tick minus candle; "
        "net GEX ratio is candle / tick; relative L1 is sum |g_tick - g_candle| / sum |g_tick| on common strikes.",
        "",
        "## Medians",
        "",
        "| Convention | Rows (both ok / all) | median abs dF | median abs d flip | pin agreement | median pin dist "
        "| median corr | median rel L1 | median net ratio |",
        "|---|---|---|---|---|---|---|---|---|",
    ]
    worst = []
    for name in df["convention"].unique(maintain_order=True):
        d = df.filter(pl.col("convention") == name)
        ok = d.filter(pl.col("tick_ok") & pl.col("candle_ok"))

        def med(expr: pl.Expr) -> float | None:
            return ok.select(expr.median()).item() if ok.height else None

        pins = ok["pin_equal"].drop_nulls()
        agree = f"{pins.sum()}/{pins.len()}" if pins.len() else "n/a"
        lines.append(
            f"| {name} | {ok.height}/{d.height} | {_fmt(med(pl.col('forward_diff').abs()))} "
            f"| {_fmt(med(pl.col('flip_diff').abs()))} | {agree} | {_fmt(med(pl.col('pin_dist')), 0)} "
            f"| {_fmt(med(pl.col('gex_corr')), 4)} | {_fmt(med(pl.col('gex_rel_l1')), 4)} "
            f"| {_fmt(med(pl.col('net_gex_ratio')), 4)} |"
        )
        for label, expr, desc in (
            ("lowest GEX corr", pl.col("gex_corr"), False),
            ("highest rel L1", pl.col("gex_rel_l1"), True),
            ("largest abs dF", pl.col("forward_diff").abs(), True),
            ("largest abs d flip", pl.col("flip_diff").abs(), True),
        ):
            w = ok.with_columns(expr.alias("_v")).drop_nulls("_v").sort("_v", descending=desc)
            if w.height:
                worst.append(f"| {name} | {label} | {w['ts'][0]:%H:%M} | {_fmt(w['_v'][0], 4)} |")
        failed = d.filter(~(pl.col("tick_ok") & pl.col("candle_ok")))
        for ts, t_ok, c_ok in failed.select("ts", "tick_ok", "candle_ok").iter_rows():
            side = " and ".join(s for s, good in (("tick", t_ok), ("candle", c_ok)) if not good)
            worst.append(f"| {name} | no analysis ({side}) | {ts:%H:%M} | |")

    lines += ["", "## Worst cases", "", "| Convention | Metric | Time | Value |", "|---|---|---|---|", *worst]
    at = df.filter(pl.col("ts").dt.time() == decision)
    lines += ["", f"## Decision time {decision:%H:%M} IST (H1)", ""]
    if not at.height:
        lines.append(f"{decision:%H:%M} is not on the comparison grid.")
    else:
        lines += [
            "| Convention | F tick | F candle | flip tick | flip candle | pin tick | pin candle | corr | rel L1 | net ratio |",
            "|---|---|---|---|---|---|---|---|---|---|",
        ]
        for r in at.iter_rows(named=True):
            lines.append(
                f"| **{r['convention']}** | {_fmt(r['forward_tick'])} | {_fmt(r['forward_candle'])} "
                f"| {_fmt(r['flip_tick'])} | {_fmt(r['flip_candle'])} | {_fmt(r['pin_tick'], 0)} "
                f"| {_fmt(r['pin_candle'], 0)} | {_fmt(r['gex_corr'], 4)} | {_fmt(r['gex_rel_l1'], 4)} "
                f"| {_fmt(r['net_gex_ratio'], 4)} |"
            )
    return "\n".join(lines) + "\n"


def resolve_weights(names: list[str], day: dt.date) -> dict[str, tuple[float, float]]:
    """S0 is fixed; S1 and S2 come from participant OI dated strictly before `day`."""
    table = {"S0": tuple(gp.S0)}
    if set(names) - {"S0"}:
        import participant_oi

        table = participant_oi.weights_for(day, participant_oi.load())
    unknown = [n for n in names if n not in table]
    if unknown:
        raise ValueError(f"unknown conventions {unknown}; expected some of {sorted(table)}")
    return {n: tuple(table[n]) for n in names}


def main() -> None:
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--date", type=dt.date.fromisoformat, required=True, help="capture date YYYY-MM-DD")
    ap.add_argument("--expiry", type=dt.date.fromisoformat, required=True)
    ap.add_argument("--merged-dir", type=Path, default=MERGED)
    ap.add_argument("--candles", type=Path, help="options candle parquet (default: gpdata.options_path)")
    ap.add_argument("--r", type=float, required=True, help="RBI 91-day T-bill yield, decimal")
    ap.add_argument("--weights", default="S0", help="comma-separated conventions, e.g. S0,S1,S2")
    ap.add_argument("--start", type=dt.time.fromisoformat, default=dt.time(9, 30))
    ap.add_argument("--end", type=dt.time.fromisoformat, default=dt.time(15, 15))
    ap.add_argument("--step", type=int, default=15, help="minutes between comparisons")
    ap.add_argument("--holidays", default="", help="comma-separated YYYY-MM-DD between date and expiry")
    ap.add_argument("--out", type=Path, default=OUTPUT)
    a = ap.parse_args()

    holidays = {dt.date.fromisoformat(s) for s in a.holidays.split(",") if s}
    weights = resolve_weights([s.strip() for s in a.weights.split(",") if s.strip()], a.date)
    events = load_events(a.merged_dir, a.date)
    opts = pl.read_parquet(a.candles or gpdata.options_path(a.expiry))
    times = time_grid(a.date, a.start, a.end, a.step)
    df = validate(events, opts, a.expiry, times, a.r, weights, weekdays(a.date, a.expiry, holidays))

    a.out.mkdir(parents=True, exist_ok=True)
    csv, md = a.out / f"validate_{a.date.isoformat()}.csv", a.out / f"validate_{a.date.isoformat()}.md"
    df.write_csv(csv)
    md.write_text(summarize(df, a.date, a.expiry), encoding="utf-8")
    print(f"wrote {csv} and {md}")


if __name__ == "__main__":
    main()
