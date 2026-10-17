//! gp-monitor: consume the unified live/replay stream, recompute the chain with
//! `gp_quant::analyze` (C3), and serve a C2 snapshot every second.

pub mod book;
pub mod config;
pub mod engine;
pub mod health;
pub mod snapshot;
