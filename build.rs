use std::env;
use std::path::PathBuf;

fn main() {
    let am_src = PathBuf::from(env::var("AM_SRC_DIR").unwrap_or_else(|_| "./am/src".into()));

    // cc needs us to explicitly list what c files to include
    let c_files: Vec<PathBuf> = std::fs::read_dir(&am_src)
        .expect("AM_SRC_DIR not found — set it to the am source directory")
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "c") && p.file_name().unwrap() != "main.c")
        .collect();

    // build using C99
    let mut build = cc::Build::new();
    build
        .files(&c_files)
        .include(&am_src)
        .std("c99")
        .warnings(false)
        .opt_level(3);

    // feature flag to enable openmp
    if env::var("CARGO_FEATURE_OPENMP").is_ok() {
        build.flag("-fopenmp");
        // Linux uses GCC's libgomp
        // macOS/clang uses libomp
        // In manylinux wheel builds, auditwheel bundles libgomp into the wheel,
        // so no static linking is needed.
        if env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos") {
            println!("cargo:rustc-link-lib=omp");
        } else {
            println!("cargo:rustc-link-lib=gomp");
        }
    }

    // all other am compile-time flags can be passed too
    if let Ok(flags) = env::var("EXTRA_CFLAGS") {
        for flag in flags.split_whitespace() {
            build.flag(flag);
        }
    }

    // actually compile
    build.compile("am");

    println!("cargo:rustc-link-lib=m");

    for f in &c_files {
        println!("cargo:rerun-if-changed={}", f.display());
    }
    println!("cargo:rerun-if-env-changed=AM_SRC_DIR");
    println!("cargo:rerun-if-env-changed=EXTRA_CFLAGS");

    // bindgen only runs when feature is enabled
    // see we are able to enable it only when needed
    #[cfg(feature = "bindgen")]
    {
        let bindings = bindgen::Builder::default()
            .header(am_src.join("am_types.h").to_str().unwrap())
            .header(am_src.join("model.h").to_str().unwrap())
            .header(am_src.join("config.h").to_str().unwrap())
            .header(am_src.join("am_alloc.h").to_str().unwrap())
            .header(am_src.join("output.h").to_str().unwrap())
            .header(am_src.join("errlog.h").to_str().unwrap())
            .header(am_src.join("kcache.h").to_str().unwrap())
            .header(am_src.join("nscale.h").to_str().unwrap())
            .header(am_src.join("tags.h").to_str().unwrap())
            .header(am_src.join("simplex.h").to_str().unwrap())
            .header(am_src.join("jacobian.h").to_str().unwrap())
            .header(am_src.join("units.h").to_str().unwrap())
            .clang_arg(format!("-I{}", am_src.display()))
            .allowlist_type("model_t|layer_t|column_t|abscoeff_t|fit_data_t|simplex_t|output_tabentry|unit_tabentry")
            .allowlist_function(
                "parse_config_file|compute_model|setup_atmospheric_model|\
                 total_airmass|total_refraction|\
                 free_model_entities|free_fit_data_entities|free_simplex_entities|\
                 kcache_free_all|free_Nscale_list|free_tag_string_table|\
                 errstat|errtest|print_errlog|set_active_outputs|\
                 write_model_config_data|\
                 compute_jacobians|alloc_jacobians|free_jacobians|\
                 copy_model_dimensions",
            )
            .allowlist_var("MODEL_INIT|FIT_DATA_INIT|SIMPLEX_INIT|output|outcol|OUTPUT_END_OF_TABLE|ALL_OUTPUTS|OUTPUT_USER|OUTPUT_JACOBIAN|JACOBIAN_ALLOWED|OUTPUT_FITTED|OUTPUT_ACTIVE|unit_tab")
            .blocklist_type("FILE|_IO_FILE|_IO_marker|_IO_codecvt|_IO_wide_data")
            .raw_line("pub type FILE = std::os::raw::c_void;")
            .generate()
            .expect("bindgen failed");

        bindings
            .write_to_file("src/ffi.rs")
            .expect("failed to write ffi.rs");
    }
}
