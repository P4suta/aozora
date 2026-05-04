//! Permanent regression cases for the `decode_sjis` cargo-fuzz harness
//! under `crates/aozora-encoding/fuzz/`.
//!
//! Workflow lives in the top-level `tests/fuzz_regressions/README.md`.

use std::fs;
use std::panic;
use std::path::{Path, PathBuf};
use std::str;

use aozora_encoding::decode_sjis;

#[test]
fn decode_sjis_regressions_replay_cleanly() {
    replay_each("decode_sjis", |bytes| {
        let Ok(text) = decode_sjis(bytes) else {
            return;
        };
        assert!(
            str::from_utf8(text.as_bytes()).is_ok(),
            "decode_sjis returned non-UTF-8 String",
        );
    });
}

fn replay_each(target: &str, assert_one: impl Fn(&[u8])) {
    let dir = regression_dir(target);
    let artifacts = collect_artifacts(&dir);
    if artifacts.is_empty() {
        return;
    }
    for path in artifacts {
        let path_display = path.display();
        let bytes = fs::read(&path)
            .unwrap_or_else(|e| panic!("failed to read regression artifact {path_display}: {e}"));
        let label = path.display().to_string();
        // The closure works on raw bytes here — `decode_sjis` accepts
        // arbitrary input and skips internally, mirroring the fuzz
        // target.
        let result = panic::catch_unwind(panic::AssertUnwindSafe(|| assert_one(&bytes)));
        if let Err(payload) = result {
            let message = payload
                .downcast_ref::<String>()
                .cloned()
                .or_else(|| {
                    payload
                        .downcast_ref::<&'static str>()
                        .map(|s| (*s).to_owned())
                })
                .unwrap_or_else(|| "<non-string panic payload>".to_owned());
            panic!("regression artifact {label} still crashes:\n{message}\n  bytes = {bytes:?}");
        }
    }
}

fn regression_dir(target: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fuzz_regressions")
        .join(target)
}

fn collect_artifacts(dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut out: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.is_file()
                && path
                    .extension()
                    .is_none_or(|ext| ext != "txt" && ext != "md")
        })
        .collect();
    out.sort();
    out
}
