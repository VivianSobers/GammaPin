//! A local stand-in for the Upstox V3 feed, for tests, benchmarks and dry runs.
//!
//! One [`Market`] generates frames on a fixed interval and every connection gets the same
//! frames, filtered to the keys it subscribed to, as two nodes would from Upstox. The first
//! message on a connection is a `market_info` frame, as on the real feed. [`MockFeed::cut`]
//! simulates a network failure; [`MockFeed::restore`] ends it.

use std::collections::HashSet;
use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicU8, AtomicU64, Ordering};
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use gp_feed::proto::{FeedResponse, Type};
use gp_feed::synth::{Market, MarketConfig};
use gp_feed::{Instrument, parse_request};
use prost::Message as _;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::broadcast;
use tokio_tungstenite::tungstenite::Message;

/// How a cut looks to the client.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fault {
    /// Sockets are dropped and new connections refused.
    Drop = 1,
    /// Sockets stay open but nothing is sent (a stalled network path).
    Silence = 2,
}

#[derive(Debug, Clone)]
pub struct MockConfig {
    pub market: MarketConfig,
    pub instruments: Vec<Instrument>,
    /// Time between frames; zero means each connection gets its own market at full speed.
    pub interval: Duration,
}

pub struct MockFeed {
    addr: SocketAddr,
    fault: Arc<AtomicU8>,
    generated: Arc<AtomicU64>,
    sent: Arc<AtomicU64>,
    tasks: Vec<tokio::task::JoinHandle<()>>,
}

impl Drop for MockFeed {
    fn drop(&mut self) {
        for t in &self.tasks {
            t.abort();
        }
    }
}

impl MockFeed {
    pub async fn start(cfg: MockConfig, bind: &str) -> std::io::Result<Self> {
        let listener = TcpListener::bind(bind).await?;
        let addr = listener.local_addr()?;
        let fault = Arc::new(AtomicU8::new(0));
        let generated = Arc::new(AtomicU64::new(0));
        let sent = Arc::new(AtomicU64::new(0));
        let (tx, _) = broadcast::channel::<Arc<FeedResponse>>(4_096);
        let mut tasks = Vec::new();

        if !cfg.interval.is_zero() {
            let (tx, generated) = (tx.clone(), generated.clone());
            let (mcfg, instruments, interval) = (cfg.market, cfg.instruments.clone(), cfg.interval);
            tasks.push(tokio::spawn(async move {
                let mut market = Market::new(mcfg, instruments);
                let mut tick = tokio::time::interval(interval);
                loop {
                    tick.tick().await;
                    generated.fetch_add(1, Ordering::Relaxed);
                    let _ = tx.send(Arc::new(market.next_frame()));
                }
            }));
        }

        let (fault2, sent2, generated2) = (fault.clone(), sent.clone(), generated.clone());
        tasks.push(tokio::spawn(async move {
            while let Ok((tcp, _)) = listener.accept().await {
                if fault2.load(Ordering::Relaxed) == Fault::Drop as u8 {
                    drop(tcp);
                    continue;
                }
                let conn = Conn {
                    cfg: cfg.clone(),
                    rx: tx.subscribe(),
                    fault: fault2.clone(),
                    sent: sent2.clone(),
                    generated: generated2.clone(),
                };
                tokio::spawn(conn.serve(tcp));
            }
        }));
        Ok(Self {
            addr,
            fault,
            generated,
            sent,
            tasks,
        })
    }

    pub fn url(&self) -> String {
        format!("ws://{}", self.addr)
    }

    pub fn cut(&self, fault: Fault) {
        self.fault.store(fault as u8, Ordering::Relaxed);
    }

    pub fn restore(&self) {
        self.fault.store(0, Ordering::Relaxed);
    }

    /// Frames produced by the shared market so far.
    pub fn generated(&self) -> u64 {
        self.generated.load(Ordering::Relaxed)
    }

    /// Frames written to sockets, over all connections.
    pub fn sent(&self) -> u64 {
        self.sent.load(Ordering::Relaxed)
    }
}

