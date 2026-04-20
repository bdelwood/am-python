use crate::error::{AmErr, AmResult};
use crate::ffi;
use log::{debug, info, warn};
use std::collections::HashSet;
use std::ffi::{CString, c_char, c_int};
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

// create struct that contains 3 objects needed to run compute:
//  model_t        model = MODEL_INIT;
//  model_t       lmodel = MODEL_INIT;
//  fit_data_t  fit_data = FIT_DATA_INIT;
//  simplex_t    simplex = SIMPLEX_INIT;
// see main.c lines 44-47
// For now, ignore "lmodel" (aka "last model") as we're not doing fits yet
// Dedup warnings across model instances: only log each unique warning once.
static LOGGED_WARNINGS: Mutex<Option<HashSet<String>>> = Mutex::new(None);

pub(crate) struct AmModel {
    model: ffi::model_t,
    fit_data: ffi::fit_data_t,
    simplex: ffi::simplex_t,
    /// Bitmask of OUTPUT_* indices that had the OUTPUT_USER flag set after parsing.
    /// Bit i is set when output[i].flags & OUTPUT_USER != 0.
    requested: u32,
    computed: bool,
}

// now we just need to expose the methods needed to compute
impl AmModel {
    // constructor will load from amc file
    // args are just like one would pass in the command line
    pub fn from_amc(path: &Path, args: &[String]) -> AmResult<Self> {
        let _lock = AM_MUTEX.lock().unwrap();

        // Reset am's global output state before parsing a new config.
        // This restores output[] and outcol[] to their initial values,
        // clearing any dangling spectrum pointers and stale flags from
        // a previous model run.
        debug!("Resetting am global output state");
        unsafe { reset_output_globals() };

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

        let mut model = unsafe { ffi::MODEL_INIT };
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
                    &mut model,
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

        // Snapshot which outputs the user requested: bit i is set when
        // output[i].flags has OUTPUT_USER set. Must be read now, under the mutex,
        // before reset_output_globals wipes it for the next model.
        const OUTPUT_USER: i32 = 0x1;
        let requested: u32 = (0..14_usize)
            .filter(|&i| (unsafe { ffi::output[i].flags } & OUTPUT_USER) != 0)
            .fold(0u32, |acc, i| acc | (1 << i));

        info!(
            "Parsed config: {path_str}, ngrid={}, nlayers={}, requested=0x{requested:x}",
            model.ngrid, model.nlayers
        );

        Ok(Self {
            model,
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
        let ret = unsafe { ffi::compute_model(&mut self.model, std::ptr::null_mut()) };

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

    /// Get the full resolved model configuration summary, equivalent to
    /// what am writes to stderr via write_model_config_data in the CLI.
    pub fn summary(&mut self) -> String {
        let _lock = AM_MUTEX.lock().unwrap();
        unsafe {
            capture_file_output(|stream| {
                ffi::write_model_config_data(
                    stream,
                    &mut self.model,
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
            ffi::free_model_entities(&mut self.model);
            ffi::free_fit_data_entities(&mut self.fit_data);
            ffi::free_simplex_entities(&mut self.simplex);
            // Unlike main.c, Do NOT call kcache_free_all(), free_Nscale_list(),free_tag_string_table()
            // They free memory but leave their file-scope static pointers dangling;
            // they are not reset to null.
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
}
