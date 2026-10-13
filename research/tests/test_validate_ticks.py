import datetime as dt
import sys
import types

import numpy as np
import polars as pl
import pytest

gp = pytest.importorskip("gp_quant")
import validate_ticks as vt  # noqa: E402

DAY = dt.date(2026, 10, 27)
F, R, LOT = 25_012.5, 0.065, 75
STRIKES = np.arange(24_000.0, 26_001.0, 50.0)
START, MINUTES = dt.time(13, 30), 61


def world(minutes=MINUTES, start=START):
    """Black-76 prices per (minute, kind), priced for the end of each minute, plus OI per kind."""
    t0 = dt.datetime.combine(DAY, start)
    ones = np.ones_like(STRIKES)
    sigma = 0.12 + 0.6 * ((STRIKES - F) / F) ** 2
    oi = {
        "CE": 2_000.0 + 8_000.0 * np.exp(-(((STRIKES - 25_150.0) / 200.0) ** 2)),
        "PE": 2_000.0 + 8_000.0 * np.exp(-(((STRIKES - 24_850.0) / 200.0) ** 2)),
    }
    for m in range(minutes):
        ts = t0 + dt.timedelta(minutes=m)
        t = gp.t_years(gp.minutes_to_expiry(ts.hour * 60 + ts.minute + 1, 0))
        for kind in ("CE", "PE"):
            px = gp.price(ones * F, STRIKES, ones * t, ones * R, sigma, np.full(STRIKES.shape, kind == "CE"))
            yield ts, kind, px, oi[kind]


def candles(oi_scale=None) -> pl.DataFrame:
    rows = []
    for ts, kind, px, oi in world():
        if oi_scale is not None and kind == "CE":
            oi = oi * oi_scale
        for k, p, o in zip(STRIKES, px, oi):
            rows.append((ts, f"K{k}{kind}", kind, k, p, p, p, p, 10, o, LOT))
    return pl.DataFrame(
        rows,
        schema=["ts", "instrument_key", "opt_type", "strike", "open", "high", "low", "close", "volume", "oi", "lot_size"],
        orient="row",
    )


def c1_events() -> pl.DataFrame:
    """C1 rows: one event per contract per minute at second 59.5, L1 quote = price -/+ 0.05."""
    rows = []
    for ts, kind, px, oi in world():
        ltt = vt.ist_ms(ts + dt.timedelta(seconds=59.5))
        for i, (k, p, o) in enumerate(zip(STRIKES, px, oi)):
            key = f"NSE_FO|{40_000 + 2 * i + (kind == 'PE')}"
            bid, ask = [p - 0.05, 0, 0, 0, 0], [p + 0.05, 0, 0, 0, 0]
            rows.append((key, "NIFTY", kind, DAY, k, LOT, ltt, ltt * 1_000_000 + 7, p, 75, 1_000, int(o),
                         bid, [75, 0, 0, 0, 0], ask, [75, 0, 0, 0, 0], 3))
        rows.append(("NSE_INDEX|Nifty 50", "NIFTY", "IDX", None, None, 0, ltt, ltt * 1_000_000, F, 0, 0, 0,
                     [0.0] * 5, [0] * 5, [0.0] * 5, [0] * 5, 1))
    return pl.DataFrame(
        rows,
        schema={
            "instrument_key": pl.Utf8, "underlying": pl.Utf8, "kind": pl.Utf8, "expiry": pl.Date,
            "strike": pl.Float64, "lot_size": pl.Int32, "ltt_ms": pl.Int64, "recv_ns": pl.Int64,
            "ltp": pl.Float64, "ltq": pl.Int64, "cum_volume": pl.Int64, "oi": pl.Int64,
            "bid_px": pl.Array(pl.Float64, 5), "bid_qty": pl.Array(pl.Int64, 5),
            "ask_px": pl.Array(pl.Float64, 5), "ask_qty": pl.Array(pl.Int64, 5), "src_mask": pl.UInt8,
        },
        orient="row",
    )


def write_c1(root, events):
    for (key,), part in events.group_by(["instrument_key"]):
        d = root / f"date={DAY.isoformat()}" / f"instrument={key.replace('|', '_')}"
        d.mkdir(parents=True)
        part.sort(["ltt_ms", "recv_ns"]).write_parquet(d / "part-0.parquet")


@pytest.fixture(scope="module")
def events():
    return c1_events()


@pytest.fixture(scope="module")
def merged(tmp_path_factory, events):
    root = tmp_path_factory.mktemp("merged")
    write_c1(root, events)
    return root


def at(h, m):
    return dt.datetime.combine(DAY, dt.time(h, m))


def test_ist_ms():
    assert vt.ist_ms(dt.datetime(1970, 1, 1, 5, 30)) == 0


def test_load_events_roundtrip(merged, events):
    loaded = vt.load_events(merged, DAY)
    assert loaded.height == events.height
    assert loaded["ltt_ms"].is_sorted()
    with pytest.raises(FileNotFoundError):
        vt.load_events(merged, DAY + dt.timedelta(days=1))


