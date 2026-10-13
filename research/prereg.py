"""Pre-registered parameters (prereg.toml) and the freeze guard for the test set.

Values still "TBD" fail loudly on the test split. The train split may use a documented
provisional default and prints a warning.
"""

from __future__ import annotations

import datetime as dt
import math
import subprocess
import sys
import tomllib
from pathlib import Path

PATH = Path(__file__).resolve().parent / "prereg.toml"
REPO = Path(__file__).resolve().parent.parent
TAG = "prereg-v1"
TBD = "TBD"
SPLITS = ("train", "test")
PROVISIONAL_BLOCK_LENGTH = 3


def load(path: Path | None = None) -> dict:
    with open(path or PATH, "rb") as f:
        return tomllib.load(f)


def is_tbd(value) -> bool:
    return isinstance(value, str) and value.strip().upper() == TBD


def warn(msg: str) -> None:
    print(f"WARNING (provisional, not pre-registered): {msg}", file=sys.stderr)


def tbd_error(key: str) -> SystemExit:
    return SystemExit(f"prereg.toml: {key} is TBD. Fill it in, commit, tag {TAG}, then run the test split.")


def check_split(name: str) -> None:
    if name not in SPLITS:
        raise ValueError(f"split must be one of {SPLITS}, got {name!r}")


def split(expiries, cfg: dict | None = None) -> tuple[list[dt.date], list[dt.date]]:
    """Chronological (train, test): up to last_train_expiry when set, else the first ceil(fraction * n)."""
    cfg = cfg or load()
    days = sorted(set(expiries))
    last = cfg["split"]["last_train_expiry"]
    if not is_tbd(last):
        cut = dt.date.fromisoformat(str(last))
        return [d for d in days if d <= cut], [d for d in days if d > cut]
    n_train = math.ceil(round(float(cfg["split"]["train_fraction"]) * len(days), 9))
    return days[:n_train], days[n_train:]


def expiries_for(name: str, expiries, cfg: dict | None = None) -> list[dt.date]:
    """The expiries of one split. The test split requires a fixed last_train_expiry."""
    check_split(name)
    cfg = cfg or load()
    if is_tbd(cfg["split"]["last_train_expiry"]):
        if name == "test":
            raise tbd_error("split.last_train_expiry")
        warn(f"split.last_train_expiry is TBD; train = first ceil({cfg['split']['train_fraction']} * n) expiries")
    train, test = split(expiries, cfg)
    return train if name == "train" else test


def block_length(name: str, cfg: dict | None = None) -> int:
    check_split(name)
    cfg = cfg or load()
    value = cfg["bootstrap"]["block_length"]
    if is_tbd(value):
        if name == "test":
            raise tbd_error("bootstrap.block_length")
        warn(f"bootstrap.block_length is TBD; using {PROVISIONAL_BLOCK_LENGTH}")
        return PROVISIONAL_BLOCK_LENGTH
    return int(value)


def resamples(cfg: dict | None = None) -> int:
    return int((cfg or load())["bootstrap"]["resamples"])


def seed(cfg: dict | None = None) -> int:
    return int((cfg or load())["bootstrap"]["seed"])


def decision_time(cfg: dict | None = None) -> dt.time:
    return dt.time.fromisoformat(str((cfg or load())["h1"]["decision_time"]))


def git_tags(pattern: str) -> str:
    """Output of `git tag -l <pattern>` in the repository."""
    out = subprocess.run(["git", "tag", "-l", pattern], cwd=REPO, capture_output=True, text=True, check=True)
    return out.stdout


def require_frozen() -> None:
    """Exit unless the pre-registration has been frozen with the prereg-v1 tag."""
    try:
        tags = git_tags(TAG)
    except (OSError, subprocess.CalledProcessError) as e:
        raise SystemExit(f"Cannot check for git tag {TAG} ({e}); refusing to run the test split.") from e
    if not tags.strip():
        raise SystemExit(
            f"Pre-registration is not frozen: git tag {TAG} does not exist. "
            f"Fill every TBD, commit, tag {TAG}, then run the test split."
        )
