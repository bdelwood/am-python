use crate::error::{AmErr, AmResult};
use crate::ffi;
use log::{debug, info, warn};
use std::collections::{HashMap, HashSet};
use std::ffi::{CStr, CString, c_char, c_int};
use std::io::Read;
use std::os::fd::FromRawFd;
use std::path::Path;
use std::sync::{Mutex, OnceLock};

// Mutex serializes access to am's global mutable state.
static AM_MUTEX: Mutex<()> = Mutex::new(());

// Wrapper to make output_tabentry array Sync
// safety: safe because access is serialized by the mutex.
struct SyncOutputTable([ffi::output_tabentry; 14]);
unsafe impl Sync for SyncOutputTable {}
unsafe impl Send for SyncOutputTable {}

// Save initial state of am's output[] and outcol[] globals.
// Captured on first use, restored before each new model run.
static INITIAL_OUTPUT: OnceLock<SyncOutputTable> = OnceLock::new();
static INITIAL_OUTCOL: OnceLock<[c_int; 13]> = OnceLock::new();

/// Save am's global output state on first call, then restore it every call)
/// Must be called under the mutex.
unsafe fn reset_output_globals() {
    INITIAL_OUTPUT.get_or_init(|| SyncOutputTable(unsafe { ffi::output }));
    INITIAL_OUTCOL.get_or_init(|| unsafe { ffi::outcol });

    if let Some(saved) = INITIAL_OUTPUT.get() {
        unsafe { ffi::output = saved.0 };
    }
    if let Some(saved) = INITIAL_OUTCOL.get() {
        unsafe { ffi::outcol = *saved };
    }
}

/// Redirect fd 2 (stderr) to a pipe, run a closure, then read the captured output.
/// Returns tuple (closure result, captured stderr text).
///
/// Safety: must not be called concurrently: the fd 2 redirect is process-global.
/// This is already guaranteed by AM_MUTEX.
unsafe fn capture_stderr<F, R>(f: F) -> (R, String)
where
    F: FnOnce() -> R,
{
    let mut pipe_fds: [libc::c_int; 2] = [0; 2];
    if unsafe { libc::pipe(pipe_fds.as_mut_ptr()) } != 0 {
        return (f(), String::new());
    }
    let read_end = pipe_fds[0];
    let write_end = pipe_fds[1];

    let saved_stderr = unsafe { libc::dup(2) };
    if saved_stderr < 0 {
        unsafe { libc::close(read_end) };
        unsafe { libc::close(write_end) };
        return (f(), String::new());
    }
    unsafe { libc::dup2(write_end, 2) };
    unsafe { libc::close(write_end) }; // fd 2 still holds the write end

    let result = f();

    // Restore stderr, closing the pipe's write end
    unsafe { libc::dup2(saved_stderr, 2) };
    unsafe { libc::close(saved_stderr) };

    // File takes ownership of read_end, handles read + close on drop
    let mut captured = String::new();
    let _ = unsafe { std::fs::File::from_raw_fd(read_end) }.read_to_string(&mut captured);

    (result, captured)
}

/// Capture the output of a C function that writes to a FILE* stream.
/// Uses tmpfile so there's no buffer size limit.
unsafe fn capture_file_output<F>(f: F) -> String
where
    F: FnOnce(*mut ffi::FILE),
{
    let tmp = unsafe { libc::tmpfile() };
    if tmp.is_null() {
        return String::new();
    }

    f(tmp as *mut ffi::FILE);
    unsafe { libc::fflush(tmp) };
    unsafe { libc::fseek(tmp, 0, libc::SEEK_SET) };

    // dup the fd so Rust File owns an independent copy; fclose handles the FILE*.
    let fd = unsafe { libc::dup(libc::fileno(tmp)) };
    unsafe { libc::fclose(tmp) };

    let mut captured = String::new();
    if fd >= 0 {
        let _ = unsafe { std::fs::File::from_raw_fd(fd) }.read_to_string(&mut captured);
    }
    captured
}

