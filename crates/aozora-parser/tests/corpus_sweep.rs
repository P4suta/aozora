//! Corpus sweep — property-based invariants over any aozora-format text.
//!
//! Runs when `AFM_CORPUS_ROOT` points at a directory. Without the variable
//! set the test runtime-skips with a diagnostic message (and passes), so
//! machines and CI jobs without a corpus configured stay green.
//!
//! Invariants checked in this harness (M2-S3):
//!
//! - **I1 — no panic.** The parser must never `panic!()` for any input.
//!   Failures here are hard: the sweep fails with a diagnostic listing the
//!   offending corpus labels. `parse` and `render_to_string` are wrapped in
//!   `panic::catch_unwind` to recover across iterations.
//! - **I2 — no unconsumed `［＃` markers (HARD GATE).** After rendering
//!   to HTML and stripping the `afm-annotation` wrapper spans, no bare
//!   `［＃` may remain. Post-F5 (paired container wrap) + G1 (gaiji
//!   table) the recogniser set is comprehensive enough to make this
//!   an enforcement line. An `AFM_CORPUS_LEAK_BUDGET` env var can
//!   override the budget to a non-zero int for staging a sweep over a
//!   dirty corpus before promoting the fix back into the classifier.
//! - **I5 — SJIS decode stable.** Every `.txt` file in a Aozora-format
//!   corpus should be valid Shift_JIS. Decode failures are logged and
//!   counted but don't abort the sweep (a corpus may legitimately contain
//!   non-SJIS files, e.g. README files walked in by a loose configuration).
//!
//! Invariants I3 (round-trip) and I4 (HTML well-formedness) are deferred
//! to M2-S6 and M2-S4 respectively, pending a serializer and an
//! html5ever-based validator.

use core::fmt;
use std::env;
use std::panic::{self, AssertUnwindSafe};

use afm_corpus::{CorpusError, from_env};
use afm_encoding::decode_sjis;
use afm_parser::html::render_to_string;
use afm_parser::test_support::strip_annotation_wrappers;

/// Sweep entry point. The name is explicit so `cargo nextest run
/// corpus_sweep` matches just this one test, and so CI log lines are
/// self-describing when the test's diagnostic output is all users see.
#[test]
fn corpus_sweep_i1_no_panic_i2_report_i5_report() {
    let Some(corpus) = from_env() else {
        eprintln!(
            "corpus sweep: AFM_CORPUS_ROOT not set; skipping (pass). \
             Set AFM_CORPUS_ROOT to a directory of aozora-format .txt \
             files to enable the sweep."
        );
        return;
    };

    eprintln!("corpus sweep: provenance = {}", corpus.provenance());

    let mut stats = SweepStats::default();

    for result in corpus.iter() {
        let item = match result {
            Ok(item) => item,
            Err(CorpusError::Io { path, source }) => {
                eprintln!(
                    "corpus sweep: skipping unreadable item {}: {}",
                    path.display(),
                    source
                );
                stats.io_skips += 1;
                continue;
            }
            Err(other) => {
                eprintln!("corpus sweep: skipping item after unexpected error: {other}");
                stats.io_skips += 1;
                continue;
            }
        };

        // I5 — Shift_JIS decode. Non-SJIS files are reported and skipped.
        let text = match decode_sjis(&item.bytes) {
            Ok(text) => text,
            Err(err) => {
                if stats.decode_error_samples.len() < MAX_SAMPLES {
                    stats
                        .decode_error_samples
                        .push(format!("{}: {err}", item.label));
                }
                stats.decode_errors += 1;
                continue;
            }
        };

        // I1 — render_to_string must not panic. We wrap the renderer (which
        // drives both parse and render inside a single call) in catch_unwind
        // so that one panicking input does not abort the sweep over the
        // rest of the corpus.
        let Ok(html) = panic::catch_unwind(AssertUnwindSafe(|| render_to_string(&text))) else {
            if stats.panic_samples.len() < MAX_SAMPLES {
                stats.panic_samples.push(item.label.clone());
            }
            stats.panics += 1;
            continue;
        };

        // I2 — no bare `［＃` outside afm-annotation wrappers.
        let leaked = count_leaked_markers(&html);
        if leaked > 0 {
            if stats.leaked_marker_samples.len() < MAX_SAMPLES {
                stats
                    .leaked_marker_samples
                    .push(format!("{}: {leaked} leak(s)", item.label));
            }
            stats.leaked_markers += 1;
        }

        stats.ok += 1;
    }

    eprintln!("{stats}");

    // I1 is the hard gate. Fail loudly with the first handful of offending
    // labels so the developer has immediate pointers for reproduction.
    assert_eq!(
        stats.panics,
        0,
        "parser panicked on {} corpus item(s); first {}: {:?}",
        stats.panics,
        stats.panic_samples.len(),
        stats.panic_samples,
    );

    // I2 — hard gate as of G3. `AFM_CORPUS_LEAK_BUDGET` allows a
    // non-zero budget while preparing a classifier fix, so sweep runs
    // over an unclean corpus don't block unrelated work.
    let budget = leak_budget_from_env();
    assert!(
        stats.leaked_markers <= budget,
        "I2: {} corpus item(s) leaked ［＃ markers outside the afm-annotation wrapper \
         (budget = {budget}; first {}: {:?})",
        stats.leaked_markers,
        stats.leaked_marker_samples.len(),
        stats.leaked_marker_samples,
    );

    // I5 is still a report — corpora may legitimately contain non-SJIS
    // files (READMEs, bundled metadata); promoting to hard-gate would
    // force every sweep configuration to curate input shape first.
}

