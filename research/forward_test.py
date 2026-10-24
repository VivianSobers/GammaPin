"""Forward test of H1: publish the prediction before settlement, then score it.

    predict-eod --expiry YYYY-MM-DD                    end-of-day design (primary): eve-close bhavcopy prediction
    predict --snapshot snap.json --expiry YYYY-MM-DD   14:00 design: writes forward/{expiry}.json (+ a copy of the snapshot)
    verify forward/{expiry}.json                        recomputes the payload hash
    score --expiry YYYY-MM-DD [--settlement X]          writes forward/{expiry}_score.json

`predict-eod` uses h1_eod.features on the cached bhavcopy of the expiry eve (the H1 design in
PREREGISTRATION.md) and records the bhavcopy file's SHA-256; it must run before 15:30 IST on expiry day.
`predict` uses a C2 monitor snapshot (docs/CONTRACTS.md). The record holds a `payload`
and `sha256`, the SHA-256 of the payload's canonical JSON (sorted keys, no whitespace, ASCII).
The public timestamp is the git commit pushed to GitHub before 15:30 IST; posting the hash
elsewhere adds a second, independent witness.
"""

from __future__ import annotations

import argparse
import datetime as dt
import hashlib
import json
import math
import shutil
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
FORWARD = ROOT / "forward"
IST = dt.timezone(dt.timedelta(hours=5, minutes=30))
DECISION = dt.time(14, 0)
CUTOFF = dt.time(15, 0)
TOLERANCE = dt.timedelta(minutes=5)
MAX_PAIN_NOTE = "max_pain omitted: a C2 snapshot carries GEX per strike but no OI per strike, so max pain is not computable"


def canonical(obj: object) -> bytes:
    return json.dumps(obj, sort_keys=True, separators=(",", ":"), ensure_ascii=True, allow_nan=False).encode("ascii")


