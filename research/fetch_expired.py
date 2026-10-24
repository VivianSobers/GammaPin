"""Fetch 1-minute OHLCV + OI for past Nifty expiries via the Upstox Expired Instruments API (Plus plan).

For each expiry: the option contracts (`/v2/expired-instruments/option/contract`), each contract's
1-minute candles (`/v2/expired-instruments/historical-candle/{key}/1minute/{to}/{from}`) and the Nifty 50
index 1-minute candles (`/v3/historical-candle/NSE_INDEX|Nifty 50/minutes/1/{to}/{from}`), from
expiry - `WINDOW_DAYS` calendar days to expiry. Every request covers <= `MAX_REQUEST_DAYS` days (Upstox
allows 1 month per request for 1-minute data). The expiries endpoint returns only ~6 months of history.

Output, in `research/cache/expired/`, exactly the schema in gpdata.py:
    options_{expiry}.parquet  ts (naive IST, candle start, ms), instrument_key (expired key such as
                              NSE_FO|73507|24-04-2025), opt_type, strike, open, high, low, close, volume,
                              oi (contracts), lot_size
    index_{expiry}.parquet    ts, open, high, low, close
    coverage.csv              one row per contract: rows, first/last candle, rows / expected minutes
    coverage_summary.csv      one row per expiry; `complete` when both the index minutes and the union of
                              option minutes reach `COMPLETE_MIN` of 375 x (sessions in the index data)

`--band-pct X` keeps strikes within +-X% of the index close on the session before expiry day. That
universe is chosen with the expiry-eve close, so earlier days in the window see a strike set picked with
later information (harmless for expiry-day analyses, worth noting for others).

OI units are NOT verified yet, see `OI_IN_UNITS`. Resumable: an expiry whose two parquet files exist is
skipped unless --force. Token: --token-file or env UPSTOX_ACCESS_TOKEN.
"""

from __future__ import annotations

import argparse
import datetime as dt
import os
import time
from pathlib import Path
from urllib.parse import quote

import polars as pl
import requests

import gpdata

BASE_V2 = "https://api.upstox.com/v2"
BASE_V3 = "https://api.upstox.com/v3"
UNDERLYING = "NSE_INDEX|Nifty 50"
OUT = gpdata.EXPIRED
WINDOW_DAYS = 6
MAX_REQUEST_DAYS = 28
SESSION_MINUTES = 375
COMPLETE_MIN = 0.95
RETRY_STATUS = {429, 500, 502, 503, 504}
IST = dt.timezone(dt.timedelta(hours=5, minutes=30))

OI_IN_UNITS = True
"""Assumed: the candle OI field is in units (lot_size x contracts), as Upstox market quotes report it.
When True, oi is divided by lot_size so the parquet holds contracts. UNVERIFIED: check against the first
real response (e.g. an ATM strike's OI vs NSE's bhavcopy open interest in contracts) and flip if wrong."""

OPTION_SCHEMA = {
    "ts": pl.Datetime("ms"), "instrument_key": pl.String, "opt_type": pl.String, "strike": pl.Float64,
    "open": pl.Float64, "high": pl.Float64, "low": pl.Float64, "close": pl.Float64,
    "volume": pl.Int64, "oi": pl.Float64, "lot_size": pl.Int64,
}
CANDLE_SCHEMA = {
    "ts": pl.Datetime("ms"), "open": pl.Float64, "high": pl.Float64, "low": pl.Float64,
    "close": pl.Float64, "volume": pl.Int64, "oi": pl.Float64,
}


class ApiError(RuntimeError):
    def __init__(self, status: int, url: str, body: str):
        super().__init__(f"HTTP {status} for {url}: {body[:300]}")
        self.status = status


def enc(key: str) -> str:
    return quote(key, safe="")


def windows(start: dt.date, end: dt.date, max_days: int = MAX_REQUEST_DAYS) -> list[tuple[dt.date, dt.date]]:
    """Inclusive (from, to) chunks covering start..end, each spanning at most `max_days` days."""
    out, lo = [], start
    while lo <= end:
        hi = min(lo + dt.timedelta(days=max_days - 1), end)
        out.append((lo, hi))
        lo = hi + dt.timedelta(days=1)
    return out


