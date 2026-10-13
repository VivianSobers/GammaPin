import datetime as dt
from pathlib import Path

import pytest
import requests

import participant_oi
import settlements as st

FIXTURE = Path(__file__).parent / "fixtures" / "index_close_sample.csv"
DAY = dt.date(2025, 10, 10)


def response(status: int, body: bytes = b"") -> requests.Response:
    r = requests.Response()
    r.status_code, r._content, r.url = status, body, "https://example.test"
    return r


class FakeSession:
    def __init__(self, script: dict[str, requests.Response]):
        self.script, self.calls = script, []

    def get(self, url, headers=None, timeout=None):
        self.calls.append(url)
        return self.script[url]


def test_parse_picks_nifty_50_close():
    assert st.parse(FIXTURE.read_text()) == (DAY, 25250.55)
    assert st.parse(FIXTURE.read_text(), "Nifty Next 50") == (DAY, 68700.0)
    assert st.parse(FIXTURE.read_text(), "Nifty Bank") is None


def test_parse_date_formats():
    assert st.parse_date("10-10-2025") == st.parse_date("10-Oct-2025") == DAY
    with pytest.raises(ValueError):
        st.parse_date("2025/10/10")


def test_fetch_load_settlement(tmp_path, monkeypatch):
    monkeypatch.setattr(participant_oi.time, "sleep", lambda s: None)
    holiday = dt.date(2025, 10, 9)
    session = FakeSession({
        st.URL.format(stamp="10102025"): response(200, FIXTURE.read_bytes()),
        st.URL.format(stamp="09102025"): response(404, b"<html>"),
    })
    res = st.fetch([holiday, DAY], tmp_path, session=session)
    assert res["downloaded"] == [DAY] and res["missing"] == [holiday]
    assert st.path_for(DAY, tmp_path).exists()

    closes = st.load(tmp_path)
    assert closes == {DAY: 25250.55}
    assert st.settlement(DAY, closes) == 25250.55
    with pytest.raises(LookupError):
        st.settlement(holiday, closes)

    again = st.fetch([DAY], tmp_path, session=FakeSession({}))
    assert again["cached"] == [DAY]