/// Log any new warnings from am's errlog to log::warn.
/// Filters on "! Warning:" entries only,
/// since errors were already raised as exceptions
/// strips "Count: N" suffixes, and deduplicates.
/// Must be called under AM_MUTEX or you'll have a bad day.
fn emit_warnings() {
    if unsafe { ffi::errstat() } == 0 {
        return;
    }
    let (_, output) = unsafe { capture_stderr(|| ffi::print_errlog()) };

    // Only keep warning entries, not error entries. Each errlog entry
    // starts with "! Warning:" or "! Error:", continuation lines start
    // with "!" + spaces.
    // I kinda hate I have to do this parsing,
    // but am doesn't expose the error code table
    // in order to parse what returns actually are
    let mut lines = Vec::new();
    let mut in_warning = false;
    for line in output.trim().lines() {
        if line.is_empty() {
            continue;
        }
        if line.starts_with("! Warning:") {
            in_warning = true;
        } else if line.starts_with("! Error:") {
            in_warning = false;
        }
        if in_warning {
            lines.push(line.split("  Count:").next().unwrap_or(line));
        }
    }
    let cleaned = lines.join("\n");
    if cleaned.is_empty() {
        return;
    }

    let mut logged = LOGGED_WARNINGS.lock().unwrap();
    let set = logged.get_or_insert_with(HashSet::new);
    if set.insert(cleaned.clone()) {
        cleaned.lines().for_each(|line| warn!("{line}"));
    }
}

/// Maps Python-facing output names to their C output[] index.
/// Used by Jacobian extraction (reads from the output[] global within one lock).
pub(crate) const OUTPUT_TABLE: &[(&str, u32)] = &[
    ("opacity", ffi::OUTPUT_OPACITY),
    ("transmittance", ffi::OUTPUT_TRANSMITTANCE),
    ("radiance", ffi::OUTPUT_RADIANCE),
    ("radiance_diff", ffi::OUTPUT_RADIANCE_DIFF),
    ("tb_planck", ffi::OUTPUT_TB_PLANCK),
    ("tb_rj", ffi::OUTPUT_TB_RAYLEIGH_JEANS),
    ("tsys", ffi::OUTPUT_TSYS),
    ("y_factor", ffi::OUTPUT_Y),
    ("delay", ffi::OUTPUT_DELAY),
    ("free_space_loss", ffi::OUTPUT_FREE_SPACE_LOSS),
    ("absorption_coeff", ffi::OUTPUT_K),
];

macro_rules! model_output_accessor {
    ($name:ident, $field:ident, $idx:expr) => {
        pub fn $name(&self) -> Option<&[f64]> {
            if !self.computed || self.requested & (1 << $idx) == 0 {
                return None;
            }
            let ptr = self.model.$field;
            if ptr.is_null() {
                None
            } else {
                Some(unsafe { std::slice::from_raw_parts(ptr, self.model.ngrid as usize) })
            }
        }
    };
}

// Dedup warnings across model instances: only log each unique warning once.
static LOGGED_WARNINGS: Mutex<Option<HashSet<String>>> = Mutex::new(None);

// create struct that contains 4 objects needed to run compute:
//  model_t        model = MODEL_INIT;
//  model_t       lmodel = MODEL_INIT;
//  fit_data_t  fit_data = FIT_DATA_INIT;
//  simplex_t    simplex = SIMPLEX_INIT;
// see main.c lines 44-47
// "lmodel" (aka "last model") needed for Jacobian/fits
// Need smart pointers here to provide owned heap allocation
// so that pointers into simplex remain valid when the struct's ownership moves to PyO3
pub(crate) struct AmModel {
    model: Box<ffi::model_t>,
    lmodel: Option<Box<ffi::model_t>>,
    fit_data: ffi::fit_data_t,
    simplex: ffi::simplex_t,
    /// Bitmask of OUTPUT_* indices that had the OUTPUT_USER flag set after parsing.
    /// Bit i is set when `output[i].flags & OUTPUT_USER != 0`.
    requested: u32,
    computed: bool,
}

