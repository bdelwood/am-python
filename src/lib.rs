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

use std::collections::HashMap;

use log::{debug, info};
use numpy::PyArray1;
use pyo3::prelude::*;
use pyo3_stub_gen::derive::*;

// generate stubs with `cargo run --release --bin stub_gen`
pyo3_stub_gen::define_stub_info_gatherer!(stub_info);

#[pyo3::pymodule]
mod _am {
    use super::*;
    use std::path::PathBuf;

    #[pymodule_init]
    fn init(m: &Bound<'_, PyModule>) -> PyResult<()> {
        pyo3_log::Logger::new(m.py(), pyo3_log::Caching::LoggersAndLevels)?
            .filter(log::LevelFilter::Trace)
            .install()
            .ok();
        error::register(m)?;
        Ok(())
    }

    /// An am atmospheric model loaded from an ``.amc`` configuration file.
    ///
    /// Parameters
    /// ----------
    /// path:
    ///     Path to the ``.amc`` file.
    /// args:
    ///     Positional substitution values for ``%1``, ``%2``, … placeholders
    ///     in the config (frequency grid, zenith angle, PWV scale, etc.).
    ///
    /// Examples
    /// --------
    /// .. code-block:: python
    ///
    ///     m = am.Model("SPole_JJA_75.amc", [0, "GHz", 350, "GHz", 0.5, "GHz", 35, "deg", 1.0])
    ///     m.compute()
    ///     m.outputs["tb_rj"]
    #[gen_stub_pyclass]
    #[pyclass(module = "am._am")]
    struct Model {
        inner: models::AmModel,
    }

    #[gen_stub_pymethods]
    #[pymethods]
    impl Model {
        #[new]
        #[pyo3(signature = (path, args))]
        fn new(path: PathBuf, args: Vec<Bound<'_, PyAny>>) -> PyResult<Self> {
            info!("Loading model from {}", path.display());
            let string_args: Vec<String> = args
                .iter()
                .map(|a| a.str().map(|s| s.to_string()))
                .collect::<PyResult<_>>()?;
            Ok(Self {
                inner: models::AmModel::from_amc(&path, &string_args)?,
            })
        }

        /// Run the radiative transfer computation.
        ///
        /// Releases the GIL while running so other Python threads are not blocked.
        /// Must be called before accessing :attr:`outputs`.
        fn compute(&mut self, py: Python<'_>) -> PyResult<()> {
            debug!("Running compute");
            py.detach(|| self.inner.compute())?;
            Ok(())
        }

        /// Frequency grid in GHz.
        #[getter]
        fn frequency<'py>(&self, py: Python<'py>) -> Bound<'py, PyArray1<f64>> {
            PyArray1::from_slice(py, self.inner.frequency())
        }

        /// Dict mapping output name to computed spectrum array.
        ///
        /// Keys are Python-style names (e.g. ``"tb_rj"``, ``"opacity"``).
        /// Only outputs listed in the ``output`` directive of the ``.amc`` file
        /// are present. Empty before :meth:`compute` is called.
        #[getter]
        fn outputs<'py>(&self, py: Python<'py>) -> HashMap<&str, Bound<'py, PyArray1<f64>>> {
            [
                ("opacity", self.inner.opacity()),
                ("transmittance", self.inner.transmittance()),
                ("radiance", self.inner.radiance()),
                ("radiance_diff", self.inner.radiance_diff()),
                ("tb_planck", self.inner.tb_planck()),
                ("tb_rj", self.inner.tb_rj()),
                ("tsys", self.inner.tsys()),
                ("y_factor", self.inner.y_factor()),
                ("delay", self.inner.delay()),
                ("free_space_loss", self.inner.free_space_loss()),
                ("absorption_coeff", self.inner.absorption_coeff()),
            ]
            .into_iter()
            // filter will skip those names which return None
            // which occurs when that particular output is not requested
            .filter_map(|(name, s)| s.map(|s| (name, PyArray1::from_slice(py, s))))
            .collect()
        }

        /// Full resolved model configuration, as am would print it to stderr.
        ///
        /// Equivalent to running ``am`` on the command line with the same arguments.
        /// Also available as ``str(model)`` / ``print(model)``.
        fn summary(&mut self) -> String {
            self.inner.summary()
        }

        fn __str__(&mut self) -> PyResult<String> {
            Ok(self.summary())
        }
    }
}
