//! V4.2, simulated: two capture nodes on one mock feed, each through its own TCP proxy.
//! Cut one node's path at a time; after the merge the union of both logs must hold every
//! frame the feed sent, with no window lost.

use std::collections::BTreeSet;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use gp_capture::backoff::Backoff;
use gp_capture::capture::{Summary, capture};
use gp_capture::client::ConnOpts;
use gp_capture::endpoint::Endpoint;
use gp_capture::mock::{MockConfig, MockFeed};
use gp_capture::recorder::Recorder;
use gp_capture::subscriber::Subscriber;
use gp_feed::proto::{FeedResponse, Type};
use gp_feed::synth::{MarketConfig, universe};
use gp_feed::{Date, Master, Selection};
use gp_log::WriterConfig;
use gp_merge::config::Merge;
use gp_merge::pipeline::merge_day;
use gp_merge::read::read_node;
use prost::Message;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::watch;
use tokio::task::JoinHandle;

/// A TCP relay whose links can be cut: open connections are dropped and new ones refused.
struct Proxy {
    addr: SocketAddr,
    down: Arc<AtomicBool>,
    links: Arc<Mutex<Vec<JoinHandle<()>>>>,
    /// Bytes relayed from the feed to the node.
    relayed: Arc<AtomicU64>,
    accept: JoinHandle<()>,
}

/// Relay one connection until either side closes, counting feed-to-node bytes.
async fn relay(client: TcpStream, upstream: SocketAddr, relayed: Arc<AtomicU64>) {
    let Ok(server) = TcpStream::connect(upstream).await else {
        return;
    };
    let (mut cr, mut cw) = client.into_split();
    let (mut sr, mut sw) = server.into_split();
    let down = async {
        let mut buf = vec![0; 64 * 1024];
        while let Ok(n @ 1..) = sr.read(&mut buf).await {
            if cw.write_all(&buf[..n]).await.is_err() {
                break;
            }
            relayed.fetch_add(n as u64, Ordering::SeqCst);
        }
    };
    tokio::select! {
        _ = tokio::io::copy(&mut cr, &mut sw) => {}
        _ = down => {}
    }
}

