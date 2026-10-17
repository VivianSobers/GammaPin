//! What to subscribe to, and when: the index first, then (once its first price arrives)
//! the option band around it. After a reconnect the whole set is requested at once.

use gp_feed::{
    Date, Instrument, Mode, NIFTY_INDEX_KEY, Selection, decode_frame, select, subscribe_requests,
    updates,
};

pub struct Subscriber {
    instruments: Vec<Instrument>,
    today: Date,
    sel: Selection,
    guid: String,
    keys: Vec<String>,
    banded: bool,
}

impl Subscriber {
    pub fn new(instruments: Vec<Instrument>, today: Date, sel: Selection, node: &str) -> Self {
        Self {
            instruments,
            today,
            sel,
            guid: format!("gp-{node}"),
            keys: vec![NIFTY_INDEX_KEY.to_string()],
            banded: false,
        }
    }

    /// Keys subscribed so far.
    pub fn keys(&self) -> &[String] {
        &self.keys
    }

    /// Whether the option band has been chosen.
    pub fn banded(&self) -> bool {
        self.banded
    }

    /// Requests to send on a fresh connection.
    pub fn connect_requests(&self) -> Vec<Vec<u8>> {
        subscribe_requests(&self.guid, &self.keys, Mode::Full)
    }

    /// Look at a received frame; returns the requests to send now (empty almost always).
    pub fn on_frame(&mut self, payload: &[u8]) -> Vec<Vec<u8>> {
        if self.banded {
            return Vec::new();
        }
        let Ok(resp) = decode_frame(payload) else {
            return Vec::new();
        };
        let Some(spot) = updates(&resp)
            .into_iter()
            .find(|u| u.instrument_key == NIFTY_INDEX_KEY && u.ltp > 0.0)
            .map(|u| u.ltp)
        else {
            return Vec::new();
        };
        self.banded = true;
        let new: Vec<String> = select(&self.instruments, self.today, spot, self.sel)
            .into_iter()
            .map(|i| i.key.clone())
            .filter(|k| !self.keys.contains(k))
            .collect();
        self.keys.extend(new.iter().cloned());
        subscribe_requests(&format!("{}-band", self.guid), &new, Mode::Full)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gp_feed::parse_request;
    use gp_feed::synth::{Market, MarketConfig, universe};
    use prost::Message;

    #[test]
    fn index_first_then_band() {
        let date = Date::from_ymd(2026, 10, 19);
        let u = universe(date, 25_000.0, 3, 10.0, 50.0);
        let sel = Selection {
            band_pct: 2.0,
            ..Selection::default()
        };
        let mut s = Subscriber::new(u.clone(), date, sel, "A");
        let first = s.connect_requests();
        assert_eq!(first.len(), 1);
        assert_eq!(
            parse_request(&first[0]).unwrap().data.instrument_keys,
            [NIFTY_INDEX_KEY]
        );
        assert!(s.on_frame(b"\x08\x02").is_empty()); // market_info: no prices yet
        assert!(s.on_frame(&[0xFF; 3]).is_empty()); // garbage

        let mut m = Market::new(MarketConfig::new(date), u);
        let reqs = s.on_frame(&m.next_frame().encode_to_vec());
        assert!(s.banded());
        let keys: Vec<String> = reqs
            .iter()
            .flat_map(|r| parse_request(r).unwrap().data.instrument_keys)
            .collect();
        // future + 2 expiries × (±500 points / 50 ≈ 20–21 strikes) × 2 sides; index already in
        assert!((81..=85).contains(&keys.len()), "{}", keys.len());
        assert!(!keys.contains(&NIFTY_INDEX_KEY.to_string()));
        assert_eq!(s.keys().len(), keys.len() + 1);
        assert!(s.on_frame(&m.next_frame().encode_to_vec()).is_empty());
        // a reconnect asks for everything again
        let all: usize = s
            .connect_requests()
            .iter()
            .map(|r| parse_request(r).unwrap().data.instrument_keys.len())
            .sum();
        assert_eq!(all, s.keys().len());
    }
}
