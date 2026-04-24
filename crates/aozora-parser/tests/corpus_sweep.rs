//! Corpus sweep — property-based invariants over any aozora-format text.
//!
//! Runs when `AFM_CORPUS_ROOT` points at a directory. Without the variable
//! set the test runtime-skips with a diagnostic message (and passes), so
//! machines and CI jobs without a corpus configured stay green.
//!
//! Five invariants are checked:
//!
//! - **I1 — no panic.** The parser must never `panic!()` for any input.
//!   Failures here are hard: the sweep fails with a diagnostic listing the
//!   offending corpus labels. `parse` and `render_to_string` are wrapped in
//!   `panic::catch_unwind` to recover across iterations.
//! - **I2 — no unconsumed `［＃` markers (HARD GATE).** After rendering
//!   to HTML and stripping the `afm-annotation` wrapper spans, no bare
//!   `［＃` may remain. An `AFM_CORPUS_LEAK_BUDGET` env var can override
//!   the budget to a non-zero int for staging a sweep over a dirty
//!   corpus before promoting the fix back into the classifier.
//! - **I3 — `serialize ∘ parse` is a round-trip fixed point (HARD
//!   GATE).** One parse+serialize canonicalises the afm source; a
//!   second parse+serialize of that output must be byte-identical.
//!   Catches classifier / serializer drift where the round trip
//!   oscillates or drops bytes. `AFM_CORPUS_I3_BUDGET` mirrors the
//!   I2/I4 escape hatch; zero by default.
//! - **I4 — rendered HTML is tag-balanced (HARD GATE).** Every render
//!   must produce HTML whose open/close tags balance per the minimal
//!   validator at `tests/common/mod.rs`. Catches renderer bugs that
//!   don't leak `［＃` but still produce malformed markup (e.g. a
//!   `<div>` open without its `</div>` close). `AFM_CORPUS_I4_BUDGET`
//!   overrides the budget for staging fixes on a dirty corpus.
//! - **I5 — SJIS decode stable.** Every `.txt` file in an Aozora-format
//!   corpus should be valid Shift_JIS. Decode failures are logged and
//!   counted but don't abort the sweep (a corpus may legitimately contain
//!   non-SJIS files, e.g. README files walked in by a loose configuration).

use core::fmt;
use std::collections::BTreeMap;
use std::env;
use std::panic::{self, AssertUnwindSafe};

use afm_corpus::{CorpusError, from_env};
use afm_encoding::decode_sjis;
use afm_parser::html::render_to_string;
use afm_parser::test_support::{
    check_annotation_wrapper_shape, check_css_class_contract, check_heading_integrity,
    check_no_sentinel_leak, check_no_xss_marker, check_well_formed, strip_annotation_wrappers,
};
use afm_parser::{Options, parse, serialize};
use comrak::Arena;

// ---------------------------------------------------------------------------
// Tier name registry
//
// Every invariant has a `&'static str` key in [`SweepStats::tiers`]. Keeping
// them as string constants rather than inline literals makes adding a new
// invariant a single-line addition (one constant + one `stats.record_in(...)`
// call site) and keeps the stats-struct field count from ballooning as I6-I10
// land.
// ---------------------------------------------------------------------------

const TIER_I1_PANIC: &str = "I1_panic";
const TIER_I2_LEAKED: &str = "I2_leaked";
const TIER_I3_ROUND_TRIP: &str = "I3_round_trip";
const TIER_I4_MALFORMED: &str = "I4_malformed";
const TIER_I5_DECODE: &str = "I5_decode";
// I6-I10 added in Phase 4 of the negative-test enhancement. Landed
// as report-only — stderr-printed counts only, no hard assertion —
// until the 17 k corpus observation run records their baselines in
// ADR-0007. Promotion to hard gate happens in a follow-up commit via
// `AFM_CORPUS_I6_BUDGET` (etc.) defaulting to the observed count.
const TIER_I6_SENTINEL_LEAK: &str = "I6_sentinel";
const TIER_I7_UNKNOWN_CSS_CLASS: &str = "I7_css_class";
const TIER_I8_XSS_MARKER: &str = "I8_xss";
const TIER_I9_WRAPPER_SHAPE: &str = "I9_wrapper_shape";
const TIER_I10_HEADING_INTEGRITY: &str = "I10_heading";

