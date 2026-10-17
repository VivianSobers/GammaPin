//! Market Data Feed V3 subscription requests: JSON, sent as a binary WebSocket frame.
//!
//! ```json
//! {"guid": "gp-0", "method": "sub", "data": {"mode": "full", "instrumentKeys": ["NSE_INDEX|Nifty 50"]}}
//! ```

use serde::{Deserialize, Serialize};

/// Keys per request; a full subscription is sent as several requests.
pub const KEYS_PER_REQUEST: usize = 100;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    Ltpc,
    /// LTPC, 5-level depth, OI and volume (`full_d5`).
    Full,
    OptionGreeks,
    FullD30,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RequestData {
    pub mode: Mode,
    pub instrument_keys: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Request {
    pub guid: String,
    pub method: String,
    pub data: RequestData,
}

/// `sub` requests for `keys`, at most [`KEYS_PER_REQUEST`] keys each, as frame payloads.
pub fn subscribe_requests(guid_prefix: &str, keys: &[String], mode: Mode) -> Vec<Vec<u8>> {
    keys.chunks(KEYS_PER_REQUEST)
        .enumerate()
        .map(|(n, chunk)| {
            let req = Request {
                guid: format!("{guid_prefix}-{n}"),
                method: "sub".into(),
                data: RequestData {
                    mode,
                    instrument_keys: chunk.to_vec(),
                },
            };
            serde_json::to_vec(&req).expect("plain JSON")
        })
        .collect()
}

/// Parse a request frame (the mock feed's side of the protocol).
pub fn parse_request(payload: &[u8]) -> Option<Request> {
    serde_json::from_slice(payload).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wire_format() {
        let reqs = subscribe_requests("gp-A", &["NSE_INDEX|Nifty 50".to_string()], Mode::Full);
        assert_eq!(
            String::from_utf8(reqs[0].clone()).unwrap(),
            r#"{"guid":"gp-A-0","method":"sub","data":{"mode":"full","instrumentKeys":["NSE_INDEX|Nifty 50"]}}"#
        );
    }

    #[test]
    fn chunks_and_parses_back() {
        let keys: Vec<String> = (0..250).map(|i| format!("NSE_FO|{i}")).collect();
        let reqs = subscribe_requests("g", &keys, Mode::FullD30);
        assert_eq!(reqs.len(), 3);
        let parsed: Vec<Request> = reqs.iter().map(|r| parse_request(r).unwrap()).collect();
        assert_eq!(parsed[2].guid, "g-2");
        assert_eq!(parsed[2].data.mode, Mode::FullD30);
        let all: Vec<String> = parsed
            .into_iter()
            .flat_map(|r| r.data.instrument_keys)
            .collect();
        assert_eq!(all, keys);
        assert!(parse_request(b"not json").is_none());
        assert!(subscribe_requests("g", &[], Mode::Ltpc).is_empty());
    }
}
