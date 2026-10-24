//! Raw WebSocket frame → `FeedResponse` → one flat [`Update`] per instrument.

use prost::Message;

use crate::proto::{FeedResponse, feed::FeedUnion, full_feed::FullFeedUnion};

/// Market depth levels kept per side (Upstox `full_d5`).
pub const DEPTH: usize = 5;

/// One instrument's state as carried by one frame. Fields the feed mode does not send are 0.
#[derive(Debug, Clone, PartialEq)]
pub struct Update {
    pub instrument_key: String,
    /// Exchange last-trade time, Unix ms.
    pub ltt_ms: i64,
    pub ltp: f64,
    pub ltq: i64,
    /// Cumulative day volume (`vtt`).
    pub cum_volume: i64,
    /// Open interest exactly as the feed sends it.
    pub oi: f64,
    pub bid_px: [f64; DEPTH],
    pub bid_qty: [i64; DEPTH],
    pub ask_px: [f64; DEPTH],
    pub ask_qty: [i64; DEPTH],
    /// Upstox server time of the frame, Unix ms.
    pub current_ts: i64,
}

pub fn decode_frame(bytes: &[u8]) -> Result<FeedResponse, prost::DecodeError> {
    FeedResponse::decode(bytes)
}

fn blank(instrument_key: &str, current_ts: i64) -> Update {
    Update {
        instrument_key: instrument_key.to_string(),
        ltt_ms: 0,
        ltp: 0.0,
        ltq: 0,
        cum_volume: 0,
        oi: 0.0,
        bid_px: [0.0; DEPTH],
        bid_qty: [0; DEPTH],
        ask_px: [0.0; DEPTH],
        ask_qty: [0; DEPTH],
        current_ts,
    }
}

/// Flatten a response into updates sorted by instrument key.
///
/// Feeds without an `LTPC` block are skipped: the trade time they are ordered by comes from it.
pub fn updates(resp: &FeedResponse) -> Vec<Update> {
    let mut keys: Vec<&String> = resp.feeds.keys().collect();
    keys.sort();
    let mut out = Vec::with_capacity(keys.len());
    for key in keys {
        let Some(union) = &resp.feeds[key].feed_union else {
            continue;
        };
        let mut u = blank(key, resp.current_ts);
        let ltpc = match union {
            FeedUnion::Ltpc(l) => Some(l),
            FeedUnion::FirstLevelWithGreeks(f) => {
                u.cum_volume = f.vtt;
                u.oi = f.oi;
                if let Some(q) = &f.first_depth {
                    (u.bid_px[0], u.bid_qty[0]) = (q.bid_p, q.bid_q);
                    (u.ask_px[0], u.ask_qty[0]) = (q.ask_p, q.ask_q);
                }
                f.ltpc.as_ref()
            }
            FeedUnion::FullFeed(full) => match &full.full_feed_union {
                Some(FullFeedUnion::MarketFf(m)) => {
                    u.cum_volume = m.vtt;
                    u.oi = m.oi;
                    if let Some(level) = &m.market_level {
                        for (i, q) in level.bid_ask_quote.iter().take(DEPTH).enumerate() {
                            (u.bid_px[i], u.bid_qty[i]) = (q.bid_p, q.bid_q);
                            (u.ask_px[i], u.ask_qty[i]) = (q.ask_p, q.ask_q);
                        }
                    }
                    m.ltpc.as_ref()
                }
                Some(FullFeedUnion::IndexFf(i)) => i.ltpc.as_ref(),
                None => None,
            },
        };
        let Some(l) = ltpc else { continue };
        u.ltt_ms = l.ltt;
        u.ltp = l.ltp;
        u.ltq = l.ltq;
        out.push(u);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::proto::*;
    use std::collections::HashMap;

    fn ltpc(ltp: f64, ltt: i64) -> Option<Ltpc> {
        Some(Ltpc {
            ltp,
            ltt,
            ltq: 75,
            cp: 0.0,
            iep: None,
        })
    }

    fn response(feeds: Vec<(&str, FeedUnion)>) -> FeedResponse {
        FeedResponse {
            r#type: Type::LiveFeed as i32,
            feeds: feeds
                .into_iter()
                .map(|(k, f)| {
                    (
                        k.to_string(),
                        Feed {
                            feed_union: Some(f),
                            request_mode: RequestMode::FullD5 as i32,
                        },
                    )
                })
                .collect::<HashMap<_, _>>(),
            current_ts: 99,
            market_info: None,
        }
    }

    fn quote(i: i64) -> Quote {
        Quote {
            bid_q: 100 + i,
            bid_p: 10.0 - i as f64 * 0.05,
            ask_q: 200 + i,
            ask_p: 10.1 + i as f64 * 0.05,
        }
    }

    #[test]
    fn full_feed_option() {
        let resp = response(vec![(
            "NSE_FO|1",
            FeedUnion::FullFeed(FullFeed {
                full_feed_union: Some(FullFeedUnion::MarketFf(MarketFullFeed {
                    ltpc: ltpc(10.05, 1_000),
                    market_level: Some(MarketLevel {
                        bid_ask_quote: (0..7).map(quote).collect(),
                    }),
                    vtt: 5_000,
                    oi: 1_234.0,
                    ..Default::default()
                })),
            }),
        )]);
        let u = &updates(&decode_frame(&resp.encode_to_vec()).unwrap())[0];
        assert_eq!((u.ltt_ms, u.ltp, u.ltq), (1_000, 10.05, 75));
        assert_eq!((u.cum_volume, u.oi, u.current_ts), (5_000, 1_234.0, 99));
        assert_eq!(u.bid_qty, [100, 101, 102, 103, 104]);
        assert_eq!(u.ask_px[4], 10.1 + 4.0 * 0.05);
    }

    #[test]
    fn index_ltpc_and_first_level() {
        let resp = response(vec![
            (
                "NSE_INDEX|Nifty 50",
                FeedUnion::FullFeed(FullFeed {
                    full_feed_union: Some(FullFeedUnion::IndexFf(IndexFullFeed {
                        ltpc: ltpc(25_000.0, 2_000),
                        market_ohlc: None,
                    })),
                }),
            ),
            ("NSE_FO|3", FeedUnion::Ltpc(ltpc(5.0, 3_000).unwrap())),
            (
                "NSE_FO|2",
                FeedUnion::FirstLevelWithGreeks(FirstLevelWithGreeks {
                    ltpc: ltpc(7.0, 4_000),
                    first_depth: Some(quote(0)),
                    vtt: 10,
                    oi: 20.0,
                    ..Default::default()
                }),
            ),
        ]);
        let us = updates(&resp);
        let keys: Vec<_> = us.iter().map(|u| u.instrument_key.as_str()).collect();
        assert_eq!(keys, ["NSE_FO|2", "NSE_FO|3", "NSE_INDEX|Nifty 50"]);
        assert_eq!(
            (us[0].bid_px[0], us[0].bid_qty[1], us[0].oi),
            (10.0, 0, 20.0)
        );
        assert_eq!((us[1].ltp, us[1].oi), (5.0, 0.0));
        assert_eq!((us[2].ltp, us[2].bid_px), (25_000.0, [0.0; DEPTH]));
    }

    #[test]
    fn skips_feeds_without_trade_time() {
        let resp = response(vec![(
            "NSE_FO|9",
            FeedUnion::FullFeed(FullFeed {
                full_feed_union: Some(FullFeedUnion::MarketFf(MarketFullFeed::default())),
            }),
        )]);
        assert!(updates(&resp).is_empty());
        assert!(decode_frame(&[0xFF, 0xFF, 0xFF]).is_err());
    }
}