/// Order used by [`SweepStats`] Display so a human reading the output sees
/// invariants in I1 → In sequence rather than [`HashMap`] hash order.
const TIER_REPORT_ORDER: &[&str] = &[
    TIER_I1_PANIC,
    TIER_I2_LEAKED,
    TIER_I3_ROUND_TRIP,
    TIER_I4_MALFORMED,
    TIER_I5_DECODE,
    TIER_I6_SENTINEL_LEAK,
    TIER_I7_UNKNOWN_CSS_CLASS,
    TIER_I8_XSS_MARKER,
    TIER_I9_WRAPPER_SHAPE,
    TIER_I10_HEADING_INTEGRITY,
];

/// Human-readable label for each tier, rendered by [`SweepStats`] Display.
fn tier_label(name: &str) -> &'static str {
    match name {
        "I1_panic" => "panics",
        "I2_leaked" => "with leaked ［＃ markers",
        "I3_round_trip" => "round-trip divergences",
        "I4_malformed" => "malformed HTML",
        "I5_decode" => "decode errors",
        "I6_sentinel" => "PUA sentinel leaks",
        "I7_css_class" => "unknown afm-* CSS classes",
        "I8_xss" => "XSS markers",
        "I9_wrapper_shape" => "malformed afm-annotation wrappers",
        "I10_heading" => "heading bodies with forbidden classes",
        _ => "other",
    }
}

/// Sweep entry point. The name is explicit so `cargo nextest run
/// corpus_sweep` matches just this one test, and so CI log lines are
/// self-describing when the test's diagnostic output is all users see.
#[test]
fn corpus_sweep_i1_through_i10() {
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
        sweep_one(result, &mut stats);
    }

    eprintln!("{stats}");
    assert_all_hard_gates(&stats);

    // I5 is still a report — corpora may legitimately contain non-SJIS
    // files (READMEs, bundled metadata); promoting to hard-gate would
    // force every sweep configuration to curate input shape first.
}

/// Assert every hard-gated invariant. Split out of the test entry
/// point so the entry stays under clippy's too-many-lines threshold
/// and so each gate reads as a single line.
fn assert_all_hard_gates(stats: &SweepStats) {
    // I1 is non-budgeted — a single panic is always a test failure.
    let panics = stats.count(TIER_I1_PANIC);
    assert_eq!(
        panics,
        0,
        "I1: parser panicked on {panics} corpus item(s); first {}: {:?}",
        stats.samples(TIER_I1_PANIC).len(),
        stats.samples(TIER_I1_PANIC),
    );

    // Budgeted hard gates. Each row: (tier, invariant id, env var,
    // default budget, failure description). Defaults are zero except
    // I6, which accommodates the known 56656 leak — see ADR-0007
    // amendment for rationale.
    let rows: &[(&str, &str, &str, usize, &str)] = &[
        (
            TIER_I2_LEAKED,
            "I2",
            "AFM_CORPUS_LEAK_BUDGET",
            0,
            "leaked ［＃ markers outside the afm-annotation wrapper",
        ),
        (
            TIER_I3_ROUND_TRIP,
            "I3",
            "AFM_CORPUS_I3_BUDGET",
            0,
            "diverged under a second `serialize ∘ parse`",
        ),
        (
            TIER_I4_MALFORMED,
            "I4",
            "AFM_CORPUS_I4_BUDGET",
            0,
            "rendered to malformed HTML",
        ),
        (
            TIER_I6_SENTINEL_LEAK,
            "I6",
            "AFM_CORPUS_I6_BUDGET",
            1,
            "leaked PUA sentinel (U+E001–U+E004) into rendered HTML",
        ),
        (
            TIER_I7_UNKNOWN_CSS_CLASS,
            "I7",
            "AFM_CORPUS_I7_BUDGET",
            0,
            "emitted unknown afm-* CSS class",
        ),
        (
            TIER_I8_XSS_MARKER,
            "I8",
            "AFM_CORPUS_I8_BUDGET",
            0,
            "leaked an XSS marker",
        ),
        (
            TIER_I9_WRAPPER_SHAPE,
            "I9",
            "AFM_CORPUS_I9_BUDGET",
            0,
            "produced malformed afm-annotation wrappers",
        ),
        (
            TIER_I10_HEADING_INTEGRITY,
            "I10",
            "AFM_CORPUS_I10_BUDGET",
            0,
            "emitted a heading carrying a forbidden class",
        ),
    ];
    for &(tier, id, env_name, default, description) in rows {
        let budget = env_budget_or(env_name, default);
        let count = stats.count(tier);
        assert!(
            count <= budget,
            "{id}: {count} corpus item(s) {description} \
             (budget = {budget}; first {}: {:?})",
            stats.samples(tier).len(),
            stats.samples(tier),
        );
    }
}

