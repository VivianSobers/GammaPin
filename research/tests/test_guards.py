import datetime as dt

import numpy as np
import polars as pl
import pytest
from hypothesis import HealthCheck, given, settings
from hypothesis import strategies as st

gp = pytest.importorskip("gp_quant")
import gpdata  # noqa: E402
import guards  # noqa: E402
import h2_vol  # noqa: E402
from test_h2_vol import R, WEIGHTS, synthetic_chain, synthetic_index  # noqa: E402

PREV, DAY = dt.date(2026, 9, 7), dt.date(2026, 9, 8)
DAYS = [PREV, DAY]
T0 = dt.datetime.combine(DAY, dt.time(9, 15))
MINUTES = 40
OPTS = synthetic_chain([DAY], DAY, DAYS, minutes=MINUTES, every=1, strikes=np.arange(24_800.0, 25_201.0, 50.0))
INDEX = synthetic_index({PREV: np.full(8, 7e-4), DAY: np.full(8, 9e-4)}, seed=3)
PRICE_COLS = ("open", "high", "low", "close")

FAST = settings(max_examples=30, deadline=None, suppress_health_check=[HealthCheck.too_slow])
SLOW = settings(max_examples=8, deadline=None, suppress_health_check=[HealthCheck.too_slow])

decisions = st.builds(
    lambda m, s: T0 + dt.timedelta(minutes=m, seconds=s), st.integers(0, MINUTES + 5), st.integers(0, 59)
)
perturbations = st.fixed_dictionaries({
    "px": st.floats(0.5, 2.0), "oi": st.floats(-150.0, 5_000.0), "volume": st.integers(-10, 10_000),
    "drop": st.floats(0.0, 1.0), "add": st.integers(0, 15), "seed": st.integers(0, 2**32 - 1),
})


def _future(decision: dt.datetime) -> pl.Expr:
    return pl.col("ts") > decision - gpdata.CANDLE


def _added_ts(rng: np.random.Generator, decision: dt.datetime, n: int) -> list[dt.datetime]:
    """Timestamps after decision - 1 minute, on and off the minute grid, up to the next day."""
    return [decision - gpdata.CANDLE + dt.timedelta(seconds=int(s)) for s in rng.integers(1, 26 * 3600, n)]


def _scale_drop_add(df: pl.DataFrame, decision: dt.datetime, p: dict, extra: list[tuple]) -> pl.DataFrame:
    rng = np.random.default_rng(p["seed"])
    future = _future(decision)
    mult = pl.Series(rng.uniform(0.5, 1.5, df.height)) * p["px"]
    df = df.with_columns(pl.when(future).then(pl.col(c) * mult).otherwise(pl.col(c)).alias(c) for c in PRICE_COLS)
    if "oi" in df.columns:
        df = df.with_columns(
            pl.when(future).then(pl.col("oi") + p["oi"]).otherwise(pl.col("oi")).alias("oi"),
            pl.when(future).then(pl.col("volume") + p["volume"]).otherwise(pl.col("volume")).alias("volume"),
        )
    drop = df.select(future).to_series() & pl.Series(rng.random(df.height) < p["drop"])
    df = df.filter(~drop)
    if extra:
        df = pl.concat([df, pl.DataFrame(extra, schema=df.schema, orient="row")])
    return df


def perturb_options(df: pl.DataFrame, decision: dt.datetime, p: dict) -> pl.DataFrame:
    rng = np.random.default_rng(p["seed"] + 1)
    strikes = np.concatenate([df["strike"].unique().to_numpy(), [24_000.0, 26_000.0]])
    extra = []
    for ts in _added_ts(rng, decision, p["add"]):
        k, kind, px = float(rng.choice(strikes)), str(rng.choice(["CE", "PE"])), float(rng.uniform(0.05, 900.0))
        extra.append((ts, f"K{k}{kind}", kind, k, px, px, px, px, int(rng.integers(0, 1000)), float(rng.uniform(0, 9e3)), 75))
    return _scale_drop_add(df, decision, p, extra)


def perturb_index(df: pl.DataFrame, decision: dt.datetime, p: dict) -> pl.DataFrame:
    rng = np.random.default_rng(p["seed"] + 2)
    extra = []
    for ts in _added_ts(rng, decision, p["add"]):
        o, c = rng.uniform(20_000.0, 30_000.0, 2)
        extra.append((ts, float(o), float(max(o, c)), float(min(o, c)), float(c)))
    return _scale_drop_add(df, decision, p, extra)


# --- guard functions ---------------------------------------------------------------------------


def test_assert_known_at():
    ts = T0 + dt.timedelta(minutes=10)
    guards.assert_known_at(gpdata.known_at(OPTS, ts), ts)
    with pytest.raises(guards.LookAheadError):
        guards.assert_known_at(OPTS.filter(pl.col("ts") <= ts), ts)


def test_assert_participant_before():
    guards.assert_participant_before(PREV, DAY)
    for bad in (DAY, DAY + dt.timedelta(days=1)):
        with pytest.raises(guards.LookAheadError):
            guards.assert_participant_before(bad, DAY)


