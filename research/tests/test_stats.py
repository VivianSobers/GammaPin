import math
import time

import numpy as np
import pytest
from scipy import stats as sps

import stats


def test_wilcoxon_matches_scipy_on_abs_errors():
    rng = np.random.default_rng(1)
    a, b = rng.normal(0, 50, 30), rng.normal(0, 80, 30)
    out = stats.paired_wilcoxon(a, b)
    ref = sps.wilcoxon(np.abs(a) - np.abs(b))
    assert out["n"] == 30 and out["n_nonzero"] == 30
    assert out["statistic"] == pytest.approx(ref.statistic)
    assert out["pvalue"] == pytest.approx(ref.pvalue)


def test_wilcoxon_method_is_fixed_not_scipy_auto():
    # Heavily tied differences, as |err| on round strikes give: the normal approximation with tie
    # correction, whatever SciPy's own "auto" rule would pick.
    d = np.array([50.0, -50.0, 100.0, 50.0, -100.0, 50.0, 150.0, 50.0, -50.0, 100.0, 50.0, 100.0,
                  -50.0, 50.0, 100.0, 50.0, -150.0, 50.0, 100.0, 50.0])
    out = stats.paired_wilcoxon(np.maximum(d, 0), np.maximum(-d, 0))  # |a| - |b| = d
    ref = sps.wilcoxon(d, method="asymptotic")
    assert stats.wilcoxon_method(d) == "asymptotic"
    assert out["pvalue"] == pytest.approx(ref.pvalue, rel=1e-12)
    assert stats.wilcoxon_method(np.arange(1.0, 31.0)) == "exact"
    assert stats.wilcoxon_method(np.arange(1.0, 52.0)) == "asymptotic"
    assert isinstance(stats.wilcoxon_method(np.array([1.0, -1.0, 2.0])), sps.PermutationMethod)


def test_wilcoxon_uses_absolute_errors_and_drops_nan_pairs():
    out = stats.paired_wilcoxon([-10.0, 20.0, math.nan, 5.0], [10.0, -20.0, 1.0, 4.0])
    assert out["n"] == 3 and out["n_nonzero"] == 1
    assert out["mean_diff"] == pytest.approx(1 / 3)


def test_wilcoxon_all_zero_and_empty():
    out = stats.paired_wilcoxon([1.0, -2.0, 3.0], [-1.0, 2.0, 3.0])
    assert out == {**out, "statistic": 0.0, "pvalue": 1.0, "n": 3, "n_nonzero": 0}
    empty = stats.paired_wilcoxon([math.nan], [1.0])
    assert empty["n"] == 0 and math.isnan(empty["pvalue"])
    with pytest.raises(ValueError, match="equal length"):
        stats.paired_wilcoxon([1.0], [1.0, 2.0])


def test_wilcoxon_detects_clear_difference():
    rng = np.random.default_rng(2)
    out = stats.paired_wilcoxon(rng.uniform(0, 10, 40), rng.uniform(50, 60, 40))
    assert out["pvalue"] < 1e-6


def test_bootstrap_deterministic_and_fast():
    x = np.random.default_rng(3).normal(1.0, 2.0, 50)
    t0 = time.perf_counter()
    a = stats.block_bootstrap_ci(x, 3, seed=7)
    assert time.perf_counter() - t0 < 2.0
    assert a == stats.block_bootstrap_ci(x, 3, seed=7)
    assert a != stats.block_bootstrap_ci(x, 3, seed=8)
    assert a["resamples"] == 10_000 and a["block_length"] == 3 and a["n"] == 50
    assert a["lo"] < a["mean"] < a["hi"]


def test_block_length_one_is_iid_bootstrap():
    x = np.random.default_rng(4).normal(size=25)
    out = stats.block_bootstrap_ci(x, 1, resamples=2_000, seed=11)
    rng = np.random.default_rng(11)
    means = x[rng.integers(0, 25, size=(2_000, 25))].mean(axis=1)
    assert out["lo"] == pytest.approx(np.quantile(means, 0.025))
    assert out["hi"] == pytest.approx(np.quantile(means, 0.975))


def test_block_indices_are_contiguous_blocks():
    idx = stats.block_indices(10, 4, 50, np.random.default_rng(0))
    assert idx.shape == (50, 10)
    assert idx.min() >= 0 and idx.max() <= 9
    for row in idx:
        for blk in (row[:4], row[4:8], row[8:]):
            assert np.all(np.diff(blk) == 1)


def test_full_length_block_is_degenerate():
    x = np.array([1.0, 2.0, 6.0])
    out = stats.block_bootstrap_ci(x, 3, resamples=100, seed=0)
    assert out["lo"] == out["hi"] == out["mean"] == pytest.approx(3.0)


def test_coverage_on_iid_data():
    rng = np.random.default_rng(12345)
    hits = 0
    for run in range(200):
        x = rng.normal(0.3, 1.0, 50)
        ci = stats.block_bootstrap_ci(x, 1, resamples=1_000, seed=run)
        hits += ci["lo"] <= 0.3 <= ci["hi"]
    assert hits >= 180


def test_blocks_widen_ci_for_autocorrelated_series():
    rng = np.random.default_rng(5)
    e = rng.normal(size=400)
    x = np.empty(400)
    x[0] = e[0]
    for i in range(1, 400):
        x[i] = 0.8 * x[i - 1] + e[i]
    iid = stats.block_bootstrap_ci(x, 1, resamples=2_000, seed=1)
    blk = stats.block_bootstrap_ci(x, 20, resamples=2_000, seed=1)
    assert blk["hi"] - blk["lo"] > 1.5 * (iid["hi"] - iid["lo"])


@pytest.mark.parametrize(
    "x, b, kw, msg",
    [
        ([], 1, {}, "non-empty"), ([1.0, math.nan], 1, {}, "non-finite"), ([1.0, 2.0], 3, {}, "block_length"),
        ([1.0, 2.0], 0, {}, "block_length"), ([1.0], 1, {"alpha": 1.0}, "alpha"),
    ],
)
def test_bootstrap_bad_input(x, b, kw, msg):
    with pytest.raises(ValueError, match=msg):
        stats.block_bootstrap_ci(x, b, seed=0, **kw)


def test_single_observation():
    out = stats.block_bootstrap_ci([4.0], 1, resamples=10, seed=0)
    assert out["lo"] == out["hi"] == 4.0
