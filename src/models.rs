use crate::error::{AmError, AmResult};
use crate::ffi;
use log::{debug, info};
use std::ffi::{CString, c_char, c_int};
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

macro_rules! model_output_accessor {
    ($name:ident, $field:ident) => {
        pub fn $name(&self) -> Option<&[f64]> {
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
pub(crate) struct AmModel {
    model: ffi::model_t,
    fit_data: ffi::fit_data_t,
    simplex: ffi::simplex_t,
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
            .ok_or_else(|| AmError::ConfigParse("Path contains invalid UTF-8".into()))?;

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

        let ret = unsafe {
            ffi::parse_config_file(
                argc,
                argv.as_mut_ptr(),
                &mut model,
                &mut fit_data,
                &mut simplex,
            )
        };

        if ret != 0 {
            return Err(AmError::ConfigParse(format!(
                "parse_config_file returned error code {ret}"
            )));
        }

        info!(
            "Parsed config: {path_str}, ngrid={}, nlayers={}",
            model.ngrid, model.nlayers
        );

        Ok(Self {
            model,
            fit_data,
            simplex,
            computed: false,
        })
    }

    pub fn compute(&mut self) -> AmResult<()> {
        let _lock = AM_MUTEX.lock().unwrap();

        // pass in model struct to first arg
        // null for second, as we've not implemented lmodel yet.
        debug!("Calling compute_model (ngrid={})", self.model.ngrid);
        let ret = unsafe { ffi::compute_model(&mut self.model, std::ptr::null_mut()) };

        if ret != 0 {
            return Err(AmError::Compute(format!(
                "compute_model returned error code {ret}"
            )));
        }

        self.computed = true;
        debug!("Computation complete");

        Ok(())
    }

    pub fn frequency(&self) -> &[f64] {
        unsafe { std::slice::from_raw_parts(self.model.f, self.model.ngrid as usize) }
    }

    model_output_accessor!(opacity, tau);
    model_output_accessor!(transmittance, tx);
    model_output_accessor!(radiance, I);
    model_output_accessor!(radiance_diff, I_diff);
    model_output_accessor!(tb_planck, Tb);
    model_output_accessor!(tb_rj, Trj);
    model_output_accessor!(tsys, Tsys);
    model_output_accessor!(y_factor, Y);
    model_output_accessor!(delay, L);
    model_output_accessor!(free_space_loss, tau_fsl);
    model_output_accessor!(absorption_coeff, k_out);
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

    #[test]
    fn test_sequential_runs() {
        let amc = Path::new("assets/SPole_JJA_75.amc");
        let args: Vec<String> = ["0", "GHz", "350", "GHz", "0.5", "GHz", "35", "deg", "1.0"]
            .iter()
            .map(|s| s.to_string())
            .collect();

        for i in 0..3 {
            let mut m = AmModel::from_amc(amc, &args).unwrap();
            m.compute().unwrap();
            assert_eq!(m.frequency().len(), 701, "run {i}: wrong grid size");
            assert!(m.transmittance().is_some(), "run {i}: no transmittance");
        }
    }
}
