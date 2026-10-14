"""H1: does the largest-positive-GEX strike at 14:00 IST predict expiry settlement?

Compares S0, S1, S2 against max pain, nearest 100 and F at 14:00. Paired Wilcoxon + block bootstrap CI.

Per expiry, `features` sees only the candle chain as of the decision time (gpdata.chain_at) and the
inputs known before it. The settlement enters only through `target` and `abs_errors`, after the
features are fixed.

Sample: cached expiries flagged `complete` in coverage_summary.csv (gpdata.complete_expiries), split
chronologically by prereg.expiries_for over that kept sample. `--include-incomplete` keeps every cached
expiry for train diagnostics only; it is refused for the test split.

Outputs: output/h1_{split}.csv (one row per expiry) and output/h1_{split}_summary.json.
"""

from __future__ import annotations

import argparse
import datetime as dt
import json
import math
from pathlib import Path
from typing import Callable

import numpy as np
import polars as pl

import baselines
import gpdata
import prereg
import stats

OUTPUT = Path(__file__).resolve().parent / "output"
CONVENTIONS = ("S0", "S1", "S2")
GEX_METHODS = tuple(f"pin_{c}" for c in CONVENTIONS)
BASELINES = ("max_pain", "nearest_100", "no_move")
METHODS = GEX_METHODS + BASELINES

SCHEMA = {
    "expiry": pl.Date,
    "status": pl.Utf8,
    "decision_ts": pl.Datetime("ms"),
    "weights_ok": pl.Boolean,
    "r": pl.Float64,
    "forward": pl.Float64,
    "quotes_used": pl.Int64,
    "quotes_total": pl.Int64,
    **{f"{k}_{c}": pl.Float64 for c in CONVENTIONS for k in ("w_c", "w_p", "net_gex", "flip")},
    **{m: pl.Float64 for m in METHODS},
    "settlement": pl.Float64,
    **{f"abs_err_{m}": pl.Float64 for m in METHODS},
}


def nan_if_none(x) -> float:
    return math.nan if x is None else float(x)


def features(
    opts: pl.DataFrame,
    expiry: dt.date,
    decision_ts: dt.datetime,
    r: float,
    weights: dict[str, tuple[float, float]],
    trading_days: list[dt.date],
) -> dict:
    """Predictions and diagnostics from the chain as of `decision_ts`. Conventions missing from `weights` are NaN."""
    names = [c for c in CONVENTIONS if c in weights]
    out = gpdata.analyze_at(opts, decision_ts, expiry, r, [tuple(weights[c]) for c in names], trading_days)
    return predictions(out, gpdata.chain_at(opts, decision_ts), weights)


def predictions(out: dict | None, ch: dict, weights: dict[str, tuple[float, float]]) -> dict:
    """Feature row from one gp_quant.analyze result `out` (None if it failed) and the chain `ch` it ran on.

    `out["conventions"]` follows the order of CONVENTIONS restricted to those in `weights`.
    """
    names = [c for c in CONVENTIONS if c in weights]
    row: dict = {"forward": math.nan, "quotes_used": None, "quotes_total": None}
    for c in CONVENTIONS:
        w_c, w_p = weights.get(c, (math.nan, math.nan))
        row |= {f"w_c_{c}": float(w_c), f"w_p_{c}": float(w_p), f"net_gex_{c}": math.nan, f"flip_{c}": math.nan, f"pin_{c}": math.nan}
    if out is not None:
        row |= {"forward": float(out["forward"]), "quotes_used": int(out["quotes_used"]), "quotes_total": int(out["quotes_total"])}
        for c, conv in zip(names, out["conventions"]):
            row |= {f"net_gex_{c}": float(conv["net_gex"]), f"flip_{c}": nan_if_none(conv["flip"]), f"pin_{c}": nan_if_none(conv["pin"])}
    row["max_pain"] = baselines.max_pain(ch.get("strike", []), ch.get("call_oi", []), ch.get("put_oi", []))
    row["nearest_100"] = baselines.nearest_multiple(row["forward"], 100.0)
    row["no_move"] = baselines.no_move(row["forward"])
    return row


def target(expiry: dt.date, settlements: dict[dt.date, float]) -> float:
    """Official settlement (Nifty 50 close) on the expiry date; NaN when unknown."""
    return nan_if_none(settlements.get(expiry))