/// Read `AFM_CORPUS_LEAK_BUDGET` and return the allowed number of
/// leaked ［＃ markers. Defaults to `0` (strict). Malformed values
/// produce a warning and fall back to `0` so a typo does not
/// silently paper over a real regression.
fn leak_budget_from_env() -> usize {
    env::var("AFM_CORPUS_LEAK_BUDGET").map_or(0, |s| {
        s.trim().parse::<usize>().unwrap_or_else(|_| {
            eprintln!(
                "corpus sweep: AFM_CORPUS_LEAK_BUDGET={s:?} is not a non-negative integer — \
                 defaulting to 0 (strict)."
            );
            0
        })
    })
}

const MAX_SAMPLES: usize = 10;

#[derive(Debug, Default)]
struct SweepStats {
    ok: usize,
    panics: usize,
    panic_samples: Vec<String>,
    leaked_markers: usize,
    leaked_marker_samples: Vec<String>,
    decode_errors: usize,
    decode_error_samples: Vec<String>,
    io_skips: usize,
}

impl fmt::Display for SweepStats {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(
            f,
            "corpus sweep summary: {ok} passed, {panics} panics, \
             {leaks} with leaked ［＃ markers, {decode} decode errors, {io} I/O skips",
            ok = self.ok,
            panics = self.panics,
            leaks = self.leaked_markers,
            decode = self.decode_errors,
            io = self.io_skips,
        )?;
        if !self.panic_samples.is_empty() {
            writeln!(f, "  first panic samples: {:?}", self.panic_samples)?;
        }
        if !self.leaked_marker_samples.is_empty() {
            writeln!(
                f,
                "  first leaked-marker samples: {:?}",
                self.leaked_marker_samples
            )?;
        }
        if !self.decode_error_samples.is_empty() {
            writeln!(
                f,
                "  first decode-error samples: {:?}",
                self.decode_error_samples
            )?;
        }
        Ok(())
    }
}

fn count_leaked_markers(html: &str) -> usize {
    // Unknown annotations are currently rendered inside `afm-annotation`
    // wrappers (a `<span hidden>` structure); strip those first, then
    // count bare occurrences of the open-annotation sequence. The helper
    // reused here is the same one `golden_56656.rs` uses for its Tier A
    // assertion, so we enforce the same definition of "leaked".
    let bare = strip_annotation_wrappers(html);
    bare.matches("［＃").count()
}
