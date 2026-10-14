import datetime as dt
import io
import zipfile
from pathlib import Path

import pytest

import bhavcopy as bc

FIXTURE = Path(__file__).parent / "fixtures" / "bhavcopy_fo_sample.csv"
DAY = dt.date(2026, 10, 13)
EXPIRY = dt.date(2026, 10, 19)


def sample():
    return bc.parse(FIXTURE.read_text())


def test_parse_keeps_only_nifty_index_options():
    df = sample()
    assert df.height == 5
    assert set(df["opt_type"]) == {"CE", "PE"}
    assert set(df["date"]) == {DAY} and set(df["expiry"]) == {EXPIRY}
    row = df.filter((df["strike"] == 22700.0) & (df["opt_type"] == "CE")).row(0, named=True)
    assert row["close"] == 207.20 and row["oi_units"] == 3922620 and row["lot_size"] == 65
    assert row["volume"] == 496918 and row["underlying"] == 22721.95


def test_chain_converts_oi_to_contracts_and_drops_untraded_prices():
    ch = bc.chain(sample(), EXPIRY)
    assert ch["strike"].tolist() == [20050.0, 22600.0, 22700.0]
    assert ch["call_px"].tolist() == [0.0, 280.90, 207.20]  # 20050 CE did not trade
    assert ch["put_px"].tolist() == [0.0, 75.25, 105.05]    # no 20050 PE row
    assert ch["call_oi"][2] == pytest.approx(3922620 / 65)
    assert ch["put_oi"][1] == pytest.approx(4665440 / 65)
    assert ch["lot_size"] == 65


def test_chain_of_unknown_expiry_is_empty():
    assert bc.chain(sample(), dt.date(2026, 10, 27)) == {}


def test_read_zip_and_cached_days(tmp_path):
    path = bc.path_for(DAY, tmp_path)
    with zipfile.ZipFile(path, "w") as z:
        z.writestr(path.name.removesuffix(".zip"), FIXTURE.read_text())
    assert bc.read(path).height == 5
    assert bc.cached_days(tmp_path) == [DAY]
    assert bc.load(DAY, tmp_path).height == 5
    with pytest.raises(LookupError):
        bc.load(dt.date(2026, 10, 14), tmp_path)


def test_eves_take_the_last_trading_day_before_each_expiry():
    days = [dt.date(2026, 10, 9), dt.date(2026, 10, 12), dt.date(2026, 10, 13), dt.date(2026, 10, 16)]
    got = bc.eves(days, [dt.date(2026, 10, 13), dt.date(2026, 10, 19), dt.date(2026, 10, 1)])
    assert got == {dt.date(2026, 10, 13): dt.date(2026, 10, 12), dt.date(2026, 10, 19): dt.date(2026, 10, 16)}


def test_looks_valid_and_url():
    buf = io.BytesIO()
    with zipfile.ZipFile(buf, "w") as z:
        z.writestr("x.csv", "a")
    assert bc.looks_valid(buf.getvalue()) and not bc.looks_valid(b"<html>")
    assert bc.URL.format(stamp=bc.stamp(DAY)).endswith("BhavCopy_NSE_FO_0_0_0_20261013_F_0000.csv.zip")


def test_fetch_skips_days_before_udiff(tmp_path):
    class NoSession:
        def get(self, *a, **k):
            raise AssertionError("no request expected")

    assert bc.fetch([dt.date(2024, 7, 5)], tmp_path, session=NoSession()) == {
        "downloaded": [], "cached": [], "missing": [], "invalid": []
    }
