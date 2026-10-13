import datetime as dt
import json
import math
import sys
import types

import numpy as np
import polars as pl
import pytest

gp = pytest.importorskip("gp_quant")
import gpdata  # noqa: E402
import h2_vol  # noqa: E402

F, R = 25_000.0, 0.065
OPEN = dt.time(9, 15)
WEIGHTS = {"S0": (1.0, -1.0), "S1": (0.4, -0.2), "S2": (-0.3, 0.5)}


def synthetic_index(day_sigmas: dict[dt.date, np.ndarray], seed: int = 0, p0: float = F) -> pl.DataFrame:
    """1-minute candles 09:15-15:29; minute returns of bar k are N(0, sigma_k / sqrt(5)); random overnight gaps."""
    rng = np.random.default_rng(seed)
    rows = []
    for day, sig in sorted(day_sigmas.items()):
        rets = rng.normal(0.0, 1.0, len(sig) * 5) * np.repeat(sig, 5) / math.sqrt(5)
        px = p0 * math.exp(rng.normal(0, 0.01))
        t0 = dt.datetime.combine(day, OPEN)
        for m, r in enumerate(rets):
            close = px * math.exp(r)
            rows.append((t0 + dt.timedelta(minutes=m), px, max(px, close), min(px, close), close))
            px = close
    return pl.DataFrame(rows, schema=["ts", "open", "high", "low", "close"], orient="row").with_columns(
        pl.col("ts").cast(pl.Datetime("ms"))
    )


def synthetic_chain(
    days: list[dt.date], expiry: dt.date, trading_days: list[dt.date], minutes: int = 30, every: int = 5,
    strikes: np.ndarray | None = None, seed: int = 0,
) -> pl.DataFrame:
    """Black-76 closes with random OI, one candle per strike every `every` minutes from 09:15."""
    rng = np.random.default_rng(seed)
    strikes = np.arange(24_750.0, 25_251.0, 50.0) if strikes is None else strikes
    ones = np.ones_like(strikes)
    sigma = 0.12 + 0.6 * ((strikes - F) / F) ** 2
    rows = []
    for day in days:
        n_after = gpdata.sessions_after(day, expiry, trading_days)
        for m in range(every - 1, minutes, every):
            ts = dt.datetime.combine(day, OPEN) + dt.timedelta(minutes=m)
            t = gp.t_years(gp.minutes_to_expiry(ts.hour * 60 + ts.minute + 1, n_after))
            for kind, is_call in (("CE", True), ("PE", False)):
                px = gp.price(ones * F, strikes, ones * t, ones * R, sigma, np.full(strikes.shape, is_call))
                oi = rng.uniform(200.0, 2_000.0, len(strikes))
                for k, p, o in zip(strikes, px, oi):
                    rows.append((ts, f"K{k}{kind}", kind, k, p, p, p, p, 10, o, 75))
    return pl.DataFrame(
        rows,
        schema=["ts", "instrument_key", "opt_type", "strike", "open", "high", "low", "close", "volume", "oi", "lot_size"],
        orient="row",
    ).with_columns(pl.col("ts").cast(pl.Datetime("ms")))


# --- realized vol bars -------------------------------------------------------------------------


def test_realized_vol_bars_hand_check():
    d1, d2 = dt.date(2026, 9, 1), dt.date(2026, 9, 2)
    closes = [100, 101, 100.5, 102, 101, 103, 102, 104, 103, 105]
    rows = [(dt.datetime.combine(d1, OPEN) + dt.timedelta(minutes=m), 99.0 if m == 0 else 0.0, c) for m, c in enumerate(closes)]
    rows += [(dt.datetime.combine(d2, OPEN) + dt.timedelta(minutes=m), 200.0 if m == 0 else 0.0, 202.0 + m) for m in range(5)]
    rows += [(dt.datetime.combine(d1, dt.time(9, 14)), 1.0, 1.0), (dt.datetime.combine(d1, dt.time(15, 30)), 1.0, 1.0)]
    rows += [(dt.datetime.combine(d1, OPEN) + dt.timedelta(minutes=2, seconds=30), 1.0, 500.0)]
    idx = pl.DataFrame(rows, schema=["ts", "open", "close"], orient="row")
    bars = h2_vol.realized_vol_bars(idx)

    r = np.log([closes[0] / 99.0] + [closes[i] / closes[i - 1] for i in range(1, 10)])
    day2 = np.log([202.0 / 200.0] + [(203.0 + m) / (202.0 + m) for m in range(4)])
    assert bars["bar_end"].to_list() == [
        dt.datetime.combine(d1, dt.time(9, 20)), dt.datetime.combine(d1, dt.time(9, 25)),
        dt.datetime.combine(d2, dt.time(9, 20)),
    ]
    assert bars["bar"].to_list() == [0, 1, 0]
    assert bars["n_returns"].to_list() == [5, 5, 5]
    np.testing.assert_allclose(
        bars["rv"].to_numpy(), [np.sqrt(np.sum(r[:5] ** 2)), np.sqrt(np.sum(r[5:] ** 2)), np.sqrt(np.sum(day2**2))]
    )


