//! Golden frames: synthetic `FeedResponse` bytes encoded independently of prost by
//! `golden/make_golden.py`. Each `.pb` must decode to the updates in its `.txt`.
//! Set `UPDATE_GOLDEN=1` to rewrite the `.txt` files after a deliberate change.

use std::fmt::Write;
use std::path::Path;

fn render(bytes: &[u8]) -> String {
    let resp = gp_feed::decode_frame(bytes).expect("golden frame decodes");
    let mut out = format!("type={} current_ts={}\n", resp.r#type, resp.current_ts);
    for u in gp_feed::updates(&resp) {
        writeln!(
            out,
            "{} ltt={} ltp={} ltq={} vol={} oi={} bid={:?}x{:?} ask={:?}x{:?}",
            u.instrument_key,
            u.ltt_ms,
            u.ltp,
            u.ltq,
            u.cum_volume,
            u.oi,
            u.bid_px,
            u.bid_qty,
            u.ask_px,
            u.ask_qty
        )
        .unwrap();
    }
    out
}

fn check(name: &str) {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden");
    let got = render(&std::fs::read(dir.join(format!("{name}.pb"))).unwrap());
    let expected_path = dir.join(format!("{name}.txt"));
    if std::env::var_os("UPDATE_GOLDEN").is_some() {
        std::fs::write(&expected_path, &got).unwrap();
    }
    let expected = std::fs::read_to_string(&expected_path).unwrap();
    assert_eq!(got, expected, "{name}.pb no longer decodes to {name}.txt");
}

#[test]
fn option_full() {
    check("option_full");
}

#[test]
fn index_full() {
    check("index_full");
}

#[test]
fn mixed() {
    check("mixed");
}

#[test]
fn market_info() {
    check("market_info");
}
