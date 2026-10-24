"""NSE participant-wise open interest (Client, DII, FII, Pro, TOTAL) and the S1/S2 weights built from it.

Source: https://nsearchives.nseindia.com/content/nsccl/fao_participant_oi_DDMMYYYY.csv (needs a browser
User-Agent). One file per trading day; a 404 means no trading that day. Files are cached as-is in
`research/cache/participant_oi/`.

Look-ahead rule: day d's file is published after the close, so `weights_for(day)` only uses files dated
strictly before `day`, and refuses a file more than `MAX_AGE` old.

Units: number of contracts, as published. Numeric columns are snake_cased with spaces stripped
(`Option Index Call Long` -> `option_index_call_long`).
"""

from __future__ import annotations

import argparse
import datetime as dt
import io
import re
import time
from collections.abc import Callable, Iterable
from pathlib import Path

import polars as pl
import requests

URL = "https://nsearchives.nseindia.com/content/nsccl/fao_participant_oi_{stamp}.csv"
CACHE_DIR = Path(__file__).resolve().parent / "cache" / "participant_oi"
HEADERS = {
    "User-Agent": "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 "
    "(KHTML, like Gecko) Chrome/124.0 Safari/537.36",
    "Accept": "text/csv,text/plain,*/*",
}
MAX_AGE = dt.timedelta(days=7)
CLIENT_TYPES = ("Client", "DII", "FII", "Pro", "TOTAL")
OPTION_INDEX = ("option_index_call_long", "option_index_call_short", "option_index_put_long", "option_index_put_short")
TITLE_DATE = re.compile(r"as on ([A-Za-z]{3} \d{1,2}, \d{4})")


def stamp(day: dt.date) -> str:
    return day.strftime("%d%m%Y")


def path_for(day: dt.date, cache_dir: Path = CACHE_DIR) -> Path:
    return cache_dir / f"fao_participant_oi_{stamp(day)}.csv"


def weekdays(start: dt.date, end: dt.date) -> list[dt.date]:
    n = (end - start).days + 1
    return [d for d in (start + dt.timedelta(days=i) for i in range(max(n, 0))) if d.weekday() < 5]


def get(
    session: requests.Session, url: str, retries: int = 4, backoff: float = 2.0, timeout: float = 30.0
) -> requests.Response | None:
    """GET with retries on 429/5xx and network errors. None on 404."""
    for attempt in range(retries + 1):
        last = attempt == retries
        try:
            resp = session.get(url, headers=HEADERS, timeout=timeout)
        except (requests.Timeout, requests.ConnectionError):
            if last:
                raise
        else:
            if resp.status_code == 404:
                return None
            if resp.status_code != 429 and resp.status_code < 500:
                resp.raise_for_status()
                return resp
            if last:
                resp.raise_for_status()
        time.sleep(backoff * 2**attempt)
    raise AssertionError("unreachable")


def download(
    days: Iterable[dt.date],
    url_for: Callable[[dt.date], str],
    dest_for: Callable[[dt.date], Path],
    valid: Callable[[bytes], bool] = lambda body: True,
    session: requests.Session | None = None,
    pause: float = 1.0,
) -> dict[str, list[dt.date]]:
    """Fetch one archive file per day into `dest_for(day)`. Skips cached files, sleeps `pause` between requests."""
    session = session or requests.Session()
    out: dict[str, list[dt.date]] = {"downloaded": [], "cached": [], "missing": [], "invalid": []}
    first = True
    for day in days:
        dest = dest_for(day)
        if dest.exists():
            out["cached"].append(day)
            continue
        if not first:
            time.sleep(pause)
        first = False
        resp = get(session, url_for(day))
        if resp is None:
            out["missing"].append(day)
        elif not valid(resp.content):
            out["invalid"].append(day)
        else:
            dest.parent.mkdir(parents=True, exist_ok=True)
            tmp = dest.with_suffix(dest.suffix + ".tmp")
            tmp.write_bytes(resp.content)
            tmp.replace(dest)
            out["downloaded"].append(day)
    return out


def snake(name: str) -> str:
    return re.sub(r"[^a-z0-9]+", "_", name.strip().lower()).strip("_")