def test_bar_with_too_few_returns_has_no_rv():
    d = dt.date(2026, 9, 1)
    idx = pl.DataFrame(
        {"ts": [dt.datetime.combine(d, OPEN) + dt.timedelta(minutes=m) for m in (0, 1, 5, 6, 7)],
         "open": [100.0] * 5, "close": [100.0, 101.0, 102.0, 101.0, 103.0]}
    )
    bars = h2_vol.realized_vol_bars(idx)
    assert bars["rv"][0] is None and bars["rv"][1] is not None


# --- features ----------------------------------------------------------------------------------

WARM = dt.date(2026, 9, 1)
EXP1, EXP2 = dt.date(2026, 9, 8), dt.date(2026, 9, 15)


def weekdays(a: dt.date, b: dt.date) -> list[dt.date]:
    return [a + dt.timedelta(days=i) for i in range((b - a).days + 1) if (a + dt.timedelta(days=i)).weekday() < 5]


DAYS = weekdays(WARM, EXP2)


def signal_world(beta: float, seed: int = 1):
    """Index whose next-bar vol is base * exp(-beta * g_t); snapshots expose net GEX S0 = scaled g, S1 = noisy g, S2 = noise."""
    rng = np.random.default_rng(seed)
    g, sigmas, snaps = {}, {}, {}
    for day in DAYS:
        level = 0.0007 * math.exp(rng.normal(0, 0.2))
        x, sig = 0.0, np.empty(75)
        for k in range(75):
            sig[k] = level * math.exp(-beta * x)
            x = 0.7 * x + rng.normal()
            g[(day, k)] = x
        sigmas[day] = sig
    for (day, k), x in g.items():
        ts = dt.datetime.combine(day, OPEN) + dt.timedelta(minutes=5 * (k + 1))
        flip = None if rng.random() < 0.2 else F * (1 + rng.uniform(-0.02, 0.02))
        snaps[ts] = {
            "forward": F, "net_gex_S0": 3e8 + 1e9 * x, "net_gex_S1": 1e9 * (0.6 * x + 0.8 * rng.normal()),
            "net_gex_S2": 1e9 * rng.normal(), "flip_S0": flip, "flip_S1": flip, "flip_S2": None if flip is None else flip * 1.001,
        }
    bars = h2_vol.realized_vol_bars(synthetic_index(sigmas, seed=seed))
    train = h2_vol.features(bars, EXP1, DAYS, snaps.get)
    test = h2_vol.features(bars, EXP2, DAYS, snaps.get, prev_expiry=EXP1)
    return bars, train, test


@pytest.fixture(scope="module")
def signal():
    return signal_world(beta=0.35)


@pytest.fixture(scope="module")
def null():
    return signal_world(beta=0.0, seed=2)


def test_features_week_and_targets(signal):
    bars, train, test = signal
    assert set(train["date"].unique()) == set(weekdays(dt.date(2026, 9, 2), EXP1))
    assert set(test["date"].unique()) == set(weekdays(dt.date(2026, 9, 9), EXP2))
    assert train["dte"].max() == 4 and train.filter(pl.col("date") == EXP1)["dte"].unique().to_list() == [0]
    assert train["tod"].min() == 0 and train["tod"].max() == 12
    assert train.null_count().select(h2_vol.HAR_COLS).row(0) == (0, 0, 0)

    # har_1d on the first week bar averages the 75 bars ending there (all of the warm-up session's bars but the first, plus this one)
    first = train.sort("bar_end").row(0, named=True)
    window = bars.filter(pl.col("bar_end") <= first["bar_end"]).tail(75)
    assert first["har_1d"] == pytest.approx(math.log(window["rv"].mean()))
    assert first["har_1h"] == pytest.approx(math.log(window.tail(12)["rv"].mean()))

    flips = train.filter(pl.col("flip_none_S0") == 1.0)
    assert flips.height > 0 and (flips["flip_dist_S0"] == 0.0).all()
    row = train.filter(pl.col("flip_none_S0") == 0.0).row(0, named=True)
    assert row["flip_dist_S0"] == pytest.approx((row["forward"] - row["flip_S0"]) / row["forward"])


