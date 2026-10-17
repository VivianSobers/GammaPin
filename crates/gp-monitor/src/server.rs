//! The monitor's HTTP side: `/ws` pushes each new C2 snapshot, `/snapshot.json` returns the
//! latest one, and everything else is served from `static/`.

use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use axum::Router;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Path as UrlPath, State};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use futures_util::{SinkExt, StreamExt};
use tokio::sync::watch;

/// The latest snapshot as JSON; None until the engine has built one.
pub type Latest = watch::Receiver<Option<Arc<str>>>;

#[derive(Clone)]
pub struct AppState {
    pub latest: Latest,
    pub static_dir: PathBuf,
}

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/ws", get(ws))
        .route("/snapshot.json", get(snapshot))
        .route("/", get(index))
        .route("/{*path}", get(file))
        .with_state(state)
}

async fn ws(upgrade: WebSocketUpgrade, State(st): State<AppState>) -> Response {
    upgrade.on_upgrade(move |socket| push(socket, st.latest))
}

/// Send the current snapshot, then each new one, until the client goes away.
async fn push(socket: WebSocket, mut latest: Latest) {
    let (mut tx, mut rx) = socket.split();
    let current = latest.borrow_and_update().clone();
    if let Some(s) = current
        && tx.send(Message::Text(s.as_ref().into())).await.is_err()
    {
        return;
    }
    loop {
        tokio::select! {
            changed = latest.changed() => {
                if changed.is_err() {
                    break; // engine stopped
                }
                let next = latest.borrow_and_update().clone();
                if let Some(s) = next
                    && tx.send(Message::Text(s.as_ref().into())).await.is_err()
                {
                    break;
                }
            }
            msg = rx.next() => match msg {
                Some(Ok(Message::Close(_))) | None | Some(Err(_)) => break,
                Some(Ok(_)) => {} // the page sends nothing; ignore pings and noise
            },
        }
    }
    let _ = tx.close().await;
}

async fn snapshot(State(st): State<AppState>) -> Response {
    match st.latest.borrow().clone() {
        Some(s) => ([(header::CONTENT_TYPE, "application/json")], s.to_string()).into_response(),
        None => (StatusCode::SERVICE_UNAVAILABLE, "no snapshot yet\n").into_response(),
    }
}

async fn index(st: State<AppState>) -> Response {
    file(st, UrlPath("index.html".into())).await
}

fn content_type(path: &Path) -> &'static str {
    match path.extension().and_then(|e| e.to_str()) {
        Some("html") => "text/html; charset=utf-8",
        Some("js") => "text/javascript; charset=utf-8",
        Some("css") => "text/css; charset=utf-8",
        Some("json") => "application/json",
        Some("svg") => "image/svg+xml",
        Some("png") => "image/png",
        _ => "application/octet-stream",
    }
}

/// A file under `static/`; paths that try to leave it are not found.
async fn file(State(st): State<AppState>, UrlPath(path): UrlPath<String>) -> Response {
    let rel = Path::new(&path);
    if !rel.components().all(|c| matches!(c, Component::Normal(_))) {
        return StatusCode::NOT_FOUND.into_response();
    }
    let full = st.static_dir.join(rel);
    match tokio::fs::read(&full).await {
        Ok(bytes) => ([(header::CONTENT_TYPE, content_type(&full))], bytes).into_response(),
        Err(_) => StatusCode::NOT_FOUND.into_response(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio_tungstenite::tungstenite::Message as WsMessage;

    async fn start() -> (String, watch::Sender<Option<Arc<str>>>) {
        let (tx, rx) = watch::channel(None);
        let state = AppState {
            latest: rx,
            static_dir: Path::new(env!("CARGO_MANIFEST_DIR")).join("static"),
        };
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, router(state)).await.unwrap() });
        (format!("{addr}"), tx)
    }

    #[tokio::test]
    async fn pushes_each_new_snapshot() {
        let (addr, tx) = start().await;
        tx.send_replace(Some(Arc::from(r#"{"n":1}"#)));
        let (mut ws, _) = tokio_tungstenite::connect_async(format!("ws://{addr}/ws"))
            .await
            .unwrap();
        let first = ws.next().await.unwrap().unwrap();
        assert_eq!(first, WsMessage::Text(r#"{"n":1}"#.into()));
        tx.send_replace(Some(Arc::from(r#"{"n":2}"#)));
        let second = ws.next().await.unwrap().unwrap();
        assert_eq!(second, WsMessage::Text(r#"{"n":2}"#.into()));
        ws.close(None).await.unwrap();
    }

    #[tokio::test]
    async fn serves_static_files_and_the_latest_snapshot() {
        let (addr, tx) = start().await;
        let get = |p: &str| reqwest::get(format!("http://{addr}{p}"));
        assert_eq!(get("/snapshot.json").await.unwrap().status(), 503);
        tx.send_replace(Some(Arc::from("{}")));
        let r = get("/snapshot.json").await.unwrap();
        assert_eq!(r.headers()["content-type"], "application/json");
        assert_eq!(r.text().await.unwrap(), "{}");

        let page = get("/").await.unwrap();
        assert_eq!(page.headers()["content-type"], "text/html; charset=utf-8");
        assert!(
            page.text()
                .await
                .unwrap()
                .contains("Analytics, not trading advice.")
        );
        let js = get("/feed.js").await.unwrap();
        assert!(
            js.headers()["content-type"]
                .to_str()
                .unwrap()
                .starts_with("text/javascript")
        );
        assert_eq!(get("/mock/snapshot.json").await.unwrap().status(), 200);
        assert_eq!(get("/nope.js").await.unwrap().status(), 404);
        assert_eq!(get("/..%2FCargo.toml").await.unwrap().status(), 404);
    }
}
