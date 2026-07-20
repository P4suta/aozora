//! Stage the drift-gated C header beside each built library.

use std::env;
use std::path::PathBuf;

fn main() {
    let crate_dir = env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR set by cargo");
    let out_dir = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR set by cargo"));
    let generated = PathBuf::from(&crate_dir).join("include/aozora.h");
    let header_in_out = out_dir.join("aozora.h");

    println!("cargo:rerun-if-changed=include/aozora.h");
    std::fs::copy(&generated, &header_in_out).expect("copy generated aozora.h to OUT_DIR");

    if let Some(profile_dir) = out_dir.ancestors().nth(3) {
        let mirrored = profile_dir.join("aozora.h");
        std::fs::copy(&header_in_out, &mirrored).expect("copy generated aozora.h beside library");
    }
}
