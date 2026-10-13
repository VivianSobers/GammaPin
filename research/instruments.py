"""Nifty option contracts, lot size and tick size from the Upstox instrument master.

Source: https://assets.upstox.com/market-quote/instruments/exchange/NSE.json.gz (public, no auth),
a JSON list of records cached at `research/cache/NSE.json.gz`. It lists live contracts only; expired
ones come from fetch_expired.py.

Field units, checked against the real file (Oct 2026):
    expiry        epoch ms at 23:59:59 IST of the expiry day -> converted to an IST date
    tick_size     paise (options 5.0 = Rs 0.05, futures 10.0 = Rs 0.10) -> converted to rupees
    strike_price  rupees; lot_size in units (shares of the index) per contract
Nifty options are records with segment NSE_FO, name NIFTY, underlying_key "NSE_INDEX|Nifty 50"
and instrument_type CE or PE.
"""

from __future__ import annotations

import argparse
import datetime as dt
import gzip
import json
from pathlib import Path

import polars as pl
import requests

URL = "https://assets.upstox.com/market-quote/instruments/exchange/NSE.json.gz"
PATH = Path(__file__).resolve().parent / "cache" / "NSE.json.gz"
UNDERLYING_KEY = "NSE_INDEX|Nifty 50"
TICK_UNIT = 0.01  # rupees per tick_size unit (paise)
IST_OFFSET_MS = 19_800_000


def download(path: Path = PATH, session: requests.Session | None = None) -> Path:
    resp = (session or requests.Session()).get(URL, timeout=60)
    resp.raise_for_status()
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_bytes(resp.content)
    return path


def read_records(path: Path) -> list[dict]:
    opener = gzip.open if Path(path).suffix == ".gz" else open
    with opener(path, "rt", encoding="utf-8") as f:
        return json.load(f)


def is_nifty_option(rec: dict) -> bool:
    return (
        rec.get("segment") == "NSE_FO"
        and rec.get("name") == "NIFTY"
        and rec.get("underlying_key") == UNDERLYING_KEY
        and rec.get("instrument_type") in ("CE", "PE")
    )


def nifty_options(src: pl.DataFrame | Path | str = PATH) -> pl.DataFrame:
    """One row per Nifty option: instrument_key, trading_symbol, expiry, strike, opt_type, lot_size, tick_size, weekly."""
    if isinstance(src, pl.DataFrame):
        raw = src.filter(
            (pl.col("segment") == "NSE_FO")
            & (pl.col("name") == "NIFTY")
            & (pl.col("underlying_key") == UNDERLYING_KEY)
            & pl.col("instrument_type").is_in(["CE", "PE"])
        )
    else:
        raw = pl.DataFrame([r for r in read_records(Path(src)) if is_nifty_option(r)], infer_schema_length=None)
    return raw.select(
        "instrument_key",
        "trading_symbol",
        pl.from_epoch(pl.col("expiry").cast(pl.Int64) + IST_OFFSET_MS, time_unit="ms").dt.date().alias("expiry"),
        pl.col("strike_price").cast(pl.Float64).alias("strike"),
        pl.col("instrument_type").alias("opt_type"),
        pl.col("lot_size").cast(pl.Int64),
        (pl.col("tick_size").cast(pl.Float64) * TICK_UNIT).round(4).alias("tick_size"),
        pl.col("weekly").cast(pl.Boolean),
    ).sort("expiry", "strike", "opt_type")


def expiries(opts: pl.DataFrame) -> list[dt.date]:
    return opts["expiry"].unique().sort().to_list()


def lot_size(expiry: dt.date, opts: pl.DataFrame) -> int:
    lots = opts.filter(pl.col("expiry") == expiry)["lot_size"].unique()
    if lots.is_empty():
        raise LookupError(f"no Nifty options expiring {expiry}")
    if lots.len() > 1:
        raise ValueError(f"several lot sizes for {expiry}: {lots.to_list()}")
    return int(lots[0])


def main() -> None:
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--path", type=Path, default=PATH)
    ap.add_argument("--refresh", action="store_true", help="download the master again")
    ap.add_argument("-n", type=int, default=2, help="number of upcoming expiries to show")
    args = ap.parse_args()
    if args.refresh or not args.path.exists():
        download(args.path)
    opts = nifty_options(args.path)
    today = dt.date.today()
    for e in [e for e in expiries(opts) if e >= today][: args.n]:
        chain = opts.filter(pl.col("expiry") == e)
        kind = "weekly" if chain["weekly"].all() else "monthly"
        print(
            f"{e} ({kind}): {chain.height} contracts, strikes {chain['strike'].min():.0f}-{chain['strike'].max():.0f}, "
            f"lot {lot_size(e, opts)}, tick Rs {chain['tick_size'][0]}"
        )


if __name__ == "__main__":
    main()
