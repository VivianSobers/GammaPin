import datetime as dt
import json
import types
import sys
from pathlib import Path

import pytest

import forward_test as ft

MOCK = Path(__file__).resolve().parents[2] / "crates" / "gp-monitor" / "static" / "mock" / "snapshot.json"
EXPIRY = dt.date(2026, 10, 19)


def ts_ms(h, m, s=0, day=EXPIRY):
    return int(dt.datetime.combine(day, dt.time(h, m, s), ft.IST).timestamp() * 1000)


def snapshot_file(tmp_path, **changes) -> Path:
    """The C2 mock, with `ts_ms` moved to 14:00 IST on its own expiry day unless overridden."""
    snap = json.loads(MOCK.read_text(encoding="utf-8")) | {"ts_ms": ts_ms(14, 0, 3)} | changes
    tmp_path.mkdir(parents=True, exist_ok=True)
    path = tmp_path / "snap.json"
    path.write_text(json.dumps(snap), encoding="utf-8")
    return path


def test_off_day_snapshot_is_refused(tmp_path):
    eve = ts_ms(14, 0, day=EXPIRY - dt.timedelta(days=3))
    with pytest.raises(ValueError, match="not expiry day"):
        ft.predict(snapshot_file(tmp_path, ts_ms=eve), EXPIRY, tmp_path / "fwd")


def test_predict_on_mock(tmp_path):
    path = ft.predict(snapshot_file(tmp_path), EXPIRY, tmp_path / "fwd")
    assert path == tmp_path / "fwd" / "2026-10-19.json"
    rec = json.loads(path.read_text(encoding="utf-8"))
    p = rec["payload"]
    assert p["conventions"]["S0"] == {"pin": 25000, "flip": 24871.3, "regime": "long_gamma", "net_gex": 306134000000.0}
    assert p["conventions"]["S1"]["pin"] == 25050 and p["conventions"]["S1"]["regime"] == "short_gamma"
    assert p["baselines"] == {"no_move": 25012.4, "nearest_100": 25000.0}
    assert "max_pain" not in p["baselines"] and any("max_pain" in n for n in p["notes"])
    assert p["warnings"] == ["snapshot source is 'replay', not 'live'"]
    assert p["decision_time_ist"] == "2026-10-19T14:00:03+05:30"
    copy = tmp_path / "fwd" / "2026-10-19_snapshot.json"
    assert ft.sha256_hex(copy.read_bytes()) == p["snapshot"]["sha256"]
    assert ft.verify(path)[0]
    with pytest.raises(FileExistsError):
        ft.predict(snapshot_file(tmp_path), EXPIRY, tmp_path / "fwd")


def test_hash_is_canonical_and_stable():
    a = {"b": [1, 2.5, None], "a": {"y": "x", "x": 1}}
    b = {"a": {"x": 1, "y": "x"}, "b": [1, 2.5, None]}
    assert ft.canonical(a) == b'{"a":{"x":1,"y":"x"},"b":[1,2.5,null]}'
    assert ft.seal(a)["sha256"] == ft.seal(b)["sha256"] == ft.sha256_hex(ft.canonical(a))
    rec = json.loads(json.dumps(ft.seal(a), indent=4))
    assert ft.verify_record(rec)


def test_tamper_detection(tmp_path):
    path = ft.predict(snapshot_file(tmp_path), EXPIRY, tmp_path)
    rec = json.loads(path.read_text(encoding="utf-8"))
    rec["payload"]["conventions"]["S0"]["pin"] = 25050
    path.write_text(json.dumps(rec), encoding="utf-8")
    ok, msgs = ft.verify(path)
    assert not ok and "MISMATCH" in msgs[0]
    with pytest.raises(ValueError, match="does not verify"):
        ft.score(EXPIRY, 25_010.0, tmp_path)