def test_no_cross_session_pairs(signal):
    bars, train, _ = signal
    nxt = bars.with_columns(bar=pl.col("bar") - 1, y_true=pl.col("rv").log()).select("date", "bar", "y_true")
    joined = train.join(nxt, on=["date", "bar"], how="left")
    paired = joined.filter(pl.col("y").is_not_null())
    np.testing.assert_allclose(paired["y"].to_numpy(), paired["y_true"].to_numpy())
    assert train.filter(pl.col("bar") == 74)["y"].null_count() == train.filter(pl.col("bar") == 74).height


def test_gap_inside_session_breaks_pair():
    day = dt.date(2026, 9, 2)
    idx = synthetic_index({WARM: np.full(75, 7e-4), day: np.full(75, 7e-4)})
    gap = dt.datetime.combine(day, dt.time(10, 0))
    idx = idx.filter((pl.col("ts") < gap) | (pl.col("ts") >= gap + dt.timedelta(minutes=5)))
    snap = {"forward": F, **{f"net_gex_{c}": 1.0 for c in h2_vol.CONVENTIONS}, **{f"flip_{c}": None for c in h2_vol.CONVENTIONS}}
    feats = h2_vol.features(h2_vol.realized_vol_bars(idx), day, [WARM, day], lambda ts: snap)
    before = feats.filter(pl.col("bar_end") == gap)
    assert before.height == 1 and before["y"][0] is None
    assert feats.filter(pl.col("bar_end") == gap + dt.timedelta(minutes=5)).height == 0


def test_standardize_uses_train_stats_only():
    train = pl.DataFrame({f"net_gex_{c}": [1.0, 2.0, 3.0, 4.0] for c in h2_vol.CONVENTIONS})
    test = pl.DataFrame({f"net_gex_{c}": [100.0, 200.0] for c in h2_vol.CONVENTIONS})
    tr, te, stats = h2_vol.standardize(train, test)
    std = np.std([1, 2, 3, 4], ddof=1)
    assert stats["S0"] == {"mean": 2.5, "std": pytest.approx(std)}
    np.testing.assert_allclose(te["gex_z_S1"].to_numpy(), (np.array([100.0, 200.0]) - 2.5) / std)
    tr2, _, stats2 = h2_vol.standardize(train, test * 7.0)
    assert stats2 == stats and tr2.equals(tr)


def test_evaluate_standardizes_on_train_rows(signal):
    _, train, test = signal
    a = h2_vol.evaluate(train, test)
    b = h2_vol.evaluate(train, test.with_columns(pl.col("net_gex_S0") * 5 + 1e12))
    assert a["standardization"] == b["standardization"] == h2_vol.gex_stats(train.filter(pl.col("y").is_not_null()))
    assert a["S0"]["model"] == b["S0"]["model"]


def test_signal_recovered(signal):
    _, train, test = signal
    out = h2_vol.evaluate(train, test)
    s0 = out["S0"]
    assert s0["gex"]["b"] < 0 and s0["gex"]["p"] < 1e-4
    assert "gex_z_S0" not in out["S0"]["baseline"]["coef"]
    assert set(s0["baseline"]["coef"]) | {"gex_z_S0"} == set(s0["model"]["coef"])
    assert s0["oos"]["r2_oos"] > 0.05
    assert s0["oos"]["r2_model"] > s0["oos"]["r2_baseline"]
    assert out["S1"]["oos"]["r2_oos"] > 0.0
    assert abs(out["S2"]["oos"]["r2_oos"]) < 0.02


def test_null_has_no_gain(null):
    _, train, test = null
    out = h2_vol.evaluate(train, test)
    for c in h2_vol.CONVENTIONS:
        assert abs(out[c]["oos"]["r2_oos"]) < 0.02
    assert out["S0"]["gex"]["p"] > 0.01


def test_hac_six_lags(signal):
    _, train, _ = signal
    tr, _, _ = h2_vol.standardize(train.filter(pl.col("y").is_not_null()), None)
    model, base = h2_vol.fit(tr, "S0")
    assert model.result.cov_type == "HAC" and model.result.cov_kwds["maxlags"] == 6
    assert base.columns == [c for c in model.columns if c != "gex_z_S0"]


