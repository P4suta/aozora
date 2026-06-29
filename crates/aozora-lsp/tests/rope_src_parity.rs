//! `RopeSrc` ↔ `&str` parity for the incremental engine's byte source (#237
//! Tier 2, Mechanism B).
//!
//! The diagnostics-only splice is generic over [`aozora::SanitizedSrc`]. The
//! `&str` impl lives in the core crate and is the byte-for-byte reference; this
//! gate proves the `aozora-lsp` rope impl ([`RopeSrc`]) is bit-identical to it,
//! both at the primitive level (`byte`/`slice`, including the chunk cursor and
//! chunk-straddling `Cow::Owned` slices) and end-to-end through the real engine
//! ([`aozora::reparse_incremental_diagnostics_only_in`] vs the `&str`
//! [`aozora::reparse_incremental_diagnostics_only`]). A divergence here is
//! exactly the class of bug that would let the rope-backed LSP cache desync from
//! a full parse.

#![cfg(feature = "internals")]

use std::ops::Range;

use aozora::{
    DiagBaseRef, Diagnostic, Document, PieceSeq, SanitizedSrc,
    reparse_incremental_diagnostics_only, reparse_incremental_diagnostics_only_in,
};
use aozora_lsp::internals::RopeSrc;
use proptest::prelude::*;
use proptest::sample::Index;
use ropey::Rope;

/// Assert `RopeSrc::byte`/`slice` match `&str` indexing/`get` for `s`.
fn assert_primitive_parity(s: &str) {
    let rope = Rope::from(s);
    let src = RopeSrc::new(rope.byte_slice(..));

    assert_eq!(src.len(), s.len(), "len mismatch");
    assert_eq!(src.is_empty(), s.is_empty(), "is_empty mismatch");

    // `byte(i)` for every i, forward — the monotone outward scan's hot path.
    for (i, &want) in s.as_bytes().iter().enumerate() {
        assert_eq!(src.byte(i), want, "byte {i} (forward) for {s:?}");
    }
    // And backward over a fresh cursor: the memo must never disagree with the
    // underlying bytes regardless of probe direction.
    let back = RopeSrc::new(rope.byte_slice(..));
    for i in (0..s.len()).rev() {
        assert_eq!(
            back.byte(i),
            s.as_bytes()[i],
            "byte {i} (backward) for {s:?}"
        );
    }
}

/// Assert `RopeSrc::slice` matches `str::get` for `range` (covers off-bounds,
/// non-char-boundary, single-chunk borrow, and chunk-straddling owned cases).
fn assert_slice_parity(src: &RopeSrc<'_>, s: &str, range: Range<usize>) {
    assert_eq!(
        src.slice(range.clone()).as_deref(),
        s.get(range.clone()),
        "slice {range:?} for {s:?}",
    );
}

/// Render a diagnostics slice to a comparable `Vec<String>`.
fn diag_debug(ds: &[Diagnostic]) -> Vec<String> {
    ds.iter().map(|d| format!("{d:?}")).collect()
}

/// Run one sanitized-coordinate edit through the engine on **both** backends and
/// assert the splice (accept/decline and, when accepted, every field) is
/// identical. Both backends receive byte-identical inputs (the same `doc` /
/// `edited` strings — only the source *type* differs), so any divergence is a
/// `RopeSrc`-vs-`&str` bug, not a fixed-point artefact; `doc` need not be a
/// sanitize fixed point.
///
/// This also pins **region-find parity transitively**:
/// `reparse_incremental_diagnostics_only_in` runs the engine's region finder
/// (`minimal_balanced_region`) over the `RopeSrc` base as its first step, and a
/// `RopeSrc::byte` divergence at a chunk boundary would change the region and so
/// the reuse counts / node spans asserted below.
fn compare_backends(doc: &str, edit: Range<usize>, ins: &str) {
    let out = Document::new(doc).parse_owned();
    let pieces = PieceSeq::from_contiguous(
        &out.source_nodes,
        &out.pairs,
        &out.diagnostics,
        out.sanitized_len,
    );

    let mut edited = String::with_capacity(doc.len() + ins.len());
    edited.push_str(&doc[..edit.start]);
    edited.push_str(ins);
    edited.push_str(&doc[edit.end..]);

    // `&str` reference path.
    let str_splice = reparse_incremental_diagnostics_only(
        DiagBaseRef::from_cached(&out, &pieces),
        &edited,
        edit.clone(),
    );

    // Rope path: same maintained piece sequence, rope-backed sanitized views.
    let old_rope = Rope::from(doc);
    let new_rope = Rope::from(edited.as_str());
    let rope_base = DiagBaseRef {
        sanitized: RopeSrc::new(old_rope.byte_slice(..)),
        pieces: &pieces,
    };
    let new_src = RopeSrc::new(new_rope.byte_slice(..));
    let rope_splice = reparse_incremental_diagnostics_only_in(&rope_base, &new_src, edit.clone());

    match (str_splice, rope_splice) {
        (Some(s), Some(r)) => {
            let (s_nodes, s_pairs, s_diags) = s.pieces.flatten();
            let (r_nodes, r_pairs, r_diags) = r.pieces.flatten();
            assert_eq!(diag_debug(&s_diags), diag_debug(&r_diags), "diagnostics");
            assert_eq!(s_nodes.len(), r_nodes.len(), "node count");
            assert_eq!(s_pairs.len(), r_pairs.len(), "pair count");
            assert_eq!(s.reused_nodes, r.reused_nodes, "reused_nodes");
            assert_eq!(s.relexed_nodes, r.relexed_nodes, "relexed_nodes");
        }
        (None, None) => {}
        (s, r) => panic!(
            "str vs rope accept/decline mismatch (str={}, rope={}) for edit {edit:?} in {doc:?}",
            s.is_some(),
            r.is_some(),
        ),
    }
}

