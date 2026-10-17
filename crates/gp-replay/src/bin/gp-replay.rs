//! gp-replay: inspect, hash and dump replay logs written by gp-merge.

use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;

use clap::{Parser, Subcommand};
use gp_feed::Date;
use gp_replay::{EventStream, Paced, Poll, ReplayFile, ReplayStream, Speed, SystemClock};

#[derive(Parser)]
#[command(about = "Inspect, hash and dump GPR1 replay logs")]
struct Args {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Event and instrument counts, time span.
    Info { file: PathBuf },
    /// SHA-256 of the replayed events (identical on every run for the same file).
    Hash {
        file: PathBuf,
        /// 1x, 10x, max …
        #[arg(long, default_value = "max")]
        speed: String,
    },
    /// Events as CSV.
    Dump {
        file: PathBuf,
        #[arg(long)]
        limit: Option<u64>,
    },
}

fn ist(ms: i64) -> String {
    let m = gp_feed::date::ist_minute_of_day(ms);
    format!(
        "{} {:02}:{:02}:{:06.3}",
        Date::ist_from_unix_ms(ms),
        (m / 60.0) as u32,
        (m % 60.0) as u32,
        (m * 60.0) % 60.0
    )
}

fn run(args: Args) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    match args.cmd {
        Cmd::Info { file } => {
            let f = ReplayFile::open(&file)?;
            println!("events       {}", f.len());
            println!("instruments  {}", f.metas().len());
            if !f.is_empty() {
                println!("first ltt    {} IST", ist(f.tick(0).ltt_ms));
                println!("last ltt     {} IST", ist(f.tick(f.len() - 1).ltt_ms));
                let both = (0..f.len()).filter(|&i| f.tick(i).src_mask == 0b11).count();
                println!("both nodes   {both}");
            }
        }
        Cmd::Hash { file, speed } => {
            let speed = Speed::parse(&speed).ok_or("bad --speed (1x, 10x, max)")?;
            let f = Arc::new(ReplayFile::open(&file)?);
            let mut s = Paced::new(ReplayStream::new(f), speed, Box::<SystemClock>::default());
            let (hash, n) = gp_replay::stream_digest(&mut s)?;
            println!("{hash}  {n} events");
        }
        Cmd::Dump { file, limit } => {
            let f = Arc::new(ReplayFile::open(&file)?);
            let mut s = ReplayStream::new(f);
            println!(
                "instrument_key,kind,expiry,strike,ltt_ms,recv_ns,ltp,ltq,cum_volume,oi,bid,ask,src_mask"
            );
            let mut n = 0;
            while let Poll::Ready(e) = s.poll_event()? {
                if limit.is_some_and(|l| n >= l) {
                    break;
                }
                println!(
                    "{},{},{},{},{},{},{},{},{},{},{},{},{}",
                    e.instrument_key,
                    e.kind.as_str(),
                    e.expiry.map(|d| d.to_string()).unwrap_or_default(),
                    e.strike.map(|k| k.to_string()).unwrap_or_default(),
                    e.ltt_ms,
                    e.recv_ns,
                    e.ltp,
                    e.ltq,
                    e.cum_volume,
                    e.oi,
                    e.bid_px[0],
                    e.ask_px[0],
                    e.src_mask
                );
                n += 1;
            }
        }
    }
    Ok(())
}

fn main() -> ExitCode {
    match run(Args::parse()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("gp-replay: {e}");
            ExitCode::FAILURE
        }
    }
}
