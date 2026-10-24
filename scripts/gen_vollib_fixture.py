"""Write py_vollib Black-76 reference values for crates/gp-quant/tests/vollib.rs.

Run: python scripts/gen_vollib_fixture.py  (needs py_vollib: pip install -r scripts/requirements.txt)
"""

import csv
import random
from pathlib import Path

from py_vollib.black import black
from py_vollib.black.greeks.analytical import gamma
from py_vollib.black.implied_volatility import implied_volatility

OUT = Path(__file__).resolve().parents[1] / "crates/gp-quant/tests/data/vollib_black76.csv"


def main() -> None:
    rng = random.Random(20261013)
    rows = []
    while len(rows) < 300:
        f = rng.uniform(15_000, 30_000)
        t = rng.uniform(1 / (252 * 375), 30 / 252)
        sigma = rng.uniform(0.05, 1.0)
        r = rng.uniform(0.0, 0.08)
        k = round(f * (1 + rng.uniform(-0.08, 0.08)) / 50) * 50
        flag = "c" if k >= f else "p"
        price = black(flag, f, k, t, r, sigma)
        if price < 0.05:
            continue
        rows.append([flag, f, k, t, r, sigma, price, gamma(flag, f, k, t, r, sigma),
                     implied_volatility(price, f, k, r, t, flag)])
    with OUT.open("w", newline="") as fh:
        w = csv.writer(fh)
        w.writerow(["flag", "f", "k", "t", "r", "sigma", "price", "gamma", "iv"])
        for row in rows:
            w.writerow([row[0]] + [repr(float(x)) for x in row[1:]])


if __name__ == "__main__":
    main()