/// As [`compare_backends`], but first asserts `doc` is a sanitize fixed point — a
/// sanity check on the hand-written deterministic fixtures, so a typo making
/// `out.sanitized != doc` is caught rather than silently comparing a base that
/// does not describe `doc`.
fn assert_engine_parity(doc: &str, edit: Range<usize>, ins: &str) {
    assert_eq!(
        Document::new(doc).parse_owned().sanitized.as_str(),
        doc,
        "the parity fixture must be a sanitize fixed point",
    );
    compare_backends(doc, edit, ins);
}

#[test]
fn primitive_parity_multichunk_and_edges() {
    // A multi-chunk rope (well past ropey's ~1 KiB chunk size) so `byte`'s
    // cursor crosses chunk boundaries, plus small edge strings.
    let long: String = "あいうえお、本文の段落です。\n\n".repeat(400);
    for s in [
        "",
        "a",
        "あ",
        "ascii and 日本語 mixed\n\nsecond",
        long.as_str(),
    ] {
        assert_primitive_parity(s);
    }

    // Slice parity, including ranges that straddle chunk boundaries (forcing the
    // `Cow::Owned` path) and a non-char-boundary range (both must yield `None`).
    let rope = Rope::from(long.as_str());
    let src = RopeSrc::new(rope.byte_slice(..));
    for range in [0..0, 0..long.len(), 30..3000, 1500..1503, 1..2] {
        assert_slice_parity(&src, &long, range);
    }
}

#[test]
fn engine_parity_representative_docs() {
    // (1) Plain interior insert in the middle paragraph (fast path, 0 reuse).
    let plain = "あいうえお\n\nかきくけこ\n\nさしすせそ";
    let plain_at = plain.find("きくけこ").unwrap();
    assert_engine_parity(plain, plain_at..plain_at, "ん");
    // (2) Leading ruby reused across a middle-paragraph insert (non-zero reuse).
    let ruby = "｜青空《あおぞら》\n\nほんぶんです\n\nまつびです";
    let at = ruby.find("ほんぶん").unwrap();
    assert_engine_parity(ruby, at..at, "も");
    // (3) Multi-chunk doc: the region scan walks the rope cursor across chunks.
    let long = "だんらくの本文です。\n\n".repeat(300);
    let mid = long.len() / 2;
    let at = (mid..long.len())
        .find(|&i| long.is_char_boundary(i) && !long[i..].starts_with('\n'))
        .unwrap();
    assert_engine_parity(&long, at..at, "ぞ");
    // (4) An edit carrying a line terminator: both backends must decline.
    assert_engine_parity(plain, plain_at..plain_at, "x\ny");
}

proptest! {
    // `byte`/`slice` parity over arbitrary UTF-8 — the strongest pin that the
    // rope cursor never disagrees with `as_bytes()` and that chunk-straddling
    // slices reproduce `str::get` exactly.
    #[test]
    fn prop_primitive_parity(s in "\\PC{0,400}") {
        assert_primitive_parity(&s);
        let rope = Rope::from(s.as_str());
        let src = RopeSrc::new(rope.byte_slice(..));
        // Exhaustive small-range slice parity (bounded so the O(n^2) sweep stays
        // cheap); covers non-char-boundary ranges, which must yield `None`.
        let cap = s.len().min(40);
        for start in 0..=cap {
            for end in start..=cap {
                assert_slice_parity(&src, &s, start..end);
            }
        }
    }
}

