"""H1 baselines: max pain, nearest multiple of 100, and no move (F at the decision time).

Each baseline maps information known at the decision time to a predicted settlement level.
"""

from __future__ import annotations

import math

import numpy as np


def payout(strikes: np.ndarray, call_oi: np.ndarray, put_oi: np.ndarray) -> np.ndarray:
    """Total intrinsic value paid to option holders if the index settles at each listed strike.

    payout[i] = sum_j call_oi[j] * max(K_i - K_j, 0) + put_oi[j] * max(K_j - K_i, 0)
    """
    k = np.asarray(strikes, dtype=np.float64)
    c = np.asarray(call_oi, dtype=np.float64)
    p = np.asarray(put_oi, dtype=np.float64)
    diff = k[:, None] - k[None, :]
    return (np.maximum(diff, 0.0) * c[None, :]).sum(axis=1) + (np.maximum(-diff, 0.0) * p[None, :]).sum(axis=1)


def max_pain(strikes, call_oi, put_oi) -> float:
    """Listed strike minimising the total intrinsic payout to option holders.

    Only listed strikes are candidates. Ties go to the lower strike. Non-finite or negative OI
    counts as 0. Returns NaN when there are no strikes or no open interest at all.
    """
    k = np.asarray(strikes, dtype=np.float64)
    c = np.asarray(call_oi, dtype=np.float64)
    p = np.asarray(put_oi, dtype=np.float64)
    if not (k.shape == c.shape == p.shape) or k.ndim != 1:
        raise ValueError("strikes, call_oi and put_oi must be 1-D arrays of equal length")
    keep = np.isfinite(k)
    k, c, p = k[keep], c[keep], p[keep]
    c = np.where(np.isfinite(c) & (c > 0), c, 0.0)
    p = np.where(np.isfinite(p) & (p > 0), p, 0.0)
    if k.size == 0 or (c.sum() + p.sum()) == 0.0:
        return math.nan
    order = np.argsort(k, kind="stable")
    k, c, p = k[order], c[order], p[order]
    return float(k[int(np.argmin(payout(k, c, p)))])


def nearest_multiple(x: float, step: float = 100.0) -> float:
    """`x` rounded to the nearest multiple of `step`; exact halves round up (towards +inf). NaN stays NaN."""
    if not math.isfinite(x):
        return math.nan
    if step <= 0:
        raise ValueError("step must be positive")
    return float(math.floor(x / step + 0.5) * step)


def no_move(forward: float) -> float:
    """Predict that the settlement equals the forward at the decision time."""
    return float(forward) if forward is not None else math.nan
