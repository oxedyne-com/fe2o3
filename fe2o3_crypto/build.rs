extern crate bindgen;

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

#[allow(dead_code)] // for unused Scheme variants
fn main() {

    env::set_current_dir(&Path::new("src/c")).expect("Could not change to C directory");

    let output = Command::new("./build_all")
        .output()
        .expect("Failed to execute script to generate SABER C static libraries.");

    env::set_current_dir(&Path::new("../..")).expect("Could not change back to workspace directory");

    println!("status: {}", output.status);
    let mut file = File::create("build.log").expect("Creation of build.log file failed.");
    file.write_all(b"Stdout>>>>>>>>>>>>>\n").expect("Write to build.log failed.");
    file.write_all(output.stdout.as_slice()).expect("Write to build.log failed.");
    file.write_all(b"Stderr>>>>>>>>>>>>>\n").expect("Write to build.log failed.");
    file.write_all(output.stderr.as_slice()).expect("Write to build.log failed.");

    enum Scheme {
        LightSaber,
        Saber,
        FireSaber,
    }
    // Single point to select which C library to use:
    let scheme = Scheme::FireSaber; //<<<<<<<<<< YOU SET THIS

    println!("cargo:rustc-link-search=/usr/lib/x86_64-linux-gnu");
    println!("cargo:rustc-link-lib=static=crypto");

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
