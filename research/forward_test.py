"""Forward test of H1: publish the 14:00 IST prediction before settlement, then score it.

    predict --snapshot snap.json --expiry YYYY-MM-DD   writes forward/{expiry}.json (+ a copy of the snapshot)
    verify forward/{expiry}.json                        recomputes the payload hash
    score --expiry YYYY-MM-DD [--settlement X]          writes forward/{expiry}_score.json

The prediction comes from a C2 monitor snapshot (docs/CONTRACTS.md). The record holds a `payload`
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
    """Payload hash check, plus the snapshot copy's hash when the copy is present."""
    record = json.loads(path.read_text(encoding="utf-8"))
    ok, msgs = verify_record(record), []
    msgs.append(f"payload sha256 {'OK' if ok else 'MISMATCH'}: {record.get('sha256')}")
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
        if a.cmd == "predict":
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