def sha256_hex(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def snapshot_time(snapshot: dict) -> dt.datetime:
    return dt.datetime.fromtimestamp(snapshot["ts_ms"] / 1000, IST)


def check_window(ts: dt.datetime, expiry: dt.date) -> list[str]:
    """Raise when `ts` (aware) is not on expiry day or is after 15:00 IST; warnings otherwise."""
    local = ts.astimezone(IST)
    if local.date() != expiry:
        raise ValueError(f"snapshot is from {local.date()} IST, not expiry day {expiry}")
    if local.time() > CUTOFF:
        raise ValueError(f"snapshot time {local:%H:%M:%S} IST is after the {CUTOFF:%H:%M} cutoff")
    target = dt.datetime.combine(expiry, DECISION, IST)
    if abs(local - target) > TOLERANCE:
        return [f"snapshot time {local:%H:%M:%S} IST is more than {TOLERANCE.seconds // 60} min from {DECISION:%H:%M}"]
    return []


def nearest_100(x: float) -> float:
    return math.floor(x / 100.0 + 0.5) * 100.0


def build_payload(snapshot: dict, snapshot_bytes: bytes, expiry: dt.date, created_at: dt.datetime) -> dict:
    """The prediction: per-convention pin (plus flip, regime, net GEX) and the C2-computable baselines."""
    if snapshot.get("expiry") != expiry.isoformat():
        raise ValueError(f"snapshot chain expiry {snapshot.get('expiry')} != {expiry}")
    ts = snapshot_time(snapshot)
    warnings = check_window(ts, expiry)
    if snapshot.get("source") != "live":
        warnings.append(f"snapshot source is {snapshot.get('source')!r}, not 'live'")
    f = float(snapshot["forward"])
    return {
        "hypothesis": "H1",
        "underlying": snapshot["underlying"],
        "expiry": expiry.isoformat(),
        "decision_time_ist": ts.isoformat(),
        "forward": f,
        "spot": snapshot.get("spot"),
        "t_years": snapshot.get("t_years"),
        "conventions": {
            name: {k: c.get(k) for k in ("pin", "flip", "regime", "net_gex")}
            for name, c in sorted(snapshot["conventions"].items())
        },
        "baselines": {"no_move": f, "nearest_100": nearest_100(f)},
        "notes": [MAX_PAIN_NOTE],
        "warnings": warnings,
        "snapshot": {"sha256": sha256_hex(snapshot_bytes), "ts_ms": snapshot["ts_ms"], "source": snapshot.get("source")},
        "created_at_utc": created_at.astimezone(dt.timezone.utc).isoformat(timespec="seconds"),
    }


def build_eod_payload(
    expiry: dt.date,
    created_at: dt.datetime,
    days: list[dt.date],
    load,
    weights_for,
    rate_on,
    source_sha256: str,
    calendar=(),
) -> dict:
    """The end-of-day prediction from the eve's bhavcopy (h1_eod.features), with max pain.

    `calendar` is an independent list of trading days (participant OI and index-close files). If one of
    them falls strictly between the last cached bhavcopy and the expiry, the true eve's bhavcopy is
    missing and the prediction is refused rather than made from an older day.
    """
    import bhavcopy
    import h1_eod

    eves = bhavcopy.eves(days, [expiry])
    if expiry not in eves:
        raise LookupError(f"no cached bhavcopy before {expiry}")
    eve = eves[expiry]
    skipped = sorted(d for d in calendar if eve < d < expiry)
    if skipped:
        raise LookupError(f"bhavcopy for trading day {skipped[-1]} is not cached; the eve would fall back to {eve}")
    local = created_at.astimezone(IST)
    close = dt.datetime.combine(expiry, dt.time(15, 30), IST)
    if local >= close:
        raise ValueError(f"created at {local:%Y-%m-%d %H:%M} IST, not before the {expiry} 15:30 close")
    warnings = []
    if local >= dt.datetime.combine(expiry, dt.time(9, 15), IST):
        warnings.append(f"created at {local:%H:%M} IST on expiry day, after the 09:15 open")
    try:
        weights, weights_ok = dict(weights_for(eve)), True
    except LookupError:
        weights, weights_ok = {"S0": (1.0, -1.0)}, False
    r = float(rate_on(eve))
    row = h1_eod.features(load(eve), expiry, eve, r, weights, days)
    if math.isnan(row["forward"]):
        raise ValueError(f"no forward or smile in the {eve} bhavcopy for {expiry}")

    def num(x):
        return None if x is None or (isinstance(x, float) and math.isnan(x)) else float(x)

    return {
        "hypothesis": "H1",
        "design": "eod",
        "underlying": "NIFTY",
        "expiry": expiry.isoformat(),
        "eve": eve.isoformat(),
        "decision_time_ist": dt.datetime.combine(eve, h1_eod.CLOSE, IST).isoformat(),
        "forward": float(row["forward"]),
        "r": r,
        "weights_ok": weights_ok,
        "conventions": {
            c: {"pin": num(row[f"pin_{c}"]), "flip": num(row[f"flip_{c}"]), "net_gex": num(row[f"net_gex_{c}"]),
                "weights": [num(row[f"w_c_{c}"]), num(row[f"w_p_{c}"])]}
            for c in ("S0", "S1", "S2")
        },
        "baselines": {"max_pain": num(row["max_pain"]), "nearest_100": num(row["nearest_100"]), "no_move": num(row["no_move"])},
        "notes": [],
        "warnings": warnings,
        "source": {"bhavcopy": bhavcopy.path_for(eve).name, "sha256": source_sha256},
        "created_at_utc": created_at.astimezone(dt.timezone.utc).isoformat(timespec="seconds"),
    }


def predict_eod(expiry: dt.date, out: Path = FORWARD, force: bool = False) -> Path:
    """Write forward/{expiry}.json from the cached eve bhavcopy, participant OI and T-bill rate."""
    import bhavcopy
    import participant_oi
    import rates
    import settlements

    days = bhavcopy.cached_days()
    eve = bhavcopy.eves(days, [expiry]).get(expiry)
    if eve is None:
        raise LookupError(f"no cached bhavcopy before {expiry}")
    poi, rate_df = participant_oi.load(), rates.load()
    payload = build_eod_payload(
        expiry, dt.datetime.now(dt.timezone.utc), days, bhavcopy.load,
        lambda d: participant_oi.weights_for(d, poi), lambda d: rates.rate_on(d, rate_df),
        sha256_hex(bhavcopy.path_for(eve).read_bytes()),
        calendar=set(poi["date"].to_list()) | set(settlements.load()),
    )
    path = prediction_path(out, expiry)
    if path.exists() and not force:
        raise FileExistsError(f"{path} exists; a published prediction must not be replaced (use --force before publishing only)")
    out.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(seal(payload), indent=2, sort_keys=True) + "\n", encoding="utf-8")
    return path


def seal(payload: dict) -> dict:
    return {"payload": payload, "sha256": sha256_hex(canonical(payload))}


def verify_record(record: dict) -> bool:
    return sha256_hex(canonical(record["payload"])) == record.get("sha256")


def prediction_path(out: Path, expiry: dt.date) -> Path:
    return out / f"{expiry.isoformat()}.json"


def snapshot_copy_path(out: Path, expiry: dt.date) -> Path:
    return out / f"{expiry.isoformat()}_snapshot.json"


