"""RBI 91-day T-bill cut-off yield, as a continuously compounded decimal rate for Black-76 discounting.

No machine-downloadable source was found (checked Oct 2026): RBI DBIE (https://data.rbi.org.in/DBIE/)
is a JavaScript app without a documented download API, and rbi.org.in publishes the auction results as
press releases and Handbook/WSS tables. So the series is downloaded by hand and saved as a CSV:

1. Get the weekly 91-day T-bill auction results: DBIE -> Financial Markets -> Government Securities
   Market -> Treasury bill auctions (91-day), or the Handbook of Statistics on Indian Economy table
   "Auctions of 91-Day Government of India Treasury Bills", or the weekly RBI press releases
   "Treasury Bills: Full Auction Result". (Menu names are from memory and not verified.)
2. Keep two columns, the auction date and the cut-off (implicit) yield in percent, one row per auction,
   covering the whole study window plus a few weeks before it.
3. Save as `research/cache/tbill_91d.csv` with the header `date,yield_pct`, e.g. `2026-10-07,5.4512`.
   Dates may be YYYY-MM-DD, DD-MM-YYYY, DD/MM/YYYY or DD-Mon-YYYY; header case and spaces are ignored.

Conversion. RBI's implicit yield y is simple interest, actual/365 (since 27 Oct 2004), so a 91-day bill
costs P = 100 / (1 + y * 91/365). Black-76 discounts with exp(-r T); matching the same price gives
    r = (365 / 91) * ln(1 + y * 91/365)          (y and r as decimals)

Look-ahead rule: `rate_on(day)` uses the latest auction dated strictly before `day` (auctions are on
Wednesdays; a same-day cut-off may not be public by the 14:00 IST decision), constant within the day.
"""

from __future__ import annotations

import argparse
import datetime as dt
import math
from pathlib import Path

import polars as pl

PATH = Path(__file__).resolve().parent / "cache" / "tbill_91d.csv"
TENOR_DAYS = 91
DATE_FORMATS = ("%Y-%m-%d", "%d-%m-%Y", "%d/%m/%Y", "%d-%b-%Y")


def continuous(yield_pct: float, days: int = TENOR_DAYS) -> float:
    """Simple actual/365 yield in percent -> continuously compounded decimal rate."""
    tau = days / 365
    return math.log1p(yield_pct / 100 * tau) / tau


def parse_dates(col: pl.Series) -> pl.Series:
    for fmt in DATE_FORMATS:
        out = col.str.strip_chars().str.to_date(fmt, strict=False)
        if out.null_count() == 0:
            return out
    bad = col.filter(col.str.strip_chars().str.to_date(DATE_FORMATS[0], strict=False).is_null()).head(3).to_list()
    raise ValueError(f"unrecognised dates, e.g. {bad}")


def load(path: Path = PATH) -> pl.DataFrame:
    """Columns date, yield_pct, r (continuous decimal), one row per auction date, sorted."""
    raw = pl.read_csv(path, infer_schema=False)
    raw = raw.rename({c: c.strip().lower().replace(" ", "_") for c in raw.columns})
    missing = {"date", "yield_pct"} - set(raw.columns)
    if missing:
        raise ValueError(f"{path} lacks columns {sorted(missing)}; expected header date,yield_pct")
    raw = raw.filter(pl.col("date").str.strip_chars().fill_null("") != "")
    df = pl.DataFrame({
        "date": parse_dates(raw["date"]),
        "yield_pct": raw["yield_pct"].str.strip_chars().cast(pl.Float64),
    })
    tau = TENOR_DAYS / 365
    df = df.unique("date", keep="last").sort("date")
    return df.with_columns(((pl.col("yield_pct") / 100 * tau).log1p() / tau).alias("r"))


def rate_on(day: dt.date, df: pl.DataFrame) -> float:
    """Continuous decimal rate from the latest auction strictly before `day`.

    A same-day auction is excluded: its cut-off may not be public by the 14:00 IST decision.
    """
    prior = df.filter(pl.col("date") < day)
    if prior.is_empty():
        raise LookupError(f"no 91-day T-bill auction before {day}")
    return float(prior["r"][-1])


def main() -> None:
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--path", type=Path, default=PATH)
    ap.add_argument("--date", type=dt.date.fromisoformat, default=dt.date.today())
    args = ap.parse_args()
    if not args.path.exists():
        ap.error(f"{args.path} not found; the module docstring lists the manual download steps")
    df = load(args.path)
    print(f"{df.height} auctions, {df['date'].min()} to {df['date'].max()}")
    print(f"r on {args.date}: {rate_on(args.date, df):.6f}")


if __name__ == "__main__":
    main()
