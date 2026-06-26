//! Per-document parse holder for the LSP backend.
//!
//! Stores the latest source text plus the owned output of the most recent
//! parse, and lends borrowed [`Tree`] views on demand when a request handler
//! needs structural access.
//!
//! # Design
//!
//! Stage 0 of #237 made a parsed document an owned, lifetime-free
//! [`OwnedLexOutput`] that is `Send + Sync`. [`ParseCache`] therefore retains
//! that owned output across edits rather than only the text, and hands out
//! cheap borrowed trees via [`Tree::view`] — no re-parse per request. The LSP
//! backend wraps every per-document state in `Arc<DashMap<Url, OpenDocument>>`
//! (which requires `Sync`); because the retained output is `Sync`, stashing it
//! here is sound.
//!
//! Every [`ParseCache::reparse`] still performs a **full** parse: incremental
//! re-lex/reuse lands in a later #237 PR. The win delivered here is that the
//! per-request [`ParseCache::with_tree`] no longer re-parses from scratch.

use std::cmp::Ordering;
use std::time::{Duration, Instant};

use aozora::{Diagnostic, Document, OwnedLexOutput, Tree};
use tracing::field::Empty as TracingEmpty;

use crate::text_edit::ByteEdit;

/// Documents larger than this skip whole-document semantic analysis —
/// diagnostics, the HTML preview, and the per-request tree access that
/// powers hover / completion / inlay hints. Tree-sitter syntax features
/// and plain editing keep working; only the `aozora`-parser-backed
/// paths degrade.
///
/// This is a denial-of-service backstop. The upstream parser is `O(n)`
/// and runs on the editor's behalf for every keystroke (debounced) and
/// every preview refresh, so an adversarial multi-hundred-MiB paste
/// could otherwise peg a core or exhaust memory. Real aozora-bunko
/// prose is single-digit MiB, so 16 MiB never rejects a genuine
/// document. Mirrors the per-paragraph `MAX_PARAGRAPH_BYTES` cap at the
/// whole-document level; enforced in [`ParseCache::reparse`] (which stores
/// no output for oversized text, so [`ParseCache::with_tree`] degrades to
/// `None`), with the user-facing notice published by the backend.
pub(crate) const MAX_DOCUMENT_BYTES: usize = 16 * 1024 * 1024;

/// Per-call statistics emitted by [`ParseCache::reparse`] /
/// [`ParseCache::reparse_incremental`].
///
/// The caller (typically the LSP backend's `OpenDocument`) feeds these
/// into the per-document `Metrics` so parse latency and cache fields are
/// observable from a third party reading the log. Under the current full-parse
/// foundation (#237 Stage B'1) every reparse re-lexes the whole document, so
/// `cache_hits == 0` and `cache_misses == 1` for a parse that ran
/// (`cache_misses == 0` when the parse was skipped). Incremental reuse — and
/// non-zero `cache_hits` — returns in a later #237 PR.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ReparseStats {
    pub parse_count: u64,
    pub cache_hits: u64,
    pub cache_misses: u64,
    pub cache_entries_after: u64,
    pub cache_bytes_estimate: u64,
    pub latency_us: u64,
}

/// Per-document state holder for the LSP backend.
///
/// Retains the owned output of the most recent parse so the
/// `publishDiagnostics` path can answer in O(1) (the diagnostics are stored
/// position-sorted) and so reads needing the [`Tree`] (hover, inlay hints,
/// completion) get a cheap borrowed view via [`Self::with_tree`] without
/// re-parsing.
#[derive(Debug, Default)]
pub struct ParseCache {
    /// Latest source text. Owned so reads don't have to borrow back
    /// into the parent `OpenDocument`, and so the borrowed [`Tree`] view
    /// handed out by [`Self::with_tree`] can borrow it alongside the output.
    text: String,
    /// Owned output of the most recent [`Self::reparse`]. `None` until the
    /// first parse, or when the document is empty / oversized (those store no
    /// output, so [`Self::with_tree`] degrades to `None`). Its `diagnostics`
    /// are sorted into position order at store time so reads are byte-identical
    /// to the previous merged-segmentation output.
    output: Option<OwnedLexOutput>,
}

