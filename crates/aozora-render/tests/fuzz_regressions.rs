//! Permanent regression cases for the cargo-fuzz harnesses under
//! `crates/aozora-render/fuzz/`.
//!
//! See the top-level `tests/fuzz_regressions/README.md` for the
//! triage / promote workflow. Every promoted artifact replays here
//! on `just test` without a nightly toolchain.

use std::fs;
use std::panic;
use std::path::{Path, PathBuf};
use std::str;

use aozora_pipeline::lex_into_arena;
use aozora_render::html::render_to_string;
use aozora_render::serialize::serialize;
use aozora_syntax::borrowed::Arena;

const PUA_SENTINELS: [char; 4] = ['\u{E001}', '\u{E002}', '\u{E003}', '\u{E004}'];

#[test]
fn render_html_regressions_replay_cleanly() {
    replay_each("render_html", |src| {
        let arena = Arena::new();
        let lex_out = lex_into_arena(src, &arena);
        let html = render_to_string(&lex_out);
        for sentinel in PUA_SENTINELS {
            assert!(
                !html.contains(sentinel),
                "PUA sentinel {sentinel:?} leaked into rendered HTML\n  html = {html:?}",
            );
        }
    });
}

#[test]
fn serialize_round_trip_regressions_replay_cleanly() {
    replay_each("serialize_round_trip", |src| {
        let arena1 = Arena::new();
        let lex1 = lex_into_arena(src, &arena1);
        let first = serialize(&lex1);
        let arena2 = Arena::new();
        let lex2 = lex_into_arena(&first, &arena2);
        let second = serialize(&lex2);
        assert!(
            first == second,
            "I3 fixed-point broken\n  first  = {first:?}\n  second = {second:?}",
        );
    });
}

fn replay_each(target: &str, assert_one: impl Fn(&str)) {
    let dir = regression_dir(target);
    let artifacts = collect_artifacts(&dir);
    if artifacts.is_empty() {
        return;
    }
    for path in artifacts {
        let path_display = path.display();
        let bytes = fs::read(&path)
            .unwrap_or_else(|e| panic!("failed to read regression artifact {path_display}: {e}"));
        let Ok(src) = str::from_utf8(&bytes) else {
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
