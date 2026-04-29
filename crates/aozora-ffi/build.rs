//! Generate the C header `aozora.h` from this crate's `extern "C"`
//! surface.
//!
//! Output goes to `$OUT_DIR/aozora.h` (cargo's standard build-script
//! convention so multiple cargo profiles don't collide) and a copy
//! is dropped at `target/<profile>/aozora.h` for host-side consumers
//! that don't want to spelunk into `OUT_DIR`.

use std::env;
use std::path::PathBuf;

fn main() {
    let crate_dir = env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR set by cargo");
    let out_dir = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR set by cargo"));
    let header_in_out = out_dir.join("aozora.h");

    // Re-run when any of these change. cbindgen otherwise reruns on
    // every cargo invocation since it has no implicit input tracking.
    println!("cargo:rerun-if-changed=src/lib.rs");
    println!("cargo:rerun-if-changed=cbindgen.toml");
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=Cargo.toml");

    let config = cbindgen::Config::from_file(PathBuf::from(&crate_dir).join("cbindgen.toml"))
        .expect("cbindgen.toml must be parseable");

    cbindgen::Builder::new()
        .with_crate(&crate_dir)
        .with_config(config)
        .with_language(cbindgen::Language::C)
        .generate()
        .expect("cbindgen failed to generate aozora.h")
        .write_to_file(&header_in_out);

    // Mirror to target/<profile>/aozora.h so consumers can find it
    // without parsing `OUT_DIR`. `OUT_DIR` walks up to the profile
    // dir as `out_dir/../../../`, e.g.
    //     target/release/build/aozora-ffi-XXXX/out/aozora.h
    //   → target/release/aozora.h
    if let Some(profile_dir) = out_dir.ancestors().nth(3) {
        let mirrored = profile_dir.join("aozora.h");
        let _ = std::fs::copy(&header_in_out, &mirrored);
    }
}
