import datetime as dt

import numpy as np
import polars as pl
import pytest

gp = pytest.importorskip("gp_quant")
import gpdata  # noqa: E402

EXPIRY = dt.date(2026, 10, 27)
F, R = 25_012.5, 0.065


def synthetic_options(day=EXPIRY, start=dt.time(9, 15), minutes=375, f=F) -> pl.DataFrame:
    """Black-76 closes for a 41-strike chain, one candle per minute."""
    t0 = dt.datetime.combine(day, start)
    strikes = np.arange(24_000.0, 26_001.0, 50.0)
    rows = []
    for m in range(minutes):
        ts = t0 + dt.timedelta(minutes=m)
        t = gp.t_years(gp.minutes_to_expiry(ts.hour * 60 + ts.minute + 1, 0))
        ones = np.ones_like(strikes)
        sigma = 0.12 + 0.6 * ((strikes - f) / f) ** 2
        for kind, is_call in (("CE", True), ("PE", False)):
            px = gp.price(ones * f, strikes, ones * t, ones * R, sigma, np.full(strikes.shape, is_call))
            for k, p in zip(strikes, px):
                rows.append((ts, f"K{k}{kind}", kind, k, p, p, p, p, 10, 1_000.0 + m, 75))
    return pl.DataFrame(
        rows,
        schema=["ts", "instrument_key", "opt_type", "strike", "open", "high", "low", "close", "volume", "oi", "lot_size"],
        orient="row",
    )


def test_known_at_excludes_open_candle():
    opts = synthetic_options(minutes=10)
    ts = dt.datetime.combine(EXPIRY, dt.time(9, 20))
    assert gpdata.known_at(opts, ts)["ts"].max() == dt.datetime.combine(EXPIRY, dt.time(9, 19))


def test_chain_at_as_of_oi_and_staleness():
    opts = synthetic_options(minutes=10)
    ch = gpdata.chain_at(opts, dt.datetime.combine(EXPIRY, dt.time(9, 20)))
    assert len(ch["strike"]) == 41
    assert np.all(ch["call_oi"] == 1_004.0)
    stale = gpdata.chain_at(opts, dt.datetime.combine(EXPIRY, dt.time(11, 0)))
    assert np.all(stale["call_px"] == 0.0) and np.all(stale["call_oi"] == 1_009.0)


def test_sessions_and_time():
    days = [dt.date(2026, 10, d) for d in (22, 23, 26, 27)]
    assert gpdata.sessions_after(dt.date(2026, 10, 22), EXPIRY, days) == 3
    ts = dt.datetime.combine(EXPIRY, dt.time(14, 0))
    assert gpdata.t_years_at(ts, EXPIRY, days) == pytest.approx(90 / (252 * 375))


def test_analyze_at_recovers_forward():
    opts = synthetic_options(start=dt.time(13, 30), minutes=31)
    out = gpdata.analyze_at(opts, dt.datetime.combine(EXPIRY, dt.time(14, 0)), EXPIRY, R, [gp.S0], [EXPIRY])
    assert out is not None
    assert out["forward"] == pytest.approx(F, abs=0.5)
