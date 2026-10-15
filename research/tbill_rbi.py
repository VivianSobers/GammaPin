"""Build `research/cache/tbill_91d.csv` (rates.py's input) from RBI's weekly T-bill auction press releases.

RBI publishes each Wednesday auction as a press release titled "91-Day, 182-Day and 364-Day T-Bill Auction
Result: Cut-off" at https://www.rbi.org.in/scripts/BS_PressReleaseDisplay.aspx?prid=N, with N sequential
across all press releases. This scans a prid range, keeps those releases and parses the 91-day line
"Cut-off Price (₹) and Implicit Yield at Cut-Off Price 98.7086 (YTM: 5.2476%) ...".

The CSV date is the release date, which is the auction date (results are released the same evening), so
rates.rate_on's strictly-before rule still applies. Releases where the 91-day tenor shows no YTM are skipped.
Every page found is listed in `research/cache/tbill_rbi_pages.csv` (prid, date, yield or blank) so a rerun
only fetches prids not seen before.
"""

from __future__ import annotations

import argparse
import datetime as dt
import html
import re
import time
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

import polars as pl
import requests

import rates
from participant_oi import HEADERS

URL = "https://www.rbi.org.in/scripts/BS_PressReleaseDisplay.aspx?prid={prid}"
PAGES = Path(__file__).resolve().parent / "cache" / "tbill_rbi_pages.csv"
# Titles before mid-2025 read "91 days, 182 days and 364 days T-Bill Auction Result: Cut-off".
TITLE = re.compile(
    r"Date : (\w{3} \d{1,2}, \d{4}) 91[- ]days?, 182[- ]days? and 364[- ]days? T-Bill Auction Result: Cut-off", re.I
)
FAILED = "failed"  # fetch gave up; left out of the pages file so a rerun retries it
YTM_91 = re.compile(r"Implicit Yield at Cut-Off Price ([\d.]+|NIL) \(YTM: ([\d.]+)%\)")


def text_of(page: str) -> str:
    page = re.sub(r"(?s)<script.*?</script>|<style.*?</style>", " ", page)
    return html.unescape(re.sub(r"\s+", " ", re.sub(r"<[^>]+>", " ", page)))


def parse(page: str) -> tuple[dt.date, float | None] | None:
    """(release date, 91-day YTM in percent or None) for an auction-result release; None for any other page."""
    t = text_of(page)
    m = TITLE.search(t)
    if not m:
        return None
    day = dt.datetime.strptime(m.group(1), "%b %d, %Y").date()
    y = YTM_91.search(t, m.end())
    return day, (float(y.group(2)) if y else None)


def fetch_one(session: requests.Session, prid: int, pause: float) -> tuple[int, tuple | None]:
    for attempt in range(4):
        try:
            resp = session.get(URL.format(prid=prid), headers=HEADERS, timeout=30)
            if resp.status_code == 200:
                time.sleep(pause)
                return prid, parse(resp.text)
        except (requests.Timeout, requests.ConnectionError):
            pass
        time.sleep(2.0 * 2**attempt)
    return prid, FAILED


def load_pages(path: Path = PAGES) -> pl.DataFrame:
    if not path.exists():
        return pl.DataFrame(schema={"prid": pl.Int64, "date": pl.Date, "yield_pct": pl.Float64, "auction": pl.Boolean})
    return pl.read_csv(path, schema={"prid": pl.Int64, "date": pl.Date, "yield_pct": pl.Float64, "auction": pl.Boolean})


def scan(lo: int, hi: int, path: Path = PAGES, workers: int = 4, pause: float = 0.5) -> pl.DataFrame:
    """Fetch every prid in [lo, hi] not yet in `path`, saving every 250; return all pages.

    Prids that still fail after retries are not saved, so rerunning the same range picks them up.
    """
    seen = set(load_pages(path)["prid"].to_list())
    todo = [p for p in range(lo, hi + 1) if p not in seen]
    rows, failed = [], 0

    def save() -> pl.DataFrame:
        old = load_pages(path)
        out = pl.concat([old, pl.DataFrame(rows, schema=old.schema)]).sort("prid")
        path.parent.mkdir(parents=True, exist_ok=True)
        out.write_csv(path)
        rows.clear()
        return out

    with ThreadPoolExecutor(workers) as ex:
        sessions = [requests.Session() for _ in range(workers)]
        for i, (prid, got) in enumerate(ex.map(lambda a: fetch_one(sessions[a[0] % workers], a[1], pause), enumerate(todo))):
            if got == FAILED:
                failed += 1
            else:
                day, y = got if got else (None, None)
                rows.append({"prid": prid, "date": day, "yield_pct": y, "auction": got is not None})
            if (i + 1) % 250 == 0:
                save()
                print(f"{i + 1}/{len(todo)} ({failed} failed)", flush=True)
    if failed:
        print(f"{failed} prids failed; rerun the same range to retry them", flush=True)
    return save()


def to_rates_csv(pages: pl.DataFrame, path: Path = rates.PATH) -> pl.DataFrame:
    """rates.py's `date,yield_pct` file from the auction pages (one row per date, latest prid wins)."""
    df = (
        pages.filter(pl.col("auction") & pl.col("yield_pct").is_not_null())
        .sort("prid").unique("date", keep="last").sort("date")
        .select(pl.col("date").dt.strftime("%Y-%m-%d"), "yield_pct")
    )
    path.parent.mkdir(parents=True, exist_ok=True)
    df.write_csv(path)
    return df


def main() -> None:
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--lo", type=int, required=True, help="first press-release id to scan")
    ap.add_argument("--hi", type=int, required=True, help="last press-release id to scan")
    ap.add_argument("--workers", type=int, default=4)
    ap.add_argument("--pause", type=float, default=0.5, help="seconds each worker waits between requests")
    args = ap.parse_args()
    pages = scan(args.lo, args.hi, workers=args.workers, pause=args.pause)
    df = to_rates_csv(pages)
    print(f"{pages.height} pages scanned, {df.height} auctions, {df['date'].min()} to {df['date'].max()} -> {rates.PATH}")


if __name__ == "__main__":
    main()