class Client:
    def __init__(
        self, token: str, session: requests.Session | None = None, pause: float = 0.3,
        retries: int = 5, backoff: float = 1.0, timeout: float = 30.0,
    ):
        self.session = session or requests.Session()
        self.headers = {"Authorization": f"Bearer {token}", "Accept": "application/json"}
        self.pause, self.retries, self.backoff, self.timeout = pause, retries, backoff, timeout
        self._last = float("-inf")

    def _throttle(self) -> None:
        wait = self._last + self.pause - time.monotonic()
        if wait > 0:
            time.sleep(wait)
        self._last = time.monotonic()

    def get(self, url: str):
        """`data` of a successful JSON response; retries 429/5xx and network errors with exponential backoff."""
        for attempt in range(self.retries + 1):
            self._throttle()
            try:
                resp = self.session.get(url, headers=self.headers, timeout=self.timeout)
            except (requests.Timeout, requests.ConnectionError):
                if attempt == self.retries:
                    raise
            else:
                if resp.status_code == 200:
                    body = resp.json()
                    if body.get("status") != "success":
                        raise ApiError(resp.status_code, url, str(body))
                    return body["data"]
                if resp.status_code not in RETRY_STATUS or attempt == self.retries:
                    raise ApiError(resp.status_code, url, resp.text)
            time.sleep(self.backoff * 2**attempt)
        raise AssertionError("unreachable")

    def expiries(self, underlying: str = UNDERLYING) -> list[dt.date]:
        data = self.get(f"{BASE_V2}/expired-instruments/expiries?instrument_key={enc(underlying)}")
        return sorted(dt.date.fromisoformat(d) for d in data)

    def contracts(self, expiry: dt.date, underlying: str = UNDERLYING) -> list[dict]:
        url = f"{BASE_V2}/expired-instruments/option/contract?instrument_key={enc(underlying)}&expiry_date={expiry}"
        return self.get(url)

    def option_candles(self, key: str, start: dt.date, end: dt.date) -> list[list]:
        out = []
        for lo, hi in windows(start, end):
            out += self.get(f"{BASE_V2}/expired-instruments/historical-candle/{enc(key)}/1minute/{hi}/{lo}")["candles"]
        return out

    def index_candles(self, start: dt.date, end: dt.date, key: str = UNDERLYING) -> list[list]:
        out = []
        for lo, hi in windows(start, end):
            out += self.get(f"{BASE_V3}/historical-candle/{enc(key)}/minutes/1/{hi}/{lo}")["candles"]
        return out


def ist_naive(stamp: str) -> dt.datetime:
    """ISO timestamp -> naive IST; a stamp without an offset is taken as IST."""
    t = dt.datetime.fromisoformat(stamp)
    return t if t.tzinfo is None else t.astimezone(IST).replace(tzinfo=None)


def candles_frame(candles: list[list]) -> pl.DataFrame:
    """[[ts_iso, o, h, l, c, volume, oi], ...] -> CANDLE_SCHEMA frame, ts naive IST, sorted, one row per ts."""
    if not candles:
        return pl.DataFrame(schema=CANDLE_SCHEMA)
    ts = pl.Series([ist_naive(c[0]) for c in candles], dtype=pl.Datetime("ms"))
    rows = [
        (float(c[1]), float(c[2]), float(c[3]), float(c[4]), int(c[5] or 0), float(c[6] if len(c) > 6 and c[6] is not None else 0))
        for c in candles
    ]
    vals = pl.DataFrame(rows, schema=list(CANDLE_SCHEMA)[1:], orient="row")
    return vals.insert_column(0, ts.alias("ts")).cast(CANDLE_SCHEMA).unique("ts", keep="last").sort("ts")


