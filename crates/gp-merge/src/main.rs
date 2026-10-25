//! gp-merge: decode GPL1 segments from Node A and Node B, de-duplicate by
//! content, and write Parquet, a sorted replay log and a coverage report.

use std::path::PathBuf;
use std::process::ExitCode;

use clap::Parser;
use gp_feed::{Date, Master, read_master};
use gp_merge::config::Config;
use gp_merge::pipeline::merge_day;

#[derive(Parser)]
#[command(about = "Merge one day of node logs into C1 Parquet and a replay log")]
struct Args {
    #[arg(long, default_value = "/etc/gammapin/gammapin.toml")]
    config: PathBuf,
    /// Trading day to merge: YYYY-MM-DD, or `today` (IST).
    #[arg(long, default_value = "today")]
    date: String,
    /// Print the coverage report as JSON instead of Markdown.
    #[arg(long)]
    json: bool,
}

fn parse_date(s: &str) -> anyhow::Result<Date> {
    if s == "today" {
        let ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_millis() as i64;
        return Ok(Date::ist_from_unix_ms(ms));
    }
    Date::parse(s).ok_or_else(|| anyhow::anyhow!("bad --date {s:?}, want YYYY-MM-DD or today"))
}

/// Above this share of updates with keys missing from the master, the master is probably
/// not the one the day was captured with.
const MAX_UNKNOWN_SHARE: f64 = 0.01;

fn run(args: &Args) -> anyhow::Result<u8> {
    let cfg = Config::load(&args.config)?.merge;
    let date = parse_date(&args.date)?;
    let master_path = cfg.master_for(date);
    let master = Master::new(
        read_master(&master_path).map_err(|e| anyhow::anyhow!("{}: {e}", master_path.display()))?,
    );
    let cov = merge_day(&cfg, date, &master)?;
    if args.json {
        println!("{}", serde_json::to_string_pretty(&cov)?);
    } else {
        print!("{}", cov.markdown());
    }
    let updates: u64 = cov.nodes.iter().map(|n| n.decode.updates).sum();
    let unknown: u64 = cov
        .nodes
        .iter()
        .map(|n| n.decode.unknown_keys.values().sum::<u64>())
        .sum();
    if updates > 0 && unknown as f64 > MAX_UNKNOWN_SHARE * updates as f64 {
        eprintln!(
            "gp-merge: {unknown} of {updates} updates have keys missing from {}; is it the master for {date}?",
            master_path.display()
        );
        return Ok(4);
    }
    // Merged, but some window was missed by every node: exit 3 so a timer can alert.
    Ok(if cov.lost.is_empty() { 0 } else { 3 })
}

fn main() -> ExitCode {
    let args = Args::parse();
    match run(&args) {
        Ok(code) => ExitCode::from(code),
        Err(e) => {
            eprintln!("gp-merge: {e:#}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates() {
        assert_eq!(
            parse_date("2026-10-19").unwrap(),
            Date::from_ymd(2026, 10, 19)
        );
        assert!(parse_date("19/10/2026").is_err());
        assert!(parse_date("today").is_ok());
    }
}
