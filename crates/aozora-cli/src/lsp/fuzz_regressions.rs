//! Replays promoted language-server fuzz artifacts with their original assertions.

use std::fs;
use std::path::{Path, PathBuf};

use super::position::{byte_offset_to_position, position_to_byte_offset};
use super::text_edit::ByteEdit;

const OFFSET_FIELD_BYTES: usize = 2;
const EDIT_HEADER_BYTES: usize = 2 * OFFSET_FIELD_BYTES;
const REPLACEMENT_END_BYTE_LIMIT: usize = 16;
const POSITION_ROUND_TRIP_BYTE_LIMIT: usize = 8192;

#[test]
#[expect(
    clippy::print_stderr,
    reason = "fuzz replay reports its artifact count in the test log"
)]
fn promoted_fuzz_artifacts_replay_cleanly() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/lsp/fuzz_regressions");
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
            let data = fs::read(&artifact).expect("read fuzz artifact");
            replay_edit_pipeline(&data);
            replayed += 1;
        }
    }
    eprintln!("fuzz_regressions: replayed {replayed} artifact(s)");
}

/// Replays the `edit_pipeline` fuzz properties.
fn replay_edit_pipeline(data: &[u8]) {
    let text = String::from_utf8_lossy(data).into_owned();

    if data.len() >= EDIT_HEADER_BYTES {
        let a = u16::from_le_bytes([data[0], data[1]]) as usize;
        let b =
            u16::from_le_bytes([data[OFFSET_FIELD_BYTES], data[OFFSET_FIELD_BYTES + 1]]) as usize;
        let repl = String::from_utf8_lossy(
            &data[EDIT_HEADER_BYTES..data.len().min(REPLACEMENT_END_BYTE_LIMIT)],
        )
        .into_owned();
        let edit = ByteEdit::new(a..b, repl);
        let mut document = aozora::parse(text.clone()).expect("fuzz source fits");
        drop(document.edit([aozora::TextEdit::new(edit.range, edit.new_text)]));
    }

    if text.len() <= POSITION_ROUND_TRIP_BYTE_LIMIT {
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

fn read_dir_sorted(dir: &Path) -> Vec<PathBuf> {
    let mut entries: Vec<PathBuf> = fs::read_dir(dir)
        .map(|rd| rd.flatten().map(|entry| entry.path()).collect())
        .unwrap_or_default();
    entries.sort();
    entries
}
