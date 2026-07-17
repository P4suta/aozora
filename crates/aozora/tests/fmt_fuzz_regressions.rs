//! Stable replay of promoted fmt fuzz artifacts — no nightly required.
//!
//! `just fuzz-promote <target> <artifact>` drops a crash input into
//! `tests/fuzz_regressions/<target>/`. This test runs each one back through
//! [`aozora::fmt::format_source`] and re-asserts the property the libFuzzer
//! target checks (no panic + idempotent), so a fixed crash can never silently
//! regress. It is a no-op until the first promotion.
#![cfg(feature = "fmt")]

use aozora::fmt::format_source;
use std::fs;
use std::path::{Path, PathBuf};

#[test]
fn promoted_fuzz_artifacts_replay_cleanly() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fmt_fuzz_regressions");
    if !root.exists() {
        return;
    }
    let mut replayed = 0usize;
    for target_dir in read_dir_sorted(&root) {
        if !target_dir.is_dir() {
            continue; // skip the README and any stray files
        }
        for artifact in read_dir_sorted(&target_dir) {
            if !artifact.is_file() {
                continue;
            }
            let bytes = fs::read(&artifact).expect("read fuzz artifact");
            let text = String::from_utf8_lossy(&bytes);
            // The property the format_idempotent fuzz target asserts:
            // no panic, and a fixed point after one pass.
            let once = format_source(&text);
            let twice = format_source(&once);
            assert_eq!(
                once,
                twice,
                "promoted artifact {} is no longer idempotent",
                artifact.display(),
            );
            replayed += 1;
        }
    }
    eprintln!("fmt_fuzz_regressions: replayed {replayed} artifact(s)");
}

/// Directory entries as a sorted `Vec` so replay order is deterministic.
fn read_dir_sorted(dir: &Path) -> Vec<PathBuf> {
    let mut entries: Vec<PathBuf> = fs::read_dir(dir)
        .map(|rd| rd.flatten().map(|entry| entry.path()).collect())
        .unwrap_or_default();
    entries.sort();
    entries
}