impl ParseCache {
    /// Re-parse `text` from scratch. Returns the diagnostics plus per-call
    /// statistics (`cache_hits == 0` — every parse re-lexes the whole
    /// document under the current foundation).
    pub fn reparse(&mut self, text: &str) -> (Vec<Diagnostic>, ReparseStats) {
        self.reparse_full(text)
    }

    /// Re-parse `text` after `edits`. Under #237 Stage B'1 this performs the
    /// same full parse as [`Self::reparse`] — incremental reuse keyed off
    /// `edits` returns in a later PR. The `edits` argument is accepted now so
    /// the call site and signature are stable across that change.
    ///
    /// The result is always identical to a from-scratch parse of `text`.
    pub fn reparse_incremental(
        &mut self,
        text: &str,
        _edits: &[ByteEdit],
    ) -> (Vec<Diagnostic>, ReparseStats) {
        self.reparse_full(text)
    }

    /// Core full re-parse. Stores the owned output (with diagnostics sorted
    /// into position order) and the text, and reports per-call statistics.
    #[tracing::instrument(
        level = "debug",
        skip_all,
        fields(
            text_bytes = text.len(),
            latency_us = TracingEmpty,
        ),
    )]
    fn reparse_full(&mut self, text: &str) -> (Vec<Diagnostic>, ReparseStats) {
        let started_at = Instant::now();

        // Skip the O(n) parse for empty or oversized documents (see
        // `MAX_DOCUMENT_BYTES`). Store the text so size checks stay
        // consistent, store no output — the backend publishes a single
        // "too large" notice for oversized text, and empty text has nothing
        // to surface — and report a zero-parse reparse so metrics don't count
        // phantom work. With no stored output, `with_tree` degrades to `None`.
        if text.is_empty() || text.len() > MAX_DOCUMENT_BYTES {
            text.clone_into(&mut self.text);
            self.output = None;
            let stats = ReparseStats {
                parse_count: 0,
                cache_hits: 0,
                cache_misses: 0,
                cache_entries_after: 0,
                cache_bytes_estimate: u64::try_from(text.len()).unwrap_or(u64::MAX),
                latency_us: duration_as_us(started_at.elapsed()),
            };
            return (Vec::new(), stats);
        }

        let mut out = Document::new(text).parse_owned();
        // `OwnedLexOutput.diagnostics` are in pipeline-stage order; the LSP
        // surface expects them position-sorted (the prior segmentation path
        // returned `merged_diagnostics()`, which is sorted by
        // `(span.start, span.end)` then debug string). Sort once here at store
        // time so every read is byte-identical and O(1).
        out.diagnostics.sort_by(diagnostic_order);

        let diagnostics = out.diagnostics.clone();
        let latency_us = duration_as_us(started_at.elapsed());

        text.clone_into(&mut self.text);
        self.output = Some(out);

        let stats = ReparseStats {
            parse_count: 1,
            cache_hits: 0,
            cache_misses: 1,
            cache_entries_after: 1,
            cache_bytes_estimate: u64::try_from(text.len()).unwrap_or(u64::MAX),
            latency_us,
        };
        tracing::Span::current().record("latency_us", latency_us);
        (diagnostics, stats)
    }

    /// Borrow the most recent diagnostics, position-sorted. Empty until the
    /// first successful [`Self::reparse`], and empty for empty / oversized
    /// documents (which store no output).
    #[must_use]
    pub fn diagnostics(&self) -> &[Diagnostic] {
        self.output.as_ref().map_or(&[][..], |o| &o.diagnostics)
    }

    /// Run `f` against a borrowed [`Tree`] over the most recent parse.
    /// Returns the closure's result, or `None` when there is no stored output
    /// — before the first [`Self::reparse`], for empty text, or for an
    /// oversized document (see `MAX_DOCUMENT_BYTES`, which skips the parse).
    ///
    /// Cheap: the owned output is retained, so this lends a borrowed
    /// [`Tree::view`] without re-parsing. No `Document` is built and no parse
    /// runs.
    pub fn with_tree<R>(&self, f: impl FnOnce(&Tree<'_>) -> R) -> Option<R> {
        let output = self.output.as_ref()?;
        Some(f(&Tree::view(&self.text, output)))
    }

    /// Whether any text has been parsed yet.
    #[cfg(test)]
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.text.is_empty() && self.output.is_none()
    }
}

