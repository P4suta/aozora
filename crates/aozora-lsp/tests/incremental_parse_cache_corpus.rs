//! Corpus differential gate for the LSP `ParseCache` incremental path (#284).
//!
//! The engine-level gate (`aozora/tests/corpus_incremental_merge.rs`) feeds the
//! splice a `&str` sanitized buffer directly, so it never exercises
//! `ParseCache`'s own re-sanitize + common-prefix/suffix edit derivation
//! (`parse_cache.rs::try_incremental_resanitize`). This gate closes that gap: it
//! drives the *real* `ParseCache` over the corpus in the realistic aozora-bunko
//! shape (BOM-prefixed, CRLF line endings, header `----` decorative-rule lines),
//! applies one edit, and asserts the incremental diagnostics are byte-identical
//! to a from-scratch parse.
//!
//! It is the load-bearing proof that #284's coverage recovery — dropping the
//! whole-document `has_long_rule_line` decline so rule documents fast-path —
//! stays byte-identical end-to-end through the LSP layer.
//!
//! Gated on `AOZORA_CORPUS_ROOT`: with the variable unset the test returns
//! immediately (a vacuous pass), exactly like the other corpus sweeps.

#![cfg(feature = "internals")]

use aozora::Diagnostic;
use aozora::pipeline::lexer::sanitize::sanitize;
use aozora_encoding::decode_auto;
use aozora_lsp::internals::{ByteEdit, ParseCache};
use ropey::Rope;

/// Render diagnostics to a comparable `Vec<String>`.
fn diag_debug(ds: &[Diagnostic]) -> Vec<String> {
    ds.iter().map(|d| format!("{d:?}")).collect()
}

/// Build the real corpus shape from an LF fixed point: prepend a BOM and turn
/// every `\n` into `\r\n`. Returns `None` unless `sanitize` reproduces `lf`
/// exactly with no diagnostic (a true BOM-strip + CRLF→LF + already-isolated
/// fixed point), so the variant routes through the LSP's re-sanitize fast path.
fn crlf_variant(lf: &str) -> Option<String> {
    let mut s = String::with_capacity(lf.len() + lf.len() / 16 + 3);
    s.push('\u{FEFF}');
    for ch in lf.chars() {
        if ch == '\n' {
            s.push('\r');
        }
        s.push(ch);
    }
    let san = sanitize(&s);
    (san.diagnostics.is_empty() && &*san.text == lf).then_some(s)
}

/// A char-boundary byte offset at or after `from` (snapped up) that is not
/// adjacent to a line break — a realistic interior keystroke position.
fn interior_boundary_from(s: &str, from: usize) -> Option<usize> {
    for i in from.min(s.len())..s.len() {
        if !s.is_char_boundary(i) {
            continue;
        }
        let before = s.as_bytes().get(i.wrapping_sub(1)).copied();
        let at = s.as_bytes().get(i).copied();
        let edge = |b: Option<u8>| matches!(b, Some(b'\r' | b'\n'));
        if i > 0 && !edge(before) && !edge(at) {
            return Some(i);
        }
    }
    None
}

/// A char-boundary byte offset near the document midpoint that is not adjacent
/// to a line break — a realistic interior keystroke position.
fn mid_line_boundary(s: &str) -> Option<usize> {
    interior_boundary_from(s, s.len() / 2)
}

/// A byte offset *inside* the first `［＃…］` directive that closes on its own
/// line — the adversarial position that surfaced the #284 corpus divergence
/// (an edit inside `［＃割り注終わり］` re-pairs inline containers document-wide).
/// The engine's `inside_directive` guard must decline these, so they fall back
/// to a byte-identical full parse.
fn directive_interior(s: &str) -> Option<usize> {
    let open = s.find("［＃")?;
    let inner = open + "［＃".len();
    let rest = &s[inner..];
    let close = rest.find('］')?;
    if rest[..close].contains('\n') {
        return None; // directive does not close on this line
    }
    Some(inner)
}