/// Generic env-var budget reader with a caller-supplied default.
///
/// Defaults to `0` (strict) for I2 / I3 / I4 / I7–I10; `1` for I6 to
/// accommodate the known U+E003 leak in
/// `spec/aozora/fixtures/56656/input.sjis.txt` — see ADR-0007
/// amendment. Malformed values (non-integer) fall back to `default`
/// with a stderr warning so a typo in CI config cannot silently
/// paper over a real regression.
fn env_budget_or(name: &str, default: usize) -> usize {
    env::var(name).map_or(default, |s| {
        s.trim().parse::<usize>().unwrap_or_else(|_| {
            eprintln!(
                "corpus sweep: {name}={s:?} is not a non-negative integer — \
                 defaulting to {default} (strict)."
            );
            default
        })
    })
}

/// Drive one corpus item through the invariant suite, accumulating
/// results into `stats`. Extracted from the sweep entry point to
/// keep the latter under clippy's too-many-lines threshold and so
/// each invariant's branch stays visually close to its siblings.
fn sweep_one(result: Result<afm_corpus::CorpusItem, CorpusError>, stats: &mut SweepStats) {
    let item = match result {
        Ok(item) => item,
        Err(CorpusError::Io { path, source }) => {
            eprintln!(
                "corpus sweep: skipping unreadable item {}: {}",
                path.display(),
                source
            );
            stats.io_skips += 1;
            return;
        }
        Err(other) => {
            eprintln!("corpus sweep: skipping item after unexpected error: {other}");
            stats.io_skips += 1;
            return;
        }
    };

    // I5 — Shift_JIS decode. Non-SJIS files are reported and skipped.
    let text = match decode_sjis(&item.bytes) {
        Ok(text) => text,
        Err(err) => {
            stats.record_in(TIER_I5_DECODE, format!("{}: {err}", item.label));
            return;
        }
    };

    // I1 — render_to_string must not panic. `catch_unwind` bounds a
    // single item's panic so the rest of the corpus keeps sweeping.
    let Ok(html) = panic::catch_unwind(AssertUnwindSafe(|| render_to_string(&text))) else {
        stats.record_in(TIER_I1_PANIC, item.label);
        return;
    };

    // I2 — no bare `［＃` outside afm-annotation wrappers.
    let leaked = count_leaked_markers(&html);
    if leaked > 0 {
        stats.record_in(TIER_I2_LEAKED, format!("{}: {leaked} leak(s)", item.label));
    }

    // I4 — rendered HTML must be tag-balanced.
    let wf_errors = check_well_formed(&html);
    if !wf_errors.is_empty() {
        let first = wf_errors
            .first()
            .map_or_else(|| "?".to_owned(), ToString::to_string);
        stats.record_in(
            TIER_I4_MALFORMED,
            format!("{}: {first} ({} total)", item.label, wf_errors.len()),
        );
    }

    // I3 — `serialize ∘ parse` must be a round-trip fixed point.
    check_round_trip(&text, &item.label, stats);

    // I6-I10 — report-only invariants. Every predicate returns
    // `Result<(), Violation>`; a failure is recorded to the tier
    // counter for later promotion to a hard gate. Until then, stderr
    // reports the count so developers can observe drift.
    if let Err(v) = check_no_sentinel_leak(&html) {
        stats.record_in(TIER_I6_SENTINEL_LEAK, format!("{}: {v}", item.label));
    }
    if let Err(v) = check_css_class_contract(&html) {
        stats.record_in(TIER_I7_UNKNOWN_CSS_CLASS, format!("{}: {v}", item.label));
    }
    if let Err(v) = check_no_xss_marker(&html) {
        stats.record_in(TIER_I8_XSS_MARKER, format!("{}: {v}", item.label));
    }
    if let Err(v) = check_annotation_wrapper_shape(&html) {
        stats.record_in(TIER_I9_WRAPPER_SHAPE, format!("{}: {v}", item.label));
    }
    if let Err(v) = check_heading_integrity(&html) {
        stats.record_in(TIER_I10_HEADING_INTEGRITY, format!("{}: {v}", item.label));
    }

    stats.ok += 1;
}

