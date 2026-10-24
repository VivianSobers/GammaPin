"""Official Nifty 50 closing value on each expiry day: the H1 target.

NSE index options settle on the index closing price of the expiry day. NSE publishes all index closes
daily at https://nsearchives.nseindia.com/content/indices/ind_close_all_DDMMYYYY.csv (browser User-Agent
needed; 404 = holiday). Header: Index Name, Index Date (DD-MM-YYYY), Open/High/Low Index Value,
Closing Index Value, ... The Nifty 50 row is `Index Name == "Nifty 50"` (it can appear twice, identical).

Files are cached as-is in `research/cache/index_close/`. The settlement is a target, never a feature.
"""

from __future__ import annotations

import argparse
import datetime as dt
import io
from collections.abc import Iterable
from pathlib import Path

import polars as pl
import requests

from participant_oi import download, weekdays

URL = "https://nsearchives.nseindia.com/content/indices/ind_close_all_{stamp}.csv"
CACHE_DIR = Path(__file__).resolve().parent / "cache" / "index_close"
INDEX = "Nifty 50"
DATE_FORMATS = ("%d-%m-%Y", "%d-%b-%Y", "%d/%m/%Y", "%Y-%m-%d")


def stamp(day: dt.date) -> str:
    return day.strftime("%d%m%Y")


def path_for(day: dt.date, cache_dir: Path = CACHE_DIR) -> Path:
    return cache_dir / f"ind_close_all_{stamp(day)}.csv"


def looks_valid(body: bytes) -> bool:
    return b"Closing Index Value" in body[:2048]


def fetch(
    days: Iterable[dt.date], cache_dir: Path = CACHE_DIR, session: requests.Session | None = None, pause: float = 1.0
) -> dict[str, list[dt.date]]:
    """Download the daily index-close file for each day (cached files skipped)."""
    return download(
        days,
        lambda d: URL.format(stamp=stamp(d)),
        lambda d: path_for(d, cache_dir),
        valid=looks_valid,
        session=session,
        pause=pause,
    )


def parse_date(text: str) -> dt.date:
    for fmt in DATE_FORMATS:
        try:
            return dt.datetime.strptime(text.strip(), fmt).date()
        except ValueError:
            pass
    raise ValueError(f"unrecognised index date {text!r}")


def parse(text: str, index: str = INDEX) -> tuple[dt.date, float] | None:
    """(date, close) of `index` in one file, None when the row is absent."""
    df = pl.read_csv(io.StringIO(text), infer_schema=False)
    df = df.rename({c: c.strip() for c in df.columns})
    row = df.filter(pl.col("Index Name").str.strip_chars().str.to_lowercase() == index.lower())
    if row.is_empty():
        return None
    return parse_date(row["Index Date"][0]), float(row["Closing Index Value"][0].strip().replace(",", ""))


def load(cache_dir: Path = CACHE_DIR, index: str = INDEX) -> dict[dt.date, float]:
    """Close of `index` by date from every cached file."""
    out: dict[dt.date, float] = {}
    for p in sorted(Path(cache_dir).glob("ind_close_all_*.csv")):
        got = parse(p.read_text(encoding="utf-8", errors="replace"), index)
        if got is not None:
            out[got[0]] = got[1]
    return out


def settlement(expiry: dt.date, closes: dict[dt.date, float] | None = None) -> float:
    """Official Nifty 50 close on `expiry`. LookupError if that day's file is not cached."""
    closes = load() if closes is None else closes
    if expiry not in closes:
        raise LookupError(f"no Nifty 50 close for {expiry}")
    return closes[expiry]


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
    closes = load(args.out)
    print(f"{len(closes)} Nifty 50 closes cached, {min(closes, default=None)} to {max(closes, default=None)}")


if __name__ == "__main__":
    main()
