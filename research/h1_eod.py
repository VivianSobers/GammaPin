"""H1 from free end-of-day data: does the largest-positive-GEX strike at the close of the expiry eve predict settlement?

Same predictions, baselines, errors and tests as h1_pinning.py, with the chain taken from the NSE F&O bhavcopy
of the eve (the last trading day before the expiry) instead of 14:00 candles on expiry day.

Decision time: 15:30 IST on the eve. Inputs known then: the eve's bhavcopy (closing prices, OI), the latest
participant OI file dated strictly before the eve, and the latest T-bill auction strictly before the eve.
T = trading minutes from the eve close to the expiry close (375 for a one-session gap). The settlement enters
only through h1_pinning.target and abs_errors, after the features are fixed.

Sample: every Nifty expiry e with sample_start <= e <= sample_end (prereg.toml [h1_eod]) that is the nearest
listed Nifty option expiry in some cached bhavcopy and whose eve bhavcopy is cached. Which expiries exist is
read from the listings alone, never from prices or the settlement. The split is prereg.expiries_for over it.

Outputs: output/h1_eod_{split}.csv (one row per expiry) and output/h1_eod_{split}_summary.json.
"""

from __future__ import annotations

import argparse
import datetime as dt
import math
from collections.abc import Callable
from pathlib import Path

import polars as pl

import bhavcopy
import gpdata
import h1_pinning
import prereg

OUTPUT = h1_pinning.OUTPUT
CLOSE = dt.time(15, 30)
SCHEMA = {"eve": pl.Date, **h1_pinning.SCHEMA}


def sample(
    days: list[dt.date], load: Callable[[dt.date], pl.DataFrame], start: dt.date, end: dt.date
) -> dict[dt.date, dt.date]:
    """expiry -> eve for the H1 sample: nearest listed expiries in [start, end] with a cached eve."""
    listed = set()
    for d in days:
        later = load(d).filter(pl.col("expiry") > d)["expiry"]
        if later.len():
            listed.add(later.min())
    return {e: eve for e, eve in bhavcopy.eves(days, listed).items() if start <= e <= end}


def t_years(eve: dt.date, expiry: dt.date, days: list[dt.date]) -> float:
    """Trading-time years from the eve close to the expiry close (gpdata convention, floor 1 minute)."""
    return gpdata.t_years_at(dt.datetime.combine(eve, CLOSE), expiry, sorted(set(days) | {expiry}))


def features(
    df: pl.DataFrame, expiry: dt.date, eve: dt.date, r: float, weights: dict[str, tuple[float, float]], days: list[dt.date]
) -> dict:
    """Predictions from the eve's bhavcopy `df` (rows dated after `eve` are refused)."""
    if df.height and df["date"].max() > eve:
        raise ValueError(f"bhavcopy rows dated after the eve {eve}")
    ch = bhavcopy.chain(df, expiry)
    names = [c for c in h1_pinning.CONVENTIONS if c in weights]
    out = None
    if ch:
        out = gpdata.analyze_chain(ch, t_years(eve, expiry, days), r, float(ch["lot_size"]), [tuple(weights[c]) for c in names])
    return h1_pinning.predictions(out, ch, weights)


def run_expiry(
    expiry: dt.date,
    eve: dt.date,
    load: Callable[[dt.date], pl.DataFrame],
    weights_for: Callable[[dt.date], dict],
    rate_on: Callable[[dt.date], float],
    days: list[dt.date],
) -> dict:
    """Feature row for one expiry, without the settlement."""
    row: dict = {
        "expiry": expiry, "eve": eve, "decision_ts": dt.datetime.combine(eve, CLOSE),
        "status": "ok", "weights_ok": True, "r": math.nan,
    }
    empty = {m: math.nan for m in h1_pinning.METHODS}
    try:
        df = load(eve)
    except LookupError:
        return row | empty | {"status": "no_bhavcopy"}
    try:
        row["r"] = float(rate_on(eve))
    except LookupError:
        return row | empty | {"status": "no_rate"}
    try:
        weights = dict(weights_for(eve))
    except LookupError:
        weights, row["weights_ok"] = {"S0": (1.0, -1.0)}, False
    row |= features(df, expiry, eve, row["r"], weights, days)
    if math.isnan(row["forward"]):
        row["status"] = "no_chain"
    return row


def build_table(
    eves: dict[dt.date, dt.date],
    load: Callable[[dt.date], pl.DataFrame],
    weights_for: Callable[[dt.date], dict],
    rate_on: Callable[[dt.date], float],
    settlements: dict[dt.date, float],
    days: list[dt.date],
) -> pl.DataFrame:
    rows = []
    for e in sorted(eves):
        row = run_expiry(e, eves[e], load, weights_for, rate_on, days)
        settle = h1_pinning.target(e, settlements)
        rows.append(row | {"settlement": settle} | h1_pinning.abs_errors(row, settle))
    return pl.from_dicts(rows, schema=SCHEMA)


def sample_window(cfg: dict) -> tuple[dt.date, dt.date]:
    w = cfg["h1_eod"]
    return dt.date.fromisoformat(str(w["sample_start"])), dt.date.fromisoformat(str(w["sample_end"]))


def main(argv: list[str] | None = None) -> None:
    p = argparse.ArgumentParser(description="H1 pinning on end-of-day bhavcopy chains at the expiry-eve close.")
    p.add_argument("--split", choices=prereg.SPLITS, required=True)
    p.add_argument("--out", type=Path, default=OUTPUT)
    p.add_argument("--list", action="store_true", help="print the sample expiries and the split, then stop")
    args = p.parse_args(argv)
    if args.split == "test" and not args.list:
        prereg.require_frozen()
    cfg = prereg.load()
    days = bhavcopy.cached_days()
    eves = sample(days, bhavcopy.load, *sample_window(cfg))
    expiries = prereg.expiries_for(args.split, list(eves), cfg)
    if args.list:
        print(f"{len(eves)} sample expiries, {min(eves, default=None)} to {max(eves, default=None)}")
        print(f"{args.split}: {len(expiries)} expiries, {min(expiries, default=None)} to {max(expiries, default=None)}")
        return
    if not expiries:
        raise SystemExit(f"No {args.split} expiries in {bhavcopy.CACHE_DIR} ({len(days)} bhavcopies cached)")
    block_length = prereg.block_length(args.split, cfg)

    import participant_oi
    import rates
    import settlements

    poi, rate_df = participant_oi.load(), rates.load()
    table = build_table(
        {e: eves[e] for e in expiries},
        bhavcopy.load,
        lambda d: participant_oi.weights_for(d, poi),
        lambda d: rates.rate_on(d, rate_df),
        settlements.load(),
        days,
    )
    summary = {
        "split": args.split,
        "source": "bhavcopy_eod",
        "sample_window": sample_window(cfg),
        "n_sample": len(eves),
        "expiries": expiries,
        "decision": "eve close 15:30 IST",
        "block_length": block_length,
        "resamples": prereg.resamples(cfg),
        "seed": prereg.seed(cfg),
        "status": dict(table["status"].value_counts().iter_rows()),
        **h1_pinning.evaluate(table, block_length, prereg.resamples(cfg), prereg.seed(cfg)),
    }
    csv, js = h1_pinning.write(table, summary, args.out, f"eod_{args.split}")
    for m, v in summary["mae"].items():
        print(f"{m:12s} MAE {v['mae']:9.2f}  n={v['n']}")
    print(f"wrote {csv} and {js}")


if __name__ == "__main__":
    main()
