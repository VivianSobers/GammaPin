import datetime as dt

import pytest

import prereg

DAYS = [dt.date(2026, 1, 6) + dt.timedelta(weeks=i) for i in range(10)]


def cfg(last="TBD", block="TBD"):
    return {
        "split": {"train_fraction": 0.7, "last_train_expiry": last},
        "bootstrap": {"resamples": 10_000, "block_length": block, "seed": 20261013},
        "h1": {"decision_time": "14:00"},
    }


def test_repo_toml_loads():
    c = prereg.load()
    assert c["split"]["train_fraction"] == 0.7
    assert prereg.resamples(c) == 10_000 and prereg.seed(c) == 20261013
    assert prereg.decision_time(c) == dt.time(14, 0)


def test_split_by_fraction_is_chronological():
    train, test = prereg.split(list(reversed(DAYS)) + DAYS[:2], cfg())
    assert train == DAYS[:7] and test == DAYS[7:]
    train, test = prereg.split(DAYS[:9], cfg())
    assert len(train) == 7 and len(test) == 2  # ceil(6.3)


def test_split_by_last_train_expiry():
    train, test = prereg.split(DAYS, cfg(last="2026-01-27"))
    assert train == DAYS[:4] and test == DAYS[4:]
    train, test = prereg.split(DAYS, cfg(last=dt.date(2026, 1, 27)))  # unquoted TOML date
    assert train == DAYS[:4]


def test_tbd_is_provisional_for_train_and_fatal_for_test(capsys):
    assert prereg.expiries_for("train", DAYS, cfg()) == DAYS[:7]
    assert prereg.block_length("train", cfg()) == prereg.PROVISIONAL_BLOCK_LENGTH
    assert "provisional" in capsys.readouterr().err
    with pytest.raises(SystemExit, match="last_train_expiry is TBD"):
        prereg.expiries_for("test", DAYS, cfg())
    with pytest.raises(SystemExit, match="block_length is TBD"):
        prereg.block_length("test", cfg())
    assert prereg.expiries_for("test", DAYS, cfg(last="2026-02-24")) == DAYS[8:]
    assert prereg.block_length("test", cfg(block=4)) == 4
    with pytest.raises(ValueError):
        prereg.block_length("valid", cfg())


def test_require_frozen(monkeypatch):
    monkeypatch.setattr(prereg, "git_tags", lambda pattern: "")
    with pytest.raises(SystemExit, match="not frozen"):
        prereg.require_frozen()
    monkeypatch.setattr(prereg, "git_tags", lambda pattern: "prereg-v1\n")
    prereg.require_frozen()

    def broken(pattern):
        raise OSError("git not found")

    monkeypatch.setattr(prereg, "git_tags", broken)
    with pytest.raises(SystemExit, match="Cannot check"):
        prereg.require_frozen()
