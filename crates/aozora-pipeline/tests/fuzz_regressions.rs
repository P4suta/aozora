//! Permanent regression cases lifted from cargo-fuzz artifacts under
//! `crates/aozora-pipeline/fuzz/`.
//!
//! Whenever `just fuzz-deep aozora-pipeline <target>` (or `fuzz-quick`)
//! flags an input, run the artifact through `just fuzz-triage
//! aozora-pipeline <target>` to see the panic message, fix the
//! underlying issue, then call `just fuzz-promote aozora-pipeline
//! <target> <artifact>` to lift the input into
//! `tests/fuzz_regressions/<target>/`. From that point on, every
//! `just test` run replays the fixed-up case — no nightly toolchain
//! required.
//!
//! ## Layout
//!
//! ```text
//! tests/fuzz_regressions/
//!   lex_into_arena/
//!     <hash>             ── raw byte payload, fed verbatim
//!     <hash>.expect.txt  ── (optional) panic snippet, archaeology only
//! ```

use std::fs;
use std::panic;
use std::path::{Path, PathBuf};
use std::str;

use aozora_pipeline::lex_into_arena;
use aozora_syntax::borrowed::Arena;

#[test]
fn lex_into_arena_regressions_replay_cleanly() {
    replay_each("lex_into_arena", |src| {
        let arena = Arena::new();
        let out = lex_into_arena(src, &arena);
        for diag in &out.diagnostics {
            let span = diag.span();
            assert!(
                span.start <= span.end,
                "diagnostic span {span:?} has start > end",
            );
        }
    });
}

/// Walk every artifact under `tests/fuzz_regressions/<target>/` and
/// hand the decoded UTF-8 source to `assert_one`. Panics from the
/// closure are caught and re-raised with the artifact path prefix so
/// a failure points straight at the file on disk.
fn replay_each(target: &str, assert_one: impl Fn(&str)) {
    let dir = regression_dir(target);
    let artifacts = collect_artifacts(&dir);
    if artifacts.is_empty() {
        // Empty regression set is the steady-state of a healthy
        // target. The test stays green so we can tell missing
        // tests/fuzz_regressions/ apart from "no crashes recorded".
        return;
    }
    for path in artifacts {
        let path_display = path.display();
        let bytes = fs::read(&path)
            .unwrap_or_else(|e| panic!("failed to read regression artifact {path_display}: {e}"));
        let Ok(src) = str::from_utf8(&bytes) else {
            // The fuzzer accepts raw bytes; the lex_into_arena target
            // skips invalid UTF-8 with `return`, so the corresponding
            // regression-test arm is the same skip.
            continue;
        };
        let label = path.display().to_string();
        let result = panic::catch_unwind(panic::AssertUnwindSafe(|| assert_one(src)));
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