def test_assert_no_settlement_feature():
    guards.assert_no_settlement_feature(list(h2_vol.FEATURE_SCHEMA) + h2_vol.controls("S1") + ["close", "forward"])
    for bad in ("settlement", "Settle_px", "close_on_expiry", "nifty_expiry_close", "final_close"):
        with pytest.raises(guards.LookAheadError):
            guards.assert_no_settlement_feature(["har_5m", bad])
    with pytest.raises(guards.LookAheadError):
        h2_vol.ols(pl.DataFrame({"y": [1.0, 2.0, 3.0], "bar_end": [1, 2, 3], "settle": [1.0, 0.0, 2.0]}), ["settle"])


def test_perturbation_check_detects_a_leaky_builder():
    decision = T0 + dt.timedelta(minutes=10)
    p = {"px": 1.3, "oi": 10.0, "volume": 1, "drop": 0.0, "add": 0, "seed": 0}
    leaky = lambda df, ts: df.filter(pl.col("ts") <= ts)["close"].to_numpy()  # noqa: E731
    honest = lambda df, ts: gpdata.known_at(df, ts)["close"].to_numpy()  # noqa: E731
    perturb = lambda df, ts: perturb_options(df, ts, p)  # noqa: E731
    assert not guards.perturbation_check(leaky, OPTS, decision, perturb)
    assert guards.perturbation_check(honest, OPTS, decision, perturb)


def test_same():
    assert guards.same({"a": [np.array([1.0, np.nan]), None]}, {"a": [np.array([1.0, np.nan]), None]})
    assert not guards.same({"a": np.array([1.0])}, {"a": np.array([1.0, 2.0])})
    assert not guards.same(pl.DataFrame({"x": [1]}), pl.DataFrame({"x": [2]}))


# --- perturbation properties -------------------------------------------------------------------


@FAST
@given(decision=decisions, p=perturbations)
def test_chain_at_ignores_unknown_candles(decision, p):
    build = gpdata.chain_at
    assert guards.perturbation_check(build, OPTS, decision, lambda df, ts: perturb_options(df, ts, p))


@SLOW
@given(decision=decisions, p=perturbations)
def test_analyze_at_ignores_unknown_candles(decision, p):
    def build(df, ts):
        return gpdata.analyze_at(df, ts, DAY, R, list(WEIGHTS.values()), DAYS)

    assert guards.perturbation_check(build, OPTS, decision, lambda df, ts: perturb_options(df, ts, p))


@FAST
@given(
    minute=st.integers(1, MINUTES), strike=st.integers(0, 8), kind=st.sampled_from(["CE", "PE"]),
    field=st.sampled_from(["oi", "close"]), delta=st.floats(1.0, 500.0),
)
def test_perturbing_a_known_candle_changes_chain_at(minute, strike, kind, field, delta):
    decision = T0 + dt.timedelta(minutes=minute)
    k = float(np.arange(24_800.0, 25_201.0, 50.0)[strike])
    target = (pl.col("ts") == decision - gpdata.CANDLE) & (pl.col("strike") == k) & (pl.col("opt_type") == kind)

    def perturb(df, ts):
        return df.with_columns(pl.when(target).then(pl.col(field) + delta).otherwise(pl.col(field)).alias(field))

    assert not guards.perturbation_check(gpdata.chain_at, OPTS, decision, perturb)


def _h2_rows(data: tuple[pl.DataFrame, pl.DataFrame], ts: dt.datetime) -> pl.DataFrame:
    index, opts = data

    def snap(t: dt.datetime) -> dict | None:
        return None if t > ts else h2_vol.gex_snapshot(opts, t, DAY, R, WEIGHTS, DAYS)

    feats = h2_vol.features(h2_vol.realized_vol_bars(index), DAY, DAYS, snap, prev_expiry=PREV, windows=(2, 4))
    return feats.filter(pl.col("bar_end") <= ts).drop("y")


@SLOW
@given(decision=decisions, p=perturbations, q=perturbations)
def test_h2_features_ignore_unknown_candles(decision, p, q):
    def perturb(data, ts):
        return perturb_index(data[0], ts, p), perturb_options(data[1], ts, q)

    assert guards.perturbation_check(_h2_rows, (INDEX, OPTS), decision, perturb)


def test_h2_features_change_with_a_known_index_candle():
    decision = T0 + dt.timedelta(minutes=MINUTES)
    rows = _h2_rows((INDEX, OPTS), decision)
    assert rows.height >= 4 and rows["net_gex_S0"].null_count() == 0

    def perturb(data, ts):
        hit = pl.col("ts") == T0 + dt.timedelta(minutes=12)
        return data[0].with_columns(pl.when(hit).then(pl.col("close") * 1.01).otherwise(pl.col("close")).alias("close")), data[1]

    assert not guards.perturbation_check(_h2_rows, (INDEX, OPTS), decision, perturb)
