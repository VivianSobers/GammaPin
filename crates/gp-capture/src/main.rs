//! gp-capture: subscribe to the Upstox Market Data Feed V3 and write every WebSocket frame
//! to the GPL1 log before decoding it.

use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use gp_capture::app;
use tokio::sync::watch;

#[derive(Parser)]
#[command(about = "Capture the Upstox V3 feed into GPL1 segments")]
struct Args {
    #[arg(long, default_value = "/etc/gammapin/gammapin.toml")]
    config: PathBuf,
    /// Start now and run until stopped, ignoring the capture window.
    #[arg(long)]
    now: bool,
    #[command(subcommand)]
    cmd: Option<Cmd>,
}

#[derive(Subcommand)]
enum Cmd {
    /// Exit non-zero unless a token file written today exists (the 09:05 alert).
    CheckToken,
}

#[tokio::main]
async fn main() -> ExitCode {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();
    let args = Args::parse();
    let cfg = match gp_capture::config::Config::load(&args.config) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("gp-capture: {e:#}");
            return ExitCode::from(2);
        }
    };
    if let Some(Cmd::CheckToken) = args.cmd {
        return match app::check_token(&cfg, app::now_ms()) {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("gp-capture: no valid token: {e}");
                ExitCode::FAILURE
            }
        };
    }
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
    match app::run(&cfg, app::RunOpts { now: args.now }, rx).await {
        Ok(_) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("gp-capture: {e:#}");
            ExitCode::FAILURE
        }
    }
}
