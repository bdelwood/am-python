use pyo3::exceptions::PyException;
use pyo3::prelude::*;
use thiserror::Error;

// create exceptions for Python
pyo3_stub_gen::create_exception!(
    am._am,
    AmError,
    PyException,
    "Base exception for am errors."
);
pyo3_stub_gen::create_exception!(
    am._am,
    ConfigError,
    AmError,
    "Error parsing an .amc configuration file."
);
pyo3_stub_gen::create_exception!(
    am._am,
    ComputeError,
    AmError,
    "Error during model computation."
);

/// Register exception types on the am module so they're accessible from Python.
pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add("AmError", m.py().get_type::<AmError>())?;
    m.add("ConfigError", m.py().get_type::<ConfigError>())?;
    m.add("ComputeError", m.py().get_type::<ComputeError>())?;
    Ok(())
}

#[derive(Error, Debug)]
pub enum AmErr {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("{0}")]
    Config(String),

    #[error("{0}")]
    Compute(String),
}

pub type AmResult<T> = Result<T, AmErr>;

impl From<AmErr> for PyErr {
    fn from(value: AmErr) -> Self {
        // map crate errors to Python errors
        match value {
            AmErr::Io(e) => ConfigError::new_err(e.to_string()),
            AmErr::Config(msg) => ConfigError::new_err(msg),
            AmErr::Compute(msg) => ComputeError::new_err(msg),
        }
    }
}
