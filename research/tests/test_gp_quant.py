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
