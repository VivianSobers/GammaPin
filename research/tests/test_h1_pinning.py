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
import baselines  # noqa: E402
import gpdata  # noqa: E402
import h1_pinning  # noqa: E402
import prereg  # noqa: E402

R = 0.065
DECISION = dt.time(14, 0)
STRIKES = np.arange(24_500.0, 25_501.0, 50.0)
# expiry -> (forward, call-wall strike, put-wall strike, settlement)
CASES = {
    dt.date(2026, 10, 6): (25_012.5, 25_100.0, 24_900.0, 25_090.0),
    dt.date(2026, 10, 13): (24_960.0, 25_000.0, 24_800.0, 24_985.0),
    dt.date(2026, 10, 20): (25_140.0, 25_200.0, 25_000.0, 25_230.0),
}
S1 = (0.3, -0.1)
S2 = (-1.0, 1.0)


def synthetic_options(expiry, f, call_wall, put_wall, start=dt.time(13, 45), minutes=46) -> pl.DataFrame:
    """Black-76 closes; OI 1,000 everywhere plus a call wall and a put wall."""
    t0 = dt.datetime.combine(expiry, start)
    ones = np.ones_like(STRIKES)
    sigma = 0.12 + 0.6 * ((STRIKES - f) / f) ** 2
    rows = []
    for m in range(minutes):
        ts = t0 + dt.timedelta(minutes=m)
        t = gp.t_years(gp.minutes_to_expiry(ts.hour * 60 + ts.minute + 1, 0))
        for kind, is_call, wall, extra in (("CE", True, call_wall, 50_000.0), ("PE", False, put_wall, 20_000.0)):
            px = gp.price(ones * f, STRIKES, ones * t, ones * R, sigma, np.full(STRIKES.shape, is_call))
            for k, p in zip(STRIKES, px):
                oi = 1_000.0 + (extra if k == wall else 0.0)
                rows.append((ts, f"K{k}{kind}", kind, k, p, p, p, p, 10, oi, 75))
    return pl.DataFrame(
        rows,
        schema=["ts", "instrument_key", "opt_type", "strike", "open", "high", "low", "close", "volume", "oi", "lot_size"],
        orient="row",
    )


def synthetic_index(expiry, f) -> pl.DataFrame:
    ts = [dt.datetime.combine(expiry - dt.timedelta(days=d), dt.time(9, 15)) for d in (7, 6, 5, 2, 1, 0)]
    return pl.DataFrame({"ts": ts, "open": f, "high": f, "low": f, "close": f})


@pytest.fixture
def cache(tmp_path, monkeypatch):
    expired = tmp_path / "expired"
    expired.mkdir()
    monkeypatch.setattr(gpdata, "EXPIRED", expired)
    for e, (f, cw, pw, _) in CASES.items():
        synthetic_options(e, f, cw, pw).write_parquet(gpdata.options_path(e))
        synthetic_index(e, f).write_parquet(gpdata.index_path(e))
    toml = tmp_path / "prereg.toml"
    toml.write_text(
        '[split]\ntrain_fraction = 0.7\nlast_train_expiry = "TBD"\n'
        '[bootstrap]\nresamples = 10000\nblock_length = "TBD"\nseed = 20261013\n'
        '[h1]\ndecision_time = "14:00"\n'
    )
    monkeypatch.setattr(prereg, "PATH", toml)

    no_weights = max(CASES)

    def weights_for(day, df):
        if day == no_weights:
            raise LookupError("no participant OI before this day")
        return {"S0": (1.0, -1.0), "S1": S1, "S2": S2}

    fakes = {
        "participant_oi": types.SimpleNamespace(load=lambda: pl.DataFrame(), weights_for=weights_for),
        "rates": types.SimpleNamespace(load=lambda: pl.DataFrame(), rate_on=lambda day, df: R),
        "settlements": types.SimpleNamespace(load=lambda: {e: c[3] for e, c in CASES.items()}),
    }
    for name, mod in fakes.items():
        monkeypatch.setitem(sys.modules, name, mod)
    return tmp_path


def test_features_cannot_see_settlement():
    params = inspect.signature(h1_pinning.features).parameters
    assert not any("settle" in p or "target" in p for p in params)


def test_features_pins_and_baselines():
    e = min(CASES)
    f, cw, pw, _ = CASES[e]
    opts = synthetic_options(e, f, cw, pw)
    ts = dt.datetime.combine(e, DECISION)
    row = h1_pinning.features(opts, e, ts, R, {"S0": (1.0, -1.0), "S1": S1, "S2": S2}, [e])
    assert row["pin_S0"] == cw and row["pin_S1"] == cw and row["pin_S2"] == pw
    assert row["forward"] == pytest.approx(f, abs=0.5)
    assert row["nearest_100"] == baselines.nearest_multiple(row["forward"])
    assert row["no_move"] == row["forward"]
    ch = gpdata.chain_at(opts, ts)
    assert row["max_pain"] == baselines.max_pain(ch["strike"], ch["call_oi"], ch["put_oi"])
    assert pw <= row["max_pain"] <= cw
    assert row["quotes_used"] > 0 and math.isfinite(row["flip_S0"])