/// If `s` contains a forward bouten/bsen directive `「X」に傍点／傍線`, return the
/// target `X` and the byte offset of the enclosing directive's `［`. Inserting a
/// copy of `X` earlier provokes `BoutenTargetAmbiguous` from the directive's
/// whole-prefix look-back — the adversarial edit a single `x` can never produce.
/// The `node_forbids_region_reuse` fix must decline it byte-identically (#284).
fn forward_bouten_target(s: &str) -> Option<(&str, usize)> {
    for marker in ["」に傍点", "」に傍線"] {
        let Some(m) = s.find(marker) else {
            continue;
        };
        let before = &s[..m];
        let open = before.rfind('「')?;
        let target = &s[open + '「'.len_utf8()..m];
        if target.is_empty() || target.contains(['\n', '\r', '［', '］', '《', '》', '「', '」'])
        {
            return None;
        }
        let bracket = before[..open].rfind('［')?;
        return Some((target, bracket));
    }
    None
}

/// Seed a fresh `ParseCache` with `crlf`, insert `ins` at raw offset `at`, and
/// assert the incremental diagnostics are byte-identical to a full parse of the
/// edited text. Returns whether the splice fast-pathed with node reuse.
fn assert_incremental_matches_full(crlf: &str, at: usize, ins: &str, label: &str) -> bool {
    let mut new_raw = String::with_capacity(crlf.len() + ins.len());
    new_raw.push_str(&crlf[..at]);
    new_raw.push_str(ins);
    new_raw.push_str(&crlf[at..]);
    let edit = ByteEdit::new(at..at, ins.to_owned());

    let mut cache = ParseCache::default();
    drop(cache.reparse(crlf));
    let (diags, stats) = cache.reparse_incremental(&Rope::from(new_raw.as_str()), &[edit]);

    let mut fresh = ParseCache::default();
    let (want, _) = fresh.reparse(&new_raw);
    assert_eq!(
        diag_debug(&diags),
        diag_debug(&want),
        "incremental diagnostics diverged from a full parse for {label} (edit at {at})",
    );
    // A full parse always reports zero reused nodes; a positive count is an
    // unambiguous "the incremental splice fired" signal.
    stats.cache_hits > 0
}

#[test]
fn parse_cache_incremental_equals_full_on_crlf_corpus() {
    let Some(corpus) = aozora_corpus::from_env() else {
        return; // AOZORA_CORPUS_ROOT unset → vacuous pass.
    };

    let mut count: u64 = 0;
    let mut fast_with_reuse: u64 = 0;
    let mut directive_edits: u64 = 0;
    let mut bouten_edits: u64 = 0;

    for item in corpus.iter().filter_map(Result::ok) {
        let Ok(text) = decode_auto(&item.bytes) else {
            continue;
        };
        if text.is_empty() {
            continue;
        }
        // Work from a sanitize fixed point so the cached base is stable, exactly
        // like the engine gate and the measure example.
        let san = aozora::Document::new(text.as_ref()).parse_owned().sanitized;
        if san.is_empty() {
            continue;
        }
        if aozora::Document::new(san.as_str()).parse_owned().sanitized != san {
            continue;
        }
        let Some(crlf) = crlf_variant(&san) else {
            continue;
        };
        let Some(mid) = mid_line_boundary(&crlf) else {
            continue;
        };

        // (1) Realistic interior keystroke.
        if assert_incremental_matches_full(&crlf, mid, "x", &item.label) {
            fast_with_reuse += 1;
        }
        // (2) Adversarial: an edit inside a `［＃…］` directive must stay
        // byte-identical (the `inside_directive` guard declines to a full parse).
        if let Some(inner) = directive_interior(&crlf) {
            assert_incremental_matches_full(&crlf, inner, "x", &item.label);
            directive_edits += 1;
        }
        // (3) Adversarial: duplicate a forward-bouten target in an earlier
        // region. A full parse emits BoutenTargetAmbiguous from the directive's
        // whole-prefix look-back; the `node_forbids_region_reuse` fix must keep
        // it byte-identical (decline), where a single 'x' could never provoke it.
        if let Some((target, bracket)) = forward_bouten_target(&crlf)
            && let Some(early) = mid_line_boundary(&crlf[..bracket])
        {
            assert_incremental_matches_full(&crlf, early, target, &item.label);
            bouten_edits += 1;
        }
        count += 1;
    }

    assert!(count > 0, "corpus yielded no usable documents");
    assert!(
        fast_with_reuse > 0,
        "no document fast-pathed with node reuse — the coverage recovery was \
         not exercised ({count} docs checked)",
    );
    assert!(
        directive_edits > 0,
        "no directive-interior edit was exercised ({count} docs checked)",
    );
    assert!(
        bouten_edits > 0,
        "no forward-bouten duplicate-target edit was exercised ({count} docs checked)",
    );
}

