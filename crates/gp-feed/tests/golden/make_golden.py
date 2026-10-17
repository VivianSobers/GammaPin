"""Write the synthetic golden frames for tests/golden.rs.

The frames are encoded here by hand from the protobuf wire format, independently of the
prost-generated decoder, so the Rust test checks the decoder against a second encoder.
All values are made up; no real market data is committed.

    python3 make_golden.py   # run from this directory; rewrites *.pb
"""

import struct
from pathlib import Path


def varint(n: int) -> bytes:
    n &= (1 << 64) - 1  # int64 negatives are ten-byte two's complement
    out = bytearray()
    while True:
        b = n & 0x7F
        n >>= 7
        if n:
            out.append(b | 0x80)
        else:
            out.append(b)
            return bytes(out)


def key(field: int, wire: int) -> bytes:
    return varint(field << 3 | wire)


def i64(field: int, v: int) -> bytes:
    return key(field, 0) + varint(v)


def f64(field: int, v: float) -> bytes:
    return key(field, 1) + struct.pack("<d", v)


def msg(field: int, body: bytes) -> bytes:
    return key(field, 2) + varint(len(body)) + body


def string(field: int, s: str) -> bytes:
    return msg(field, s.encode())


def ltpc(ltp: float, ltt: int, ltq: int, cp: float) -> bytes:
    return f64(1, ltp) + i64(2, ltt) + i64(3, ltq) + f64(4, cp)


def quote(bid_q: int, bid_p: float, ask_q: int, ask_p: float) -> bytes:
    return i64(1, bid_q) + f64(2, bid_p) + i64(3, ask_q) + f64(4, ask_p)


def feed_entry(instrument: str, feed: bytes) -> bytes:
    return msg(2, string(1, instrument) + msg(2, feed))


def response(kind: int, entries: list[bytes], current_ts: int, extra: bytes = b"") -> bytes:
    return i64(1, kind) + b"".join(entries) + i64(3, current_ts) + extra


T0 = 1_792_384_200_000  # 2026-10-19 10:00 IST

option_full = response(1, [
    feed_entry("NSE_FO|40001", msg(2, msg(1,
        msg(1, ltpc(152.35, T0 + 1_250, 75, 140.0))
        + msg(2, b"".join(msg(1, quote(750 * (i + 1), 152.30 - 0.05 * i, 600 + i, 152.40 + 0.05 * i)) for i in range(5)))
        + msg(3, f64(1, 0.52) + f64(3, 0.0011))  # option greeks: ignored
        + f64(5, 151.9)  # atp: ignored
        + i64(6, 1_234_500)
        + f64(7, 98_250.0)
        + f64(8, 0.1325)  # iv: ignored
    )) + i64(4, 1)),
], T0 + 1_260)

index_full = response(1, [
    feed_entry("NSE_INDEX|Nifty 50", msg(2, msg(2, msg(1, ltpc(25_012.4, T0 + 900, 0, 24_950.0)))) + i64(4, 1)),
], T0 + 905)

mixed = response(1, [
    feed_entry("NSE_FO|40002", msg(1, ltpc(88.0, T0 + 3_000, 150, 90.0)) + i64(4, 0)),
    feed_entry("NSE_FO|40003", msg(3,
        msg(1, ltpc(12.5, T0 + 2_000, 75, 13.0))
        + msg(2, quote(1_500, 12.45, 2_250, 12.55))
        + i64(4, 40_000)
        + f64(5, 7_125.0)
    ) + i64(4, 2)),
    feed_entry("NSE_FO|40001", msg(2, msg(1, msg(1, ltpc(-0.0, T0 - 5, -1, 0.0)))) + i64(4, 1)),
], T0 + 3_100)

# market_info only, plus an unknown field 99 that a newer feed version might add
market_info = response(2, [], T0, extra=msg(4, b"") + i64(99, 7))

for name, data in [
    ("option_full", option_full),
    ("index_full", index_full),
    ("mixed", mixed),
    ("market_info", market_info),
]:
    Path(f"{name}.pb").write_bytes(data)