def test_snapshot_copy_tamper_detected(tmp_path):
    path = ft.predict(snapshot_file(tmp_path), EXPIRY, tmp_path)
    copy = ft.snapshot_copy_path(tmp_path, EXPIRY)
    copy.write_text(copy.read_text(encoding="utf-8").replace("25012.4", "25112.4"), encoding="utf-8")
    ok, msgs = ft.verify(path)
    assert not ok and "snapshot copy" in msgs[1]


@pytest.mark.parametrize(
    "when, match",
    [((15, 0, 1), "after the 15:00 cutoff"), ((15, 20), "after")],
)
def test_after_cutoff_is_refused(tmp_path, when, match):
    with pytest.raises(ValueError, match=match):
        ft.predict(snapshot_file(tmp_path, ts_ms=ts_ms(*when)), EXPIRY, tmp_path)


def test_wrong_day_and_wrong_chain_refused(tmp_path):
    with pytest.raises(ValueError, match="not expiry day"):
        ft.predict(snapshot_file(tmp_path, ts_ms=ts_ms(14, 0, day=EXPIRY - dt.timedelta(days=1))), EXPIRY, tmp_path)
    with pytest.raises(ValueError, match="chain expiry"):
        ft.predict(snapshot_file(tmp_path, expiry="2026-10-27"), EXPIRY, tmp_path)


def test_window_warnings():
    assert ft.check_window(dt.datetime.combine(EXPIRY, dt.time(14, 4, 59), ft.IST), EXPIRY) == []
    assert ft.check_window(dt.datetime.combine(EXPIRY, dt.time(13, 54, 59), ft.IST), EXPIRY)
    assert ft.check_window(dt.datetime.combine(EXPIRY, dt.time(14, 30), ft.IST), EXPIRY)
    utc = dt.datetime(2026, 10, 19, 8, 30, tzinfo=dt.timezone.utc)
    assert ft.check_window(utc, EXPIRY) == []
    assert ft.check_window(dt.datetime.combine(EXPIRY, dt.time(15, 0), ft.IST), EXPIRY)


def test_nearest_100():
    assert [ft.nearest_100(x) for x in (25_049.99, 25_050.0, 24_950.0, 25_012.4)] == [25_000.0, 25_100.0, 25_000.0, 25_000.0]


def test_score_maths(tmp_path):
    ft.predict(snapshot_file(tmp_path), EXPIRY, tmp_path)
    dest = ft.score(EXPIRY, 25_037.15, tmp_path)
    s = json.loads(dest.read_text(encoding="utf-8"))
    assert dest.name == "2026-10-19_score.json"
    assert s["abs_error"] == pytest.approx({"S0": 37.15, "S1": 12.85, "no_move": 24.75, "nearest_100": 37.15})
    assert s["prediction_sha256"] == json.loads((tmp_path / "2026-10-19.json").read_text())["sha256"]


def test_score_with_missing_pin():
    rec = ft.seal({"expiry": "2026-10-19", "conventions": {"S0": {"pin": None}}, "baselines": {"no_move": 100.0},
                   "notes": []})
    assert ft.score_record(rec, 90.0)["abs_error"] == {"S0": None, "no_move": 10.0}


def test_cli_predict_verify_score(tmp_path, monkeypatch, capsys):
    snap, out = snapshot_file(tmp_path), tmp_path / "fwd"
    monkeypatch.setattr(sys, "argv", ["forward_test.py", "predict", "--snapshot", str(snap), "--expiry", "2026-10-19",
                                      "--out", str(out)])
    ft.main()
    printed = capsys.readouterr()
    sha = json.loads((out / "2026-10-19.json").read_text())["sha256"]
    assert "git push origin main" in printed.out and sha in printed.out and "WARNING" in printed.err
    monkeypatch.setattr(sys, "argv", ["forward_test.py", "verify", str(out / "2026-10-19.json")])
    ft.main()
    monkeypatch.setattr(sys, "argv", ["forward_test.py", "score", "--expiry", "2026-10-19", "--settlement", "25000",
                                      "--out", str(out)])
    ft.main()
    assert json.loads((out / "2026-10-19_score.json").read_text())["abs_error"]["S0"] == 0.0
    off_day = snapshot_file(tmp_path / "off", ts_ms=ts_ms(14, 0, day=EXPIRY - dt.timedelta(days=3)))
    monkeypatch.setattr(sys, "argv", ["forward_test.py", "predict", "--snapshot", str(off_day), "--expiry", "2026-10-19",
                                      "--out", str(tmp_path / "x")])
    with pytest.raises(SystemExit, match="refused"):
        ft.main()


