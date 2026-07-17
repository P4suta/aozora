//! End-to-end LSP check for the incremental sanitized rope (#237 Tier 2,
//! Mechanism B): drive the **real** [`OpenDocument`] (`apply_changes` →
//! `reparse_pending`) over the realistic aozora-bunko document shape
//! (BOM-prefixed, CRLF line endings, a header `----` decorative-rule line) and
//! assert that after **every** single edit the cache's stored sanitized rope and
//! published diagnostics are byte-identical to a from-scratch full parse — the
//! production proof that the rope splice + line-correspondence map + windowed
//! re-sanitize never desync from a full parse.

use std::sync::Arc;

use aozora::Diagnostic;
use aozora::unstable::sanitize::sanitize;
use proptest::prelude::*;
use proptest::sample::Index;
use proptest::test_runner::TestCaseError;
use ropey::Rope;

use super::parse_cache::ParseCache;
use super::state::OpenDocument;
use super::text_edit::ByteEdit;

/// Render diagnostics to a comparable `Vec<String>`.
fn diag_debug(ds: &[Diagnostic]) -> Vec<String> {
    ds.iter().map(|d| format!("{d:?}")).collect()
}

/// Drain the pending edits and assert the incremental result is byte-identical
/// to a from-scratch parse of the current raw text — across the published
/// diagnostics and the cache's stored sanitized rope.
fn assert_step_matches_full(state: &OpenDocument) {
    let (raw, diags, _ver) = state.reparse_pending();
    let text = raw.to_string();

    // Full parse of the same raw text via a fresh cache.
    let mut fresh = ParseCache::default();
    let (want_diags, _) = fresh.reparse(&text);
    assert_eq!(
        diag_debug(&diags),
        diag_debug(&want_diags),
        "diagnostics diverged from a full parse of {text:?}",
    );

    // The stored sanitized rope must equal a full `sanitize` of the raw text.
    let spliced_san = state
        .with_parse_cache(|c| c.sanitized().map(ToString::to_string))
        .expect("a parsed document stores a sanitized buffer");
    assert_eq!(
        spliced_san,
        sanitize(&text).text.as_ref(),
        "stored sanitized rope diverged from a full sanitize of {text:?}",
    );
}

/// Cumulative incremental cache-hit total — non-zero once an edit has taken the
/// rope splice with node reuse.
fn cache_hits(state: &OpenDocument) -> u64 {
    state.metrics.snapshot().cache_hit_total
}

/// Insert `ins` immediately before the first occurrence of `needle` in the
/// document's current text, in current (doc) coordinates.
fn insert_before(state: &Arc<OpenDocument>, needle: &str, ins: &str) {
    let text = state.snapshot().doc_text().to_string();
    let at = text
        .find(needle)
        .unwrap_or_else(|| panic!("needle {needle:?} present in {text:?}"));
    state
        .apply_changes(&[ByteEdit::new(at..at, ins.to_owned())])
        .expect("valid single edit");
}

#[test]
fn bom_crlf_rule_document_stays_byte_identical_across_edits() {
    // Realistic aozora-bunko front matter: BOM, CRLF, a 10-char decorative rule
    // that sanitize isolates with a blank line, a leading ruby, then body prose.
    let src = "\u{FEFF}｜青空《あおぞら》\r\n\
               ----------\r\n\r\n\
               ほんぶんのだんらくいち。\r\n\r\n\
               だんらくに、つづきます。\r\n";
    let state = OpenDocument::new(src.to_owned());
    // Seed the cache with the initial full parse.
    assert_step_matches_full(&state);

    // A sequence of interior edits in the body paragraphs (away from the rule),
    // each of which must take the incremental rope splice and stay byte-identical.
    insert_before(&state, "だんらくいち", "もうひとつ");
    assert_step_matches_full(&state);
    let hits1 = cache_hits(&state);
    assert!(
        hits1 > 0,
        "an interior body edit past the isolated rule must reuse the leading ruby \
         via the rope splice (cumulative hits = {hits1})",
    );

    insert_before(&state, "つづきます", "しっかり");
    assert_step_matches_full(&state);
    let hits2 = cache_hits(&state);
    assert!(
        hits2 >= hits1,
        "cumulative hits never regress: {hits2} < {hits1}"
    );

    // An edit that grows the body — still byte-identical.
    insert_before(&state, "ほんぶん", "じょしょうの");
    assert_step_matches_full(&state);
}