/// #237 Tier 2 (PR-2'): a **run of consecutive** incremental edits on one
/// `ParseCache` must keep `diagnostics()` byte-identical to a fresh full parse
/// after *every* edit — the load-bearing pin that the cache maintains the
/// multi-piece `PieceSeq` across edits, not just for a single splice. Each edit
/// lands in a distinct region (ascending fractions of the growing document), so
/// successful splices accumulate live pieces rather than collapsing to one.
///
/// `>=2 consecutive splices with node reuse` (`cache_hits > 0`, which a
/// leading-ruby document guarantees on every edit past it) proves at least one
/// document fed an already-multi-piece sequence back into a splice and stayed
/// byte-identical — the property a single-edit gate cannot reach.
#[test]
fn parse_cache_multi_edit_run_equals_full() {
    let Some(corpus) = aozora_corpus::from_env() else {
        return; // AOZORA_CORPUS_ROOT unset → vacuous pass.
    };

    let mut count: u64 = 0;
    let mut multi_piece_docs: u64 = 0;

    for item in corpus.iter().filter_map(Result::ok) {
        let Ok(text) = decode_auto(&item.bytes) else {
            continue;
        };
        if text.is_empty() {
            continue;
        }
        let san = aozora::Document::new(text.as_ref()).parse_owned().sanitized;
        if san.is_empty() {
            continue;
        }
        if aozora::Document::new(san.as_str()).parse_owned().sanitized != san {
            continue;
        }
        let Some(crlf) = crlf_variant(&san) else {
            continue;
        };

        let mut cache = ParseCache::default();
        drop(cache.reparse(&crlf));

        // Apply several plain interior keystrokes in distinct regions, recomputed
        // against the growing text each step. After each, both the returned and
        // the stored `diagnostics()` must equal a fresh full parse of the new
        // text; `max_run` tracks the longest streak of reuse-bearing splices.
        let mut current = crlf.clone();
        let mut run = 0u32;
        let mut max_run = 0u32;
        for &(num, den) in &[(1usize, 5usize), (2, 5), (3, 5), (4, 5)] {
            let from = current.len() * num / den;
            let Some(at) = interior_boundary_from(&current, from) else {
                continue;
            };
            let mut new_raw = String::with_capacity(current.len() + 1);
            new_raw.push_str(&current[..at]);
            new_raw.push('x');
            new_raw.push_str(&current[at..]);
            let edit = ByteEdit::new(at..at, "x".to_owned());

            let (diags, stats) = cache.reparse_incremental(&Rope::from(new_raw.as_str()), &[edit]);

            let mut fresh = ParseCache::default();
            let (want, _) = fresh.reparse(&new_raw);
            assert_eq!(
                diag_debug(&diags),
                diag_debug(&want),
                "returned diagnostics diverged mid-run for {} (edit at {at})",
                item.label,
            );
            assert_eq!(
                diag_debug(cache.diagnostics()),
                diag_debug(&want),
                "stored diagnostics() diverged mid-run for {} (edit at {at})",
                item.label,
            );

            if stats.cache_hits > 0 {
                run += 1;
                max_run = max_run.max(run);
            } else {
                run = 0;
            }
            current = new_raw;
        }
        if max_run >= 2 {
            multi_piece_docs += 1;
        }
        count += 1;
    }

    assert!(count > 0, "corpus yielded no usable documents");
    assert!(
        multi_piece_docs > 0,
        "no document sustained >=2 consecutive incremental splices — the cache's \
         maintained multi-piece PieceSeq was never exercised ({count} docs checked)",
    );
}

