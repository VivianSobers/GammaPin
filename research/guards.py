"""Look-ahead guards from PREREGISTRATION.md, shared by the research pipelines and their tests.

- Candles: a 1-minute candle stamped `ts` (its start) is known only from `ts + 1 minute` (gpdata.known_at).
- Participant OI: the file for day d is used only on days after d.
- Settlement is never a feature.
- Perturbation: changing data not yet known at the decision time must leave the features unchanged.
"""

from __future__ import annotations

import datetime as dt
import re
from collections.abc import Callable, Iterable
from typing import Any

import numpy as np
import polars as pl

from gpdata import CANDLE

SETTLEMENT_NAME = re.compile(r"settle|close_on_expiry|expiry_close|closing_price|final_close", re.IGNORECASE)


class LookAheadError(AssertionError):
    """Data not available at the decision time would be used."""


def assert_known_at(df: pl.DataFrame, ts: dt.datetime) -> None:
    """Raise if any candle in `df` is not yet known at `ts` (start + 1 minute > ts)."""
    late = df.filter(pl.col("ts") > ts - CANDLE)
    if late.height:
        raise LookAheadError(f"{late.height} candle(s) not known at {ts}, first stamped {late['ts'].min()}")


def assert_participant_before(file_date: dt.date, day: dt.date) -> None:
    """Raise unless the participant OI file for `file_date` predates `day`."""
    if not file_date < day:
        raise LookAheadError(f"participant OI of {file_date} used on {day}; only files before the day are allowed")


def assert_no_settlement_feature(feature_names: Iterable[str]) -> None:
    """Raise if a feature name looks like the settlement or the expiry-day close."""
    bad = [n for n in feature_names if SETTLEMENT_NAME.search(n)]
    if bad:
        raise LookAheadError(f"settlement-like feature(s): {bad}")


def same(a: Any, b: Any) -> bool:
    """Exact equality for frames, arrays, dicts, sequences and floats (NaN equals NaN, None equals None)."""
    if isinstance(a, pl.DataFrame) or isinstance(b, pl.DataFrame):
        return isinstance(a, pl.DataFrame) and isinstance(b, pl.DataFrame) and a.equals(b, null_equal=True)
    if isinstance(a, np.ndarray) or isinstance(b, np.ndarray):
        a, b = np.asarray(a), np.asarray(b)
        if a.shape != b.shape:
            return False
        if a.dtype.kind in "fc" and b.dtype.kind in "fc":
            return bool(np.array_equal(a, b, equal_nan=True))
        return bool(np.array_equal(a, b))
    if isinstance(a, dict) or isinstance(b, dict):
        return isinstance(a, dict) and isinstance(b, dict) and a.keys() == b.keys() and all(same(a[k], b[k]) for k in a)
    if isinstance(a, (list, tuple)) or isinstance(b, (list, tuple)):
        return (
            isinstance(a, (list, tuple)) and isinstance(b, (list, tuple))
            and len(a) == len(b) and all(same(x, y) for x, y in zip(a, b, strict=True))
        )
    if isinstance(a, float) and isinstance(b, float) and np.isnan(a) and np.isnan(b):
        return True
    return bool(a == b)


def perturbation_check(
    build_features: Callable[[Any, dt.datetime], Any],
    data: Any,
    decision_ts: dt.datetime,
    perturb: Callable[[Any, dt.datetime], Any],
) -> bool:
    """True when build_features(data, ts) == build_features(perturb(data, ts), ts).

    `perturb` should only change what is not known at `decision_ts`; then any difference is a leak.
    """
    return same(build_features(data, decision_ts), build_features(perturb(data, decision_ts), decision_ts))
