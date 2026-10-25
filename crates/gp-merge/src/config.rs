//! The `[merge]` section of `gammapin.toml`.

use std::path::{Path, PathBuf};

use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub merge: Merge,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Merge {
    /// Capture nodes, in `src_mask` bit order: the first is bit 0.
    pub nodes: Vec<String>,
    /// Node segments arrive at `{inbox}/{node}/{YYYY-MM-DD}/seg-*.gpl`.
    pub inbox: PathBuf,
    /// C1 output root: `{out}/date=YYYY-MM-DD/…`.
    pub out: PathBuf,
    /// The instrument master used during capture (`.json` or `.json.gz`).
    pub master_path: PathBuf,
    /// Treat the feed's OI as units and divide it by the lot size. Unverified for the live
    /// feed (see PREREGISTRATION.md): set it once a live day has been checked.
    pub oi_in_units: bool,
    /// Compress each node's segments to `.gpl.zst` after a successful merge.
    pub compress: bool,
}

impl Default for Merge {
    fn default() -> Self {
        Self {
            nodes: vec!["A".into(), "B".into()],
            inbox: "/srv/gammapin/inbox".into(),
            out: "data/merged".into(),
            master_path: "data/NSE.json.gz".into(),
            oi_in_units: true,
            compress: true,
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

impl Merge {
    pub fn node_dir(&self, node: &str, date: gp_feed::Date) -> PathBuf {
        self.inbox.join(node).join(date.to_string())
    }

    pub fn day_dir(&self, date: gp_feed::Date) -> PathBuf {
        self.out.join(format!("date={date}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gp_feed::Date;

    #[test]
    fn example_config() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../config/gammapin.example.toml");
        let m = Config::load(&path).unwrap().merge;
        assert_eq!(m.nodes, ["A", "B"]);
        let d = Date::from_ymd(2026, 10, 19);
        assert_eq!(
            m.node_dir("B", d),
            PathBuf::from("/srv/gammapin/inbox/B/2026-10-19")
        );
        assert_eq!(m.day_dir(d), PathBuf::from("data/merged/date=2026-10-19"));
        assert!(m.oi_in_units && m.compress);
    }

    #[test]
    fn unknown_merge_keys_are_rejected() {
        assert!(
            toml::from_str::<Config>(
                "[merge]
node = [\"A\"]"
            )
            .is_err()
        );
        assert_eq!(
            toml::from_str::<Config>("").unwrap().merge.nodes,
            ["A", "B"]
        );
    }
}