impl Proxy {
    async fn start(upstream: SocketAddr) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let down = Arc::new(AtomicBool::new(false));
        let links = Arc::new(Mutex::new(Vec::new()));
        let relayed = Arc::new(AtomicU64::new(0));
        let (down2, links2, relayed2) = (down.clone(), links.clone(), relayed.clone());
        let accept = tokio::spawn(async move {
            while let Ok((client, _)) = listener.accept().await {
                // Check and register under one lock, so a cut can't miss a new link.
                let mut links = links2.lock().unwrap();
                if down2.load(Ordering::SeqCst) {
                    continue; // dropped: the client sees a reset
                }
                links.push(tokio::spawn(relay(client, upstream, relayed2.clone())));
            }
        });
        Self {
            addr,
            down,
            links,
            relayed,
            accept,
        }
    }

    fn url(&self) -> String {
        format!("ws://{}", self.addr)
    }

    fn cut(&self) {
        let mut links = self.links.lock().unwrap();
        self.down.store(true, Ordering::SeqCst);
        for l in links.drain(..) {
            l.abort();
        }
    }

    fn restore(&self) {
        self.down.store(false, Ordering::SeqCst);
    }

    /// Wait until market data flows through again (the node reconnected and resubscribed),
    /// so that a slow machine can't stretch one outage into the next.
    async fn wait_flowing(&self) {
        let start = self.relayed.load(Ordering::SeqCst);
        tokio::time::timeout(Duration::from_secs(20), async {
            while self.relayed.load(Ordering::SeqCst) < start + 100_000 {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("node did not recover");
    }
}

impl Drop for Proxy {
    fn drop(&mut self) {
        self.accept.abort();
        self.cut();
    }
}

fn spawn_node(
    name: &str,
    bit: u8,
    url: String,
    inbox: &std::path::Path,
    date: Date,
    instruments: Vec<gp_feed::Instrument>,
    stop: watch::Receiver<bool>,
) -> JoinHandle<Summary> {
    let sel = Selection {
        band_pct: 50.0,
        ..Selection::default()
    };
    let mut sub = Subscriber::new(instruments, date, sel, name);
    let (mut rec, _) = Recorder::open(
        inbox.join(name).join(date.to_string()),
        WriterConfig::default(),
        bit,
    )
    .unwrap();
    let mut backoff = Backoff::new(
        Duration::from_millis(20),
        Duration::from_millis(100),
        bit as u64,
    );
    let opts = ConnOpts {
        // well above any scheduling hiccup on a loaded CI runner
        stall: Duration::from_secs(2),
        flush: Duration::from_millis(20),
    };
    let mut stop = stop;
    tokio::spawn(async move {
        let s = capture(
            &Endpoint::Direct(url),
            &mut sub,
            &mut rec,
            &mut backoff,
            opts,
            None,
            &mut stop,
        )
        .await
        .unwrap();
        rec.close().unwrap();
        s
    })
}

/// Exchange timestamps of a node's market frames.
fn frame_times(m: &Merge, node: &str, date: Date) -> BTreeSet<i64> {
    read_node(&m.node_dir(node, date), node, 0)
        .unwrap()
        .frames
        .iter()
        .map(|(_, p)| FeedResponse::decode(&p[..]).unwrap())
        .filter(|f| f.r#type != Type::MarketInfo as i32)
        .map(|f| f.current_ts)
        .collect()
}

/// Run both nodes for `cuts` alternating outages (A, B, A, …) of `outage` each, separated by
/// `healthy` stretches, then merge and check the union.
async fn chaos(cuts: usize, outage: Duration, healthy: Duration) {
    let date = Date::from_ymd(2026, 10, 19);
    let instruments = universe(date, 25_000.0, 1, 2.0, 50.0);
    let master = Master::new(instruments.clone());
    let mcfg = MarketConfig {
        per_frame: instruments.len(),
        ..MarketConfig::new(date)
    };
    let step = mcfg.step_ms;
    let feed = MockFeed::start(
        MockConfig {
            market: mcfg,
            instruments: instruments.clone(),
            interval: Duration::from_millis(5),
        },
        "127.0.0.1:0",
    )
    .await
    .unwrap();
    let upstream: SocketAddr = feed.url().trim_start_matches("ws://").parse().unwrap();
    let proxies = [Proxy::start(upstream).await, Proxy::start(upstream).await];

    let root = tempfile::tempdir().unwrap();
    let m = Merge {
        inbox: root.path().join("inbox"),
        out: root.path().join("merged"),
        compress: false,
        ..Merge::default()
    };
    let (tx, rx) = watch::channel(false);
    let nodes: Vec<_> = ["A", "B"]
        .iter()
        .zip(&proxies)
        .enumerate()
        .map(|(bit, (name, p))| {
            let u = instruments.clone();
            spawn_node(name, bit as u8, p.url(), &m.inbox, date, u, rx.clone())
        })
        .collect();

    for p in &proxies {
        p.wait_flowing().await;
    }
    tokio::time::sleep(healthy).await;
    for i in 0..cuts {
        let p = &proxies[i % 2];
        p.cut();
        tokio::time::sleep(outage).await;
        p.restore();
        p.wait_flowing().await;
        tokio::time::sleep(healthy).await;
    }
    tx.send(true).unwrap();
    let mut summaries = Vec::new();
    for n in nodes {
        summaries.push(n.await.unwrap());
    }

    let cov = merge_day(&m, date, &master).unwrap();
    assert!(
        cov.lost.is_empty(),
        "{:?} {:#?}",
        cov.lost,
        cov.nodes.iter().map(|n| &n.gaps).collect::<Vec<_>>()
    );
    assert_eq!(cov.replay_events, cov.dedup.output);
    for (i, s) in summaries.iter().enumerate() {
        let expected = cuts / 2 + (cuts % 2 > i) as usize;
        assert_eq!(s.gaps as usize, expected, "node {i}: {s:?}");
        assert_eq!(cov.nodes[i].gaps.len(), expected);
    }
    assert!(cov.dedup.only_b > 0 && cov.dedup.both > 0);
    if cuts > 1 {
        assert!(cov.dedup.only_a > 0);
    }

    // Each cut cost its node frames, but between them the two logs miss nothing.
    let (a, b) = (frame_times(&m, "A", date), frame_times(&m, "B", date));
    let from = *a.first().unwrap().max(b.first().unwrap());
    let to = *a.last().unwrap().min(b.last().unwrap());
    let inside = |s: &BTreeSet<i64>| s.range(from..=to).count();
    let union: Vec<i64> = a
        .union(&b)
        .copied()
        .filter(|t| (from..=to).contains(t))
        .collect();
    assert!(inside(&a) < union.len());
    if cuts > 1 {
        assert!(inside(&b) < union.len());
    }
    for w in union.windows(2) {
        assert_eq!(w[1] - w[0], step, "hole between {} and {}", w[0], w[1]);
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn cutting_one_node_loses_nothing() {
    chaos(2, Duration::from_millis(400), Duration::from_millis(300)).await;
}

/// The long form of the drill: ten 3 s outages over about a minute.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "takes about a minute"]
async fn chaos_one_minute() {
    chaos(10, Duration::from_secs(3), Duration::from_secs(3)).await;
}
