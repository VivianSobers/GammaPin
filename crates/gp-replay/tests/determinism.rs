//! V3.3: replay is deterministic. Building the same synthetic session twice gives identical
//! replay logs, and replaying a log gives the same SHA-256 on every run, through the library
//! and through two separate `gp-replay` processes.

use std::process::Command;
use std::sync::Arc;

use gp_feed::synth::{Market, MarketConfig, universe};
use gp_feed::{Date, Event, Master, NODE_A, NODE_B, Normalizer, updates};
use gp_replay::{
    EventStream, ManualClock, Paced, Poll, ReplayFile, ReplayStream, Source, Speed, VecStream,
    sort_for_replay, stream_digest, write_replay,
};

/// A synthetic session as both nodes would have recorded it (B a few µs behind A).
fn session(frames: usize) -> Vec<Event> {
    let date = Date::from_ymd(2026, 10, 19);
    let u = universe(date, 25_000.0, 2, 5.0, 50.0);
    let master = Master::new(u.clone());
    let mut market = Market::new(MarketConfig::new(date), u);
    let norm = Normalizer::default();
    let mut out = Vec::new();
    for _ in 0..frames {
        let frame = market.next_frame();
        let recv = market.now_ms() * 1_000_000 + 2_000_000;
        for (node, lag) in [(NODE_A, 0), (NODE_B, 7_000)] {
            for u in updates(&frame) {
                let ins = master.get(&u.instrument_key).unwrap();
                out.push(norm.event(&u, ins, recv + lag, node));
            }
        }
    }
    sort_for_replay(&mut out);
    out
}

fn hash_with_binary(path: &std::path::Path) -> String {
    let out = Command::new(env!("CARGO_BIN_EXE_gp-replay"))
        .arg("hash")
        .arg(path)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).unwrap()
}

#[test]
fn same_input_same_bytes_and_hash() {
    let dir = tempfile::tempdir().unwrap();
    let (a, b) = (dir.path().join("a.gpr"), dir.path().join("b.gpr"));
    write_replay(&a, &session(500)).unwrap();
    write_replay(&b, &session(500)).unwrap();
    assert_eq!(std::fs::read(&a).unwrap(), std::fs::read(&b).unwrap());

    let first = hash_with_binary(&a);
    let second = hash_with_binary(&a);
    assert_eq!(first, second);
    assert!(first.ends_with(" 40000 events\n"), "{first}");

    let file = Arc::new(ReplayFile::open(&a).unwrap());
    let (lib_hash, n) = stream_digest(&mut ReplayStream::new(file.clone())).unwrap();
    assert_eq!(format!("{lib_hash}  {n} events\n"), first);

    // Pacing changes when events come out, never which or in what order.
    let clock = ManualClock::default();
    // 100x over a ~2 minute session: about 1.2 s of paced wall time in 1 ms steps.
    let mut paced = Paced::new(
        ReplayStream::new(file),
        Speed::Multiple(100.0),
        Box::new(clock.clone()),
    );
    let mut events = Vec::new();
    let mut pending = 0;
    loop {
        match paced.poll_event().unwrap() {
            Poll::Ready(e) => events.push(e),
            Poll::Pending => {
                pending += 1;
                clock.advance(1.0);
            }
            Poll::End => break,
        }
    }
    assert!(
        pending > 1_000,
        "only {pending} Pending polls: pacing barely engaged"
    );
    let seen = events.len() as u64;
    assert_eq!(seen, n);
    let (paced_hash, _) = stream_digest(&mut VecStream::new(events, Source::Replay)).unwrap();
    assert_eq!(paced_hash, lib_hash);
}
