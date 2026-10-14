"""NSE F&O daily bhavcopy (UDiFF format): Nifty option chains at the close, free and without a broker account.

Source: https://nsearchives.nseindia.com/content/fo/BhavCopy_NSE_FO_0_0_0_YYYYMMDD_F_0000.csv.zip (browser
User-Agent; 404 = no trading that day). This UDiFF format exists from 8 Jul 2024. Files are cached as-is in
`research/cache/bhavcopy/`.

Columns used (checked on the 13 Oct 2026 file): TradDt, FinInstrmTp (IDO = index option), TckrSymb (NIFTY),
XpryDt, StrkPric, OptnTp (CE/PE), ClsPric, OpnIntrst, TtlTradgVol, NewBrdLotQty, UndrlygPric.

Units and prices:
- OpnIntrst is in units (contracts x lot size): ~97% of Nifty option rows divide by NewBrdLotQty, the rest
  are long-dated contracts listed under an older lot size. `chain` divides by each row's own lot size.
- ClsPric is NSE's option closing price (equal to SttlmPric on non-expiry days, not the last trade).
  A strike with TtlTradgVol 0 has a carried-forward close; `chain` sets its price to 0 so the quote
  filters drop it, and keeps its OI.

Look-ahead rule: the file for day d is published after d's close, so a decision at the close of d may
use it, and nothing dated after d.
"""

from __future__ import annotations

import argparse
import datetime as dt
import io
import zipfile
from collections.abc import Iterable
from pathlib import Path

import numpy as np
import polars as pl
import requests

from participant_oi import download, weekdays

URL = "https://nsearchives.nseindia.com/content/fo/BhavCopy_NSE_FO_0_0_0_{stamp}_F_0000.csv.zip"
CACHE_DIR = Path(__file__).resolve().parent / "cache" / "bhavcopy"
FIRST_UDIFF_DAY = dt.date(2024, 7, 8)
SYMBOL = "NIFTY"

SCHEMA = {
    "date": pl.Date,
    "expiry": pl.Date,
    "strike": pl.Float64,
    "opt_type": pl.Utf8,
    "close": pl.Float64,
    "oi_units": pl.Float64,
    "volume": pl.Int64,
    "lot_size": pl.Int64,
    "underlying": pl.Float64,
}


def stamp(day: dt.date) -> str:
    return day.strftime("%Y%m%d")


def path_for(day: dt.date, cache_dir: Path = CACHE_DIR) -> Path:
    return cache_dir / f"BhavCopy_NSE_FO_0_0_0_{stamp(day)}_F_0000.csv.zip"


def looks_valid(body: bytes) -> bool:
    return body[:2] == b"PK"


def fetch(
    days: Iterable[dt.date], cache_dir: Path = CACHE_DIR, session: requests.Session | None = None, pause: float = 1.0
) -> dict[str, list[dt.date]]:
    """Download the bhavcopy zip for each day (cached files skipped). Days before the UDiFF format are skipped."""
    return download(
        [d for d in days if d >= FIRST_UDIFF_DAY],
        lambda d: URL.format(stamp=stamp(d)),
        lambda d: path_for(d, cache_dir),
        valid=looks_valid,
        session=session,
        pause=pause,
    )


def parse(text: str, symbol: str = SYMBOL) -> pl.DataFrame:
    """Index-option rows of `symbol` from one bhavcopy CSV, in SCHEMA."""
    raw = pl.read_csv(io.StringIO(text), infer_schema=False)
    raw = raw.rename({c: c.strip() for c in raw.columns})
    rows = raw.filter(
        (pl.col("FinInstrmTp").str.strip_chars() == "IDO") & (pl.col("TckrSymb").str.strip_chars() == symbol)
    )

    def num(col: str, dtype) -> pl.Expr:
        return pl.col(col).str.strip_chars().str.replace_all(",", "").cast(pl.Float64).cast(dtype)

    return rows.select(
        pl.col("TradDt").str.strip_chars().str.to_date("%Y-%m-%d").alias("date"),
        pl.col("XpryDt").str.strip_chars().str.to_date("%Y-%m-%d").alias("expiry"),
        num("StrkPric", pl.Float64).alias("strike"),
        pl.col("OptnTp").str.strip_chars().alias("opt_type"),
        num("ClsPric", pl.Float64).alias("close"),
        num("OpnIntrst", pl.Float64).alias("oi_units"),
        num("TtlTradgVol", pl.Int64).alias("volume"),
        num("NewBrdLotQty", pl.Int64).alias("lot_size"),
        num("UndrlygPric", pl.Float64).alias("underlying"),
    ).cast(SCHEMA)


