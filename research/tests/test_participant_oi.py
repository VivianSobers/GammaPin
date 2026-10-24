import datetime as dt
from pathlib import Path

import pytest
import requests

pytest.importorskip("gp_quant")
import participant_oi as poi

FIXTURE = Path(__file__).parent / "fixtures" / "participant_oi_sample.csv"
DAY = dt.date(2025, 10, 10)


def response(status: int, body: bytes = b"") -> requests.Response:
    r = requests.Response()
    r.status_code, r._content, r.url = status, body, "https://example.test"
    return r


class FakeSession:
    def __init__(self, script: dict[str, list]):
        self.script, self.calls = script, []

    def get(self, url, headers=None, timeout=None):
        self.calls.append(url)
        assert "Mozilla" in headers["User-Agent"]
        item = self.script[url].pop(0)
        if isinstance(item, Exception):
            raise item
        return item


@pytest.fixture
def sleeps(monkeypatch):
    log = []
    monkeypatch.setattr(poi.time, "sleep", log.append)
    return log


def write_day(folder: Path, day: dt.date, scale: int = 1) -> None:
    text = FIXTURE.read_text().replace("Oct 10, 2025", day.strftime("%b %d, %Y"))
    if scale != 1:
        text = text.replace("FII,20,150,700,400,1000,", f"FII,20,150,700,400,{1000 * scale},")
    poi.path_for(day, folder).parent.mkdir(parents=True, exist_ok=True)
    poi.path_for(day, folder).write_text(text)


def test_parse_real_layout():
    df = poi.parse(FIXTURE.read_text())
    assert df["client_type"].to_list() == ["Client", "DII", "FII", "Pro", "TOTAL"]
    assert df["date"].unique().to_list() == [DAY]
    for c in ("future_stock_short", "total_long_contracts", *poi.OPTION_INDEX):
        assert c in df.columns
    fii = df.filter(df["client_type"] == "FII")
    assert fii["option_index_call_long"][0] == 1000 and fii["option_index_put_short"][0] == 900


def test_parse_falls_back_to_given_day():
    text = "\n".join(FIXTURE.read_text().splitlines()[1:])
    assert poi.parse(text, DAY)["date"][0] == DAY
    with pytest.raises(ValueError):
        poi.parse(text)


def test_load_and_weights_use_prior_day_only(tmp_path):
    write_day(tmp_path, DAY)
    write_day(tmp_path, DAY + dt.timedelta(days=3), scale=2)
    df = poi.load(tmp_path)
    assert df.height == 10 and df["date"].n_unique() == 2

    w = poi.weights_for(DAY + dt.timedelta(days=1), df)
    # S1 writers = FII + Pro: calls (1000+2000-800-1700)/7010, puts (1500+2500-900-2900)/9020
    assert w["S0"] == (1.0, -1.0)
    assert w["S1"] == pytest.approx((500 / 7010, 200 / 9020))
    assert w["S2"] == pytest.approx((500 / 7010, 200 / 9020))  # -(client long - short)

    same_day = poi.weights_for(DAY + dt.timedelta(days=3), df)
    assert same_day["S1"] == w["S1"]  # day d's own file is not used on day d
    later = poi.weights_for(DAY + dt.timedelta(days=4), df)
    assert later["S1"][0] == pytest.approx((2000 + 2000 - 800 - 1700) / 7010)


def test_weights_lookup_errors(tmp_path):
    write_day(tmp_path, DAY)
    df = poi.load(tmp_path)
    with pytest.raises(LookupError):
        poi.weights_for(DAY, df)
    assert poi.weights_for(DAY + dt.timedelta(days=7), df)
    with pytest.raises(LookupError):
        poi.weights_for(DAY + dt.timedelta(days=8), df)


def test_load_empty_dir(tmp_path):
    with pytest.raises(FileNotFoundError):
        poi.load(tmp_path)


def test_weekdays_skip_weekend():
    days = poi.weekdays(dt.date(2025, 10, 10), dt.date(2025, 10, 13))
    assert days == [dt.date(2025, 10, 10), dt.date(2025, 10, 13)]


def test_download_skips_cache_handles_404_and_retries(tmp_path, sleeps):
    body = FIXTURE.read_bytes()
    days = [dt.date(2025, 10, 6), dt.date(2025, 10, 7), dt.date(2025, 10, 8), dt.date(2025, 10, 9)]
    url = lambda d: poi.URL.format(stamp=poi.stamp(d))  # noqa: E731
    write_day(tmp_path, days[0])
    session = FakeSession({
        url(days[1]): [response(503), requests.Timeout(), response(200, body)],
        url(days[2]): [response(404, b"<html>")],
        url(days[3]): [response(200, b"<html>not a csv</html>")],
    })
    res = poi.download(days, url, lambda d: poi.path_for(d, tmp_path), poi.looks_valid, session, pause=1.0)
    assert res == {"cached": [days[0]], "downloaded": [days[1]], "missing": [days[2]], "invalid": [days[3]]}
    assert poi.path_for(days[1], tmp_path).read_bytes() == body
    assert not poi.path_for(days[3], tmp_path).exists()
    assert sleeps == [2.0, 4.0, 1.0, 1.0]  # two backoffs, then the pause before each later request


def test_get_gives_up_after_retries(sleeps):
    url = "https://example.test/x.csv"
    session = FakeSession({url: [response(500)] * 3})
    with pytest.raises(requests.HTTPError):
        poi.get(session, url, retries=2)
    assert len(session.calls) == 3 and sleeps == [2.0, 4.0]


def test_get_raises_on_client_error(sleeps):
    url = "https://example.test/x.csv"
    with pytest.raises(requests.HTTPError):
        poi.get(FakeSession({url: [response(403)]}), url)
    assert sleeps == []
