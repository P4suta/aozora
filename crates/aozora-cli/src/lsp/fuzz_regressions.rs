//! Stable replay of promoted LSP fuzz artifacts — no nightly.
//!
//! A promoted crash input is a raw byte file under
//! `src/lsp/fuzz_regressions/<target>/`. This test re-runs each one
//! through the same property the libFuzzer target asserts, so a fixed
//! crash can never silently regress. No-op until the first promotion.

use std::fs;
use std::path::{Path, PathBuf};

use super::position::{byte_offset_to_position, position_to_byte_offset};
use super::text_edit::ByteEdit;

#[test]
fn promoted_fuzz_artifacts_replay_cleanly() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/lsp/fuzz_regressions");
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
            let data = fs::read(&artifact).expect("read fuzz artifact");
            replay_edit_pipeline(&data);
            replayed += 1;
        }
    }
    eprintln!("fuzz_regressions: replayed {replayed} artifact(s)");
}

/// Mirror of the `edit_pipeline` fuzz target body (keep in sync). The
/// asserts must match the target so a promoted crash reproduces here.
fn replay_edit_pipeline(data: &[u8]) {
    let text = String::from_utf8_lossy(data).into_owned();

    if data.len() >= 4 {
        let a = u16::from_le_bytes([data[0], data[1]]) as usize;
        let b = u16::from_le_bytes([data[2], data[3]]) as usize;
        let repl = String::from_utf8_lossy(&data[4..data.len().min(16)]).into_owned();
        let edit = ByteEdit::new(a..b, repl);
        let mut document = aozora::parse(text.clone()).expect("fuzz source fits");
        drop(document.edit([aozora::TextEdit::new(edit.range, edit.new_text)]));
    }

    if text.len() <= 8192 {
        for byte in 0..=text.len() {
            if !text.is_char_boundary(byte) {
                continue;
            }
            let pos = byte_offset_to_position(&text, byte);
            if let Some(back) = position_to_byte_offset(&text, pos) {
                assert_eq!(back, byte, "position round-trip broke at byte {byte}");
            }
        }
    }
}

/// Directory entries as a sorted `Vec` so replay order is deterministic.
fn read_dir_sorted(dir: &Path) -> Vec<PathBuf> {
    let mut entries: Vec<PathBuf> = fs::read_dir(dir)
        .map(|rd| rd.flatten().map(|entry| entry.path()).collect())
        .unwrap_or_default();
    entries.sort();
    entries
}
