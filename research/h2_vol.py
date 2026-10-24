"""H2: does standardized net GEX at t predict log realized vol at t+1 (5-minute bars)?

Bars: 5-minute bars aligned to the 09:15 open (75 per session, bar k covers 09:15 + 5k to 09:15 + 5(k+1)).
RV of a bar = sqrt(sum of squared 1-minute log returns of the candles stamped inside it). The return of a
candle is log(close / previous close in the same session); the session's first candle uses its own open,
so no overnight return enters. A bar with fewer than MIN_RETURNS returns has no RV. Candles off the minute
grid or outside 09:15-15:30 are dropped. A bar is known at its `bar_end`.

Regression, per convention S in S0, S1, S2, on bar pairs (t, t+1) with t+1 the next bar of the same session:

    log RV_{t+1} = a + b1 har_5m + b2 har_1h + b3 har_1d + sum_k c_k tod_k + d dte
                     + e1 flip_dist_S + e2 flip_none_S + g gex_z_S + u

    har_5m       log RV_t
    har_1h       log of the mean RV over bars t-11..t (12 bars, one hour)
    har_1d       log of the mean RV over bars t-74..t (75 bars, one session)
                 (windows count bars in time order across sessions, overnight gaps skipped; a full window is required)
    tod_k        1 if bar t starts in 30-minute bucket k (k = bar // 6, 1..12; bucket 0 = 09:15-09:45 is the reference)
    dte          trading sessions d with date(t) < d <= expiry (0 on expiry day)
    flip_dist_S  (F - flip_S) / F at bar_end(t), 0 when flip_S is None
    flip_none_S  1 when flip_S is None (no sign change within F +/- 3%), else 0
    gex_z_S      (net_gex_S - mean) / std, mean and std from the training rows only

Baseline for S: the same regression without gex_z_S (HAR terms, tod, dte and the flip terms of S), so the
out-of-sample gain isolates net GEX. Secondary baseline `har_only`: HAR terms, tod and dte, shared by all
conventions. Columns with no variation in the training rows are dropped from both. OLS with Newey-West
(Bartlett, 6 lags) standard errors over the rows sorted by bar_end.

R2_oos = 1 - SSE_model / SSE_baseline on the evaluation rows. Train mode also reports a pseudo out-of-sample
run: the first 70% of training expiries (chronological) fit, the last 30% evaluate, standardized on the first part.

Expiry week: sessions from expiry - 6 days to expiry, starting after the previous cached expiry when that
is later, each analyzed with that expiry's chain through gpdata.analyze_at at bar_end(t).

Sample: cached expiries flagged `complete` in coverage_summary.csv (gpdata.complete_expiries), split by
prereg.expiries_for over that kept sample (the test split refuses while last_train_expiry is TBD and needs
the prereg-v1 tag). `--include-incomplete` keeps every cached expiry for train diagnostics only.
"""

from __future__ import annotations

import argparse
import datetime as dt
import json
from collections.abc import Callable, Mapping, Sequence
from dataclasses import dataclass
from pathlib import Path

import numpy as np
import polars as pl
import statsmodels.api as sm

import gpdata
import guards

