import importlib

import pytest

MODULES = [
    "fetch_expired", "participant_oi", "instruments", "rates", "settlements",
    "h1_pinning", "h2_vol", "validate_ticks", "forward_test", "bhavcopy", "tbill_rbi", "bhav_coverage", "h1_eod",
]


@pytest.mark.parametrize("name", MODULES)
def test_module_imports(name):
    assert callable(importlib.import_module(name).main)