def option_frame(contract: dict, candles: list[list], oi_in_units: bool = OI_IN_UNITS) -> pl.DataFrame:
    lot = int(contract["lot_size"])
    df = candles_frame(candles)
    oi = pl.col("oi") / lot if oi_in_units else pl.col("oi")
    return df.with_columns(
        pl.lit(contract["instrument_key"]).alias("instrument_key"),
        pl.lit(contract["instrument_type"]).alias("opt_type"),
        pl.lit(float(contract["strike_price"])).alias("strike"),
        oi.alias("oi"),
        pl.lit(lot).alias("lot_size"),
    ).select(list(OPTION_SCHEMA)).cast(OPTION_SCHEMA)


def prior_close(index: pl.DataFrame, expiry: dt.date) -> float | None:
    before = index.filter(pl.col("ts").dt.date() < expiry)
    return None if before.is_empty() else float(before["close"][-1])


def in_band(contracts: list[dict], center: float, pct: float) -> list[dict]:
    half = center * pct / 100
    return [c for c in contracts if abs(float(c["strike_price"]) - center) <= half]


def coverage(
    expiry: dt.date, contracts: list[dict], frames: dict[str, pl.DataFrame], index: pl.DataFrame,
    start: dt.date | None = None,
):
    """(per-contract coverage frame, one-row summary frame) for one expiry.

    Expected sessions are the days the index data itself has, so a whole missing day does not lower
    `coverage`. For comparison the summary also gives `weekdays`, the weekdays in [start, expiry]
    (holidays included; null without `start`). The `complete` rule does not use it.
    """
    sessions = index["ts"].dt.date().n_unique()
    weekdays = None if start is None else sum(
        (start + dt.timedelta(days=i)).weekday() < 5 for i in range((expiry - start).days + 1)
    )
    expected = SESSION_MINUTES * sessions
    per = pl.DataFrame(
        [
            {
                "expiry": expiry, "instrument_key": c["instrument_key"], "strike": float(c["strike_price"]),
                "opt_type": c["instrument_type"], "rows": frames[c["instrument_key"]].height,
                "first_ts": frames[c["instrument_key"]]["ts"].min(), "last_ts": frames[c["instrument_key"]]["ts"].max(),
            }
            for c in contracts
        ],
        schema={"expiry": pl.Date, "instrument_key": pl.String, "strike": pl.Float64, "opt_type": pl.String,
                "rows": pl.Int64, "first_ts": pl.Datetime("ms"), "last_ts": pl.Datetime("ms")},
    ).with_columns((pl.col("rows") / expected if expected else pl.lit(0.0)).alias("coverage"))
    days = index["ts"].dt.date().unique()
    opt_ts = pl.concat([f["ts"] for f in frames.values()]) if frames else pl.Series("ts", [], pl.Datetime("ms"))
    union = opt_ts.filter(opt_ts.dt.date().is_in(days.implode())).n_unique()
    ratio = union / expected if expected else 0.0
    summary = pl.DataFrame([{
        "expiry": expiry, "contracts": len(contracts), "contracts_with_data": int((per["rows"] > 0).sum()),
        "index_minutes": index.height, "sessions": sessions, "weekdays": weekdays, "expected_minutes": expected,
        "union_option_minutes": union, "coverage": ratio,
        "complete": expected > 0 and ratio >= COMPLETE_MIN and index.height >= COMPLETE_MIN * expected,
    }])
    return per, summary


def upsert_csv(path: Path, new: pl.DataFrame, expiry: dt.date) -> None:
    """Replace the rows of `expiry` in a CSV log, keeping the others."""
    new = new.with_columns(pl.all().cast(pl.String))
    if path.exists():
        old = pl.read_csv(path, infer_schema=False).filter(pl.col("expiry") != expiry.isoformat())
        new = pl.concat([old, new], how="diagonal")
    new.sort("expiry").write_csv(path)


def write_parquet(df: pl.DataFrame, path: Path) -> None:
    tmp = path.with_suffix(".tmp")
    df.write_parquet(tmp)
    tmp.replace(path)


