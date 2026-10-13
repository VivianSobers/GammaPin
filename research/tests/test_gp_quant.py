import numpy as np
import pytest

gp = pytest.importorskip("gp_quant")


def test_price_iv_roundtrip():
    f = np.full(4, 25_000.0)
    k = np.array([24_600.0, 24_900.0, 25_100.0, 25_500.0])
    t = np.full(4, 2 / 252)
    r = np.full(4, 0.065)
    sigma = np.array([0.16, 0.13, 0.12, 0.14])
    is_call = k >= f
    p = gp.price(f, k, t, r, sigma, is_call)
    np.testing.assert_allclose(gp.implied_vol(p, f, k, t, r, is_call), sigma, atol=1e-7)


def test_failed_iv_is_nan():
    one = np.ones(1)
    iv = gp.implied_vol(np.array([-1.0]), one * 25_000, one * 25_000, one * 0.01, one * 0.06, np.array([True]))
    assert np.isnan(iv[0])


def test_length_mismatch_raises():
    with pytest.raises(ValueError):
        gp.gamma(np.ones(2), np.ones(3), np.ones(2), np.ones(2), np.ones(2))


def test_time():
    assert gp.minutes_to_expiry(14 * 60, 0) == 90
    assert gp.t_years(0) == pytest.approx(1 / (252 * 375))


def _chain(f=25_012.5, t=1 / 252, r=0.065):
    k = np.arange(24_000.0, 26_001.0, 50.0)
    sigma = 0.12 + 0.6 * ((k - f) / f) ** 2
    ones = np.ones_like(k)
    c = gp.price(ones * f, k, ones * t, ones * r, sigma, np.ones_like(k, dtype=bool))
    p = gp.price(ones * f, k, ones * t, ones * r, sigma, np.zeros_like(k, dtype=bool))
    oi_c = np.where(k == 25_200.0, 60_000.0, 1_000.0)
    oi_p = np.where(k == 24_800.0, 60_000.0, 1_000.0)
    return k, c, p, oi_c, oi_p


def test_analyze_matches_rust_scenario():
    k, c, p, oi_c, oi_p = _chain()
    a = gp.analyze(k, np.maximum(c - 0.5, 0), c + 0.5, oi_c, np.maximum(p - 0.5, 0), p + 0.5, oi_p,
                   0.065, 1 / 252, 75.0, [gp.S0])
    assert a["forward"] == pytest.approx(25_012.5, abs=1e-6)
    s0 = a["conventions"][0]
    assert s0["pin"] == 25_200.0
    assert 24_800 < s0["flip"] < 25_200
    assert s0["net_gex"] == pytest.approx(s0["gex"].sum())


def test_analyze_candles_and_errors():
    k, c, p, oi_c, oi_p = _chain()
    a = gp.analyze(k, c, c, oi_c, p, p, oi_p, 0.065, 1 / 252, 75.0, [gp.S0], candles=True)
    assert a["quotes_used"] > 12
    with pytest.raises(ValueError):
        gp.analyze(k, c, c, oi_c, p, p, oi_p, 0.065, 1 / 252, 75.0, [gp.S0])


def test_weights():
    total = (3_778_676, 3_778_676, 4_883_697, 4_883_697)
    w_c, w_p = gp.weights_s1((467_200, 454_751, 705_720, 425_386), (989_413, 867_405, 1_265_831, 1_251_862), total)
    assert w_c == pytest.approx(134_457 / 3_778_676)
    assert w_p == pytest.approx(294_303 / 4_883_697)
    assert gp.weights_s2((2_310_910, 2_456_521, 2_881_227, 3_206_449), total)[0] == pytest.approx(145_611 / 3_778_676)
