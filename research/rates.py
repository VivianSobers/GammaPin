"""RBI 91-day T-bill cut-off yield, as a continuously compounded decimal rate for Black-76 discounting.

RBI DBIE (https://data.rbi.org.in/DBIE/) has no documented download API, but every weekly auction result is
an rbi.org.in press release: `tbill_rbi.py` scans them and writes the CSV below. To build it by hand instead:

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
This r is per calendar year, while gp_quant's T is in trading years (minutes / (252 * 375)), so exp(-r T)
mixes day counts. Over the few sessions to a weekly expiry the error is ~1e-4 relative and is ignored.

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
MAX_AGE = dt.timedelta(days=21)  # auctions are weekly; the cache has gaps of up to 21 days
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
    stripped = pl.col("d").str.strip_chars()
    fails = pl.all_horizontal([stripped.str.to_date(f, strict=False).is_null() for f in DATE_FORMATS])
    bad = col.to_frame("d").filter(fails)["d"].head(3).to_list() or col.head(3).to_list()  # else formats are mixed
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
    df = df.unique("date", keep="last").sort("date")
    return df.with_columns(pl.col("yield_pct").map_elements(continuous, return_dtype=pl.Float64).alias("r"))


def rate_on(day: dt.date, df: pl.DataFrame) -> float:
    """Continuous decimal rate from the latest auction strictly before `day`.

    A same-day auction is excluded: its cut-off may not be public by the 14:00 IST decision.
    An auction more than MAX_AGE before `day` is refused (LookupError).
    """
    prior = df.filter(pl.col("date") < day)
    if prior.is_empty():
        raise LookupError(f"no 91-day T-bill auction before {day}")
    latest = prior["date"][-1]
    if day - latest > MAX_AGE:
        raise LookupError(f"latest 91-day T-bill auction before {day} is {latest}, older than {MAX_AGE.days} days")
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