def read(path: Path, symbol: str = SYMBOL) -> pl.DataFrame:
    with zipfile.ZipFile(path) as z:
        name = next(n for n in z.namelist() if n.lower().endswith(".csv"))
        return parse(z.read(name).decode("utf-8", errors="replace"), symbol)


def cached_days(cache_dir: Path = CACHE_DIR) -> list[dt.date]:
    """Trading days with a cached bhavcopy (file names only)."""
    out = []
    for p in Path(cache_dir).glob("BhavCopy_NSE_FO_0_0_0_*_F_0000.csv.zip"):
        out.append(dt.datetime.strptime(p.name.split("_")[6], "%Y%m%d").date())
    return sorted(out)


def load(day: dt.date, cache_dir: Path = CACHE_DIR) -> pl.DataFrame:
    path = path_for(day, cache_dir)
    if not path.exists():
        raise LookupError(f"no bhavcopy cached for {day}")
    return read(path)


def eves(days: list[dt.date], expiries: Iterable[dt.date]) -> dict[dt.date, dt.date]:
    """expiry -> its eve, the last trading day in `days` before it. Expiries without an earlier day are left out."""
    days = sorted(days)
    out = {}
    for e in sorted(set(expiries)):
        prior = [d for d in days if d < e]
        if prior:
            out[e] = prior[-1]
    return out


def chain(df: pl.DataFrame, expiry: dt.date) -> dict:
    """One row per strike of `expiry`: closing price and OI (contracts) for the call and the put.

    Same keys as gpdata.chain_at, plus `lot_size`. Untraded strikes get price 0.0; missing sides 0.0.
    """
    rows = df.filter(pl.col("expiry") == expiry).with_columns(
        pl.when(pl.col("volume") > 0).then(pl.col("close")).otherwise(0.0).alias("px"),
        (pl.col("oi_units") / pl.col("lot_size")).alias("oi"),
    )
    if rows.is_empty():
        return {}

    def side(kind: str, px: str, oi: str) -> pl.DataFrame:
        return rows.filter(pl.col("opt_type") == kind).select("strike", pl.col("px").alias(px), pl.col("oi").alias(oi))

    ch = (
        side("CE", "call_px", "call_oi")
        .join(side("PE", "put_px", "put_oi"), on="strike", how="full", coalesce=True)
        .fill_null(0.0)
        .sort("strike")
    )
    out = {c: ch[c].to_numpy().astype(np.float64) for c in ch.columns}
    out["lot_size"] = int(rows["lot_size"].mode().sort()[-1])
    return out


def main() -> None:
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--start", type=dt.date.fromisoformat, required=True)
    ap.add_argument("--end", type=dt.date.fromisoformat, default=dt.date.today())
    ap.add_argument("--out", type=Path, default=CACHE_DIR)
    ap.add_argument("--pause", type=float, default=1.0, help="seconds between requests (>= 1)")
    args = ap.parse_args()
    res = fetch(weekdays(args.start, args.end), args.out, pause=max(args.pause, 1.0))
    print(" ".join(f"{k}={len(v)}" for k, v in res.items()))
    for k in ("missing", "invalid"):
        if res[k]:
            print(f"{k}: " + ", ".join(d.isoformat() for d in res[k]))
    days = cached_days(args.out)
    print(f"{len(days)} bhavcopies cached, {min(days, default=None)} to {max(days, default=None)}")


if __name__ == "__main__":
    main()