def test_features_ignore_data_after_decision():
    e = min(CASES)
    f, cw, pw, _ = CASES[e]
    opts = synthetic_options(e, f, cw, pw)
    ts = dt.datetime.combine(e, DECISION)
    w = {"S0": (1.0, -1.0), "S1": S1, "S2": S2}
    base = h1_pinning.features(opts, e, ts, R, w, [e])

    def perturb(frame, first_ts):
        late = pl.col("ts") >= first_ts
        return frame.with_columns(
            pl.when(late).then(pl.col("close") * 3).otherwise(pl.col("close")).alias("close"),
            pl.when(late & (pl.col("strike") == 24_600.0)).then(1e7).otherwise(pl.col("oi")).alias("oi"),
        )

    after = h1_pinning.features(perturb(opts, ts), e, ts, R, w, [e])
    assert after == base
    # the 13:59 candle is known at 14:00, so changing it must change the features
    boundary = h1_pinning.features(perturb(opts, ts - dt.timedelta(minutes=1)), e, ts, R, w, [e])
    assert boundary != base


def test_features_missing_conventions_and_empty_chain():
    e = min(CASES)
    f, cw, pw, _ = CASES[e]
    opts = synthetic_options(e, f, cw, pw)
    row = h1_pinning.features(opts, e, dt.datetime.combine(e, DECISION), R, {"S0": (1.0, -1.0)}, [e])
    assert row["pin_S0"] == cw and math.isnan(row["pin_S1"]) and math.isnan(row["pin_S2"])
    early = h1_pinning.features(opts, e, dt.datetime.combine(e, dt.time(13, 0)), R, {"S0": (1.0, -1.0)}, [e])
    assert all(math.isnan(early[m]) for m in h1_pinning.METHODS)


def test_evaluate_hand_computed():
    n = 6
    errs = {m: [10.0] * n for m in h1_pinning.METHODS}
    errs["pin_S0"] = [0.0, 5.0, 2.0, 1.0, 3.0, 1.0]
    errs["pin_S1"] = [math.nan, 5.0, 2.0, 1.0, 3.0, 1.0]
    table = pl.DataFrame(
        {"expiry": [dt.date(2026, 1, 6) + dt.timedelta(weeks=i) for i in range(n)][::-1]}
        | {f"abs_err_{m}": v[::-1] for m, v in errs.items()}
    )
    out = h1_pinning.evaluate(table, block_length=2, resamples=500, seed=1)
    assert out["mae"]["pin_S0"] == {"mae": 2.0, "n": 6}
    assert out["mae"]["pin_S1"] == {"mae": 2.4, "n": 5}
    cmp = {(c["gex"], c["baseline"]): c for c in out["comparisons"]}
    assert len(cmp) == 9
    s0 = cmp[("pin_S0", "max_pain")]
    assert s0["n"] == 6 and s0["n_excluded"] == 0
    assert s0["bootstrap"]["mean"] == pytest.approx(-8.0) and s0["bootstrap"]["hi"] < 0
    assert s0["wilcoxon"]["n"] == 6 and s0["wilcoxon"]["pvalue"] < 0.05
    assert cmp[("pin_S1", "no_move")]["n"] == 5 and cmp[("pin_S1", "no_move")]["n_excluded"] == 1
    short = h1_pinning.evaluate(table.head(1), block_length=2, resamples=10, seed=1)
    assert short["comparisons"][0]["bootstrap"] is None and "skipped" in short["comparisons"][0]["note"]


def test_end_to_end_train(cache, capsys):
    out = cache / "output"
    h1_pinning.main(["--split", "train", "--out", str(out)])
    assert "provisional" in capsys.readouterr().err

    table = pl.read_csv(out / "h1_train.csv", try_parse_dates=True)
    assert table.height == 3 and table["expiry"].to_list() == sorted(CASES)
    for row in table.iter_rows(named=True):
        f, cw, pw, settle = CASES[row["expiry"]]
        assert row["status"] == "ok"
        assert row["pin_S0"] == cw
        assert row["forward"] == pytest.approx(f, abs=0.5)
        assert row["nearest_100"] == baselines.nearest_multiple(row["forward"])
        assert row["settlement"] == settle
        assert row["abs_err_pin_S0"] == pytest.approx(abs(settle - cw))
        assert row["abs_err_no_move"] == pytest.approx(abs(settle - row["forward"]))
        if row["weights_ok"]:
            assert row["pin_S1"] == cw and row["pin_S2"] == pw
        else:
            assert row["expiry"] == max(CASES) and math.isnan(row["pin_S1"]) and math.isnan(row["pin_S2"])

    summary = json.loads((out / "h1_train_summary.json").read_text())
    assert summary["split"] == "train" and summary["block_length"] == 3 and summary["block_length_provisional"]
    assert summary["resamples"] == 10_000 and summary["seed"] == 20261013
    assert summary["mae"]["pin_S0"]["n"] == 3 and summary["mae"]["pin_S1"]["n"] == 2
    want = np.mean([abs(c[3] - c[1]) for c in CASES.values()])
    assert summary["mae"]["pin_S0"]["mae"] == pytest.approx(want)
    cmp = {(c["gex"], c["baseline"]): c for c in summary["comparisons"]}
    assert cmp[("pin_S0", "max_pain")]["bootstrap"]["resamples"] == 10_000
    assert cmp[("pin_S1", "max_pain")]["n"] == 2 and cmp[("pin_S1", "max_pain")]["bootstrap"] is None


def test_test_split_refuses_without_tag(cache, monkeypatch):
    monkeypatch.setattr(prereg, "git_tags", lambda pattern: "")
    with pytest.raises(SystemExit, match="not frozen"):
        h1_pinning.main(["--split", "test", "--out", str(cache / "output")])
    assert not (cache / "output").exists()


def test_test_split_refuses_tbd_even_when_tagged(cache, monkeypatch):
    monkeypatch.setattr(prereg, "git_tags", lambda pattern: "prereg-v1\n")
    with pytest.raises(SystemExit, match="TBD"):
        h1_pinning.main(["--split", "test", "--out", str(cache / "output")])
    assert not (cache / "output").exists()