#[test]
fn rule_toggling_edit_declines_but_stays_correct() {
    // Editing the dash run so it crosses the 10-char decorative-rule threshold
    // creates / destroys an isolation blank line; the trigger gate must decline
    // (full parse), and the result must still be byte-identical.
    let src = "\u{FEFF}まえがき。\r\n---------\r\n\r\nほんぶんです。\r\n";
    let state = OpenDocument::new(src.to_owned());
    assert_step_matches_full(&state);

    // Grow the 9-dash line to 10 dashes — now a decorative rule.
    insert_before(&state, "---------", "-");
    assert_step_matches_full(&state);

    // And a normal body edit afterwards still matches a full parse.
    insert_before(&state, "ほんぶん", "その");
    assert_step_matches_full(&state);
}

// ---- randomized edit sequences (per-edit byte-identity invariant) -----------

/// As [`assert_step_matches_full`], but `prop_assert`-based so a divergence
/// shrinks cleanly instead of aborting the whole proptest. Drains the pending
/// edit, then pins the published diagnostics and the stored sanitized rope to a
/// fresh full parse of the resulting raw text (handling the doc-emptied case,
/// where no base is stored).
fn prop_step_matches_full(state: &OpenDocument) -> Result<(), TestCaseError> {
    let (raw, diags, _ver) = state.reparse_pending();
    let text = raw.to_string();

    let mut fresh = ParseCache::default();
    let (want_diags, _) = fresh.reparse(&text);
    prop_assert_eq!(
        diag_debug(&diags),
        diag_debug(&want_diags),
        "diagnostics diverged for {:?}",
        text,
    );

    let spliced = state.with_parse_cache(|c| c.sanitized().map(ToString::to_string));
    let want_san = sanitize(&text).text;
    match spliced {
        Some(s) => prop_assert_eq!(s, want_san.as_ref(), "sanitized diverged for {:?}", text),
        None => prop_assert!(
            want_san.is_empty(),
            "no stored base but sanitize is non-empty for {:?}",
            text,
        ),
    }
    Ok(())
}

/// Insert-text strategy biased toward the incremental triggers (`〔〕` accent
/// brackets, the `` ` ``/`'` accent markers, decorative-rule runes, the `［`/`]`
/// directive brackets, CRLF / lone CR / BOM / a raw PUA sentinel) **and** plain
/// interior prose (the accept case), so the sequence exercises both the decline
/// and the fast path.
fn ins_strategy() -> impl Strategy<Value = String> {
    prop_oneof![
        8 => "[\\p{Hiragana}\\p{Katakana}a-z、。]{1,4}",
        1 => Just("〔".to_owned()),
        1 => Just("〕".to_owned()),
        1 => Just("`".to_owned()),
        1 => Just("'".to_owned()),
        1 => Just("-".to_owned()),
        1 => Just("=".to_owned()),
        1 => Just("_".to_owned()),
        1 => Just("［".to_owned()),
        1 => Just("］".to_owned()),
        1 => Just("\r\n".to_owned()),
        1 => Just("\n".to_owned()),
        1 => Just("\u{FEFF}".to_owned()),
        1 => Just("\r".to_owned()),
        1 => Just("\u{E001}".to_owned()),
    ]
}

proptest! {
    // Each step does a full reference parse + sanitize, so cap the case count.
    #![proptest_config(ProptestConfig::with_cases(48))]

    /// Drive the **real** [`OpenDocument`] over a random single-edit sequence on
    /// the realistic aozora-bunko shape and assert the per-edit byte-identity
    /// invariant (stored sanitized rope + published diagnostics == a fresh full
    /// parse) after **every** edit. The generator biases toward triggers and
    /// plain prose so accepts and declines interleave. Branch coverage is **not**
    /// asserted here (shrink would minimise a branch away and report a false
    /// failure) — it lives in the deterministic loop below.
    #[test]
    fn prop_random_edits_stay_byte_identical(
        recipes in prop::collection::vec((any::<Index>(), any::<u8>(), ins_strategy()), 1..12),
    ) {
        let src = "\u{FEFF}｜青空《あおぞら》\r\n----------\r\n\r\n\
                   ほんぶんいち。\r\n\r\nつぎのだんらく。\r\n";
        let state = OpenDocument::new(src.to_owned());
        prop_step_matches_full(&state)?; // initial full parse

        for (pos, del, ins) in recipes {
            let text = state.snapshot().doc_text().to_string();
            if text.is_empty() {
                let _applied = state.apply_changes(&[ByteEdit::new(0..0, "あ".to_owned())]);
                prop_step_matches_full(&state)?;
                continue;
            }
            // A char-boundary range `[at, end)`: `at` snapped up, `end` advanced a
            // few chars (so inserts, deletes, and replaces all occur), clamped.
            let mut at = pos.index(text.len());
            while at < text.len() && !text.is_char_boundary(at) {
                at += 1;
            }
            let mut end = at;
            for _ in 0..(del % 3) {
                if end >= text.len() {
                    break;
                }
                end += 1;
                while end < text.len() && !text.is_char_boundary(end) {
                    end += 1;
                }
            }
            // `apply_changes` returns `None` for an edit the buffer rejects; either
            // way the next reparse is byte-identical to a full parse, so we do not
            // assert the edit applied (that would false-fail under shrink).
            let _applied = state.apply_changes(&[ByteEdit::new(at..end, ins)]);
            prop_step_matches_full(&state)?;
        }
    }
}

