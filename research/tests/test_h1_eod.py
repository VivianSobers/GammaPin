import datetime as dt
import inspect
import json
import math
import sys
import types

import numpy as np
import polars as pl
import pytest

gp = pytest.importorskip("gp_quant")
import bhavcopy  # noqa: E402
import h1_eod  # noqa: E402
import h1_pinning  # noqa: E402
import prereg  # noqa: E402

R = 0.065
LOT = 65
STRIKES = np.arange(24_500.0, 25_501.0, 50.0)
# expiry -> (eve, forward, call-wall strike, put-wall strike, settlement)
CASES = {
    dt.date(2026, 10, 6): (dt.date(2026, 10, 5), 25_012.5, 25_100.0, 24_900.0, 25_090.0),
    dt.date(2026, 10, 13): (dt.date(2026, 10, 12), 24_960.0, 25_000.0, 24_800.0, 24_985.0),
    dt.date(2026, 10, 19): (dt.date(2026, 10, 16), 25_140.0, 25_200.0, 25_000.0, 25_230.0),
}
DAYS = sorted({c[0] for c in CASES.values()} | set(CASES))
S1 = (0.3, -0.1)
S2 = (-1.0, 1.0)


def synthetic_bhavcopy(day, expiry, f, call_wall, put_wall, later_expiry=None) -> pl.DataFrame:
    """One day's Nifty rows: Black-76 closes for `expiry` (T = eve close to expiry close), OI in units."""
    ones = np.ones_like(STRIKES)
    sigma = 0.12 + 0.6 * ((STRIKES - f) / f) ** 2
    t = h1_eod.t_years(day, expiry, DAYS) if day < expiry else gp.t_years(1.0)
    rows = []
    for e in [expiry] + ([later_expiry] if later_expiry else []):
        for kind, is_call, wall, extra in (("CE", True, call_wall, 50_000.0), ("PE", False, put_wall, 20_000.0)):
            px = gp.price(ones * f, STRIKES, ones * t, ones * R, sigma, np.full(STRIKES.shape, is_call))
            for k, p in zip(STRIKES, px):
                oi = (1_000.0 + (extra if k == wall else 0.0)) * LOT
                rows.append((day, e, k, kind, float(p), oi, 10, LOT, f))
    return pl.DataFrame(rows, schema=bhavcopy.SCHEMA, orient="row")


def book() -> dict[dt.date, pl.DataFrame]:
    """Cached bhavcopies: each eve lists its expiry as nearest; each expiry day lists the next expiry."""
    out = {}
    expiries = sorted(CASES)
    for i, e in enumerate(expiries):
        eve, f, cw, pw, _ = CASES[e]
        out[eve] = synthetic_bhavcopy(eve, e, f, cw, pw)
        nxt = expiries[i + 1] if i + 1 < len(expiries) else e + dt.timedelta(days=7)
        out[e] = synthetic_bhavcopy(e, e, f, cw, pw, later_expiry=nxt)
    return out


def loader(b):
    def load(day):
        if day not in b:
            raise LookupError(day)
        return b[day]
    return load


def weights(day):
    return {"S0": (1.0, -1.0), "S1": S1, "S2": S2}


def test_sample_finds_eves_and_respects_window():
    b = book()
    got = h1_eod.sample(sorted(b), loader(b), dt.date(2026, 10, 1), dt.date(2026, 10, 19))
    assert got == {e: c[0] for e, c in CASES.items()}
    narrow = h1_eod.sample(sorted(b), loader(b), dt.date(2026, 10, 7), dt.date(2026, 10, 13))
    assert narrow == {dt.date(2026, 10, 13): dt.date(2026, 10, 12)}


def test_t_years_is_one_session_from_a_friday_eve():
    assert h1_eod.t_years(dt.date(2026, 10, 16), dt.date(2026, 10, 19), DAYS) == pytest.approx(gp.t_years(375.0))


def test_features_cannot_see_settlement():
    assert not any("settle" in p for p in inspect.signature(h1_eod.features).parameters)


def test_features_find_forward_and_walls():
    e = dt.date(2026, 10, 13)
    eve, f, cw, pw, _ = CASES[e]
    row = h1_eod.features(book()[eve], e, eve, R, weights(eve), DAYS)
    assert row["forward"] == pytest.approx(f, abs=0.5)
    assert row["pin_S0"] == cw
    assert row["max_pain"] in STRIKES
    assert row["nearest_100"] == 25_000.0


def test_features_refuse_rows_after_the_eve():
    e = dt.date(2026, 10, 13)
    eve = CASES[e][0]
    later = pl.concat([book()[eve], book()[e]])
    with pytest.raises(ValueError):
        h1_eod.features(later, e, eve, R, weights(eve), DAYS)


def test_build_table_statuses_and_errors():
    b = book()
    eves = {e: c[0] for e, c in CASES.items()}
    missing = dt.date(2026, 10, 19)
    del b[eves[missing]]
    table = h1_eod.build_table(eves, loader(b), weights, lambda d: R, {e: c[4] for e, c in CASES.items()}, DAYS)
    assert table["status"].to_list() == ["ok", "ok", "no_bhavcopy"]
    first = table.row(0, named=True)
    assert first["eve"] == dt.date(2026, 10, 5) and first["decision_ts"] == dt.datetime(2026, 10, 5, 15, 30)
    assert first["abs_err_pin_S0"] == pytest.approx(abs(25_090.0 - 25_100.0))
    assert math.isnan(table.row(2, named=True)["abs_err_pin_S0"])


def test_main_train_split_end_to_end(tmp_path, monkeypatch):
    b = book()
    monkeypatch.setattr(bhavcopy, "cached_days", lambda: sorted(b))
    monkeypatch.setattr(bhavcopy, "load", loader(b))
    toml = tmp_path / "prereg.toml"
    toml.write_text(
        '[split]\ntrain_fraction = 0.7\nlast_train_expiry = "2026-10-13"\n'
        '[bootstrap]\nresamples = 200\nblock_length = 1\nseed = 1\n'
        '[h1]\ndecision_time = "14:00"\n'
        '[h1_eod]\nsample_start = "2026-10-01"\nsample_end = "2026-10-19"\n'
    )
    monkeypatch.setattr(prereg, "PATH", toml)
    fakes = {
        "participant_oi": types.SimpleNamespace(load=lambda: None, weights_for=lambda d, df: weights(d)),
        "rates": types.SimpleNamespace(load=lambda: None, rate_on=lambda d, df: R),
        "settlements": types.SimpleNamespace(load=lambda: {e: c[4] for e, c in CASES.items()}),
    }
    for name, mod in fakes.items():
        monkeypatch.setitem(sys.modules, name, mod)
    h1_eod.main(["--split", "train", "--out", str(tmp_path)])
    table = pl.read_csv(tmp_path / "h1_eod_train.csv")
    assert table.height == 2
    summary = json.loads((tmp_path / "h1_eod_train_summary.json").read_text())
    assert summary["source"] == "bhavcopy_eod" and summary["n_sample"] == 3
    assert set(summary["mae"]) == set(h1_pinning.METHODS)


def test_main_test_split_requires_freeze(monkeypatch):
    monkeypatch.setattr(prereg, "git_tags", lambda pattern: "")
    with pytest.raises(SystemExit):
        h1_eod.main(["--split", "test"])