// now we just need to expose the methods needed to compute
impl AmModel {
    // constructor will load from amc file
    // args are just like one would pass in the command line
    pub fn from_amc(path: &Path, args: &[String]) -> AmResult<Self> {
        let _lock = AM_MUTEX.lock().unwrap();

        // Reset am's global state before parsing a new config.
        // - output[]/outcol[]: restore initial values, ie clear dangling spectrum pointers
        // - kcache: free cached absorption coefficients that reference the
        //   previous model's layer structures
        debug!("Resetting am global state");
        unsafe {
            reset_output_globals();
            ffi::kcache_free_all();
        };

        let path_str = path
            .to_str()
            .ok_or_else(|| AmErr::Config("Path contains invalid UTF-8".into()))?;

        // parse_config_file expects argv like ["am", "./path.amc", arg0, arg1, ...]
        let c_strings: Vec<CString> = ["am", path_str]
            .into_iter()
            .chain(args.iter().map(|s| s.as_str()))
            .map(|s| CString::new(s).unwrap())
            .collect();
        let mut argv: Vec<*mut c_char> = c_strings
            .iter()
            .map(|s| s.as_ptr() as *mut c_char)
            .collect();

        let argc = argv.len() as c_int;

        // Box the model before parsing so simplex.varptr[j] gets stable
        // heap addresses from the start
        let mut model = Box::new(unsafe { ffi::MODEL_INIT });
        let mut fit_data = unsafe { ffi::FIT_DATA_INIT };
        let mut simplex = unsafe { ffi::SIMPLEX_INIT };

        // Capture stderr around parse_config_file
        // this is where am writes
        // "filename(line): error message" diagnostics and ?-style help text.
        // We don't call print_errlog here because parse_error output is
        // already probably enough, and errlog accumulates across runs.
        // so no reset needed
        let (ret, stderr_output) = unsafe {
            capture_stderr(|| {
                ffi::parse_config_file(
                    argc,
                    argv.as_mut_ptr(),
                    // remember to deref
                    &mut *model,
                    &mut fit_data,
                    &mut simplex,
                )
            })
        };

        if ret != 0 {
            let msg = if stderr_output.is_empty() {
                format!("parse_config_file returned error code {ret}")
            } else {
                stderr_output.trim().to_string()
            };
            return Err(AmErr::Config(msg));
        }

        emit_warnings();

        // Snapshot which outputs the user requested before reset_output_globals
        // wipes them for the next model.
        let requested: u32 = (0..14_usize)
            .filter(|&i| (unsafe { ffi::output[i].flags } & ffi::OUTPUT_USER as i32) != 0)
            .fold(0u32, |acc, i| acc | (1 << i));

        info!(
            "Parsed config: {path_str}, ngrid={}, nlayers={}, requested=0x{requested:x}",
            model.ngrid, model.nlayers
        );

        Ok(Self {
            model,
            lmodel: None,
            fit_data,
            simplex,
            requested,
            computed: false,
        })
    }

    pub fn compute(&mut self) -> AmResult<()> {
        let _lock = AM_MUTEX.lock().unwrap();

        // pass in model struct to first arg
        // null for second, as we've not implemented lmodel yet.
        debug!("Calling compute_model (ngrid={})", self.model.ngrid);

        // compute_model writes only to errlog, not stderr directly.
        let ret = unsafe { ffi::compute_model(&mut *self.model, std::ptr::null_mut()) };

        if ret != 0 {
            // Capture print_errlog output for the error message.
            let (_, errlog_output) = unsafe {
                capture_stderr(|| {
                    ffi::print_errlog();
                })
            };
            let msg = if errlog_output.is_empty() {
                format!("compute_model returned error code {ret}")
            } else {
                errlog_output.trim().to_string()
            };
            return Err(AmErr::Compute(msg));
        }

        emit_warnings();
        self.computed = true;
        debug!("Computation complete");
        Ok(())
    }

    pub fn frequency(&self) -> &[f64] {
        unsafe { std::slice::from_raw_parts(self.model.f, self.model.ngrid as usize) }
    }