def abs_errors(predictions: dict, settlement: float) -> dict:
    return {f"abs_err_{m}": abs(settlement - predictions[m]) for m in METHODS}


def trading_days(expiries: list[dt.date]) -> list[dt.date]:
    """Dates present in the cached index candles of these expiries, plus the expiries themselves."""
    days = set(expiries)
    for e in expiries:
        path = gpdata.index_path(e)
        if path.exists():
            days |= set(pl.read_parquet(path, columns=["ts"])["ts"].dt.date().unique().to_list())
    return sorted(days)


def available_expiries() -> list[dt.date]:
    """Expiries with a cached options file. Only file names are read, not data."""
    return sorted(dt.date.fromisoformat(p.stem.removeprefix("options_")) for p in gpdata.EXPIRED.glob("options_*.parquet"))


def coverage_filter(expiries: list[dt.date], include_incomplete: bool = False) -> tuple[list[dt.date], list[dt.date]]:
    """(kept, incomplete): `incomplete` lists the expiries not flagged complete in coverage_summary.csv.

    `kept` is the complete expiries, or every expiry with `include_incomplete` (diagnostics only).
    Without a coverage_summary.csv no expiry passes the check, so the run stops unless `include_incomplete`.
    """
    path = gpdata.coverage_summary_path()
    if not path.exists():
        msg = f"{path} not found, so no expiry passes the coverage check. Run fetch_expired.py to write it"
        if not include_incomplete:
            raise SystemExit(msg + " (or pass --include-incomplete for train diagnostics).")
        prereg.warn(msg + "; --include-incomplete keeps every expiry.")
    complete = gpdata.complete_expiries(path)
    days = sorted(set(expiries))
    incomplete = [e for e in days if e not in complete]
    if incomplete and include_incomplete:
        prereg.warn(f"--include-incomplete keeps {len(incomplete)} expiries that fail the coverage check")
    return (days if include_incomplete else [e for e in days if e in complete]), incomplete


def run_expiry(
    expiry: dt.date,
    decision_time: dt.time,
    weights_for: Callable[[dt.date], dict],
    rate_on: Callable[[dt.date], float],
    days: list[dt.date],
) -> dict:
    """Feature row for one expiry, without the settlement."""
    ts = dt.datetime.combine(expiry, decision_time)
    row: dict = {"expiry": expiry, "decision_ts": ts, "status": "ok", "weights_ok": True, "r": math.nan}
    empty = {m: math.nan for m in METHODS}
    path = gpdata.options_path(expiry)
    if not path.exists():
        return row | empty | {"status": "no_options"}
    try:
        row["r"] = float(rate_on(expiry))
    except LookupError:
        return row | empty | {"status": "no_rate"}
    try:
        weights = dict(weights_for(expiry))
    except LookupError:
        weights, row["weights_ok"] = {"S0": (1.0, -1.0)}, False
    row |= features(pl.read_parquet(path), expiry, ts, row["r"], weights, days)
    if math.isnan(row["forward"]):
        row["status"] = "no_chain"
    return row


def build_table(
    expiries: list[dt.date],
    decision_time: dt.time,
    weights_for: Callable[[dt.date], dict],
    rate_on: Callable[[dt.date], float],
    settlements: dict[dt.date, float],
) -> pl.DataFrame:
    days = trading_days(expiries)
    rows = []
    for e in sorted(expiries):
        row = run_expiry(e, decision_time, weights_for, rate_on, days)
        settle = target(e, settlements)
        rows.append(row | {"settlement": settle} | abs_errors(row, settle))
    return pl.from_dicts(rows, schema=SCHEMA)


def finite_mean(x: np.ndarray) -> float:
    return float(x.mean()) if x.size else math.nan


