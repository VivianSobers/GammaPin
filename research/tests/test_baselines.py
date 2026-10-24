import math

import numpy as np
import pytest

import baselines


def test_max_pain_hand_computed():
    # payout at 100: puts 2*100 = 200; at 200: calls 1*100 = 100; at 300: calls 1*200 = 200
    k, c, p = [100.0, 200.0, 300.0], [1.0, 0.0, 4.0], [3.0, 2.0, 0.0]
    assert baselines.payout(np.array(k), np.array(c), np.array(p)).tolist() == [200.0, 100.0, 200.0]
    assert baselines.max_pain(k, c, p) == 200.0


def test_max_pain_tie_goes_to_lower_strike_and_order_is_irrelevant():
    assert baselines.max_pain([100.0, 200.0], [1.0, 0.0], [0.0, 1.0]) == 100.0
    assert baselines.max_pain([200.0, 100.0], [0.0, 1.0], [1.0, 0.0]) == 100.0


def test_max_pain_call_wall_and_put_wall():
    k = np.arange(24_800.0, 25_201.0, 100.0)
    c = np.array([0, 0, 0, 0, 1_000.0])
    p = np.array([1_000.0, 0, 0, 0, 0])
    # any strike between the walls pays nothing; the lowest of them wins the tie
    assert baselines.max_pain(k, c, p) == 24_800.0
    c[0] = 1.0
    assert baselines.max_pain(k, c, p) == 24_800.0
    p[0], p[1] = 0.0, 1_000.0
    assert baselines.max_pain(k, c, p) == 24_900.0


def test_max_pain_bad_input():
    assert math.isnan(baselines.max_pain([], [], []))
    assert math.isnan(baselines.max_pain([100.0, 200.0], [0.0, 0.0], [0.0, 0.0]))
    assert baselines.max_pain([100.0, 200.0, 300.0], [math.nan, 0.0, 4.0], [3.0, 2.0, -1.0]) == 200.0
    with pytest.raises(ValueError, match="equal length"):
        baselines.max_pain([100.0], [1.0, 2.0], [1.0])


@pytest.mark.parametrize(
    "x, step, want",
    [
        (25_049.99, 100, 25_000.0),
        (25_050.0, 100, 25_100.0),
        (24_950.0, 100, 25_000.0),
        (25_012.5, 100, 25_000.0),
        (25_012.5, 50, 25_000.0),
        (25_025.0, 50, 25_050.0),
        (-50.0, 100, 0.0),
    ],
)
def test_nearest_multiple(x, step, want):
    assert baselines.nearest_multiple(x, step) == want


def test_nearest_multiple_nan_and_bad_step():
    assert math.isnan(baselines.nearest_multiple(math.nan))
    with pytest.raises(ValueError, match="step must be positive"):
        baselines.nearest_multiple(1.0, 0)


def test_no_move():
    assert baselines.no_move(25_012.5) == 25_012.5
    assert math.isnan(baselines.no_move(math.nan))
    assert math.isnan(baselines.no_move(None))