    model_output_accessor!(opacity, tau, ffi::OUTPUT_OPACITY);
    model_output_accessor!(transmittance, tx, ffi::OUTPUT_TRANSMITTANCE);
    model_output_accessor!(radiance, I, ffi::OUTPUT_RADIANCE);
    model_output_accessor!(radiance_diff, I_diff, ffi::OUTPUT_RADIANCE_DIFF);
    model_output_accessor!(tb_planck, Tb, ffi::OUTPUT_TB_PLANCK);
    model_output_accessor!(tb_rj, Trj, ffi::OUTPUT_TB_RAYLEIGH_JEANS);
    model_output_accessor!(tsys, Tsys, ffi::OUTPUT_TSYS);
    model_output_accessor!(y_factor, Y, ffi::OUTPUT_Y);
    model_output_accessor!(delay, L, ffi::OUTPUT_DELAY);
    model_output_accessor!(free_space_loss, tau_fsl, ffi::OUTPUT_FREE_SPACE_LOSS);
    model_output_accessor!(absorption_coeff, k_out, ffi::OUTPUT_K);

    /// Number of fit/differentiation variables defined in the .amc config.
    pub fn n_variables(&self) -> u32 {
        self.simplex.n
    }

    /// Names of the fit/differentiation variables defined in the .amc config.
    pub fn variables(&self) -> Vec<String> {
        let n = self.simplex.n as usize;
        if n == 0 || self.simplex.name.is_null() {
            return Vec::new();
        }
        (0..n)
            .map(|i| {
                let ptr = unsafe { *self.simplex.name.add(i) };
                if ptr.is_null() {
                    String::new()
                } else {
                    unsafe { CStr::from_ptr(ptr) }
                        .to_string_lossy()
                        .into_owned()
                }
            })
            .collect()
    }

    /// Lazily initialize lmodel for Jacobians/fitting (main.c lines 130-148).
    /// Caller must hold AM_MUTEX.
    fn ensure_lmodel(&mut self) -> AmResult<()> {
        if self.lmodel.is_some() {
            return Ok(());
        }
        let mut lmodel = Box::new(unsafe { ffi::MODEL_INIT });
        let ret = unsafe { ffi::setup_atmospheric_model(&mut *self.model, std::ptr::null_mut()) };
        if ret != 0 {
            return Err(AmErr::Compute("setup_atmospheric_model failed".into()));
        }
        let ret = unsafe { ffi::copy_model_dimensions(&mut *self.model, &mut *lmodel) };
        if ret != 0 {
            unsafe { ffi::free_model_entities(&mut *lmodel) };
            return Err(AmErr::Compute("copy_model_dimensions failed".into()));
        }
        self.lmodel = Some(lmodel);
        Ok(())
    }

