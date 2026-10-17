//! Contract C2 (docs/CONTRACTS.md): the snapshot pushed to the dashboard over `/ws`.

use std::collections::BTreeMap;

use gp_quant::{Analysis, ConventionResult};
use serde::{Deserialize, Serialize};

pub const SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Convention {
    pub gex: Vec<f64>,
    pub net_gex: f64,
    pub flip: Option<f64>,
    pub pin: Option<f64>,
    pub regime: Regime,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Regime {
    LongGamma,
    ShortGamma,
}

impl From<&ConventionResult> for Convention {
    fn from(c: &ConventionResult) -> Self {
        Self {
            gex: c.gex.clone(),
            net_gex: c.net_gex,
            flip: c.flip,
            pin: c.pin,
            regime: if c.net_gex > 0.0 {
                Regime::LongGamma
            } else {
                Regime::ShortGamma
            },
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Accuracy {
    pub mae: f64,
    pub n: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum NodeState {
    Up,
    Down,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Health {
    pub lag_ms_p50: f64,
    pub lag_ms_p99: f64,
    pub gaps_today: u64,
    pub nodes: BTreeMap<String, NodeState>,
    pub quotes_used: usize,
    pub quotes_total: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Snapshot {
    pub v: u32,
    pub ts_ms: i64,
    pub source: String,
    pub underlying: String,
    pub expiry: String,
    pub spot: f64,
    pub forward: f64,
    pub t_years: f64,
    pub conventions: BTreeMap<String, Convention>,
    pub strikes: Vec<f64>,
    pub h1_accuracy: Option<BTreeMap<String, Option<Accuracy>>>,
    pub health: Health,
}

/// Convention names in the order their weights were passed to `analyze`.
pub fn conventions(names: &[String], a: &Analysis) -> BTreeMap<String, Convention> {
    names
        .iter()
        .zip(&a.conventions)
        .map(|(n, c)| (n.clone(), Convention::from(c)))
        .collect()
}

impl Snapshot {
    pub fn to_json(&self) -> String {
        serde_json::to_string(self).expect("snapshot fields are serializable")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> &'static str {
        include_str!("../static/mock/snapshot.json")
    }

    #[test]
    fn mock_fixture_round_trips() {
        let s: Snapshot = serde_json::from_str(fixture()).unwrap();
        assert_eq!(s.v, SCHEMA_VERSION);
        assert_eq!(s.conventions["S0"].regime, Regime::LongGamma);
        assert_eq!(s.health.nodes["A"], NodeState::Up);
        let back: serde_json::Value = serde_json::from_str(&s.to_json()).unwrap();
        let orig: serde_json::Value = serde_json::from_str(fixture()).unwrap();
        // numbers compare as f64, so 25000 and 25000.0 are the same
        assert_eq!(normalize(back), normalize(orig));
    }

    fn normalize(v: serde_json::Value) -> serde_json::Value {
        use serde_json::Value;
        match v {
            Value::Number(n) => serde_json::json!(n.as_f64().unwrap()),
            Value::Array(a) => Value::Array(a.into_iter().map(normalize).collect()),
            Value::Object(o) => {
                Value::Object(o.into_iter().map(|(k, v)| (k, normalize(v))).collect())
            }
            v => v,
        }
    }

    #[test]
    fn regime_is_the_sign_of_net_gex() {
        let c = ConventionResult {
            gex: vec![1.0, -3.0],
            net_gex: -2.0,
            flip: None,
            pin: Some(100.0),
        };
        let json = serde_json::to_value(Convention::from(&c)).unwrap();
        assert_eq!(json["regime"], "short_gamma");
        assert_eq!(json["flip"], serde_json::Value::Null);
    }
}
