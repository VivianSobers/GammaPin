import datetime as dt

import polars as pl
import pytest
import requests

pytest.importorskip("gp_quant")
import fetch_expired as fe  # noqa: E402
import gpdata  # noqa: E402

EXPIRY = dt.date(2025, 4, 24)
SESSIONS = [dt.date(2025, 4, 22), dt.date(2025, 4, 23), EXPIRY]
INDEX_URL = "https://api.upstox.com/v3/historical-candle/NSE_INDEX%7CNifty%2050/minutes/1/2025-04-24/2025-04-18"


def contract(token: int, strike: float, kind: str) -> dict:
    return {
        "name": "NIFTY", "segment": "NSE_FO", "exchange": "NSE", "expiry": EXPIRY.isoformat(),
        "instrument_key": f"NSE_FO|{token}|24-04-2025", "exchange_token": str(token),
        "trading_symbol": f"NIFTY {strike:.0f} {kind} 24 APR 25", "tick_size": 5, "lot_size": 75,
        "minimum_lot": 75, "freeze_quantity": 1800, "instrument_type": kind, "strike_price": strike,
        "underlying_key": "NSE_INDEX|Nifty 50", "underlying_type": "INDEX", "underlying_symbol": "NIFTY", "weekly": True,
    }


CONTRACTS = [contract(73507, 24000.0, "CE"), contract(73508, 24000.0, "PE"), contract(73600, 26500.0, "CE")]


def minutes(day: dt.date, n: int = fe.SESSION_MINUTES) -> list[str]:
    t0 = dt.datetime.combine(day, dt.time(9, 15))
    return [(t0 + dt.timedelta(minutes=m)).strftime("%Y-%m-%dT%H:%M:%S+05:30") for m in range(n)]


def candles(days: list[dt.date], n: int = fe.SESSION_MINUTES, oi: float = 7500.0, base: float = 100.0) -> list[list]:
    rows = [[ts, base, base + 1, base - 1, base + 0.5, 10, oi] for d in days for ts in minutes(d, n)]
    return rows[::-1]  # newest first, as in the docs' sample


def ok(data) -> requests.Response:
    return reply(200, {"status": "success", "data": data})


def reply(status: int, body) -> requests.Response:
    r = requests.Response()
    r.status_code, r.url = status, "https://api.upstox.com"
    r._content = __import__("json").dumps(body).encode()
    return r


class FakeSession:
    def __init__(self, routes: dict[str, list[requests.Response]]):
        self.routes, self.calls = routes, []

    def get(self, url, headers=None, timeout=None):
        assert headers["Authorization"] == "Bearer T" and headers["Accept"] == "application/json"
        self.calls.append(url)
        queue = self.routes[url]
        return queue.pop(0) if len(queue) > 1 else queue[0]


def candle_url(key: str) -> str:
    return f"https://api.upstox.com/v2/expired-instruments/historical-candle/{fe.enc(key)}/1minute/2025-04-24/2025-04-18"


def routes(index_minutes: int = fe.SESSION_MINUTES) -> dict[str, list[requests.Response]]:
    index = [[ts, 24000.0, 24010.0, 23990.0, 24000.0 + i, 0, 0] for i, ts in enumerate(
        t for d in SESSIONS for t in minutes(d, index_minutes))]
    return {
        f"{fe.BASE_V2}/expired-instruments/expiries?instrument_key=NSE_INDEX%7CNifty%2050": [ok(["2025-04-17", "2025-04-24"])],
        f"{fe.BASE_V2}/expired-instruments/option/contract?instrument_key=NSE_INDEX%7CNifty%2050&expiry_date=2025-04-24":
            [ok(CONTRACTS)],
        INDEX_URL: [ok({"candles": index})],
        candle_url(CONTRACTS[0]["instrument_key"]): [ok({"candles": candles(SESSIONS)})],
        candle_url(CONTRACTS[1]["instrument_key"]): [ok({"candles": candles(SESSIONS[-1:], n=30, base=50.0)})],
        candle_url(CONTRACTS[2]["instrument_key"]): [ok({"candles": []})],
    }


@pytest.fixture
def sleeps(monkeypatch):
    log = []
    monkeypatch.setattr(fe.time, "sleep", log.append)
    return log


def test_windows_cover_range_in_month_chunks():
    w = fe.windows(dt.date(2025, 1, 1), dt.date(2025, 3, 15))
    assert w[0][0] == dt.date(2025, 1, 1) and w[-1][1] == dt.date(2025, 3, 15)
    assert all((hi - lo).days < fe.MAX_REQUEST_DAYS for lo, hi in w)
    assert all(b[0] - a[1] == dt.timedelta(days=1) for a, b in zip(w, w[1:]))
    assert fe.windows(EXPIRY, EXPIRY) == [(EXPIRY, EXPIRY)]


def test_url_encoding_of_keys():
    assert fe.enc("NSE_FO|73507|24-04-2025") == "NSE_FO%7C73507%7C24-04-2025"
    assert fe.enc("NSE_INDEX|Nifty 50") == "NSE_INDEX%7CNifty%2050"


