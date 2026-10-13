import importlib

import pytest

MODULES = ["fetch_expired", "participant_oi", "h1_pinning", "h2_vol", "validate_ticks"]


@pytest.mark.parametrize("name", MODULES)
def test_module_imports(name):
    assert callable(importlib.import_module(name).main)
