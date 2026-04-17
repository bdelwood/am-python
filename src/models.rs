use crate::error::{AmError, AmResult};
use crate::ffi;
use std::ffi::{CString, c_char, c_int};
use std::path::Path;
use std::sync::Mutex;

// guard mutex for am's global state
static AM_MUTEX: Mutex<()> = Mutex::new(());

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
                "Config parser returned non-zero error code: {}",
                ret
            )));
        }

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
        let ret = unsafe { ffi::compute_model(&mut self.model, std::ptr::null_mut()) };

        if ret != 0 {
            return Err(AmError::Compute(format!(
                "compute_model returned non-zero error code: {}",
                ret
            )));
        }

        self.computed = true;

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
        let _lock = AM_MUTEX.lock().unwrap();
        unsafe {
            ffi::free_model_entities(&mut self.model);
            ffi::free_fit_data_entities(&mut self.fit_data);
            ffi::free_simplex_entities(&mut self.simplex);
            ffi::kcache_free_all();
            ffi::free_Nscale_list();
            ffi::free_tag_string_table();
        }
    }
}

unsafe impl Send for AmModel {}
unsafe impl Sync for AmModel {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_from_amc() {
        AmModel::from_amc(
            Path::new("/home/brodi/Documents/git/github/kovac-code/projects/visualizations/outputs/SPole_JJA_75.amc"),
            ["0", "GHz", "350", "GHz", "0.01", "GHz", "35", "deg", "1.0"],
        ).unwrap();
    }
}
