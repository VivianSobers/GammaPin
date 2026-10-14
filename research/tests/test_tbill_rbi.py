import datetime as dt
from pathlib import Path

import polars as pl
import pytest

import rates
import tbill_rbi

FIXTURE = Path(__file__).parent / "fixtures" / "rbi_tbill_release.html"


def test_parse_auction_release_takes_the_91_day_ytm():
    assert tbill_rbi.parse(FIXTURE.read_text(encoding="utf-8")) == (dt.date(2026, 6, 24), 5.2476)


def test_parse_other_release_is_none():
    assert tbill_rbi.parse("<td>Date : Jun 24, 2026</td><td>Money Supply for the fortnight</td>") is None


def test_parse_auction_without_91_day_yield():
    page = "Date : Jun 03, 2026 91-Day, 182-Day and 364-Day T-Bill Auction Result: Cut-off nothing accepted"
    assert tbill_rbi.parse(page) == (dt.date(2026, 6, 3), None)


def test_to_rates_csv_is_readable_by_rates(tmp_path):
    pages = pl.DataFrame(
        {
            "prid": [1, 2, 3, 4],
            "date": [dt.date(2026, 6, 17), None, dt.date(2026, 6, 24), dt.date(2026, 6, 10)],
            "yield_pct": [5.2603, None, 5.2476, None],
            "auction": [True, False, True, True],
        }
    )
    out = tmp_path / "tbill.csv"
    tbill_rbi.to_rates_csv(pages, out)
    df = rates.load(out)
    assert df["date"].to_list() == [dt.date(2026, 6, 17), dt.date(2026, 6, 24)]
    assert rates.rate_on(dt.date(2026, 6, 24), df) == pytest.approx(rates.continuous(5.2603))