struct Conn {
    cfg: MockConfig,
    rx: broadcast::Receiver<Arc<FeedResponse>>,
    fault: Arc<AtomicU8>,
    sent: Arc<AtomicU64>,
    generated: Arc<AtomicU64>,
}

fn filtered(frame: &FeedResponse, keys: &HashSet<String>) -> Option<Vec<u8>> {
    let feeds: std::collections::HashMap<_, _> = frame
        .feeds
        .iter()
        .filter(|(k, _)| keys.contains(*k))
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    (!feeds.is_empty()).then(|| {
        FeedResponse {
            feeds,
            ..frame.clone()
        }
        .encode_to_vec()
    })
}

impl Conn {
    async fn serve(mut self, tcp: TcpStream) {
        let Ok(ws) = tokio_tungstenite::accept_async(tcp).await else {
            return;
        };
        let (mut sink, mut stream) = ws.split();
        let info = FeedResponse {
            r#type: Type::MarketInfo as i32,
            current_ts: self
                .cfg
                .market
                .date
                .ist_unix_ms(self.cfg.market.start_minute),
            ..Default::default()
        };
        if sink
            .send(Message::Binary(info.encode_to_vec().into()))
            .await
            .is_err()
        {
            return;
        }
        let mut keys = HashSet::new();
        let mut own = self
            .cfg
            .interval
            .is_zero()
            .then(|| Market::new(self.cfg.market, self.cfg.instruments.clone()));
        loop {
            match self.fault.load(Ordering::Relaxed) {
                f if f == Fault::Drop as u8 => return, // no close frame: the path just died
                f if f == Fault::Silence as u8 => {
                    // drain whatever the generator sends meanwhile
                    while self.rx.try_recv().is_ok() {}
                    tokio::time::sleep(Duration::from_millis(5)).await;
                    continue;
                }
                _ => {}
            }
            let frame = if let Some(m) = own.as_mut() {
                // Full speed: yield to the request stream between frames.
                if let Some(Ok(msg)) =
                    futures_util::FutureExt::now_or_never(stream.next()).flatten()
                {
                    Self::request(&msg, &mut keys);
                    continue;
                }
                if keys.is_empty() {
                    match stream.next().await {
                        Some(Ok(msg)) => {
                            Self::request(&msg, &mut keys);
                            continue;
                        }
                        _ => return,
                    }
                }
                self.generated.fetch_add(1, Ordering::Relaxed);
                Arc::new(m.next_frame())
            } else {
                tokio::select! {
                    msg = stream.next() => match msg {
                        Some(Ok(msg)) => { Self::request(&msg, &mut keys); continue; }
                        _ => return,
                    },
                    frame = self.rx.recv() => match frame {
                        Ok(f) => f,
                        Err(broadcast::error::RecvError::Lagged(_)) => continue,
                        Err(broadcast::error::RecvError::Closed) => return,
                    },
                }
            };
            if let Some(bytes) = filtered(&frame, &keys) {
                if sink.send(Message::Binary(bytes.into())).await.is_err() {
                    return;
                }
                self.sent.fetch_add(1, Ordering::Relaxed);
            }
        }
    }