/// #237 Tier 2 (PR-5): a long run of splices past `MAX_SPLICES_BEFORE_FULL`
/// (= 64) triggers the cache's periodic `PieceSeq::compact` (no forced full
/// parse). On a sample of real corpus documents, drive 80 consecutive interior
/// keystrokes and assert `diagnostics()` stays byte-identical to a fresh full
/// parse at every step — so compaction is exercised over genuine markup, not
/// just the synthetic unit test, and the maintained base survives crossing the
/// compaction boundary twice.
#[test]
fn parse_cache_long_run_crosses_compaction_on_corpus_sample() {
    // 80 > 64 = `MAX_SPLICES_BEFORE_FULL`, so each sampled document crosses the
    // compaction boundary at least once. Bound the sample so the O(edits·docs)
    // full re-parses stay cheap.
    const RUN_LEN: usize = 80;
    const SAMPLE_DOCS: u64 = 20;

    let Some(corpus) = aozora_corpus::from_env() else {
        return; // AOZORA_CORPUS_ROOT unset → vacuous pass.
    };
    let mut sampled: u64 = 0;
    let mut fast_path_docs: u64 = 0;

    for item in corpus.iter().filter_map(Result::ok) {
        if sampled >= SAMPLE_DOCS {
            break;
        }
        let Ok(text) = decode_auto(&item.bytes) else {
            continue;
        };
        let san = aozora::Document::new(text.as_ref()).parse_owned().sanitized;
        if san.is_empty() {
            continue;
        }
        if aozora::Document::new(san.as_str()).parse_owned().sanitized != san {
            continue;
        }
        let Some(crlf) = crlf_variant(&san) else {
            continue;
        };
        // Need an interior boundary the run can repeatedly insert before.
        let Some(_) = interior_boundary_from(&crlf, crlf.len() / 2) else {
            continue;
        };

        let mut cache = ParseCache::default();
        drop(cache.reparse(&crlf));
        let mut current = crlf.clone();
        let mut any_fast = false;
        for _ in 0..RUN_LEN {
            let Some(at) = interior_boundary_from(&current, current.len() / 2) else {
                break;
            };
            let mut new_raw = String::with_capacity(current.len() + 1);
            new_raw.push_str(&current[..at]);
            new_raw.push('x');
            new_raw.push_str(&current[at..]);
            let edit = ByteEdit::new(at..at, "x".to_owned());
            let (diags, stats) = cache.reparse_incremental(&Rope::from(new_raw.as_str()), &[edit]);

            let mut fresh = ParseCache::default();
            let (want, _) = fresh.reparse(&new_raw);
            assert_eq!(
                diag_debug(&diags),
                diag_debug(&want),
                "returned diagnostics diverged in long run for {} (edit at {at})",
                item.label,
            );
            assert_eq!(
                diag_debug(cache.diagnostics()),
                diag_debug(&want),
                "stored diagnostics() diverged in long run for {} (edit at {at})",
                item.label,
            );
            any_fast |= stats.cache_hits > 0;
            current = new_raw;
        }
        if any_fast {
            fast_path_docs += 1;
        }
        sampled += 1;
    }

    assert!(
        sampled > 0,
        "corpus yielded no usable documents for the long run"
    );
    assert!(
        fast_path_docs > 0,
        "no sampled document fast-pathed across the compaction boundary \
         ({sampled} sampled) — compaction was never exercised on real markup",
    );
}
