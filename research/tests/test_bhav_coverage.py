import datetime as dt

import polars as pl
import bhav_coverage as coverage

D = dt.date
# Mon 5 .. Fri 16 Oct 2026; 6 Oct (Tue expiry) has no bhavcopy but an index close, 9 Oct has no file at all
DAYS = [D(2026, 10, 5), D(2026, 10, 7), D(2026, 10, 8), D(2026, 10, 12), D(2026, 10, 13), D(2026, 10, 14),
        D(2026, 10, 15), D(2026, 10, 16)]


def load(day):
    nxt = [e for e in (D(2026, 10, 6), D(2026, 10, 13), D(2026, 10, 20)) if e > day][0]
    return pl.DataFrame({"date": [day, day, day], "expiry": [nxt, nxt, D(2026, 12, 29)]})


def test_report():
    poi = pl.DataFrame({"date": [D(2026, 10, 2)], "client_type": ["TOTAL"]})
    rate_df = pl.DataFrame({"date": [D(2026, 10, 1)], "r": [0.06]})
    rep = coverage.report(DAYS, load, D(2026, 10, 1), D(2026, 10, 14), poi, rate_df,
                          set(DAYS) | {D(2026, 10, 6)})
    by = {r["expiry"]: r for r in rep["rows"]}
    assert set(by) == {D(2026, 10, 6), D(2026, 10, 13)}
    assert not by[D(2026, 10, 6)]["expiry_bhavcopy"] and by[D(2026, 10, 13)]["expiry_bhavcopy"]
    assert by[D(2026, 10, 13)]["contracts"] == 3 and by[D(2026, 10, 13)]["contracts_expiry"] == 2
    # participant OI older than 7 days on the 12 Oct eve; the rate has no age limit
    assert by[D(2026, 10, 6)]["participant_oi"] and not by[D(2026, 10, 13)]["participant_oi"]
    assert all(r["tbill"] for r in rep["rows"])
    assert rep["gaps"] == [D(2026, 10, 6)] and rep["holidays"] == [D(2026, 10, 9)]
    assert "| 2026-10-06 | 2026-10-05 |" in coverage.markdown(rep)
