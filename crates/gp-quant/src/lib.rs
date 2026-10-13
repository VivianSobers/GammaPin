//! gp-quant: Black-76 pricing, implied vol, gamma, forward, GEX and flip level.
//! The live monitor and the Python research use this same code.

pub mod black76;
pub mod chain;
pub mod filter;
pub mod forward;
pub mod iv;
pub mod norm;
pub mod time;

pub use black76::OptType;
pub use chain::{
    Analysis, ChainInput, ConventionResult, QuantError, Quote, StrikeRow, Weights, analyze,
};
pub use filter::FilterConfig;
pub use time::MIN_T_YEARS;

#[cfg(feature = "python")]
mod python {
    use pyo3::prelude::*;

    #[pymodule]
    fn gp_quant(_m: &Bound<'_, PyModule>) -> PyResult<()> {
        Ok(())
    }
}