CONVENTIONS = ("S0", "S1", "S2")
SESSION_OPEN = dt.time(9, 15)
SESSION_MINUTES = 375
BAR_MINUTES = 5
MIN_RETURNS = 3
HAR_WINDOWS = (12, 75)
TOD_BARS = 6
NW_LAGS = 6
INNER_TRAIN_FRAC = 0.7
WEEK = dt.timedelta(days=6)
HAR_COLS = ["har_5m", "har_1h", "har_1d"]
TOD_COLS = [f"tod_{k}" for k in range(1, SESSION_MINUTES // (BAR_MINUTES * TOD_BARS) + 1)]
OUTPUT = Path(__file__).resolve().parent / "output"

FEATURE_SCHEMA = {
    "expiry": pl.Date, "date": pl.Date, "bar": pl.Int64, "bar_end": pl.Datetime("ms"),
    "dte": pl.Int64, "tod": pl.Int64, **{c: pl.Float64 for c in HAR_COLS}, "y": pl.Float64, "forward": pl.Float64,
    **{
        f"{name}_{c}": pl.Float64
        for c in CONVENTIONS
        for name in ("net_gex", "flip", "flip_dist", "flip_none")
    },
}

Snapshot = Callable[[dt.datetime], "dict | None"]


def realized_vol_bars(index_1m: pl.DataFrame) -> pl.DataFrame:
    """date, bar (0..74), bar_end, rv, n_returns per 5-minute bar, sorted by bar_end."""
    open_min = SESSION_OPEN.hour * 60 + SESSION_OPEN.minute
    minute = pl.col("ts").dt.hour().cast(pl.Int64) * 60 + pl.col("ts").dt.minute().cast(pl.Int64) - open_min
    on_grid = (pl.col("ts").dt.second() == 0) & (pl.col("ts").dt.nanosecond() == 0)
    ts_type = index_1m.schema["ts"]
    return (
        index_1m.select("ts", "open", "close")
        .filter(on_grid)
        .with_columns(date=pl.col("ts").dt.date(), minute=minute)
        .filter((pl.col("minute") >= 0) & (pl.col("minute") < SESSION_MINUTES))
        .sort("ts")
        .with_columns(prev=pl.col("close").shift(1).over("date"))
        .with_columns(
            ret=(pl.col("close") / pl.coalesce("prev", "open")).log(),
            bar=pl.col("minute") // BAR_MINUTES,
        )
        .group_by("date", "bar")
        .agg(rv=(pl.col("ret") ** 2).sum().sqrt(), n_returns=pl.len().cast(pl.Int64))
        .with_columns(
            rv=pl.when(pl.col("n_returns") >= MIN_RETURNS).then("rv"),
            bar_end=pl.col("date").cast(ts_type)
            + pl.duration(minutes=open_min + BAR_MINUTES * (pl.col("bar") + 1)),
        )
        .sort("bar_end")
        .select("date", "bar", "bar_end", "rv", "n_returns")
    )


def _log_pos(e: pl.Expr) -> pl.Expr:
    return pl.when(e > 0).then(e.log())


def har_terms(bars: pl.DataFrame, windows: tuple[int, int] = HAR_WINDOWS) -> pl.DataFrame:
    """Adds har_5m, har_1h, har_1d (trailing, bar t included) and the target y = log RV of the next bar of the same session."""
    h, d = windows
    nxt_bar = pl.col("bar").shift(-1).over("date")
    nxt_rv = pl.col("rv").shift(-1).over("date")
    return bars.sort("bar_end").with_columns(
        har_5m=_log_pos(pl.col("rv")),
        har_1h=_log_pos(pl.col("rv").rolling_mean(h, min_samples=h)),
        har_1d=_log_pos(pl.col("rv").rolling_mean(d, min_samples=d)),
        y=pl.when(nxt_bar == pl.col("bar") + 1).then(_log_pos(nxt_rv)),
    )


def week_sessions(expiry: dt.date, trading_days: Sequence[dt.date], prev_expiry: dt.date | None = None) -> list[dt.date]:
    start = expiry - WEEK
    if prev_expiry is not None:
        start = max(start, prev_expiry + dt.timedelta(days=1))
    return [d for d in sorted(trading_days) if start <= d <= expiry]


def gex_snapshot(
    opts: pl.DataFrame,
    ts: dt.datetime,
    expiry: dt.date,
    r: float,
    weights: Mapping[str, tuple[float, float]],
    trading_days: list[dt.date],
) -> dict | None:
    """Forward and, per convention, net GEX and flip from gpdata.analyze_at at `ts`."""
    out = gpdata.analyze_at(opts, ts, expiry, r, [weights[c] for c in CONVENTIONS], trading_days)
    if out is None:
        return None
    snap = {"forward": out["forward"]}
    for c, conv in zip(CONVENTIONS, out["conventions"]):
        snap[f"net_gex_{c}"] = conv["net_gex"]
        snap[f"flip_{c}"] = conv["flip"]
    return snap


def features(
    bars: pl.DataFrame,
    expiry: dt.date,
    trading_days: list[dt.date],
    snapshot: Snapshot,
    prev_expiry: dt.date | None = None,
    windows: tuple[int, int] = HAR_WINDOWS,
) -> pl.DataFrame:
    """One row per bar t of the expiry week with full HAR windows and a snapshot; y may be null (no next bar)."""
    days = week_sessions(expiry, trading_days, prev_expiry)
    base = har_terms(bars, windows).filter(pl.col("date").is_in(days)).drop_nulls(HAR_COLS)
    rows = []
    for rec in base.iter_rows(named=True):
        snap = snapshot(rec["bar_end"])
        if snap is None:
            continue
        f = snap["forward"]
        row = {
            "expiry": expiry, "date": rec["date"], "bar": rec["bar"], "bar_end": rec["bar_end"],
            "dte": gpdata.sessions_after(rec["date"], expiry, trading_days), "tod": rec["bar"] // TOD_BARS,
            **{c: rec[c] for c in HAR_COLS}, "y": rec["y"], "forward": f,
        }
        for c in CONVENTIONS:
            flip = snap[f"flip_{c}"]
            row[f"net_gex_{c}"] = snap[f"net_gex_{c}"]
            row[f"flip_{c}"] = flip
            row[f"flip_dist_{c}"] = 0.0 if flip is None else (f - flip) / f
            row[f"flip_none_{c}"] = float(flip is None)
        rows.append(row)
    return pl.DataFrame(rows, schema=FEATURE_SCHEMA) if rows else pl.DataFrame(schema=FEATURE_SCHEMA)


def gex_stats(train: pl.DataFrame) -> dict:
    """Mean and std (ddof 1) of net GEX per convention over the training rows."""
    stats = {}
    for c in CONVENTIONS:
        x = train[f"net_gex_{c}"]
        mean, std = x.mean(), x.std()
        if std is None or not np.isfinite(std) or std == 0:
            raise ValueError(f"net_gex_{c} has no spread in the training rows")
        stats[c] = {"mean": float(mean), "std": float(std)}
    return stats


def apply_z(df: pl.DataFrame, stats: dict) -> pl.DataFrame:
    return df.with_columns(
        ((pl.col(f"net_gex_{c}") - s["mean"]) / s["std"]).alias(f"gex_z_{c}") for c, s in stats.items()
    )


def standardize(
    train: pl.DataFrame, test: pl.DataFrame | None
) -> tuple[pl.DataFrame, pl.DataFrame | None, dict]:
    """gex_z_S = (net_gex_S - mean) / std with mean and std from `train` only, applied to both."""
    stats = gex_stats(train)
    return apply_z(train, stats), None if test is None else apply_z(test, stats), stats


def controls(convention: str) -> list[str]:
    return HAR_COLS + TOD_COLS + ["dte", f"flip_dist_{convention}", f"flip_none_{convention}"]


def _column(df: pl.DataFrame, name: str) -> np.ndarray:
    if name.startswith("tod_"):
        return (df["tod"] == int(name.removeprefix("tod_"))).cast(pl.Float64).to_numpy()
    return df[name].cast(pl.Float64).to_numpy()


def _matrix(df: pl.DataFrame, columns: list[str]) -> np.ndarray:
    return np.column_stack([np.ones(df.height)] + [_column(df, c) for c in columns])


@dataclass
class Fit:
    columns: list[str]
    result: object

    def predict(self, df: pl.DataFrame) -> np.ndarray:
        return _matrix(df, self.columns) @ np.asarray(self.result.params)


def ols(train: pl.DataFrame, columns: list[str]) -> Fit:
    """OLS of y on a constant and `columns` (constant columns dropped), Newey-West errors."""
    guards.assert_no_settlement_feature(columns)
    data = train.filter(pl.col("y").is_not_null()).sort("bar_end")
    used = [c for c in columns if np.ptp(_column(data, c)) > 0]
    res = sm.OLS(data["y"].to_numpy(), _matrix(data, used)).fit(cov_type="HAC", cov_kwds={"maxlags": NW_LAGS})
    return Fit(used, res)


def fit(train: pl.DataFrame, convention: str) -> tuple[Fit, Fit]:
    """(model, baseline) for one convention; `train` must carry gex_z columns (see standardize)."""
    gex = f"gex_z_{convention}"
    model = ols(train, controls(convention) + [gex])
    if gex not in model.columns:
        raise ValueError(f"{gex} has no variation in the training rows")
    return model, ols(train, [c for c in model.columns if c != gex])


def oos_r2(model: Fit, baseline: Fit, test: pl.DataFrame) -> dict:
    """r2_oos = 1 - SSE_model / SSE_baseline, plus classic test R^2 = 1 - SSE / SST for both."""
    data = test.filter(pl.col("y").is_not_null())
    y = data["y"].to_numpy()
    sse_m = float(np.sum((y - model.predict(data)) ** 2))
    sse_b = float(np.sum((y - baseline.predict(data)) ** 2))
    sst = float(np.sum((y - y.mean()) ** 2))
    return {"n": len(y), "r2_oos": 1 - sse_m / sse_b, "r2_model": 1 - sse_m / sst, "r2_baseline": 1 - sse_b / sst}


def summary(f: Fit) -> dict:
    r = f.result
    names = ["const"] + f.columns
    coef = {
        n: {"b": float(b), "se": float(se), "t": float(t), "p": float(p)}
        for n, b, se, t, p in zip(names, r.params, r.bse, r.tvalues, r.pvalues)
    }
    return {"nobs": int(r.nobs), "r2": float(r.rsquared), "coef": coef}


def evaluate(train: pl.DataFrame, test: pl.DataFrame | None) -> dict:
    """Standardize on `train`, fit every convention, score on `test` when given."""
    train = train.filter(pl.col("y").is_not_null())
    test = None if test is None else test.filter(pl.col("y").is_not_null())
    train, test, stats = standardize(train, test)
    har = ols(train, HAR_COLS + TOD_COLS + ["dte"])
    out: dict = {"standardization": stats, "har_only": summary(har)}
    for c in CONVENTIONS:
        model, base = fit(train, c)
        res = {"gex": summary(model)["coef"][f"gex_z_{c}"], "model": summary(model), "baseline": summary(base)}
        if test is not None and test.height:
            res["oos"] = oos_r2(model, base, test)
            res["oos_vs_har_only"] = oos_r2(model, har, test)
        out[c] = res
    return out


def inner_split(train: pl.DataFrame, frac: float = INNER_TRAIN_FRAC) -> tuple[pl.DataFrame, pl.DataFrame] | None:
    """Chronological split by expiry: the first floor(frac * n) expiries (at least one) and the rest."""
    expiries = sorted(train["expiry"].unique().to_list())
    if len(expiries) < 2:
        return None
    cut = expiries[max(1, int(len(expiries) * frac)) - 1]
    return train.filter(pl.col("expiry") <= cut), train.filter(pl.col("expiry") > cut)


def available_expiries() -> list[dt.date]:
    out = []
    for p in sorted(gpdata.EXPIRED.glob("options_*.parquet")):
        e = dt.date.fromisoformat(p.stem.removeprefix("options_"))
        if gpdata.index_path(e).exists():
            out.append(e)
    return out


def load_index(expiries: list[dt.date]) -> pl.DataFrame:
    frames = [
        pl.read_parquet(gpdata.index_path(e)).select(pl.col("ts").cast(pl.Datetime("ms")), "open", "close")
        for e in expiries
    ]
    return pl.concat(frames).unique("ts", keep="first").sort("ts")


SPEC = {
    "bars": "5-minute, aligned to 09:15; RV = sqrt(sum squared 1-minute log returns), session-first return from open",
    "min_returns_per_bar": MIN_RETURNS,
    "har_windows_bars": {"har_5m": 1, "har_1h": HAR_WINDOWS[0], "har_1d": HAR_WINDOWS[1]},
    "tod": "30-minute buckets of bar t start, 09:15-09:45 reference",
    "dte": "trading sessions after date(t) up to expiry",
    "flip": "flip_dist = (F - flip)/F, 0 and flip_none = 1 when flip is None",
    "gex": "plain z-score of net GEX, training-row mean and std",
    "baseline": "model without gex_z (HAR, tod, dte, flip terms of the same convention)",
    "har_only": "HAR, tod, dte",
    "errors": f"Newey-West, Bartlett, {NW_LAGS} lags",
    "pseudo_oos": f"train mode: first {INNER_TRAIN_FRAC:.0%} of training expiries fit, rest evaluate",
}


def main(argv: list[str] | None = None) -> None:
    import participant_oi
    import prereg
    import rates

    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--split", choices=prereg.SPLITS, default="train")
    ap.add_argument("--out", type=Path, default=OUTPUT)
    ap.add_argument("--include-incomplete", action="store_true",
                    help="diagnostics only: keep expiries that fail the coverage check (refused for --split test)")
    args = ap.parse_args(argv)

    if args.include_incomplete and args.split == "test":
        raise SystemExit("--include-incomplete is for train diagnostics only and is refused for --split test.")
    if args.split == "test":
        prereg.require_frozen()
    cfg = prereg.load()
    all_expiries = available_expiries()
    if not all_expiries:
        raise SystemExit(f"no expiries with options and index candles in {gpdata.EXPIRED}")
    kept, incomplete = gpdata.coverage_filter(all_expiries, args.include_incomplete)
    # Same rules as H1: the test split refuses while last_train_expiry is TBD (checked before any train warning).
    test_exp = prereg.expiries_for("test", kept, cfg) if args.split == "test" else []
    train_exp = sorted(prereg.expiries_for("train", kept, cfg))
    if not train_exp or (args.split == "test" and not test_exp):
        raise SystemExit(
            f"no {args.split} expiries in {gpdata.EXPIRED} "
            f"({len(all_expiries)} cached, {len(all_expiries) - len(kept)} excluded as incomplete)"
        )
    held_out = [e for e in kept if e not in train_exp] if args.split == "train" else sorted(test_exp)
    use = train_exp if args.split == "train" else sorted(train_exp + held_out)

    index = load_index(use)
    trading_days = sorted(set(index["ts"].dt.date().to_list()))
    bars = realized_vol_bars(index)
    pdf, rdf = participant_oi.load(), rates.load()

    frames, skipped = [], []
    for e in use:
        prev = max((x for x in all_expiries if x < e), default=None)
        inputs = {}
        for d in week_sessions(e, trading_days, prev):
            try:
                inputs[d] = (rates.rate_on(d, rdf), participant_oi.weights_for(d, pdf))
            except LookupError as err:
                skipped.append({"expiry": e.isoformat(), "date": d.isoformat(), "reason": str(err)})
        opts = pl.read_parquet(gpdata.options_path(e))

        def snap(ts: dt.datetime, opts=opts, e=e, inputs=inputs) -> dict | None:
            if ts.date() not in inputs:
                return None
            r, w = inputs[ts.date()]
            return gex_snapshot(opts, ts, e, r, w, trading_days)

        frames.append(features(bars, e, trading_days, snap, prev, HAR_WINDOWS))
        print(f"{e}: {frames[-1].height} bars")

    feats = pl.concat(frames)
    train = feats.filter(pl.col("expiry").is_in(train_exp))
    report: dict = {
        "split": args.split, "spec": SPEC, "skipped_days": skipped,
        "expiries": {"train": [str(x) for x in train_exp], "test": [str(x) for x in held_out]},
        "split_provisional": prereg.is_tbd(cfg["split"]["last_train_expiry"]),
        "include_incomplete": args.include_incomplete,
        "coverage": {
            "n_cached": len(all_expiries),
            "n_incomplete": len(incomplete),
            "n_excluded_incomplete": len(all_expiries) - len(kept),
            "incomplete_expiries": [str(x) for x in incomplete],
            "n_kept": len(kept),
            "n_used": len(use),
            "n_used_incomplete": len(set(use) & set(incomplete)),
        },
    }
    if args.split == "train":
        report["in_sample"] = result = evaluate(train, None)
        inner = inner_split(train.filter(pl.col("y").is_not_null()))
        report["pseudo_oos"] = None if inner is None else evaluate(*inner)
        table = train.with_columns(sample=pl.lit("train"))
    else:
        test = feats.filter(pl.col("expiry").is_in(test_exp))
        report["test"] = result = evaluate(train, test)
        table = pl.concat([train.with_columns(sample=pl.lit("train")), test.with_columns(sample=pl.lit("test"))])
    table = apply_z(table, result["standardization"]).with_columns(
        coverage_complete=~pl.col("expiry").is_in(pl.Series(incomplete, dtype=pl.Date).implode())
    )

    args.out.mkdir(parents=True, exist_ok=True)
    (args.out / f"h2_{args.split}.json").write_text(json.dumps(report, indent=2, default=str))
    table.write_parquet(args.out / f"h2_{args.split}_features.parquet")
    for c in CONVENTIONS:
        block = result[c]
        print(f"{c}: gex b={block['gex']['b']:+.4f} t={block['gex']['t']:+.2f}", block.get("oos", ""))


if __name__ == "__main__":
    main()