    /// Compute Jacobians of all requested outputs wrt fit variables.
    /// Returns a map from output name to a 2D array (n_variables x ngrid).
    /// The .amc config must define fit variables.
    pub fn jacobian(&mut self) -> AmResult<HashMap<String, Vec<Vec<f64>>>> {
        if self.simplex.n == 0 {
            return Err(AmErr::Config(
                "No fit variables defined. Add a scale to parameters in the .amc config \
                 (e.g. 'T 250 K 5 K') to enable Jacobian computation."
                    .into(),
            ));
        }

        let _lock = AM_MUTEX.lock().unwrap();

        // Set OUTPUT_JACOBIAN flag on user-requested outputs that support it,
        // and on ALL_OUTPUTS (which compute_jacobians checks as a gate).
        for i in 1..ffi::OUTPUT_END_OF_TABLE as usize {
            let flags = unsafe { ffi::output[i].flags };
            let is_requested = self.requested & (1 << i) != 0;
            if is_requested && flags & ffi::JACOBIAN_ALLOWED as i32 != 0 {
                unsafe { ffi::output[i].flags |= ffi::OUTPUT_JACOBIAN as i32 };
            }
        }
        unsafe {
            ffi::output[ffi::ALL_OUTPUTS as usize].flags |= ffi::OUTPUT_JACOBIAN as i32;
        }

        let ret = unsafe { ffi::alloc_jacobians(&mut *self.model, &mut self.simplex) };
        if ret != 0 {
            return Err(AmErr::Compute("alloc_jacobians failed".into()));
        }

        self.ensure_lmodel()?;
        let lmodel = &mut **self.lmodel.as_mut().unwrap();

        // Compute, then unconditionally free jacobian arrays before returning.
        let err = unsafe {
            let r = ffi::compute_jacobians(&mut *self.model, lmodel, &mut self.simplex);
            if r != 0 {
                Some("compute_jacobians failed")
            } else {
                let r = ffi::compute_model(&mut *self.model, lmodel);
                if r != 0 {
                    Some("compute_model failed after jacobian")
                } else {
                    None
                }
            }
        };

        // Extract jacobian data from output[] before freeing.
        // Apply unit conversion to match CLI: scale by variable_unit_factor / output_unit_factor.
        let n_vars = self.simplex.n as usize;
        let ngrid = self.model.ngrid as usize;
        let unit_tab_ptr = std::ptr::addr_of!(ffi::unit_tab) as *const ffi::unit_tabentry;

        // Per-variable unit factors: unit_tab[simplex.unitnum[j]].factor
        let var_factors: Vec<f64> = (0..n_vars)
            .map(|j| unsafe {
                let unitnum = *self.simplex.unitnum.add(j) as usize;
                (*unit_tab_ptr.add(unitnum)).factor
            })
            .collect();

        let result: HashMap<String, Vec<Vec<f64>>> = OUTPUT_TABLE
            .iter()
            .filter_map(|&(name, idx)| {
                let jac_ptr = unsafe { ffi::output[idx as usize].jacobian };
                if jac_ptr.is_null() {
                    return None;
                }
                // Output unit factor: unit_tab[output[idx].unitnum].factor
                let out_unitnum = unsafe { ffi::output[idx as usize].unitnum } as usize;
                let out_factor = unsafe { (*unit_tab_ptr.add(out_unitnum)).factor };

                let matrix = (0..n_vars)
                    .map(|j| {
                        let scale = var_factors[j] / out_factor;
                        unsafe { std::slice::from_raw_parts(*jac_ptr.add(j), ngrid) }
                            .iter()
                            .map(|&v| v * scale)
                            .collect()
                    })
                    .collect();
                Some((name.to_string(), matrix))
            })
            .collect();

        unsafe { ffi::free_jacobians(&mut self.simplex) };

        if let Some(msg) = err {
            return Err(AmErr::Compute(msg.into()));
        }

        emit_warnings();
        self.computed = true;
        Ok(result)
    }

    /// Get the full resolved model configuration summary, equivalent to
    /// what am writes to stderr via write_model_config_data in the CLI.
    pub fn summary(&mut self) -> String {
        let _lock = AM_MUTEX.lock().unwrap();
        unsafe {
            capture_file_output(|stream| {
                ffi::write_model_config_data(
                    stream,
                    &mut *self.model,
                    &mut self.fit_data,
                    &mut self.simplex,
                );
            })
        }
    }
}

impl Drop for AmModel {
    fn drop(&mut self) {
        debug!("Dropping AmModel, freeing model entities");
        let _lock = AM_MUTEX.lock().unwrap();
        unsafe {
            // Save output[].spectrum — free_model_entities nullifies them as a
            // side effect, which would corrupt another live model's state.
            let saved_spectra: [*mut f64; 14] = std::array::from_fn(|i| ffi::output[i].spectrum);

            if let Some(ref mut lmodel) = self.lmodel {
                ffi::free_model_entities(&mut **lmodel);
            }
            ffi::free_model_entities(&mut *self.model);
            ffi::free_fit_data_entities(&mut self.fit_data);
            ffi::free_simplex_entities(&mut self.simplex);

            // Restore — another model may still need these pointers.
            for (i, ptr) in saved_spectra.into_iter().enumerate() {
                ffi::output[i].spectrum = ptr;
            }
            // kcache is cleared in from_amc() before each new model, not here.
            // free_Nscale_list/free_tag_string_table leave dangling static
            // pointers so are never safe to call in a library context.
        }
    }
}