def test_candles_frame_naive_ist_sorted():
    df = fe.candles_frame([["2025-04-24T09:16:00+05:30", 1, 2, 0.5, 1.5, 3, 150],
                           ["2025-04-24T03:45:00Z", 1, 2, 0.5, 1.5, 4, 75]])
    assert df.schema == pl.Schema(fe.CANDLE_SCHEMA)
    assert df["ts"].to_list() == [dt.datetime(2025, 4, 24, 9, 15), dt.datetime(2025, 4, 24, 9, 16)]
    assert fe.candles_frame([]).schema == pl.Schema(fe.CANDLE_SCHEMA)


def test_option_frame_oi_units():
    raw = [["2025-04-24T09:15:00+05:30", 1, 2, 0.5, 1.5, 3, 7500]]
    assert fe.option_frame(CONTRACTS[0], raw, oi_in_units=True)["oi"][0] == 100.0
    assert fe.option_frame(CONTRACTS[0], raw, oi_in_units=False)["oi"][0] == 7500.0


def test_client_retries_then_raises(sleeps):
    url = "https://api.upstox.com/x"
    session = FakeSession({url: [reply(429, {}), reply(503, {}), ok([1])]})
    assert fe.Client("T", session, pause=0.0, backoff=0.5).get(url) == [1]
    assert sleeps == [0.5, 1.0]
    with pytest.raises(fe.ApiError) as e:
        fe.Client("T", FakeSession({url: [reply(401, {"status": "error"})]}), pause=0.0).get(url)
    assert e.value.status == 401
    with pytest.raises(fe.ApiError):
        fe.Client("T", FakeSession({url: [reply(500, {})]}), pause=0.0, retries=1).get(url)


def test_client_throttles(sleeps, monkeypatch):
    clock = iter([0.0, 0.0, 0.1, 0.3])
    monkeypatch.setattr(fe.time, "monotonic", lambda: next(clock))
    url = "https://api.upstox.com/x"
    client = fe.Client("T", FakeSession({url: [ok([1])]}), pause=0.3)
    client.get(url)
    client.get(url)
    assert sleeps == [pytest.approx(0.2)]


def test_run_writes_gpdata_schema_and_coverage(tmp_path, sleeps):
    session = FakeSession(routes())
    client = fe.Client("T", session, pause=0.0)
    assert fe.run(client, tmp_path, start=dt.date(2025, 4, 20)) == [EXPIRY]

    opts = pl.read_parquet(tmp_path / gpdata.options_path(EXPIRY).name)
    assert opts.schema == pl.Schema(fe.OPTION_SCHEMA)
    assert opts.height == 3 * fe.SESSION_MINUTES + 30
    assert opts["oi"].unique().to_list() == [100.0]  # 7500 units / lot 75
    assert opts["ts"].is_sorted()
    index = pl.read_parquet(tmp_path / gpdata.index_path(EXPIRY).name)
    assert index.columns == ["ts", "open", "high", "low", "close"] and index.height == 3 * fe.SESSION_MINUTES
    assert index.schema["ts"] == pl.Datetime("ms")

    chain = gpdata.chain_at(opts, dt.datetime.combine(EXPIRY, dt.time(9, 30)))
    assert chain["strike"].tolist() == [24000.0] and chain["call_oi"][0] == 100.0

    cov = pl.read_csv(tmp_path / "coverage.csv")
    assert cov.columns == ["expiry", "instrument_key", "strike", "opt_type", "rows", "first_ts", "last_ts", "coverage"]
    assert cov.sort("instrument_key")["rows"].to_list() == [1125, 30, 0]
    summary = pl.read_csv(tmp_path / "coverage_summary.csv").row(0, named=True)
    assert summary["expected_minutes"] == 1125 and summary["union_option_minutes"] == 1125
    assert summary["complete"] is True and summary["contracts_with_data"] == 2

    calls = len(session.calls)
    assert fe.run(client, tmp_path, start=dt.date(2025, 4, 20)) == []
    assert len(session.calls) == calls + 1  # only the expiries list
    assert fe.run(client, tmp_path, only=[EXPIRY], force=True) == [EXPIRY]
    assert pl.read_csv(tmp_path / "coverage.csv").height == 3  # replaced, not appended


def test_band_filter_and_incomplete_flag(tmp_path, sleeps):
    client = fe.Client("T", FakeSession(routes(index_minutes=300)), pause=0.0)
    summary = fe.fetch_expiry(client, EXPIRY, tmp_path, band_pct=5.0).row(0, named=True)
    assert summary["contracts"] == 2  # 26500 is > 5% from the expiry-eve close (~24300)
    assert summary["complete"] is False  # index has 300 of 375 minutes per session
    assert fe.prior_close(pl.read_parquet(tmp_path / gpdata.index_path(EXPIRY).name), EXPIRY) == 24000.0 + 599


def test_contract_level_400_is_logged_not_fatal(tmp_path, sleeps):
    r = routes()
    r[candle_url(CONTRACTS[2]["instrument_key"])] = [reply(400, {"status": "error", "errors": []})]
    client = fe.Client("T", FakeSession(r), pause=0.0)
    assert fe.fetch_expiry(client, EXPIRY, tmp_path).row(0, named=True)["contracts_with_data"] == 2


def test_token_sources(tmp_path, monkeypatch):
    monkeypatch.setenv("UPSTOX_ACCESS_TOKEN", "env-token")
    assert fe.read_token(None) == "env-token"
    (tmp_path / "t.token").write_text("file-token\n")
    assert fe.read_token(tmp_path / "t.token") == "file-token"
