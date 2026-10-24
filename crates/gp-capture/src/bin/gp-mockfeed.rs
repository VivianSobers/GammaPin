//! gp-mockfeed: serve a synthetic Nifty market over the Upstox V3 protocol on a local port,
//! for dry runs of gp-capture without an Upstox account. No real data.

use std::time::Duration;

use clap::Parser;
use gp_capture::mock::{MockConfig, MockFeed};
use gp_feed::Date;
use gp_feed::synth::{MarketConfig, universe};

#[derive(Parser)]
#[command(about = "Synthetic Upstox V3 feed for dry runs")]
struct Args {
    /// Address to listen on.
    #[arg(long, default_value = "127.0.0.1:9100")]
    bind: String,
    /// Trading date of the synthetic session (YYYY-MM-DD).
    #[arg(long)]
    date: String,
    /// Milliseconds between frames (0 = as fast as each client reads).
    #[arg(long, default_value_t = 250)]
    interval_ms: u64,
    /// Starting index level.
    #[arg(long, default_value_t = 25_000.0)]
    spot: f64,
    /// Write the synthetic instrument master here (for gp-capture and gp-merge).
    #[arg(long)]
    master: Option<std::path::PathBuf>,
    #[arg(long, default_value_t = 20_261_019)]
    seed: u64,
    /// Start the market at the current IST time, one exchange step per frame, so that
    /// exchange time follows the wall clock and the monitor's lag figures are real.
    /// Needs a nonzero --interval-ms: with 0, frames go out faster than the clock moves.
    #[arg(long)]
    now: bool,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let a = Args::parse();
    let date = Date::parse(&a.date).ok_or_else(|| anyhow::anyhow!("bad --date {}", a.date))?;
    let instruments = universe(date, a.spot, 2, 10.0, 50.0);
    if let Some(path) = &a.master {
        std::fs::write(path, gp_feed::synth::master_json(&instruments))?;
    }
    let mut market = MarketConfig {
        seed: a.seed,
        spot: a.spot,
        ..MarketConfig::new(date)
    };
    if a.now {
        anyhow::ensure!(a.interval_ms > 0, "--now needs a nonzero --interval-ms");
        let step = a.interval_ms as i64;
        let now = gp_capture::app::now_ms();
        market.step_ms = step;
        market.start_minute = gp_feed::date::ist_minute_of_day(now - step);
    }
    let feed = MockFeed::start(
        MockConfig {
            market,
            instruments,
            interval: Duration::from_millis(a.interval_ms),
        },
        &a.bind,
    )
    .await?;
    eprintln!("gp-mockfeed: serving {} on {}", a.date, feed.url());
    tokio::signal::ctrl_c().await?;
    Ok(())
}