def test_tick_chain_at_is_as_of_ltt(events):
    ch = vt.tick_chain_at(events, DAY, at(14, 0))
    assert len(ch["strike"]) == len(STRIKES) and ch["lot"] == LOT
    assert np.allclose(ch["call_ask"] - ch["call_bid"], 0.1)
    expected = {kind: px for ts, kind, px, _ in vt_world_at(at(13, 59))}
    assert np.allclose(0.5 * (ch["call_bid"] + ch["call_ask"]), expected["CE"])
    assert np.allclose(0.5 * (ch["put_bid"] + ch["put_ask"]), expected["PE"])
    assert len(vt.tick_chain_at(events, DAY, at(13, 30))["strike"]) == 0
    assert len(vt.tick_chain_at(events, DAY + dt.timedelta(days=7), at(14, 0))["strike"]) == 0


def vt_world_at(ts):
    return [w for w in world() if w[0] == ts]


def test_gex_agreement_partial_overlap():
    g = vt.gex_agreement(np.array([1.0, 2.0, 3.0]), np.array([1.0, 2.0, 4.0]),
                         np.array([2.0, 3.0, 5.0]), np.array([2.0, 4.0, 9.0]))
    assert g["n_common"] == 2 and g["gex_corr"] == pytest.approx(1.0) and g["gex_rel_l1"] == 0.0
    assert vt.gex_agreement(np.array([1.0]), np.array([1.0]), np.array([1.0]), np.array([2.0]))["gex_corr"] is None


def test_compare_handles_missing_side():
    rows = vt.compare(None, {"quotes_used": 3, "quotes_total": 4}, ["S0"])
    assert rows == [{"convention": "S0", "tick_ok": False, "candle_ok": True,
                     "quotes_used_candle": 3, "quotes_total_candle": 4}]


def run(events, opts, conventions=None):
    weights = conventions or {"S0": gp.S0}
    times = vt.time_grid(DAY, dt.time(13, 45), dt.time(14, 15), 15)
    return vt.validate(events, opts, DAY, times, R, weights, [DAY])


def test_consistent_world_agrees(events):
    df = run(events, candles(), {"S0": gp.S0, "SX": (0.5, -1.0)})
    assert df.height == 6 and df["tick_ok"].all() and df["candle_ok"].all()
    assert df["forward_diff"].abs().max() < 0.5
    assert df["gex_corr"].min() > 0.99
    assert df["gex_rel_l1"].max() < 0.05
    assert df["pin_equal"].all()
    assert df["net_gex_ratio"].is_between(0.95, 1.05).all()
    assert (df["n_common"] == len(STRIKES)).all()


def test_wrong_candle_oi_is_detected(events):
    clean = run(events, candles())
    scale = np.where(np.abs(STRIKES - 25_150.0) <= 100.0, 0.1, 1.0)
    bad = run(events, candles(oi_scale=scale))
    assert (bad["forward_diff"].abs() < 0.5).all()
    assert bad["gex_rel_l1"].min() > 0.2 > clean["gex_rel_l1"].max()
    assert bad["gex_corr"].max() < clean["gex_corr"].min() - 0.05
    assert not bad["pin_equal"].all()


def test_summary_highlights_decision_row(events):
    df = run(events, candles())
    md = vt.summarize(df, DAY, DAY)
    assert "## Decision time 14:00 IST (H1)" in md and "| **S0** |" in md
    assert "Worst cases" in md and "| S0 | 3/3 |" in md
    assert "not on the comparison grid" in vt.summarize(df, DAY, DAY, dt.time(10, 0))


def test_main_writes_outputs(tmp_path, merged, monkeypatch):
    cpath = tmp_path / "candles.parquet"
    candles().write_parquet(cpath)
    out = tmp_path / "out"
    monkeypatch.setattr(sys, "argv", [
        "validate_ticks.py", "--date", DAY.isoformat(), "--expiry", DAY.isoformat(),
        "--merged-dir", str(merged), "--candles", str(cpath), "--r", str(R),
        "--start", "13:45", "--end", "14:15", "--out", str(out),
    ])
    vt.main()
    df = pl.read_csv(out / f"validate_{DAY.isoformat()}.csv")
    assert df.height == 3 and df["gex_corr"].min() > 0.99
    assert "14:00" in (out / f"validate_{DAY.isoformat()}.md").read_text(encoding="utf-8")


def test_resolve_weights(monkeypatch):
    assert vt.resolve_weights(["S0"], DAY) == {"S0": (1.0, -1.0)}
    table = {"S0": (1.0, -1.0), "S1": (0.3, -0.2), "S2": (-0.1, 0.4)}
    fake = types.SimpleNamespace(load=lambda: "df", weights_for=lambda day, df: table)
    monkeypatch.setitem(sys.modules, "participant_oi", fake)
    assert vt.resolve_weights(["S0", "S2"], DAY) == {"S0": (1.0, -1.0), "S2": (-0.1, 0.4)}
    with pytest.raises(ValueError, match="unknown"):
        vt.resolve_weights(["S9"], DAY)
