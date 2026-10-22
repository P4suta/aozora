//! Replay promoted formatter fuzz inputs for panic and idempotence regressions.
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
            continue;
        }
        for artifact in read_dir_sorted(&target_dir) {
            if !artifact.is_file() {
                continue;
            }
            let bytes = fs::read(&artifact).expect("read fuzz artifact");
            let text = String::from_utf8_lossy(&bytes);
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

fn read_dir_sorted(dir: &Path) -> Vec<PathBuf> {
    let mut entries: Vec<PathBuf> = fs::read_dir(dir)
        .map(|rd| rd.flatten().map(|entry| entry.path()).collect())
        .unwrap_or_default();
    entries.sort();
    entries
}
