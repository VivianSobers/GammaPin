//! Python module `gp_quant` (build with `maturin develop` or `pip install ./crates/gp-quant`).
//! Array functions take 1-D float64 arrays of equal length; use `np.broadcast_arrays` for scalars.

use numpy::{PyArray1, PyReadonlyArray1};
use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;

use crate::black76::{self, OptType};
use crate::forward::{self as fwd, ParityPoint};
use crate::{iv, time};

fn slices<'a, const N: usize>(
    arrays: [&'a PyReadonlyArray1<'_, f64>; N],
) -> PyResult<[&'a [f64]; N]> {
    let mut out = [&[][..]; N];
    for (o, a) in out.iter_mut().zip(arrays) {
        *o = a.as_slice()?;
    }
    if out.iter().any(|s| s.len() != out[0].len()) {
        return Err(PyValueError::new_err("arrays must have equal length"));
    }
    Ok(out)
}

fn opt(is_call: bool) -> OptType {
    if is_call { OptType::Call } else { OptType::Put }
}

/// Black-76 price for each element.
#[pyfunction]
fn price<'py>(
    py: Python<'py>,
    f: PyReadonlyArray1<'py, f64>,
    k: PyReadonlyArray1<'py, f64>,
    t: PyReadonlyArray1<'py, f64>,
    r: PyReadonlyArray1<'py, f64>,
    sigma: PyReadonlyArray1<'py, f64>,
    is_call: PyReadonlyArray1<'py, bool>,
) -> PyResult<Bound<'py, PyArray1<f64>>> {
    let [f, k, t, r, s] = slices([&f, &k, &t, &r, &sigma])?;
    let c = is_call.as_slice()?;
    if c.len() != f.len() {
        return Err(PyValueError::new_err("arrays must have equal length"));
    }
    let out = (0..f.len())
        .map(|i| black76::price(f[i], k[i], t[i], r[i], s[i], opt(c[i])))
        .collect();
    Ok(PyArray1::from_vec(py, out))
}

/// Black-76 gamma for each element (same for calls and puts).
#[pyfunction]
fn gamma<'py>(
    py: Python<'py>,
    f: PyReadonlyArray1<'py, f64>,
    k: PyReadonlyArray1<'py, f64>,
    t: PyReadonlyArray1<'py, f64>,
    r: PyReadonlyArray1<'py, f64>,
    sigma: PyReadonlyArray1<'py, f64>,
) -> PyResult<Bound<'py, PyArray1<f64>>> {
    let [f, k, t, r, s] = slices([&f, &k, &t, &r, &sigma])?;
    let out = (0..f.len())
        .map(|i| black76::gamma(f[i], k[i], t[i], r[i], s[i]))
        .collect();
    Ok(PyArray1::from_vec(py, out))
}

/// Implied vol for each element; NaN where the solver fails or the price is out of range.
#[pyfunction]
fn implied_vol<'py>(
    py: Python<'py>,
    price: PyReadonlyArray1<'py, f64>,
    f: PyReadonlyArray1<'py, f64>,
    k: PyReadonlyArray1<'py, f64>,
    t: PyReadonlyArray1<'py, f64>,
    r: PyReadonlyArray1<'py, f64>,
    is_call: PyReadonlyArray1<'py, bool>,
) -> PyResult<Bound<'py, PyArray1<f64>>> {
    let [p, f, k, t, r] = slices([&price, &f, &k, &t, &r])?;
    let c = is_call.as_slice()?;
    if c.len() != f.len() {
        return Err(PyValueError::new_err("arrays must have equal length"));
    }
    let out = (0..f.len())
        .map(|i| iv::implied_vol(p[i], f[i], k[i], t[i], r[i], opt(c[i])).unwrap_or(f64::NAN))
        .collect();
    Ok(PyArray1::from_vec(py, out))
}

/// Trading minutes from `minute_of_day` (IST) to 15:30 on expiry.
#[pyfunction]
fn minutes_to_expiry(minute_of_day: f64, sessions_after_today: u32) -> f64 {
    time::minutes_to_expiry(minute_of_day, sessions_after_today)
}

/// Trading minutes to years, floored at one minute.
#[pyfunction]
fn t_years(minutes: f64) -> f64 {
    time::t_years(minutes)
}

/// Put-call parity forward, median over the 3 strikes nearest the money. None if no usable strike.
#[pyfunction]
fn forward(
    strikes: PyReadonlyArray1<'_, f64>,
    call_mid: PyReadonlyArray1<'_, f64>,
    put_mid: PyReadonlyArray1<'_, f64>,
    r: f64,
    t: f64,
) -> PyResult<Option<f64>> {
    let [k, c, p] = slices([&strikes, &call_mid, &put_mid])?;
    let points: Vec<ParityPoint> = (0..k.len())
        .map(|i| ParityPoint {
            strike: k[i],
            call_mid: c[i],
            put_mid: p[i],
        })
        .collect();
    Ok(fwd::forward(&points, r, t))
}

#[pymodule]
fn gp_quant(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(price, m)?)?;
    m.add_function(wrap_pyfunction!(gamma, m)?)?;
    m.add_function(wrap_pyfunction!(implied_vol, m)?)?;
    m.add_function(wrap_pyfunction!(minutes_to_expiry, m)?)?;
    m.add_function(wrap_pyfunction!(t_years, m)?)?;
    m.add_function(wrap_pyfunction!(forward, m)?)?;
    Ok(())
}
