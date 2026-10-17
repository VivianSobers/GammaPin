//! Where to connect. Upstox issues a single-use `wss://` URL per connection from
//! `GET /v3/feed/market-data-feed/authorize`, so it is asked again before every attempt.

use std::path::PathBuf;

use serde::Deserialize;

pub const AUTHORIZE_PATH: &str = "/v3/feed/market-data-feed/authorize";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Endpoint {
    /// A fixed URL (the mock feed, dry runs).
    Direct(String),
    /// Authorize with the access token in `token_file` (read again on every attempt).
    Upstox {
        api_base: String,
        token_file: PathBuf,
    },
}

#[derive(Deserialize)]
struct AuthorizeData {
    #[serde(alias = "authorizedRedirectUri")]
    authorized_redirect_uri: String,
}

#[derive(Deserialize)]
struct AuthorizeResponse {
    status: String,
    data: Option<AuthorizeData>,
}

/// Read an access token file: the token is its first non-empty line.
pub fn read_token(path: &std::path::Path) -> Result<String, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    text.lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .map(str::to_string)
        .ok_or_else(|| format!("{}: empty token file", path.display()))
}

impl Endpoint {
    /// The URL for the next connection attempt. Errors are short texts for `GAP_START`
    /// and never include the token.
    pub async fn url(&self, http: &reqwest::Client) -> Result<String, String> {
        let (api_base, token_file) = match self {
            Self::Direct(url) => return Ok(url.clone()),
            Self::Upstox {
                api_base,
                token_file,
            } => (api_base, token_file),
        };
        let token = read_token(token_file)?;
        let resp = http
            .get(format!(
                "{}{AUTHORIZE_PATH}",
                api_base.trim_end_matches('/')
            ))
            .bearer_auth(token)
            .header("Accept", "application/json")
            .send()
            .await
            .map_err(|e| format!("authorize: {}", e.without_url()))?;
        let code = resp.status();
        if !code.is_success() {
            return Err(format!("authorize: HTTP {}", code.as_u16()));
        }
        let body: AuthorizeResponse = resp
            .json()
            .await
            .map_err(|e| format!("authorize: bad response: {}", e.without_url()))?;
        match (body.status.as_str(), body.data) {
            ("success", Some(d)) => Ok(d.authorized_redirect_uri),
            (status, _) => Err(format!("authorize: status {status}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::{HeaderMap, StatusCode};
    use axum::routing::get;

    async fn fake_upstox() -> String {
        let app = axum::Router::new().route(
            AUTHORIZE_PATH,
            get(|h: HeaderMap| async move {
                match h.get("authorization").and_then(|v| v.to_str().ok()) {
                    Some("Bearer good") => (
                        StatusCode::OK,
                        r#"{"status":"success","data":{"authorized_redirect_uri":"wss://feed.example/one-time"}}"#,
                    ),
                    Some("Bearer v2") => (
                        StatusCode::OK,
                        r#"{"status":"success","data":{"authorizedRedirectUri":"wss://feed.example/v2"}}"#,
                    ),
                    Some("Bearer odd") => (StatusCode::OK, r#"{"status":"error"}"#),
                    _ => (StatusCode::UNAUTHORIZED, r#"{"status":"error"}"#),
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        format!("http://{addr}/")
    }

    #[tokio::test]
    async fn authorizes_with_the_current_token() {
        let base = fake_upstox().await;
        let dir = tempfile::tempdir().unwrap();
        let token_file = dir.path().join("upstox.token");
        let ep = Endpoint::Upstox {
            api_base: base,
            token_file: token_file.clone(),
        };
        let http = reqwest::Client::new();

        assert!(ep.url(&http).await.unwrap_err().contains("upstox.token"));
        std::fs::write(&token_file, "\n  good  \n").unwrap();
        assert_eq!(ep.url(&http).await.unwrap(), "wss://feed.example/one-time");
        std::fs::write(&token_file, "v2\n").unwrap();
        assert_eq!(ep.url(&http).await.unwrap(), "wss://feed.example/v2");
        std::fs::write(&token_file, "expired\n").unwrap();
        let err = ep.url(&http).await.unwrap_err();
        assert_eq!(err, "authorize: HTTP 401");
        assert!(!err.contains("expired"));
        std::fs::write(&token_file, "odd").unwrap();
        assert_eq!(ep.url(&http).await.unwrap_err(), "authorize: status error");
        std::fs::write(&token_file, "\n\n").unwrap();
        assert!(
            ep.url(&http)
                .await
                .unwrap_err()
                .contains("empty token file")
        );

        let direct = Endpoint::Direct("ws://127.0.0.1:9100".into());
        assert_eq!(direct.url(&http).await.unwrap(), "ws://127.0.0.1:9100");
    }
}
