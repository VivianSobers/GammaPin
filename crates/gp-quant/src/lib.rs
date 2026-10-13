//! gp-quant: Black-76 pricing, implied vol, gamma, forward, GEX and flip level.
//! The live monitor and the Python research use this same code.

/// Lower bound on time to expiry: one trading minute, in years.
pub const MIN_T_YEARS: f64 = 1.0 / (252.0 * 375.0);

#[cfg(feature = "python")]
mod python {
    use pyo3::prelude::*;

    #[pymodule]
    fn gp_quant(_m: &Bound<'_, PyModule>) -> PyResult<()> {
        Ok(())
    }
}