unsafe impl Send for AmModel {}
unsafe impl Sync for AmModel {}

#[cfg(test)]
mod tests {
    use super::*;

    const AMC: &str = "assets/SPole_JJA_75.amc";

    fn args() -> Vec<String> {
        ["0", "GHz", "350", "GHz", "0.5", "GHz", "35", "deg", "1.0"]
            .iter()
            .map(|s| s.to_string())
            .collect()
    }

    #[test]
    fn test_sequential_runs() {
        let amc = Path::new(AMC);
        for i in 0..3 {
            let mut m = AmModel::from_amc(amc, &args()).unwrap();
            m.compute().unwrap();
            assert_eq!(m.frequency().len(), 701, "run {i}: wrong grid size");
            assert!(m.transmittance().is_some(), "run {i}: no transmittance");
        }
    }

    #[test]
    fn test_file_not_found() {
        let result = AmModel::from_amc(Path::new("nonexistent.amc"), &[]);
        let msg = result.err().expect("should fail").to_string();
        assert!(msg.contains("cannot open file"), "unexpected: {msg}");
    }

    #[test]
    fn test_missing_args() {
        let result = AmModel::from_amc(Path::new(AMC), &[]);
        let msg = result.err().expect("should fail").to_string();
        assert!(msg.contains("%1 has no matching"), "unexpected: {msg}");
        assert!(msg.contains("Usage:"), "should include help text: {msg}");
    }

    #[test]
    fn test_summary() {
        let mut m = AmModel::from_amc(Path::new(AMC), &args()).unwrap();
        m.compute().unwrap();
        let summary = m.summary();
        assert!(summary.contains("am version"), "missing version: {summary}");
        assert!(summary.contains("f 0 GHz"), "missing freq grid: {summary}");
    }

    const JACOBIAN_AMC: &str = "assets/MaunaKea_Jacobian.amc";

    fn jacobian_args() -> Vec<String> {
        [
            "220", "GHz", "230", "GHz", "5", "GHz", "0", "deg", "277", "K", "1.0",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect()
    }

    #[test]
    fn test_jacobian_no_variables() {
        let mut m = AmModel::from_amc(Path::new(AMC), &args()).unwrap();
        m.compute().unwrap();
        let err = m.jacobian().unwrap_err();
        assert!(
            err.to_string().contains("No fit variables"),
            "unexpected: {err}"
        );
    }

    #[test]
    fn test_jacobian() {
        let mut m = AmModel::from_amc(Path::new(JACOBIAN_AMC), &jacobian_args()).unwrap();
        assert_eq!(m.n_variables(), 1);
        assert_eq!(m.variables(), vec!["Nscale troposphere h2o"]);

        m.compute().unwrap();
        let jac = m.jacobian().unwrap();
        assert!(jac.contains_key("tb_rj"), "missing tb_rj: {:?}", jac.keys());

        let tb_jac = &jac["tb_rj"];
        assert_eq!(tb_jac.len(), 1, "expected 1 variable");
        assert_eq!(tb_jac[0].len(), 3, "expected 3 frequency points");

        // CLI: dTrj/d(Nscale_h2o) at 220 GHz = 16.67748
        let expected = 16.67748;
        let got = tb_jac[0][0];
        assert!(
            (got - expected).abs() / expected < 1e-4,
            "dTrj/dNscale at 220 GHz: got {got}, expected {expected}"
        );

        assert!(m.computed);
        assert!(m.tb_rj().is_some());
    }

    #[test]
    fn test_jacobian_without_prior_compute() {
        let mut m = AmModel::from_amc(Path::new(JACOBIAN_AMC), &jacobian_args()).unwrap();
        let jac = m.jacobian().unwrap();
        assert!(!jac.is_empty());
        assert!(m.tb_rj().is_some());
    }
}
