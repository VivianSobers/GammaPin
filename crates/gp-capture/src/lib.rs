//! gp-capture: subscribe to the Upstox Market Data Feed V3 and write every WebSocket frame
//! to the GPL1 log before decoding anything.

pub mod backoff;
pub mod config;
pub mod mock;
pub mod recorder;
pub mod subscriber;