def evaluate(table: pl.DataFrame, block_length: int, resamples: int, seed: int) -> dict:
    """MAE per method, and paired Wilcoxon + block-bootstrap CI of mean(|err_gex| - |err_baseline|) per pair.

    Pairs use only expiries where both errors are finite, in chronological order. The same seed is
    used for every comparison. The bootstrap is skipped when fewer pairs than block_length exist.
    """
    t = table.sort("expiry")
    err = {m: t[f"abs_err_{m}"].cast(pl.Float64).fill_null(math.nan).to_numpy() for m in METHODS}
    mae = {m: {"mae": finite_mean(e[np.isfinite(e)]), "n": int(np.isfinite(e).sum())} for m, e in err.items()}
    comparisons = []
    for g in GEX_METHODS:
        for b in BASELINES:
            both = np.isfinite(err[g]) & np.isfinite(err[b])
            a, c = err[g][both], err[b][both]
            n = int(both.sum())
            boot = None
            if n >= max(block_length, 1):
                boot = stats.block_bootstrap_ci(a - c, block_length, resamples, seed=seed)
            comparisons.append({
                "gex": g, "baseline": b, "n": n, "n_excluded": t.height - n,
                "mae_gex": finite_mean(a), "mae_baseline": finite_mean(c),
                "wilcoxon": stats.paired_wilcoxon(a, c),
                "bootstrap": boot,
                "note": None if boot else f"bootstrap skipped: {n} pairs < block_length {block_length}",
            })
    return {"n_expiries": t.height, "mae": mae, "comparisons": comparisons}


def jsonable(x):
    if isinstance(x, dict):
        return {k: jsonable(v) for k, v in x.items()}
    if isinstance(x, (list, tuple)):
        return [jsonable(v) for v in x]
    if isinstance(x, float) and not math.isfinite(x):
        return None
    if isinstance(x, (dt.date, dt.time)):
        return x.isoformat()
    return x


def write(table: pl.DataFrame, summary: dict, out: Path, split: str) -> tuple[Path, Path]:
    out.mkdir(parents=True, exist_ok=True)
    csv, js = out / f"h1_{split}.csv", out / f"h1_{split}_summary.json"
    table.write_csv(csv)
    js.write_text(json.dumps(jsonable(summary), indent=2), encoding="utf-8")
    return csv, js


def main(argv: list[str] | None = None) -> None:
    p = argparse.ArgumentParser(description="H1 pinning: GEX pin vs baselines at the expiry-day decision time.")
    p.add_argument("--split", choices=prereg.SPLITS, required=True)
    p.add_argument("--out", type=Path, default=OUTPUT)
    p.add_argument("--include-incomplete", action="store_true",
                   help="diagnostics only: keep expiries that fail the coverage check (refused for --split test)")
    args = p.parse_args(argv)
    if args.include_incomplete and args.split == "test":
        raise SystemExit("--include-incomplete is for train diagnostics only and is refused for --split test.")
    if args.split == "test":
        prereg.require_frozen()
    cfg = prereg.load()
    available = available_expiries()
    kept, incomplete = coverage_filter(available, args.include_incomplete)
    expiries = prereg.expiries_for(args.split, kept, cfg)
    block_length = prereg.block_length(args.split, cfg)
    if not expiries:
        raise SystemExit(
            f"No {args.split} expiries in {gpdata.EXPIRED} "
            f"({len(available)} cached, {len(available) - len(kept)} excluded as incomplete)"
        )

    import participant_oi
    import rates
    import settlements

    poi, rate_df = participant_oi.load(), rates.load()
    table = build_table(
        expiries,
        prereg.decision_time(cfg),
        lambda d: participant_oi.weights_for(d, poi),
        lambda d: rates.rate_on(d, rate_df),
        settlements.load(),
    ).with_columns(coverage_complete=~pl.col("expiry").is_in(pl.Series(incomplete, dtype=pl.Date).implode()))
    summary = {
        "split": args.split,
        "expiries": expiries,
        "include_incomplete": args.include_incomplete,
        "coverage": {
            "n_cached": len(available),
            "n_incomplete": len(incomplete),
            "n_excluded_incomplete": len(available) - len(kept),
            "incomplete_expiries": incomplete,
            "n_kept": len(kept),
            "n_split": len(expiries),
            "n_split_incomplete": len(set(expiries) & set(incomplete)),
        },
        "decision_time": prereg.decision_time(cfg),
        "block_length": block_length,
        "block_length_provisional": prereg.is_tbd(cfg["bootstrap"]["block_length"]),
        "resamples": prereg.resamples(cfg),
        "seed": prereg.seed(cfg),
        "status": dict(table["status"].value_counts().iter_rows()),
        **evaluate(table, block_length, prereg.resamples(cfg), prereg.seed(cfg)),
    }
    csv, js = write(table, summary, args.out, args.split)
    for m, v in summary["mae"].items():
        print(f"{m:12s} MAE {v['mae']:9.2f}  n={v['n']}")
    print(f"wrote {csv} and {js}")


if __name__ == "__main__":
    main()
