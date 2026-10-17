//! The `[monitor]` section of `gammapin.toml`. Node names, the inbox (live), the merged
//! output (replay) and the instrument master come from `[merge]`.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use gp_feed::Date;
use gp_merge::config::Merge;
use gp_quant::Weights;
use serde::Deserialize;

use crate::engine::EngineConfig;
use crate::snapshot::Accuracy;

#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub monitor: Monitor,
    #[serde(default)]
    pub merge: Merge,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct Monitor {
    pub bind: String,
    pub snapshot_ms: u64,
    pub static_dir: PathBuf,
    /// RBI 91-day T-bill, decimal.
    pub rate: f64,
    /// NSE holidays (YYYY-MM-DD), for time to expiry.
    pub holidays: Vec<String>,
    /// S1/S2 weights `[w_c, w_p]` from the previous day's participant OI. S0 is always shown.
    pub weights: BTreeMap<String, [f64; 2]>,
    /// Training-set H1 accuracy per convention, shown next to each pin.
    pub h1_accuracy: BTreeMap<String, Accuracy>,
}

impl Default for Monitor {
    fn default() -> Self {
        Self {
            bind: "0.0.0.0:8080".into(),
            snapshot_ms: 1_000,
            static_dir: "crates/gp-monitor/static".into(),
            rate: 0.065,
            holidays: Vec::new(),
            weights: BTreeMap::new(),
            h1_accuracy: BTreeMap::new(),
        }
    }
}

impl Config {
    pub fn load(path: &Path) -> anyhow::Result<Self> {
        let text = std::fs::read_to_string(path)
            .map_err(|e| anyhow::anyhow!("{}: {e}", path.display()))?;
        toml::from_str(&text).map_err(|e| anyhow::anyhow!("{}: {e}", path.display()))
    }
}

impl Monitor {
    pub fn engine(&self, nodes: &[String]) -> anyhow::Result<EngineConfig> {
        let holidays = self
            .holidays
            .iter()
            .map(|h| Date::parse(h).ok_or_else(|| anyhow::anyhow!("bad holiday {h:?}")))
            .collect::<anyhow::Result<_>>()?;
        let mut conventions = vec![("S0".to_string(), Weights::S0)];
        for (name, [w_c, w_p]) in &self.weights {
            anyhow::ensure!(name != "S0", "S0 weights are fixed at +1, -1");
            conventions.push((
                name.clone(),
                Weights {
                    w_c: *w_c,
                    w_p: *w_p,
                },
            ));
        }
        let h1 = (!self.h1_accuracy.is_empty()).then(|| {
            conventions
                .iter()
                .map(|(n, _)| (n.clone(), self.h1_accuracy.get(n).copied()))
                .collect()
        });
        Ok(EngineConfig {
            r: self.rate,
            conventions,
            holidays,
            nodes: nodes.to_vec(),
            h1_accuracy: h1,
            ..EngineConfig::default()
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn example_config() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../config/gammapin.example.toml");
        let c = Config::load(&path).unwrap();
        assert_eq!(c.monitor.snapshot_ms, 1_000);
        let e = c.monitor.engine(&c.merge.nodes).unwrap();
        assert_eq!(e.conventions[0].0, "S0");
        assert!(e.holidays.contains(&Date::from_ymd(2026, 10, 20)));
        let h1 = e.h1_accuracy.unwrap();
        assert_eq!(h1["S0"].unwrap().n, 66);
        assert_eq!(e.nodes, ["A", "B"]);
    }

    #[test]
    fn weights_and_accuracy() {
        let m: Monitor = toml::from_str(
            "weights = { S1 = [-0.2, 0.5] }\nh1_accuracy = { S1 = { mae = 150.0, n = 28 } }",
        )
        .unwrap();
        let e = m.engine(&[]).unwrap();
        assert_eq!(e.conventions.len(), 2);
        assert_eq!(e.conventions[1].1.w_p, 0.5);
        let h1 = e.h1_accuracy.unwrap();
        assert_eq!(h1["S0"], None);
        assert_eq!(h1["S1"].unwrap().mae, 150.0);
        let bad: Monitor = toml::from_str("weights = { S0 = [1.0, 1.0] }").unwrap();
        assert!(bad.engine(&[]).is_err());
    }
}
