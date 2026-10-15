# proto/

`MarketDataFeed.proto` is Upstox's Market Data Feed V3 schema, unchanged, from
<https://assets.upstox.com/feed/market-data-feed/v3/MarketDataFeed.proto>
(downloaded 2026-10-15, SHA-256 `3e1c939dce2c83a3fef91405c5ca268dfa089aae973319f56685f12a5afcf612`).
`gp-merge` generates Rust types from it with `prost-build`.

The feed carries no exchange sequence numbers, so de-duplication relies on the content key alone.
Every `FeedResponse` carries `currentTs`, the Upstox server time in ms, which both nodes see.
