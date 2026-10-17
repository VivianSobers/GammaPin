//! gp-merge: decode GPL1 segments from Node A and Node B, de-duplicate by content, and
//! write C1 Parquet, a sorted replay log and a coverage report.

pub mod config;
pub mod coverage;
pub mod dedup;
pub mod normalize;
pub mod parquet;
pub mod read;
