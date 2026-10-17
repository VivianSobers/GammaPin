//! Generate the feed types from proto/MarketDataFeed.proto. `protox` compiles the schema in
//! pure Rust, so no `protoc` binary is needed locally or in CI.

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let proto = "../../proto/MarketDataFeed.proto";
    println!("cargo:rerun-if-changed={proto}");
    let fds = protox::compile([proto], ["../../proto"])?;
    prost_build::Config::new().compile_fds(fds)?;
    Ok(())
}
