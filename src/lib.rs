mod error;
#[allow(
    non_camel_case_types,
    non_snake_case,
    non_upper_case_globals,
    dead_code,
    clippy::upper_case_acronyms
)]
mod ffi;
mod models;

use log::{debug, info};
use numpy::PyArray1;
use pyo3::exceptions::PyAttributeError;
use pyo3::prelude::*;
use pyo3_stub_gen::derive::*;

// generate stubs with `cargo run --release --bin stub_gen`
pyo3_stub_gen::define_stub_info_gatherer!(stub_info);

#[pyo3::pymodule]
mod _am {
    use super::*;
    use std::path::Path;

    #[pymodule_init]
    fn init(m: &Bound<'_, PyModule>) -> PyResult<()> {
        pyo3_log::Logger::new(m.py(), pyo3_log::Caching::LoggersAndLevels)?
            .filter(log::LevelFilter::Trace)
            .install()
            .ok();
        error::register(m)?;
        Ok(())
    }

    #[gen_stub_pyclass]
    #[pyclass]
    struct Model {
        inner: models::AmModel,
    }

    #[gen_stub_pymethods]
    #[pymethods]
    impl Model {
        #[new]
        #[pyo3(signature = (path, args))]
        fn new(path: &str, args: Vec<Bound<'_, PyAny>>) -> PyResult<Self> {
            info!("Loading model from {path}");
            let string_args: Vec<String> = args
                .iter()
                .map(|a| a.str().map(|s| s.to_string()))
                .collect::<PyResult<_>>()?;
            Ok(Self {
                inner: models::AmModel::from_amc(Path::new(path), &string_args)?,
            })
        }

        fn compute(&mut self, py: Python<'_>) -> PyResult<()> {
            debug!("Running compute");
            py.detach(|| self.inner.compute())?;
            Ok(())
        }

        #[getter]
        fn frequency<'py>(&self, py: Python<'py>) -> Bound<'py, PyArray1<f64>> {
            PyArray1::from_slice(py, self.inner.frequency())
        }

        // print summary like CLI
        fn summary(&mut self) -> String {
            self.inner.summary()
        }

        fn __getattr__<'py>(
            &self,
            py: Python<'py>,
            name: &str,
        ) -> PyResult<Bound<'py, PyArray1<f64>>> {
            let slice = match name {
                "opacity" => self.inner.opacity(),
                "transmittance" => self.inner.transmittance(),
                "radiance" => self.inner.radiance(),
                "radiance_diff" => self.inner.radiance_diff(),
                "tb_planck" => self.inner.tb_planck(),
                "tb_rj" => self.inner.tb_rj(),
                "tsys" => self.inner.tsys(),
                "y_factor" => self.inner.y_factor(),
                "delay" => self.inner.delay(),
                "free_space_loss" => self.inner.free_space_loss(),
                "absorption_coeff" => self.inner.absorption_coeff(),
                _ => return Err(PyAttributeError::new_err(format!("no attribute '{name}'"))),
            };

            slice.map(|s| PyArray1::from_slice(py, s)).ok_or_else(|| {
                PyAttributeError::new_err(format!(
                    "'{name}' was not computed -- add 'output {name}' to your .amc file"
                ))
            })
        }
    }
}