def predict(snapshot_file: Path, expiry: dt.date, out: Path = FORWARD, force: bool = False) -> Path:
    """Write forward/{expiry}.json and an exact copy of the snapshot it came from."""
    raw = snapshot_file.read_bytes()
    record = seal(build_payload(json.loads(raw), raw, expiry, dt.datetime.now(dt.timezone.utc)))
    path = prediction_path(out, expiry)
    if path.exists() and not force:
        raise FileExistsError(f"{path} exists; a published prediction must not be replaced (use --force before publishing only)")
    out.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(record, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    shutil.copyfile(snapshot_file, snapshot_copy_path(out, expiry))
    return path


def verify(path: Path) -> tuple[bool, list[str]]:
    """Payload hash check, plus the snapshot copy's or the cached eve bhavcopy's hash when present."""
    record = json.loads(path.read_text(encoding="utf-8"))
    ok, msgs = verify_record(record), []
    msgs.append(f"payload sha256 {'OK' if ok else 'MISMATCH'}: {record.get('sha256')}")
    source = record["payload"].get("source") or {}
    if record["payload"].get("design") == "eod" and source.get("bhavcopy"):
        import bhavcopy

        cached = bhavcopy.CACHE_DIR / source["bhavcopy"]
        if cached.exists():
            same = sha256_hex(cached.read_bytes()) == source.get("sha256")
            ok &= same
            msgs.append(f"eve bhavcopy {cached.name} {'OK' if same else 'MISMATCH'}")
    copy = path.with_name(path.stem + "_snapshot.json")
    if copy.exists():
        same = sha256_hex(copy.read_bytes()) == record["payload"]["snapshot"]["sha256"]
        ok &= same
        msgs.append(f"snapshot copy {copy.name} {'OK' if same else 'MISMATCH'}")
    return ok, msgs


def score_record(record: dict, settlement: float) -> dict:
    """|settlement - prediction| per method; None where a convention had no pin."""
    p = record["payload"]
    preds = {name: c["pin"] for name, c in p["conventions"].items()} | p["baselines"]
    errors = {m: None if v is None else abs(settlement - float(v)) for m, v in preds.items()}
    return {"expiry": p["expiry"], "settlement": settlement, "prediction_sha256": record["sha256"],
            "predictions": preds, "abs_error": errors, "notes": p["notes"]}


def score(expiry: dt.date, settlement: float | None, out: Path = FORWARD) -> Path:
    path = prediction_path(out, expiry)
    ok, msgs = verify(path)
    if not ok:
        raise ValueError(f"{path} does not verify: " + "; ".join(msgs))
    if settlement is None:
        import settlements

        settlement = settlements.settlement(expiry)
    result = score_record(json.loads(path.read_text(encoding="utf-8")), settlement)
    result["scored_at_utc"] = dt.datetime.now(dt.timezone.utc).isoformat(timespec="seconds")
    dest = out / f"{expiry.isoformat()}_score.json"
    dest.write_text(json.dumps(result, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    return dest


def _rel(path: Path) -> str:
    try:
        return path.resolve().relative_to(ROOT).as_posix()
    except ValueError:
        return path.as_posix()


def publish_instructions(files: list[Path], sha: str, expiry: dt.date) -> str:
    return "\n".join([
        "Publish now, before 15:30 IST (the pushed commit is the public timestamp):",
        f"  git add {' '.join(_rel(f) for f in files)}",
        f'  git commit -m "forward: H1 prediction for {expiry} (sha256 {sha[:16]})"',
        "  git push origin main",
        f"Then post the hash somewhere public and timestamped: sha256 {sha}",
    ])


def main() -> None:
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    sub = ap.add_subparsers(dest="cmd", required=True)
    e = sub.add_parser("predict-eod", help="write the end-of-day prediction from the eve's cached bhavcopy")
    e.add_argument("--expiry", type=dt.date.fromisoformat, required=True)
    e.add_argument("--out", type=Path, default=FORWARD)
    e.add_argument("--force", action="store_true", help="overwrite an unpublished prediction")
    p = sub.add_parser("predict", help="write the 14:00 prediction from a C2 snapshot")
    p.add_argument("--snapshot", type=Path, required=True)
    p.add_argument("--expiry", type=dt.date.fromisoformat, required=True)
    p.add_argument("--out", type=Path, default=FORWARD)
    p.add_argument("--force", action="store_true", help="overwrite an unpublished prediction")
    v = sub.add_parser("verify", help="recompute a prediction's hash")
    v.add_argument("path", type=Path)
    s = sub.add_parser("score", help="score a verified prediction against the settlement")
    s.add_argument("--expiry", type=dt.date.fromisoformat, required=True)
    s.add_argument("--settlement", type=float, help="official Nifty 50 close (default: settlements cache)")
    s.add_argument("--out", type=Path, default=FORWARD)
    a = ap.parse_args()

    try:
        if a.cmd == "predict-eod":
            path = predict_eod(a.expiry, a.out, a.force)
            record = json.loads(path.read_text(encoding="utf-8"))
            for w in record["payload"]["warnings"]:
                print(f"WARNING: {w}", file=sys.stderr)
            print(f"wrote {path}\nsha256 {record['sha256']}\n")
            print(publish_instructions([path], record["sha256"], a.expiry))
        elif a.cmd == "predict":
            path = predict(a.snapshot, a.expiry, a.out, a.force)
            record = json.loads(path.read_text(encoding="utf-8"))
            for w in record["payload"]["warnings"]:
                print(f"WARNING: {w}", file=sys.stderr)
            print(f"wrote {path}\nsha256 {record['sha256']}\n")
            print(publish_instructions([path, snapshot_copy_path(a.out, a.expiry)], record["sha256"], a.expiry))
        elif a.cmd == "verify":
            ok, msgs = verify(a.path)
            print("\n".join(msgs))
            if not ok:
                sys.exit(1)
        else:
            print(f"wrote {score(a.expiry, a.settlement, a.out)}")
    except (ValueError, LookupError, FileExistsError, FileNotFoundError) as e:
        sys.exit(f"refused: {e}")


if __name__ == "__main__":
    main()
