"""Inference for H1: paired Wilcoxon signed-rank test and moving-block bootstrap CI of a mean.

Inputs are per-expiry series in chronological order. Callers drop unpaired expiries first.
"""

from __future__ import annotations

import math

import numpy as np
from scipy import stats as sps


def paired_wilcoxon(err_a, err_b) -> dict:
    """Two-sided Wilcoxon signed-rank test on paired absolute errors |err_a| vs |err_b|.

    Pairs where either error is non-finite are dropped. Zero differences are dropped
    (zero_method="wilcox"); when every difference is zero the test is undefined and the
    result is statistic 0, p-value 1. With no pairs at all both are NaN.

    The method is chosen here, not by SciPy's "auto" (whose rule changed between versions):
    exact without ties up to 50 pairs, an exhaustive permutation test with ties up to 13,
    otherwise the tie-corrected normal approximation.
    """
    a = np.abs(np.asarray(err_a, dtype=np.float64))
    b = np.abs(np.asarray(err_b, dtype=np.float64))
    if a.shape != b.shape or a.ndim != 1:
        raise ValueError("err_a and err_b must be 1-D arrays of equal length")
    keep = np.isfinite(a) & np.isfinite(b)
    d = a[keep] - b[keep]
    n, nonzero = int(d.size), int(np.count_nonzero(d))
    out = {"n": n, "n_nonzero": nonzero, "mean_diff": float(d.mean()) if n else math.nan,
           "median_diff": float(np.median(d)) if n else math.nan}
    if n == 0:
        return {**out, "statistic": math.nan, "pvalue": math.nan}
    if nonzero == 0:
        return {**out, "statistic": 0.0, "pvalue": 1.0}
    x = d[d != 0]
    res = sps.wilcoxon(x, alternative="two-sided", zero_method="wilcox", method=wilcoxon_method(x))
    return {**out, "statistic": float(res.statistic), "pvalue": float(res.pvalue)}


def wilcoxon_method(x: np.ndarray):
    """SciPy >= 1.11 'auto' rule, fixed: exact (n <= 50, no ties), permutation (n <= 13, ties), else asymptotic."""
    n = x.size
    ties = np.unique(np.abs(x)).size < n
    if n > 50 or (ties and n > 13):
        return "asymptotic"
    return sps.PermutationMethod() if ties else "exact"


def block_indices(n: int, block_length: int, resamples: int, rng: np.random.Generator) -> np.ndarray:
    """(resamples, n) indices of moving-block resamples: ceil(n / b) blocks with uniform starts, truncated to n."""
    blocks = -(-n // block_length)
    starts = rng.integers(0, n - block_length + 1, size=(resamples, blocks))
    idx = starts[:, :, None] + np.arange(block_length)
    return idx.reshape(resamples, blocks * block_length)[:, :n]


def block_bootstrap_ci(
    diff, block_length: int, resamples: int = 10_000, alpha: float = 0.05, *, seed: int
) -> dict:
    """Moving-block bootstrap percentile CI for the mean of a chronologically ordered series.

    Deterministic for a given seed. block_length 1 is the iid bootstrap.
    """
    x = np.asarray(diff, dtype=np.float64)
    if x.ndim != 1 or x.size == 0:
        raise ValueError("diff must be a non-empty 1-D series")
    if not np.all(np.isfinite(x)):
        raise ValueError("diff contains non-finite values; drop unpaired observations first")
    if not 1 <= block_length <= x.size:
        raise ValueError(f"block_length must be in [1, {x.size}], got {block_length}")
    if resamples < 1 or not 0 < alpha < 1:
        raise ValueError("resamples must be >= 1 and alpha in (0, 1)")
    rng = np.random.default_rng(seed)
    means = x[block_indices(x.size, block_length, resamples, rng)].mean(axis=1)
    lo, hi = np.quantile(means, [alpha / 2, 1 - alpha / 2])
    return {
        "mean": float(x.mean()),
        "lo": float(lo),
        "hi": float(hi),
        "n": int(x.size),
        "alpha": alpha,
        "resamples": resamples,
        "block_length": block_length,
        "seed": seed,
    }
