//! V3.4: memory-mapped replay throughput on a 2-million-event synthetic log.
//!
//!     cargo bench -p gp-replay --bench replay

use std::hint::black_box;
use std::sync::Arc;
use std::time::Instant;

use gp_feed::synth::{Market, MarketConfig, universe};
use gp_feed::{Date, Event, Master, NODE_A, Normalizer, updates};
use gp_replay::{EventStream, Poll, ReplayFile, ReplayStream, sort_for_replay, write_replay};

const TARGET: u64 = 2_000_000;

fn events() -> Vec<Event> {
    let date = Date::from_ymd(2026, 10, 19);
    let u = universe(date, 25_000.0, 2, 10.0, 50.0);
    let master = Master::new(u.clone());
    let mut market = Market::new(MarketConfig::new(date), u);
    let norm = Normalizer::default();
    let mut base = Vec::new();
    for _ in 0..2_500 {
        let frame = market.next_frame();
        for u in updates(&frame) {
            let ins = master.get(&u.instrument_key).unwrap();
            base.push(norm.event(&u, ins, market.now_ms() * 1_000_000, NODE_A));
        }
    }
    // Repeat the 100,000-event session, shifted in time, up to TARGET events.
    let span = base.last().unwrap().ltt_ms - base[0].ltt_ms + 1;
    let mut out = Vec::with_capacity(TARGET as usize);
    let mut k = 0;
    while (out.len() as u64) < TARGET {
        for e in &base {
            let mut e = e.clone();
            e.ltt_ms += k * span;
            e.recv_ns += k * span * 1_000_000;
            out.push(e);
        }
        k += 1;
    }
    out.truncate(TARGET as usize);
    sort_for_replay(&mut out);
    out
}

fn rate(label: &str, n: u64, start: Instant) {
    let s = start.elapsed().as_secs_f64();
    println!(
        "{label:<28} {:>12.0} events/s  ({n} in {s:.3} s)",
        n as f64 / s
    );
}

fn main() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("bench.gpr");
    write_replay(&path, &events()).unwrap();
    let size = std::fs::metadata(&path).unwrap().len();
    println!("replay log: {TARGET} events, {:.0} MB", size as f64 / 1e6);

    let start = Instant::now();
    let file = Arc::new(ReplayFile::open(&path).unwrap());
    let s = start.elapsed().as_secs_f64();
    println!("{:<28} {s:>12.3} s (map + checksum + id check)", "open");

    let start = Instant::now();
    let mut sum = 0.0;
    for i in 0..file.len() {
        sum += file.tick(i).ltp;
    }
    black_box(sum);
    rate("ticks (numbers only)", file.len(), start);

    let start = Instant::now();
    let mut n = 0;
    for e in file.events() {
        black_box(&e);
        n += 1;
    }
    rate("events (C1, with metadata)", n, start);

    let start = Instant::now();
    let mut stream = ReplayStream::new(file.clone());
    let mut n = 0;
    while let Poll::Ready(e) = stream.poll_event().unwrap() {
        black_box(&e);
        n += 1;
    }
    rate("stream (max speed)", n, start);
}