def fetch_expiry(
    client: Client, expiry: dt.date, out: Path = OUT, band_pct: float | None = None, days: int = WINDOW_DAYS
) -> pl.DataFrame:
    """Fetch, write and log one expiry. Returns its coverage summary row."""
    start = expiry - dt.timedelta(days=days)
    index = candles_frame(client.index_candles(start, expiry)).select("ts", "open", "high", "low", "close")
    contracts = client.contracts(expiry)
    if band_pct is not None:
        center = prior_close(index, expiry)
        if center is None:
            print(f"{expiry}: no index close before expiry day, keeping all strikes")
        else:
            contracts = in_band(contracts, center, band_pct)
    frames: dict[str, pl.DataFrame] = {}
    for c in contracts:
        try:
            candles = client.option_candles(c["instrument_key"], start, expiry)
        except ApiError as e:
            if e.status not in (400, 404):
                raise
            print(f"{expiry}: {c['instrument_key']} skipped ({e})")
            candles = []
        frames[c["instrument_key"]] = option_frame(c, candles)
    opts = pl.concat(list(frames.values())) if frames else pl.DataFrame(schema=OPTION_SCHEMA)
    out.mkdir(parents=True, exist_ok=True)
    write_parquet(opts.sort("ts", "strike", "opt_type"), out / gpdata.options_path(expiry).name)
    write_parquet(index, out / gpdata.index_path(expiry).name)
    per, summary = coverage(expiry, contracts, frames, index, start)
    upsert_csv(out / "coverage.csv", per, expiry)
    upsert_csv(out / "coverage_summary.csv", summary, expiry)
    return summary


def done(expiry: dt.date, out: Path) -> bool:
    """Both parquets and a coverage_summary.csv row exist (a crash between them refetches the expiry)."""
    if not ((out / gpdata.options_path(expiry).name).exists() and (out / gpdata.index_path(expiry).name).exists()):
        return False
    summary = out / "coverage_summary.csv"
    if not summary.exists():
        return False
    return expiry.isoformat() in pl.read_csv(summary, infer_schema=False)["expiry"].to_list()


def read_token(token_file: Path | None) -> str | None:
    if token_file is not None:
        return token_file.read_text(encoding="utf-8").strip()
    return os.environ.get("UPSTOX_ACCESS_TOKEN")


def run(
    client: Client, out: Path = OUT, start: dt.date | None = None, end: dt.date | None = None,
    only: list[dt.date] | None = None, band_pct: float | None = None, days: int = WINDOW_DAYS, force: bool = False,
) -> list[dt.date]:
    """Fetch every listed expiry in [start, end] (or `only`). Returns the expiries fetched."""
    todo = only or [e for e in client.expiries() if (start is None or e >= start) and (end is None or e <= end)]
    fetched = []
    for e in todo:
        if done(e, out) and not force:
            print(f"{e}: on disk, skipped")
            continue
        s = fetch_expiry(client, e, out, band_pct, days).row(0, named=True)
        print(f"{e}: {s['contracts']} contracts, {s['index_minutes']} index minutes, coverage {s['coverage']:.3f}"
              f"{'' if s['complete'] else ' INCOMPLETE'}")
        fetched.append(e)
    return fetched


def main() -> None:
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--token-file", type=Path)
    ap.add_argument("--out", type=Path, default=OUT)
    ap.add_argument("--start", type=dt.date.fromisoformat, help="first expiry to fetch")
    ap.add_argument("--end", type=dt.date.fromisoformat, help="last expiry to fetch")
    ap.add_argument("--expiry", type=dt.date.fromisoformat, action="append", help="fetch only these (repeatable)")
    ap.add_argument("--band-pct", type=float, help="keep strikes within +-X%% of the expiry-eve index close")
    ap.add_argument("--days", type=int, default=WINDOW_DAYS, help="calendar days before expiry to fetch")
    ap.add_argument("--pause", type=float, default=0.3, help="seconds between requests")
    ap.add_argument("--force", action="store_true", help="refetch expiries already on disk")
    args = ap.parse_args()
    token = read_token(args.token_file)
    if not token:
        ap.error("no token: pass --token-file or set UPSTOX_ACCESS_TOKEN")
    client = Client(token, pause=args.pause)
    run(client, args.out, args.start, args.end, args.expiry, args.band_pct, args.days, args.force)


if __name__ == "__main__":
    main()
