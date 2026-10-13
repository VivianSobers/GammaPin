import datetime as dt
import math
from pathlib import Path

import pytest

import rates

FIXTURE = Path(__file__).parent / "fixtures" / "tbill_91d_sample.csv"


def test_continuous_matches_bill_price():
    y = 5.5
    price = 100 / (1 + y / 100 * 91 / 365)
    r = rates.continuous(y)
    assert 100 * math.exp(-r * 91 / 365) == pytest.approx(price, rel=1e-12)
    assert r < y / 100


def test_load_normalises_header_and_dates():
    df = rates.load(FIXTURE)
    assert df.columns == ["date", "yield_pct", "r"]
    assert df["date"].to_list() == [dt.date(2026, 9, 25), dt.date(2026, 10, 1), dt.date(2026, 10, 8)]
    assert df["r"][2] == pytest.approx(rates.continuous(5.5))


def test_rate_on_uses_latest_auction_before_day():
    df = rates.load(FIXTURE)
    assert rates.rate_on(dt.date(2026, 10, 9), df) == pytest.approx(rates.continuous(5.5))
    # same-day auction is not yet known at the decision time
    assert rates.rate_on(dt.date(2026, 10, 8), df) == pytest.approx(rates.continuous(5.4))
    with pytest.raises(LookupError):
        rates.rate_on(dt.date(2026, 9, 25), df)


def test_iso_dates_blank_rows_and_duplicates(tmp_path):
    p = tmp_path / "t.csv"
    p.write_text("date,yield_pct\n2026-10-01,5.40\n,\n2026-10-01,5.45\n2026-10-08, 5.50 \n")
    df = rates.load(p)
    assert df.height == 2 and df["yield_pct"].to_list() == [5.45, 5.5]


def test_bad_header_or_dates(tmp_path):
    p = tmp_path / "t.csv"
    p.write_text("auction,cutoff\n2026-10-01,5.4\n")
    with pytest.raises(ValueError, match="date,yield_pct"):
        rates.load(p)
    p.write_text("date,yield_pct\nOct 1 2026,5.4\n")
    with pytest.raises(ValueError, match="unrecognised"):
        rates.load(p)