    fn request(msg: &Message, keys: &mut HashSet<String>) {
        let payload: &[u8] = match msg {
            Message::Binary(b) => b,
            Message::Text(t) => t.as_bytes(),
            _ => return,
        };
        if let Some(req) = parse_request(payload) {
            match req.method.as_str() {
                "sub" => keys.extend(req.data.instrument_keys),
                "unsub" => req.data.instrument_keys.iter().for_each(|k| {
                    keys.remove(k);
                }),
                _ => {}
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gp_feed::{Date, Mode, NIFTY_INDEX_KEY, decode_frame, subscribe_requests, synth};

    fn cfg(interval_ms: u64) -> MockConfig {
        let date = Date::from_ymd(2026, 10, 19);
        MockConfig {
            market: MarketConfig::new(date),
            instruments: synth::universe(date, 25_000.0, 1, 2.0, 50.0),
            interval: Duration::from_millis(interval_ms),
        }
    }

    type WsItem = Result<Message, tokio_tungstenite::tungstenite::Error>;

    async fn client(url: &str, keys: &[&str]) -> impl StreamExt<Item = WsItem> + use<> {
        let (ws, _) = tokio_tungstenite::connect_async(url).await.unwrap();
        let (mut sink, stream) = ws.split();
        let keys: Vec<String> = keys.iter().map(|k| k.to_string()).collect();
        for r in subscribe_requests("t", &keys, Mode::Full) {
            sink.send(Message::Binary(r.into())).await.unwrap();
        }
        tokio::spawn(async move {
            // keep the sink alive for the test's duration
            let _sink = sink;
            tokio::time::sleep(Duration::from_secs(60)).await;
        });
        stream
    }

    async fn next_frame(s: &mut (impl StreamExt<Item = WsItem> + Unpin)) -> FeedResponse {
        match tokio::time::timeout(Duration::from_secs(5), s.next()).await {
            Ok(Some(Ok(Message::Binary(b)))) => decode_frame(&b).unwrap(),
            other => panic!("expected a binary frame, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn two_clients_see_the_same_filtered_frames() {
        let feed = MockFeed::start(cfg(5), "127.0.0.1:0").await.unwrap();
        let mut a = Box::pin(client(&feed.url(), &[NIFTY_INDEX_KEY]).await);
        let mut b = Box::pin(client(&feed.url(), &[NIFTY_INDEX_KEY]).await);
        assert_eq!(next_frame(&mut a).await.r#type, Type::MarketInfo as i32);
        assert_eq!(next_frame(&mut b).await.r#type, Type::MarketInfo as i32);
        let mut fa = Vec::new();
        let mut fb = Vec::new();
        for _ in 0..8 {
            fa.push(next_frame(&mut a).await);
        }
        for _ in 0..8 {
            fb.push(next_frame(&mut b).await);
        }
        assert!(fa.iter().all(|f| f.feeds.keys().eq([NIFTY_INDEX_KEY])));
        // Both joined within a frame or two of each other; overlapping frames are identical.
        let common: Vec<_> = fa
            .iter()
            .filter_map(|x| {
                fb.iter()
                    .find(|y| y.current_ts == x.current_ts)
                    .map(|y| (x, y))
            })
            .collect();
        assert!(common.len() >= 5, "only {} shared frames", common.len());
        assert!(common.iter().all(|(x, y)| x == y));
        assert!(feed.sent() >= 16);
    }

    #[tokio::test]
    async fn drop_fault_closes_and_refuses() {
        let feed = MockFeed::start(cfg(5), "127.0.0.1:0").await.unwrap();
        let mut a = Box::pin(client(&feed.url(), &[NIFTY_INDEX_KEY]).await);
        next_frame(&mut a).await;
        next_frame(&mut a).await;
        feed.cut(Fault::Drop);
        let end = tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                match a.next().await {
                    Some(Ok(_)) => continue,
                    other => return other.is_none() || matches!(other, Some(Err(_))),
                }
            }
        })
        .await
        .unwrap();
        assert!(end);
        assert!(tokio_tungstenite::connect_async(feed.url()).await.is_err());
        feed.restore();
        let mut c = Box::pin(client(&feed.url(), &[NIFTY_INDEX_KEY]).await);
        next_frame(&mut c).await;
        assert!(!next_frame(&mut c).await.feeds.is_empty());
    }

    #[tokio::test]
    async fn full_speed_mode_streams_without_a_timer() {
        let feed = MockFeed::start(cfg(0), "127.0.0.1:0").await.unwrap();
        let keys: Vec<String> =
            synth::universe(Date::from_ymd(2026, 10, 19), 25_000.0, 1, 2.0, 50.0)
                .into_iter()
                .map(|i| i.key)
                .collect();
        let refs: Vec<&str> = keys.iter().map(String::as_str).collect();
        let mut a = Box::pin(client(&feed.url(), &refs).await);
        next_frame(&mut a).await;
        for _ in 0..500 {
            assert!(!next_frame(&mut a).await.feeds.is_empty());
        }
        assert!(feed.generated() >= 500);
    }
}
