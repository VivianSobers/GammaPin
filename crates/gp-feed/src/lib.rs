//! gp-feed: the Upstox Market Data Feed V3 as seen by every GammaPin binary.

pub mod decode;

pub use decode::{DEPTH, Update, decode_frame, updates};

/// Types generated from `proto/MarketDataFeed.proto`.
#[allow(clippy::all, clippy::pedantic)]
pub mod proto {
    include!(concat!(
        env!("OUT_DIR"),
        "/com.upstox.marketdatafeederv3udapi.rpc.proto.rs"
    ));
}

#[cfg(test)]
mod tests {
    use super::proto::*;
    use prost::Message;

    #[test]
    fn feed_response_round_trip() {
        let mut feeds = std::collections::HashMap::new();
        feeds.insert(
            "NSE_INDEX|Nifty 50".to_string(),
            Feed {
                feed_union: Some(feed::FeedUnion::Ltpc(Ltpc {
                    ltp: 25003.85,
                    ltt: 1_792_398_600_000,
                    ltq: 0,
                    cp: 24950.0,
                    iep: None,
                })),
                request_mode: RequestMode::FullD5 as i32,
            },
        );
        let msg = FeedResponse {
            r#type: Type::LiveFeed as i32,
            feeds,
            current_ts: 1_792_398_600_123,
            market_info: None,
        };
        let bytes = msg.encode_to_vec();
        assert_eq!(FeedResponse::decode(&bytes[..]).unwrap(), msg);
    }
}