def looks_valid(body: bytes) -> bool:
    return b"Client Type" in body[:2048]


def parse(text: str, day: dt.date | None = None) -> pl.DataFrame:
    """One file -> rows Client/DII/FII/Pro/TOTAL. Date from the title line, else `day`."""
    lines = text.splitlines()
    m = TITLE_DATE.search(lines[0]) if lines else None
    date = dt.datetime.strptime(m.group(1), "%b %d, %Y").date() if m else day
    if date is None:
        raise ValueError("no date in title and none given")
    start = next(i for i, ln in enumerate(lines) if ln.startswith("Client Type"))
    raw = pl.read_csv(io.StringIO("\n".join(lines[start:])), infer_schema=False)
    raw = raw.rename({c: snake(c) for c in raw.columns})
    raw = raw.select([c for c in raw.columns if c])
    raw = raw.with_columns(pl.col("client_type").str.strip_chars()).filter(pl.col("client_type").is_in(CLIENT_TYPES))
    nums = [c for c in raw.columns if c != "client_type"]
    return raw.select(
        pl.lit(date).alias("date"),
        "client_type",
        *[pl.col(c).str.strip_chars().str.replace_all(",", "").cast(pl.Int64).alias(c) for c in nums],
    )


def load(cache_dir: Path = CACHE_DIR) -> pl.DataFrame:
    """All cached files as one tidy frame sorted by (date, client_type)."""
    frames = []
    for p in sorted(Path(cache_dir).glob("fao_participant_oi_*.csv")):
        day = dt.datetime.strptime(p.stem.rsplit("_", 1)[1], "%d%m%Y").date()
        frames.append(parse(p.read_text(encoding="utf-8", errors="replace"), day))
    if not frames:
        raise FileNotFoundError(f"no participant OI files in {cache_dir}")
    return pl.concat(frames, how="diagonal").sort("date", "client_type")


def option_index(df: pl.DataFrame, client_type: str) -> tuple[float, float, float, float]:
    row = df.filter(pl.col("client_type") == client_type).select(OPTION_INDEX).row(0)
    return tuple(float(x) for x in row)


def latest_before(day: dt.date, df: pl.DataFrame) -> dt.date:
    """The date of the latest file dated strictly before `day`; LookupError if none or older than MAX_AGE."""
    prior = df.filter(pl.col("date") < day)
    if prior.is_empty():
        raise LookupError(f"no participant OI before {day}")
    latest = prior["date"].max()
    if day - latest > MAX_AGE:
        raise LookupError(f"latest participant OI before {day} is {latest}, older than {MAX_AGE.days} days")
    return latest


def weights_for(day: dt.date, df: pl.DataFrame) -> dict[str, tuple[float, float]]:
    """S0, S1, S2 weights for trading on `day`, from the latest file dated strictly before `day`."""
    import gp_quant as gp  # here, so the download/parse helpers import without the extension

    latest = latest_before(day, df)
    snap = df.filter(pl.col("date") == latest)
    s1 = gp.weights_s1(option_index(snap, "FII"), option_index(snap, "Pro"), option_index(snap, "TOTAL"))
    s2 = gp.weights_s2(option_index(snap, "Client"), option_index(snap, "TOTAL"))
    return {"S0": (1.0, -1.0), "S1": (float(s1[0]), float(s1[1])), "S2": (float(s2[0]), float(s2[1]))}


def main() -> None:
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--start", type=dt.date.fromisoformat, required=True)
    ap.add_argument("--end", type=dt.date.fromisoformat, default=dt.date.today())
    ap.add_argument("--out", type=Path, default=CACHE_DIR)
    ap.add_argument("--pause", type=float, default=1.0, help="seconds between requests (>= 1)")
    args = ap.parse_args()
    res = download(
        weekdays(args.start, args.end),
        lambda d: URL.format(stamp=stamp(d)),
        lambda d: path_for(d, args.out),
        valid=looks_valid,
        pause=max(args.pause, 1.0),
    )
    print(" ".join(f"{k}={len(v)}" for k, v in res.items()))
    for k in ("missing", "invalid"):
        if res[k]:
            print(f"{k}: " + ", ".join(d.isoformat() for d in res[k]))


if __name__ == "__main__":
    main()
