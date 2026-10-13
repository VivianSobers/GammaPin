import datetime as dt
import gzip
import json
from pathlib import Path

import polars as pl
import pytest

import instruments as ins

FIXTURE = Path(__file__).parent / "fixtures" / "instruments_nse.json"
WEEKLY, MONTHLY = dt.date(2026, 10, 20), dt.date(2026, 10, 27)


def test_nifty_options_from_file():
    opts = ins.nifty_options(FIXTURE)
    assert opts.columns == ["instrument_key", "trading_symbol", "expiry", "strike", "opt_type", "lot_size", "tick_size", "weekly"]
    assert opts.height == 5  # no FUT, BANKNIFTY, NIFTYNXT50, index or equity rows
    assert set(opts["opt_type"]) == {"CE", "PE"}
    assert opts["tick_size"].unique().to_list() == [0.05]
    row = opts.filter(pl.col("instrument_key") == "NSE_FO|90003").row(0, named=True)
    assert row["expiry"] == WEEKLY and row["strike"] == 25050.0 and row["weekly"] is True


def test_gzip_and_dataframe_inputs_agree(tmp_path):
    records = json.loads(FIXTURE.read_text())
    gz = tmp_path / "NSE.json.gz"
    with gzip.open(gz, "wt", encoding="utf-8") as f:
        json.dump(records, f)
    from_gz = ins.nifty_options(gz)
    from_df = ins.nifty_options(pl.DataFrame(records, infer_schema_length=None))
    assert from_gz.equals(ins.nifty_options(FIXTURE))
    assert from_df.equals(from_gz)


def test_expiries_and_lot_size():
    opts = ins.nifty_options(FIXTURE)
    assert ins.expiries(opts) == [WEEKLY, MONTHLY]
    assert ins.lot_size(MONTHLY, opts) == 65
    with pytest.raises(LookupError):
        ins.lot_size(dt.date(2026, 10, 21), opts)
    mixed = pl.concat([opts, opts.head(1).with_columns(pl.lit(75, pl.Int64).alias("lot_size"))])
    with pytest.raises(ValueError):
        ins.lot_size(WEEKLY, mixed)
