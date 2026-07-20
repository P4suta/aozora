#![cfg(unix)]

//! Pseudo-terminal smoke coverage for interactive CLI surfaces.

use std::path::Path;
use std::process::Command;

const BIN: &str = env!("CARGO_BIN_EXE_aozora");

#[test]
fn packaged_interactive_paths_run_on_a_pseudo_terminal() {
    let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scripts/tty-smoke.py");
    let status = Command::new("python3")
        .arg(script)
        .arg(BIN)
        .status()
        .expect("python3 must run the pseudo-terminal smoke");
    assert!(status.success(), "pseudo-terminal smoke failed: {status}");
}
