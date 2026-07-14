#[cfg(feature = "pq")]
extern crate bindgen;

#[cfg(feature = "pq")]
use std::{
    env,
    fs::File,
    io::Write,
    path::{
        Path,
        PathBuf,
    },
    process::Command,
};

/// Without the `pq` feature there is no C to compile and nothing to bind, so the build script does
/// nothing at all. This is what lets the crate be built for a target that has no C toolchain, which
/// is to say for a browser.
#[cfg(not(feature = "pq"))]
fn main() {
    println!("cargo:rerun-if-changed=build.rs");
}

#[cfg(feature = "pq")]
#[allow(dead_code)] // for unused Scheme variants
fn main() {

    enum Scheme {
        LightSaber,
        Saber,
        FireSaber,
    }
    // Single point to select which C library to use:
    let scheme = Scheme::FireSaber; //<<<<<<<<<< YOU SET THIS

    let scheme_lib = match scheme {
        Scheme::LightSaber	=> "liblightsaber.a",
        Scheme::Saber		=> "libsaber.a",
        Scheme::FireSaber	=> "libfiresaber.a",
    };

    // Tell cargo to only rerun when the C source dir or build.rs
    // itself change. Without these directives cargo reruns this
    // build script on every invocation, which would re-trigger the
    // ./build_all shell script below every single time.
    println!("cargo:rerun-if-changed=src/c");
    println!("cargo:rerun-if-changed=build.rs");

    // Run the C build only if the selected static library is
    // missing. The `rerun-if-changed` directive above covers the
    // "C source changed" case; this existence check is a belt-and-
    // braces defence for the case where the tree still has a
    // previous clean build's artefacts present.
    let c_dir = Path::new("src/c");
    let lib_path = c_dir.join(scheme_lib);
    if !lib_path.exists() {
        env::set_current_dir(c_dir).expect("Could not change to C directory");

        let output = Command::new("./build_all")
            .output()
            .expect("Failed to execute script to generate SABER C static libraries.");

        env::set_current_dir(Path::new("../..")).expect("Could not change back to workspace directory");

        println!("status: {}", output.status);
        let mut file = File::create("build.log").expect("Creation of build.log file failed.");
        file.write_all(b"Stdout>>>>>>>>>>>>>\n").expect("Write to build.log failed.");
        file.write_all(output.stdout.as_slice()).expect("Write to build.log failed.");
        file.write_all(b"Stderr>>>>>>>>>>>>>\n").expect("Write to build.log failed.");
        file.write_all(output.stderr.as_slice()).expect("Write to build.log failed.");

        if !lib_path.exists() {
            panic!(
                "SABER C build did not produce {} -- check fe2o3_crypto/build.log. \
                 Common cause: libssl-dev is not installed.",
                lib_path.display(),
            );
        }
    }

    println!("cargo:rustc-link-search=/usr/lib/x86_64-linux-gnu");
    println!("cargo:rustc-link-lib=static=crypto");
    // libcrypto.a pulls in compression-library symbols (zlib + zstd)
    // via its c_zlib.o / c_zstd.o wrappers. Link them dynamically so
    // the test binary can resolve them. Downstream binaries that link
    // this crate as a lib usually pick these up transitively from
    // another dependency, so the failure only surfaces for
    // `cargo test -p fe2o3_crypto`.
    println!("cargo:rustc-link-lib=z");
    println!("cargo:rustc-link-lib=zstd");

    let dir = env::var("CARGO_MANIFEST_DIR").unwrap();
    println!(
        "cargo:rustc-link-search={}",
        Path::new(&dir).join("src/c").display(),
    );
    match scheme {
        Scheme::LightSaber => {
            println!("cargo:rustc-env=SABER_SCHEME=LIGHTSABER");
            println!("cargo:rustc-cfg=SABER_SCHEME=\"LIGHTSABER\"");
            println!("cargo:rustc-link-lib=static=lightsaber");
        },
        Scheme::Saber => {
            println!("cargo:rustc-env=SABER_SCHEME=SABER");
            println!("cargo:rustc-cfg=SABER_SCHEME=\"SABER\"");
            println!("cargo:rustc-link-lib=static=saber");
        },
        Scheme::FireSaber => {
            println!("cargo:rustc-env=SABER_SCHEME=FIRESABER");
            println!("cargo:rustc-cfg=SABER_SCHEME=\"FIRESABER\"");
            println!("cargo:rustc-link-lib=static=firesaber");
        },
    }

    println!("cargo:rustc-check-cfg=cfg(SABER_SCHEME, \
        values(\"LIGHTSABER\", \"SABER\", \"FIRESABER\"))");

    println!("cargo:rustc-link-lib=ssl");
    println!("cargo:rustc-link-lib=crypto");

    let wrapper_path = Path::new(&dir).join("src/c/bindgen_wrapper.h");
    println!(
        "cargo:rerun-if-changed={}",
        wrapper_path.display(),
    );

    let bindings = bindgen::Builder::default()
        .header(wrapper_path.to_str().unwrap())
        .parse_callbacks(Box::new(bindgen::CargoCallbacks))
        .generate()
        .expect("Unable to generate bindings");

    // Write the bindings to the $OUT_DIR/bindings.rs file.
    let out_path = PathBuf::from(env::var("OUT_DIR").unwrap());
    bindings
        .write_to_file(out_path.join("bindings.rs"))
        .expect("Couldn't write bindings!");

}