// ---- deterministic branch coverage (accept + decline both fire) -------------

/// Insert `ins` immediately before the first `needle` in `text`; returns the
/// post-edit text and the edit in `text` (pre-edit) coordinates.
fn insert_before_text(text: &str, needle: &str, ins: &str) -> (String, ByteEdit) {
    let at = text
        .find(needle)
        .unwrap_or_else(|| panic!("needle {needle:?} present in {text:?}"));
    let mut new_text = String::with_capacity(text.len() + ins.len());
    new_text.push_str(&text[..at]);
    new_text.push_str(ins);
    new_text.push_str(&text[at..]);
    (new_text, ByteEdit::new(at..at, ins.to_owned()))
}

/// Assert the cache's incremental result for `new_text` is byte-identical to a
/// fresh full parse, across both the diagnostics and the stored sanitized rope.
fn assert_byte_identical(cache: &ParseCache, diags: &[Diagnostic], new_text: &str) {
    let mut fresh = ParseCache::default();
    let (want, _) = fresh.reparse(new_text);
    assert_eq!(
        diag_debug(diags),
        diag_debug(&want),
        "diagnostics: {new_text:?}"
    );
    let spliced = cache.sanitized().map(ToString::to_string);
    assert_eq!(
        spliced.as_deref(),
        Some(sanitize(new_text).text.as_ref()),
        "sanitized: {new_text:?}",
    );
}

/// A deterministic run that pins **both** incremental branches firing, so they
/// can never silently rot to "always full-parse" or "always splice". Drives a
/// `ParseCache` (whose per-call `ReparseStats` expose the branch taken, unlike
/// the metrics-only `OpenDocument` surface) over:
///
/// 1. **> `MAX_SPLICES_BEFORE_FULL` (64) consecutive plain accepts** — every step
///    splices with node reuse (`cache_hits > 0`), *including* the steps where the
///    cache crosses the bound: the #249 bound now compacts the maintained
///    `PieceSeq` in place (`PieceSeq::compact`, no re-parse) rather than forcing a
///    full re-seed, so the run never falls off the incremental fast path;
/// 2. **an explicit trigger decline** — a line-terminator insert (T1) full-parses
///    regardless of the splice counter.
///
/// Every step is also asserted byte-identical to a full parse. Branch coverage is
/// deterministic (not proptest) precisely so shrink cannot minimise a branch away.
#[test]
fn branch_coverage_accept_and_decline_both_fire() {
    // Leading ruby so an accepted body splice reuses a node (cache_hits > 0);
    // LF-clean so the run stays on the incremental fast path.
    let mut text = "｜青空《あおぞら》\n\nいちばんめのほん。\n\nさいごのほん。\n".to_owned();
    let mut cache = ParseCache::default();
    drop(cache.reparse(&text));

    // 70 > 64 consecutive plain accepts: crossing the compaction bound must keep
    // fast-pathing (compaction, not a forced full re-parse), so *every* step
    // reuses nodes and stays byte-identical.
    let mut accepts = 0u32;
    for _ in 0..70u32 {
        let (new_text, edit) = insert_before_text(&text, "さいごのほん", "も");
        let (diags, stats) = cache.reparse_incremental(&Rope::from(new_text.as_str()), &[edit]);
        assert_byte_identical(&cache, &diags, &new_text);
        assert!(
            stats.cache_hits > 0,
            "every plain accept past the compaction bound still fast-paths: {stats:?}",
        );
        accepts += 1;
        text = new_text;
    }
    assert_eq!(
        accepts, 70,
        "every plain insert is an accept (compaction, no re-seed)"
    );

    // An explicit trigger decline: a line-terminator (T1) insert declines to a
    // full parse no matter the splice counter. The `cache_hits == 0` assert
    // below *is* the decline-branch check (a fast-path accept always reports a
    // non-zero reuse count for this reused-prefix doc).
    let (new_text, edit) = insert_before_text(&text, "いちばんめ", "\n");
    let (diags, stats) = cache.reparse_incremental(&Rope::from(new_text.as_str()), &[edit]);
    assert_byte_identical(&cache, &diags, &new_text);
    assert_eq!(stats.cache_hits, 0, "a line-terminator insert must decline");
}
