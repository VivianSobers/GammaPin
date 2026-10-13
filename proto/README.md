# proto/

Place Upstox's `MarketDataFeedV3.proto` here, unchanged, from the
[Market Data Feed V3 docs](https://upstox.com/developer/api-documentation/v3/get-market-data-feed).
`gp-merge` generates Rust types from it with `prost-build`.

Before writing the decoder, check whether the feed carries exchange sequence numbers.
If it does not, de-duplication relies on the content key alone.
