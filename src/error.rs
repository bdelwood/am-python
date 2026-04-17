use pyo3::PyErr;
use pyo3::exceptions::PyRuntimeError;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum AmError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("{0}")]
    ConfigParse(String),

    #[error("{0}")]
    Compute(String),
}

pub type AmResult<T> = Result<T, AmError>;

impl From<AmError> for PyErr {
    fn from(value: AmError) -> Self {
        PyRuntimeError::new_err(value.to_string())
    }
}
