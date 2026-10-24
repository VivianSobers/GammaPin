"""Shared research data layer: candle schema, as-of chain snapshots, time to expiry.

Every research script builds chains through `chain_at`, so the look-ahead rules live in one place:
a 1-minute candle stamped `ts` (its start) is only known from `ts + 1 minute`.

Options candles (`research/cache/expired/options_{expiry}.parquet`), written by fetch_expired.py:
    ts            Datetime(ms), naive IST, candle start
    instrument_key, opt_type ("CE" | "PE"), strike (f64)
    open, high, low, close (f64), volume (i64)
    oi            f64, open interest in contracts
    lot_size      i64

Index candles (`research/cache/expired/index_{expiry}.parquet`): ts, open, high, low, close.

Coverage (`research/cache/expired/coverage_summary.csv`, one row per expiry): `complete_expiries` reads
its `complete` flag. Only complete expiries enter the intraday `h1_pinning` and `h2_vol` samples (PREREGISTRATION "H1
implementation (intraday 14:00, not run)" and "H2 implementation"); `coverage_filter` applies the rule.
"""

from __future__ import annotations

import datetime as dt
from pathlib import Path

import numpy as np
import polars as pl

import gp_quant as gp

CACHE = Path(__file__).resolve().parent / "cache"
EXPIRED = CACHE / "expired"
CANDLE = dt.timedelta(minutes=1)


def options_path(expiry: dt.date) -> Path:
    return EXPIRED / f"options_{expiry.isoformat()}.parquet"


def index_path(expiry: dt.date) -> Path:
    return EXPIRED / f"index_{expiry.isoformat()}.parquet"


def coverage_summary_path() -> Path:
    return EXPIRED / "coverage_summary.csv"


def complete_expiries(summary_path: Path | None = None) -> set[dt.date]:
    """Expiries flagged `complete` in fetch_expired.py's coverage_summary.csv (default: in EXPIRED).

    A missing file gives an empty set; the caller decides how to report that. Expiries absent from
    the file are not complete.
    """
    path = coverage_summary_path() if summary_path is None else Path(summary_path)
    if not path.exists():
        return set()
    df = pl.read_csv(path, infer_schema=False)
    if df.height == 0:
        return set()
    flag = pl.col("complete").str.strip_chars().str.to_lowercase().is_in(["true", "1"])
    return {dt.date.fromisoformat(s) for s in df.filter(flag)["expiry"].str.strip_chars().to_list()}


def known_at(df: pl.DataFrame, ts: dt.datetime) -> pl.DataFrame:
    """Candles whose close is known at `ts` (start + 1 minute <= ts)."""
    return df.filter(pl.col("ts") <= ts - CANDLE)


def chain_at(opts: pl.DataFrame, ts: dt.datetime, max_age: dt.timedelta = dt.timedelta(minutes=30)) -> dict:
    """One row per strike as of `ts`: last known close and OI for the call and the put.

    A close older than `max_age` is treated as missing (0.0, dropped by the quote filters);
    its OI is still the latest known value.
    """
    known = known_at(opts, ts)
    last = known.sort("ts").group_by(["strike", "opt_type"], maintain_order=True).last()
    fresh = pl.col("ts") > ts - CANDLE - max_age
    last = last.with_columns(pl.when(fresh).then(pl.col("close")).otherwise(0.0).alias("px"))

    def side(kind: str, px: str, oi: str) -> pl.DataFrame:
        return last.filter(pl.col("opt_type") == kind).select("strike", pl.col("px").alias(px), pl.col("oi").alias(oi))

    chain = (
        side("CE", "call_px", "call_oi")
        .join(side("PE", "put_px", "put_oi"), on="strike", how="full", coalesce=True)
        .fill_null(0.0)
        .sort("strike")
    )
    return {c: chain[c].to_numpy().astype(np.float64) for c in chain.columns}


def sessions_after(day: dt.date, expiry: dt.date, trading_days: list[dt.date]) -> int:
    """Trading days d with day < d <= expiry."""
    return sum(1 for d in trading_days if day < d <= expiry)


def t_years_at(ts: dt.datetime, expiry: dt.date, trading_days: list[dt.date]) -> float:
    minute = ts.hour * 60 + ts.minute + ts.second / 60
    return gp.t_years(gp.minutes_to_expiry(minute, sessions_after(ts.date(), expiry, trading_days)))


def analyze_chain(ch: dict, t: float, r: float, lot: float, weights: list[tuple[float, float]]) -> dict | None:
    """gp_quant.analyze on a chain of closing prices (keys as `chain_at`). None when no forward or smile exists."""
    if len(ch.get("strike", [])) == 0:
        return None
    try:
        out = gp.analyze(
            ch["strike"], ch["call_px"], ch["call_px"], ch["call_oi"],
            ch["put_px"], ch["put_px"], ch["put_oi"], r, t, lot, weights, candles=True,
        )
    except ValueError:
        return None
    out["strikes"] = ch["strike"]
    out["call_oi"] = ch["call_oi"]
    out["put_oi"] = ch["put_oi"]
    return out


def analyze_at(
    opts: pl.DataFrame,
    ts: dt.datetime,
    expiry: dt.date,
    r: float,
    weights: list[tuple[float, float]],
    trading_days: list[dt.date],
) -> dict | None:
    """gp_quant.analyze on the candle chain as of `ts`. None when no forward or smile exists."""
    ch = chain_at(opts, ts)
    if len(ch.get("strike", [])) == 0:
        return None
    return analyze_chain(ch, t_years_at(ts, expiry, trading_days), r, float(opts["lot_size"][0]), weights)


def coverage_filter(expiries: list[dt.date], include_incomplete: bool = False) -> tuple[list[dt.date], list[dt.date]]:
    """(kept, incomplete): `incomplete` lists the expiries not flagged complete in coverage_summary.csv.

    `kept` is the complete expiries, or every expiry with `include_incomplete` (diagnostics only).
    Without a coverage_summary.csv no expiry passes the check, so the run stops unless `include_incomplete`.
    """
    import prereg

    path = coverage_summary_path()
    if not path.exists():
        msg = f"{path} not found, so no expiry passes the coverage check. Run fetch_expired.py to write it"
        if not include_incomplete:
            raise SystemExit(msg + " (or pass --include-incomplete for train diagnostics).")
        prereg.warn(msg + "; --include-incomplete keeps every expiry.")
    complete = complete_expiries(path)
    days = sorted(set(expiries))
    incomplete = [e for e in days if e not in complete]
    if incomplete and include_incomplete:
        prereg.warn(f"--include-incomplete keeps {len(incomplete)} expiries that fail the coverage check")
    return (days if include_incomplete else [e for e in days if e in complete]), incomplete