def test_inner_split_is_chronological(signal):
    _, train, test = signal
    inner = h2_vol.inner_split(pl.concat([train, test]))
    assert inner is not None
    a, b = inner
    assert a["expiry"].unique().to_list() == [EXP1] and b["expiry"].unique().to_list() == [EXP2]
    assert h2_vol.inner_split(train) is None


def test_week_sessions_start_after_previous_expiry():
    days = weekdays(dt.date(2026, 10, 12), dt.date(2026, 10, 19))
    assert h2_vol.week_sessions(dt.date(2026, 10, 19), days) == weekdays(dt.date(2026, 10, 13), dt.date(2026, 10, 19))
    assert h2_vol.week_sessions(dt.date(2026, 10, 19), days, dt.date(2026, 10, 13))[0] == dt.date(2026, 10, 14)


def test_gex_snapshot_matches_analyze_at():
    days = [dt.date(2026, 9, 7), EXP1]
    opts = synthetic_chain([EXP1], EXP1, days, minutes=15)
    ts = dt.datetime.combine(EXP1, dt.time(9, 30))
    snap = h2_vol.gex_snapshot(opts, ts, EXP1, R, WEIGHTS, days)
    ref = gpdata.analyze_at(opts, ts, EXP1, R, list(WEIGHTS.values()), days)
    assert snap["forward"] == pytest.approx(F, abs=0.5) == ref["forward"]
    for c, conv in zip(h2_vol.CONVENTIONS, ref["conventions"]):
        assert snap[f"net_gex_{c}"] == conv["net_gex"] and snap[f"flip_{c}"] == conv["flip"]
    assert h2_vol.gex_snapshot(opts, dt.datetime.combine(EXP1, dt.time(9, 15)), EXP1, R, WEIGHTS, days) is None


# --- CLI ---------------------------------------------------------------------------------------


# The fixture is tiny: S1/S2 flips are almost always None, so flip_dist and flip_none are collinear there.
@pytest.mark.filterwarnings("ignore:The design matrix is rank-deficient")
def test_main_train_writes_outputs(tmp_path, monkeypatch):
    expiries = [dt.date(2026, 9, 8), dt.date(2026, 9, 15), dt.date(2026, 9, 22)]
    days = [d for e in expiries for d in (e - dt.timedelta(days=1), e)]
    (tmp_path / "cache").mkdir()
    for i, e in enumerate(expiries):
        mine = [e - dt.timedelta(days=1), e]
        sig = {d: np.full(12, 7e-4) * (1 + 0.3 * j) for j, d in enumerate(mine)}
        synthetic_index(sig, seed=i).write_parquet(tmp_path / "cache" / f"index_{e}.parquet")
        synthetic_chain(mine, e, days, minutes=60, seed=i).write_parquet(tmp_path / "cache" / f"options_{e}.parquet")
    monkeypatch.setattr(gpdata, "EXPIRED", tmp_path / "cache")
    monkeypatch.setattr(h2_vol, "HAR_WINDOWS", (2, 3))

    def not_frozen():
        raise SystemExit("not frozen")

    fake = {
        "prereg": types.SimpleNamespace(split=lambda xs: (sorted(xs)[:2], sorted(xs)[2:]), require_frozen=not_frozen),
        "rates": types.SimpleNamespace(load=lambda: None, rate_on=lambda d, df: R),
        "participant_oi": types.SimpleNamespace(load=lambda: None, weights_for=lambda d, df: WEIGHTS),
    }
    for name, mod in fake.items():
        monkeypatch.setitem(sys.modules, name, mod)

    monkeypatch.setattr(sys, "argv", ["h2_vol", "--split", "train", "--out", str(tmp_path / "out")])
    h2_vol.main()
    report = json.loads((tmp_path / "out" / "h2_train.json").read_text())
    assert report["expiries"]["train"] == ["2026-09-08", "2026-09-15"]
    assert {"S0", "S1", "S2", "standardization"} <= set(report["in_sample"])
    assert report["pseudo_oos"] is not None and "oos" in report["pseudo_oos"]["S0"]
    table = pl.read_parquet(tmp_path / "out" / "h2_train_features.parquet")
    assert set(table["expiry"].unique()) == set(expiries[:2]) and "gex_z_S2" in table.columns

    monkeypatch.setattr(sys, "argv", ["h2_vol", "--split", "test", "--out", str(tmp_path / "out")])
    with pytest.raises(SystemExit, match="not frozen"):
        h2_vol.main()
