//! gp-monitor: consume the unified live/replay stream, recompute the chain,
//! and push a snapshot to the dashboard every second.

use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use gp_feed::Date;
use gp_monitor::app::{Mode, RunOpts, run};
use gp_monitor::config::Config;
use gp_replay::Speed;
use tokio::sync::watch;

#[derive(Parser)]
#[command(about = "Serve C2 snapshots and the dashboard from a live or replayed stream")]
struct Args {
    #[arg(long, default_value = "/etc/gammapin/gammapin.toml")]
    config: PathBuf,
    /// Listen here instead of `[monitor] bind`.
    #[arg(long)]
    bind: Option<String>,
    #[command(subcommand)]
    mode: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Follow today's node logs under `[merge] inbox`.
    Live {
        /// YYYY-MM-DD, or `today` (IST).
        #[arg(long, default_value = "today")]
        date: String,
    },
    /// Replay `{out}/date=YYYY-MM-DD/replay.gpr`.
    Replay {
        #[arg(long)]
        date: String,
        /// 1x, 10x, … or max.
        #[arg(long, default_value = "1x")]
        speed: String,
        /// Exit when the replay ends instead of serving its last snapshot.
        #[arg(long)]
        exit_at_end: bool,
    },
}

fn parse_date(s: &str) -> anyhow::Result<Date> {
    if s == "today" {
        return Ok(Date::ist_from_unix_ms(gp_monitor::driver::wall_ms()));
    }
    Date::parse(s).ok_or_else(|| anyhow::anyhow!("bad date {s:?}, want YYYY-MM-DD or today"))
}

async fn main_inner(args: Args) -> anyhow::Result<()> {
    let cfg = Config::load(&args.config)?;
    let opts = match &args.mode {
        Cmd::Live { date } => RunOpts {
            mode: Mode::Live {
                date: parse_date(date)?,
            },
            exit_at_end: false,
        },
        Cmd::Replay {
            date,
            speed,
            exit_at_end,
        } => RunOpts {
            mode: Mode::Replay {
                date: parse_date(date)?,
                speed: Speed::parse(speed)
                    .ok_or_else(|| anyhow::anyhow!("bad --speed {speed:?}, want 1x, 10x or max"))?,
            },
            exit_at_end: *exit_at_end,
        },
    };
    let bind = args.bind.unwrap_or_else(|| cfg.monitor.bind.clone());
    let listener = tokio::net::TcpListener::bind(&bind)
        .await
        .map_err(|e| anyhow::anyhow!("bind {bind}: {e}"))?;
    tracing::info!("serving on http://{}", listener.local_addr()?);
    let (tx, rx) = watch::channel(false);
    tokio::spawn(async move {
        let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("SIGTERM handler");
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {}
            _ = term.recv() => {}
        }
        let _ = tx.send(true);
    });
    let sum = run(&cfg, opts, listener, rx).await?;
    tracing::info!(
        "{} events, {} snapshots ({} before the chain was ready)",
        sum.events,
        sum.snapshots,
        sum.skipped
    );
    Ok(())
}

#[tokio::main]
async fn main() -> ExitCode {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();
    match main_inner(Args::parse()).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("gp-monitor: {e:#}");
            ExitCode::FAILURE
        }
    }
}
