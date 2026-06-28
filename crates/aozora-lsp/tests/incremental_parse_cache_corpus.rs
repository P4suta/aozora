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

/// A char-boundary byte offset near the document midpoint that is not adjacent
/// to a line break — a realistic interior keystroke position.
fn mid_line_boundary(s: &str) -> Option<usize> {
    let start = s.len() / 2;
    for i in start..s.len() {
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
    let (diags, stats) = cache.reparse_incremental(&new_raw, &[edit]);

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