proptest! {
    // The engine splice is heavier than the primitives, so cap the case count.
    #![proptest_config(ProptestConfig::with_cases(96))]

    /// End-to-end engine parity over generated multi-paragraph documents and a
    /// random interior insert: the `&str`
    /// ([`reparse_incremental_diagnostics_only`]) and `RopeSrc`
    /// ([`reparse_incremental_diagnostics_only_in`]) paths must agree on
    /// accept/decline and every spliced field. Repeating the paragraph body makes
    /// the document span multiple rope chunks, so the region scan's
    /// `RopeSrc::byte` probes cross chunk boundaries — the strongest pin that the
    /// rope cursor never diverges from `&str` *inside the real engine*, including
    /// the region finder (`minimal_balanced_region`) it drives.
    #[test]
    fn prop_engine_parity(
        paras in prop::collection::vec("[\\p{Hiragana}\\p{Katakana}a-z0-9、。]{1,48}", 2..32),
        repeat in 1usize..4,
        pick in any::<Index>(),
        ins in "[\\p{Hiragana}a-zX\n]{0,5}",
    ) {
        // Plain kana/ascii paragraphs joined by blank lines: no BOM/CRLF/〔〕/PUA
        // and no decorative-rule run, so `doc` is normally a sanitize fixed point.
        let unit = paras.join("\n\n");
        let doc = vec![unit.as_str(); repeat].join("\n\n");
        prop_assume!(!doc.is_empty());
        // Skip the rare non-fixed-point rather than feed `compare_backends` a base
        // that does not describe `doc`; the charset makes this almost never fire.
        prop_assume!(
            Document::new(doc.as_str()).parse_owned().sanitized.as_str() == doc.as_str()
        );
        // A random char-boundary insert position (snapped up; `doc.len()` is a
        // valid boundary, i.e. an append).
        let mut at = pick.index(doc.len());
        while at < doc.len() && !doc.is_char_boundary(at) {
            at += 1;
        }
        compare_backends(&doc, at..at, &ins);
    }
}

// ---- adversarial: the removed-memcmp safety net is wired (debug only) --------
//
// In release the prefix/suffix `memcmp` is gone (deleted for time) and
// `debug_assert_unchanged_outside` compiles out, so these only exist in debug.
// They feed the engine a *lying* `edit_old`: the claimed range leaves the edited
// region's bytes unchanged while the new buffer actually differs OUTSIDE it. A
// plain multi-paragraph document passes every other guard (a region is found, no
// unbalanced delimiter, the edited bytes carry no structure), so the *only*
// thing that can catch the lie is the sanitized source's
// `debug_assert_unchanged_outside`. If it ever stops firing, the splice would
// silently desync in release — these pin that it is reachable through the public
// entries for both backends.

/// Build the base tables for `old` and a `new` that lies about what changed
/// outside `edit_old` (paragraph 1 mutated, the `edit_old` region untouched).
#[cfg(debug_assertions)]
fn lying_setup() -> (&'static str, String, Range<usize>) {
    let old = "あいうえお\n\nかきくけこ\n\nさしすせそ\n";
    let at = old.find("かき").expect("middle paragraph");
    let edit_old = at..at + "かき".len();
    // あ→ん in paragraph 1: same byte length (edit_delta 0), bytes inside
    // `edit_old` unchanged, but the prefix differs — the precondition violation.
    let new = old.replacen('あ', "ん", 1);
    assert_eq!(new.len(), old.len(), "same byte length keeps edit_delta 0");
    (old, new, edit_old)
}

#[cfg(debug_assertions)]
#[test]
#[should_panic(expected = "changed bytes outside edit_old")]
fn lying_edit_trips_debug_assert_str() {
    let (old, new, edit_old) = lying_setup();
    let out = Document::new(old).parse_owned();
    let pieces = PieceSeq::from_contiguous(
        &out.source_nodes,
        &out.pairs,
        &out.diagnostics,
        out.sanitized_len,
    );
    drop(reparse_incremental_diagnostics_only(
        DiagBaseRef::from_cached(&out, &pieces),
        new.as_str(),
        edit_old,
    ));
}

#[cfg(debug_assertions)]
#[test]
#[should_panic(expected = "changed bytes outside edit_old")]
fn lying_edit_trips_debug_assert_rope() {
    let (old, new, edit_old) = lying_setup();
    let out = Document::new(old).parse_owned();
    let pieces = PieceSeq::from_contiguous(
        &out.source_nodes,
        &out.pairs,
        &out.diagnostics,
        out.sanitized_len,
    );
    let old_rope = Rope::from(old);
    let new_rope = Rope::from(new.as_str());
    let base = DiagBaseRef {
        sanitized: RopeSrc::new(old_rope.byte_slice(..)),
        pieces: &pieces,
    };
    let new_src = RopeSrc::new(new_rope.byte_slice(..));
    drop(reparse_incremental_diagnostics_only_in(
        &base, &new_src, edit_old,
    ));
}