def test_score_reads_settlements_lazily(tmp_path, monkeypatch):
    fake = types.SimpleNamespace(settlement=lambda e: {EXPIRY: 25_100.0}[e])
    monkeypatch.setitem(sys.modules, "settlements", fake)
    ft.predict(snapshot_file(tmp_path), EXPIRY, tmp_path)
    s = json.loads(ft.score(EXPIRY, None, tmp_path).read_text())
    assert s["settlement"] == 25_100.0 and s["abs_error"]["S1"] == 50.0


# --- end-of-day design ---------------------------------------------------------------------------

def _eod_payload(created_at, weights_for=None, calendar=()):
    import test_h1_eod as eod

    b = eod.book()
    e = dt.date(2026, 10, 13)
    return ft.build_eod_payload(
        e, created_at, sorted(b), eod.loader(b), weights_for or eod.weights, lambda d: eod.R, "ab" * 32,
        calendar=calendar,
    ), eod.CASES[e]


def test_eod_payload_refuses_when_the_true_eve_is_not_cached():
    import test_h1_eod as eod

    b, e = eod.book(), dt.date(2026, 10, 13)
    eve = eod.CASES[e][0]
    created = dt.datetime(2026, 10, 12, 20, 0, tzinfo=ft.IST)
    days = [d for d in sorted(b) if d != eve]  # the eve's file was never downloaded
    with pytest.raises(LookupError, match=f"{eve} is not cached"):
        ft.build_eod_payload(e, created, days, eod.loader(b), eod.weights, lambda d: eod.R, "ab" * 32,
                             calendar=[eve])
    payload, _ = _eod_payload(created, calendar=[eve - dt.timedelta(days=3), e])
    assert payload["eve"] == eve.isoformat()  # days outside (eve, expiry) change nothing


def test_eod_payload_predicts_from_the_eve_and_seals():
    payload, (eve, f, cw, _, settle) = _eod_payload(dt.datetime(2026, 10, 12, 20, 0, tzinfo=ft.IST))
    assert payload["eve"] == eve.isoformat() and payload["design"] == "eod"
    assert payload["conventions"]["S0"]["pin"] == cw
    assert payload["baselines"]["nearest_100"] == 25_000.0 and payload["baselines"]["max_pain"] is not None
    assert payload["warnings"] == []
    record = ft.seal(payload)
    assert ft.verify_record(record)
    scored = ft.score_record(record, settle)
    assert scored["abs_error"]["S0"] == pytest.approx(abs(settle - cw))


def test_eod_payload_refuses_after_the_close_and_warns_after_the_open():
    with pytest.raises(ValueError):
        _eod_payload(dt.datetime(2026, 10, 13, 15, 30, tzinfo=ft.IST))
    payload, _ = _eod_payload(dt.datetime(2026, 10, 13, 10, 0, tzinfo=ft.IST))
    assert payload["warnings"]


def test_eod_payload_without_participant_oi_keeps_s0_only():
    def none(day):
        raise LookupError(day)

    payload, _ = _eod_payload(dt.datetime(2026, 10, 12, 20, 0, tzinfo=ft.IST), none)
    assert payload["weights_ok"] is False
    assert payload["conventions"]["S1"]["pin"] is None and payload["conventions"]["S1"]["weights"] == [None, None]
    ft.seal(payload)