/// Position order for diagnostics: by `(span.start, span.end)`, then by debug
/// string as a stable tiebreaker. Matches the ordering the LSP surface
/// previously received from the merged segmentation diagnostics.
fn diagnostic_order(a: &Diagnostic, b: &Diagnostic) -> Ordering {
    let (sa, sb) = (a.span(), b.span());
    (sa.start, sa.end)
        .cmp(&(sb.start, sb.end))
        .then_with(|| format!("{a:?}").cmp(&format!("{b:?}")))
}

/// Convert a `Duration` to whole microseconds, saturating at
/// `u64::MAX`.
fn duration_as_us(d: Duration) -> u64 {
    u64::try_from(d.as_micros()).unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_reparse_populates_state() {
        let mut cache = ParseCache::default();
        assert!(cache.is_empty());
        let (diags, stats) = cache.reparse("hello, world");
        assert!(diags.is_empty());
        assert_eq!(stats.parse_count, 1);
    }

    #[test]
    fn reparse_updates_text_and_with_tree_sees_it() {
        let mut cache = ParseCache::default();
        drop(cache.reparse("first"));
        drop(cache.reparse("｜青梅《おうめ》"));
        let inline_count = cache
            .with_tree(|tree| {
                tree.lex_output()
                    .registry
                    .count_kind(aozora::Sentinel::Inline)
            })
            .expect("populated");
        assert_eq!(inline_count, 1);
    }

    #[test]
    fn reparse_reports_latency_micros() {
        let mut cache = ParseCache::default();
        let (_, stats) = cache.reparse("plain text");
        assert!(stats.latency_us < 10_000_000, "stats: {stats:?}");
    }

    #[test]
    fn pua_collision_surfaces_diagnostic() {
        let mut cache = ParseCache::default();
        let (diags, _) = cache.reparse("abc\u{E001}def");
        assert!(
            diags
                .iter()
                .any(|d| matches!(d, Diagnostic::SourceContainsPua { .. })),
            "expected SourceContainsPua, got {diags:?}",
        );
    }

    #[test]
    fn empty_text_parses_with_no_diagnostics() {
        let mut cache = ParseCache::default();
        let (diags, stats) = cache.reparse("");
        assert!(diags.is_empty());
        assert_eq!(stats.parse_count, 0, "empty text is not parsed");
        assert!(
            cache.with_tree(|_| ()).is_none(),
            "empty text stores no output",
        );
    }

    #[test]
    fn oversized_text_skips_parse_and_degrades_tree() {
        let mut cache = ParseCache::default();
        let big = "a".repeat(MAX_DOCUMENT_BYTES + 1);
        let (diags, stats) = cache.reparse(&big);
        assert!(diags.is_empty(), "oversized parse must be skipped");
        assert_eq!(stats.parse_count, 0, "no parse when oversized");
        assert!(
            cache.with_tree(|_| ()).is_none(),
            "with_tree must degrade to None for oversized documents",
        );
    }

    #[test]
    fn full_reparse_reports_zero_hits() {
        let mut cache = ParseCache::default();
        let (_, stats) = cache.reparse("alpha\n\nbeta\n\ngamma");
        assert_eq!(stats.cache_hits, 0, "a full parse reuses nothing");
        assert_eq!(stats.cache_misses, 1, "one full parse");
        assert_eq!(stats.cache_entries_after, 1);
    }

    #[test]
    fn incremental_edit_still_full_parses() {
        let mut cache = ParseCache::default();
        let old = "alpha\n\nbeta\n\ngamma";
        drop(cache.reparse(old));

        // Replace "beta" with "delta". Under B'1 this is a full parse, so
        // `cache_hits == 0`; the diagnostics must still equal a from-scratch
        // parse of the new text.
        let at = old.find("beta").unwrap();
        let edit = ByteEdit::new(at..at + "beta".len(), "delta".to_owned());
        let new_text = "alpha\n\ndelta\n\ngamma";
        let (diags, stats) = cache.reparse_incremental(new_text, &[edit]);

        assert_eq!(stats.cache_hits, 0, "B'1 always full-parses: {stats:?}");
        let mut fresh = ParseCache::default();
        let (want, _) = fresh.reparse(new_text);
        let as_debug = |ds: &[Diagnostic]| ds.iter().map(|d| format!("{d:?}")).collect::<Vec<_>>();
        assert_eq!(as_debug(&diags), as_debug(&want));
    }

    #[test]
    fn multi_edit_batch_full_parses() {
        let mut cache = ParseCache::default();
        drop(cache.reparse("alpha\n\nbeta\n\ngamma"));
        let edits = [
            ByteEdit::new(0..0, "x".to_owned()),
            ByteEdit::new(10..10, "y".to_owned()),
        ];
        let (_, stats) = cache.reparse_incremental("xalpha\n\nbexyta\n\ngamma", &edits);
        assert_eq!(stats.cache_hits, 0, "a multi-edit batch re-parses fully");
    }

    /// `n` blank-line-separated plain-prose paragraphs.
    fn plain_paragraphs(n: usize) -> String {
        let mut s = String::new();
        for i in 0..n {
            if i > 0 {
                s.push_str("\n\n");
            }
            s.push('第');
            s.push_str(&i.to_string());
            s.push_str("段落の本文です。");
        }
        s
    }

    #[test]
    fn large_single_edit_full_parses_and_matches() {
        let n = 50usize;
        let old = plain_paragraphs(n);
        let mut cache = ParseCache::default();
        drop(cache.reparse(&old));

        // Insert one plain char inside the middle paragraph's body.
        let marker = "第25段落の本文";
        let at = old.find(marker).unwrap() + marker.len();
        let mut new_text = old.clone();
        new_text.insert(at, 'ぞ');
        let edit = ByteEdit::new(at..at, "ぞ".to_owned());
        let (diags, stats) = cache.reparse_incremental(&new_text, &[edit]);

        assert_eq!(stats.cache_hits, 0, "B'1 always full-parses: {stats:?}");
        let mut fresh = ParseCache::default();
        let (want, _) = fresh.reparse(&new_text);
        let as_debug = |ds: &[Diagnostic]| ds.iter().map(|d| format!("{d:?}")).collect::<Vec<_>>();
        assert_eq!(as_debug(&diags), as_debug(&want));
    }

    #[test]
    fn deliberately_wrong_edit_range_still_equals_full() {
        // Both paths are full parses, so the result must equal a from-scratch
        // parse regardless of the (now-ignored) edit range.
        let mut cache = ParseCache::default();
        drop(cache.reparse("alpha\n\nbeta\n\ngamma"));
        let new_text = "alpha\n\nbeta edited\n\ngamma";
        let bogus = ByteEdit::new(0..0, "zzz".to_owned());
        let (diags, _) = cache.reparse_incremental(new_text, &[bogus]);
        let mut fresh = ParseCache::default();
        let (want, _) = fresh.reparse(new_text);
        let as_debug = |ds: &[Diagnostic]| ds.iter().map(|d| format!("{d:?}")).collect::<Vec<_>>();
        assert_eq!(as_debug(&diags), as_debug(&want));
    }
}