/// Run the I3 fixed-point check on `text` and accumulate any
/// divergence / panic into `stats`. Extracted from `sweep_one` to
/// keep that function under clippy's too-many-lines threshold.
/// `catch_unwind` isolates a panicky round-trip the same way I1
/// isolates a panicky render.
fn check_round_trip(text: &str, label: &str, stats: &mut SweepStats) {
    let round_trip_result = panic::catch_unwind(AssertUnwindSafe(|| {
        let opts = Options::afm_default();
        let arena_a = Arena::new();
        let first = serialize(&parse(&arena_a, text, &opts));
        let arena_b = Arena::new();
        let second = serialize(&parse(&arena_b, &first, &opts));
        (first, second)
    }));
    match round_trip_result {
        Ok((first, second)) if first != second => {
            // Record only the diff length so a differing 2-MB
            // item doesn't blow up the report.
            let diff_bytes = second.len().abs_diff(first.len());
            stats.record_in(TIER_I3_ROUND_TRIP, format!("{label}: diff {diff_bytes}B"));
        }
        Err(_) => {
            stats.record_in(TIER_I1_PANIC, format!("{label} [round-trip]"));
        }
        _ => {}
    }
}

const MAX_SAMPLES: usize = 10;

/// Per-invariant counter + bounded sample log.
///
/// Each tier records its own count and up to [`MAX_SAMPLES`] diagnostic
/// strings. Extracted from the per-invariant field soup that `SweepStats`
/// used to carry so adding a new invariant is a one-line tier insertion
/// rather than three parallel fields plus matching Display arms.
#[derive(Debug, Default)]
struct Tier {
    count: usize,
    samples: Vec<String>,
}

impl Tier {
    fn record(&mut self, sample: impl Into<String>) {
        if self.samples.len() < MAX_SAMPLES {
            self.samples.push(sample.into());
        }
        self.count += 1;
    }
}

#[derive(Debug, Default)]
struct SweepStats {
    ok: usize,
    io_skips: usize,
    /// Invariant -> Tier. `BTreeMap` gives deterministic iteration for
    /// testing without extra plumbing. Keys are the `TIER_*` constants
    /// declared at the top of this file.
    tiers: BTreeMap<&'static str, Tier>,
}

impl SweepStats {
    /// Record one occurrence of `tier` with the given diagnostic sample.
    fn record_in(&mut self, tier: &'static str, sample: impl Into<String>) {
        self.tiers.entry(tier).or_default().record(sample);
    }

    fn count(&self, tier: &'static str) -> usize {
        self.tiers.get(tier).map_or(0, |t| t.count)
    }

    fn samples(&self, tier: &'static str) -> &[String] {
        const EMPTY: &[String] = &[];
        self.tiers.get(tier).map_or(EMPTY, |t| t.samples.as_slice())
    }
}

impl fmt::Display for SweepStats {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "corpus sweep summary: {ok} passed", ok = self.ok)?;
        for &name in TIER_REPORT_ORDER {
            let n = self.count(name);
            if n > 0 {
                write!(f, ", {n} {}", tier_label(name))?;
            }
        }
        writeln!(f, ", {io} I/O skips", io = self.io_skips)?;
        for &name in TIER_REPORT_ORDER {
            let samples = self.samples(name);
            if !samples.is_empty() {
                writeln!(f, "  first {} samples: {samples:?}", tier_label(name))?;
            }
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
