//! Per-document parse wrapper for the LSP backend.
//!
//! Stores the latest source text plus the diagnostics from the most
//! recent parse, and re-derives a fresh [`Tree`] on demand
//! when a request handler needs structural access.
//!
//! # Why no stored `Document`
//!
//! `aozora::Document` owns a `bumpalo::Bump` whose interior `Cell`s
//! make it `!Sync`. The LSP backend wraps every per-document state
//! in `Arc<DashMap<Url, OpenDocument>>`, which requires `OpenDocument: Sync`.
//! Stashing a `Document` inside `OpenDocument` therefore cannot work
//! across threads. Instead, [`ParseCache`] stores the latest text
//! and re-parses with a fresh `Document` whenever a request handler
//! needs the [`Tree`]. The corpus median document re-parses in
//! single-digit milliseconds — well below the keystroke-perceptibility
//! threshold — so the per-call cost is acceptable.

use std::ops::Range;
use std::time::{Duration, Instant};

use aozora::{Diagnostic, Document, SegmentedParse, Tree};
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
/// whole-document level; enforced in [`ParseCache::reparse`] and
/// [`ParseCache::with_tree`], with the user-facing notice published by
/// the backend.
pub(crate) const MAX_DOCUMENT_BYTES: usize = 16 * 1024 * 1024;

/// Per-call statistics emitted by [`ParseCache::reparse`] /
/// [`ParseCache::reparse_incremental`].
///
/// The caller (typically the LSP backend's `OpenDocument`) feeds these
/// into the per-document `Metrics` so parse latency and cache fields are
/// observable from a third party reading the log. Under the segment cache
/// (#237) `cache_hits` counts the segments reused from the prior parse and
/// `cache_misses` the segments re-lexed; a full parse reports
/// `cache_hits == 0` with `cache_misses` equal to the segment count.
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
/// Keeps the latest diagnostics so the `publishDiagnostics` path can
/// answer in O(1) without re-parsing. Reads needing the
/// [`Tree`] (hover, inlay hints, completion) call
/// [`Self::with_tree`], which builds a fresh [`Document`] on the
/// stack and yields a borrowed tree to the closure.
#[derive(Debug, Default, Clone)]
pub struct ParseCache {
    /// Latest source text. Owned so reads don't have to borrow back
    /// into the parent `OpenDocument`.
    text: String,
    /// Diagnostics from the most recent [`Self::reparse`]. Empty
    /// until the first parse.
    diagnostics: Vec<Diagnostic>,
    /// Cached segmentation of [`Self::text`] (#237). Lets
    /// [`Self::reparse_incremental`] re-lex only the segment an edit
    /// touched and reuse the rest. `None` until the first parse or when the
    /// document is oversized.
    segmentation: Option<SegmentedParse>,
}

impl ParseCache {
    /// Re-parse `text` from scratch. Returns the diagnostics plus per-call
    /// statistics (`cache_hits == 0`, all segments freshly lexed).
    pub fn reparse(&mut self, text: &str) -> (Vec<Diagnostic>, ReparseStats) {
        self.reparse_with_edit(text, None)
    }

    /// Re-parse `text`, reusing the cached segmentation where `edits` permit
    /// (#237). When `edits` is a single byte-range replacement that produced
    /// `text` from the previously-cached text, only the touched segment is
    /// re-lexed and the rest are reused — `cache_hits` then counts the reused
    /// segments. Any other batch (zero or multiple edits) re-parses fully.
    ///
    /// The result is always identical to a from-scratch parse: the underlying
    /// [`SegmentedParse::reparse_incremental`] falls back to a full parse
    /// whenever reuse cannot be proven safe.
    pub fn reparse_incremental(
        &mut self,
        text: &str,
        edits: &[ByteEdit],
    ) -> (Vec<Diagnostic>, ReparseStats) {
        let single = match edits {
            [edit] => Some(edit.range.clone()),
            _ => None,
        };
        self.reparse_with_edit(text, single)
    }

    /// Core re-parse. `edit` is `Some(range)` to attempt incremental reuse
    /// against the cached segmentation, or `None` for a full parse.
    #[tracing::instrument(
        level = "debug",
        skip_all,
        fields(
            text_bytes = text.len(),
            latency_us = TracingEmpty,
        ),
    )]
    fn reparse_with_edit(
        &mut self,
        text: &str,
        edit: Option<Range<usize>>,
    ) -> (Vec<Diagnostic>, ReparseStats) {
        let started_at = Instant::now();

        // Skip the O(n) parse for oversized documents (see
        // `MAX_DOCUMENT_BYTES`). Store the text so size checks stay
        // consistent, leave diagnostics empty — the backend publishes a
        // single "too large" notice in their place — and report a
        // zero-segment reparse so metrics don't count phantom work.
        if text.len() > MAX_DOCUMENT_BYTES {
            text.clone_into(&mut self.text);
            self.diagnostics.clear();
            self.segmentation = None;
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

        let (segmentation, cache_hits, cache_misses) =
            if let (Some(range), Some(prior)) = (edit, self.segmentation.take()) {
                let (next, outcome) = prior.reparse_incremental(text, range);
                (next, outcome.reused_segments, outcome.relexed_segments)
            } else {
                let next = SegmentedParse::of(text);
                let segments = u64::try_from(next.segment_count()).unwrap_or(u64::MAX);
                (next, 0, segments)
            };

        let diagnostics = segmentation.merged_diagnostics();
        let cache_entries_after = u64::try_from(segmentation.segment_count()).unwrap_or(u64::MAX);
        let latency_us = duration_as_us(started_at.elapsed());

        text.clone_into(&mut self.text);
        self.diagnostics.clone_from(&diagnostics);
        self.segmentation = Some(segmentation);

        let stats = ReparseStats {
            parse_count: 1,
            cache_hits,
            cache_misses,
            cache_entries_after,
            cache_bytes_estimate: u64::try_from(text.len()).unwrap_or(u64::MAX),
            latency_us,
        };
        tracing::Span::current().record("latency_us", latency_us);
        (diagnostics, stats)
    }

    /// Borrow the most recent diagnostics. Empty until the first
    /// successful [`Self::reparse`].
    #[must_use]
    pub fn diagnostics(&self) -> &[Diagnostic] {
        &self.diagnostics
    }

    /// Run `f` against a freshly parsed [`Tree`]. Returns the
    /// closure's result, or `None` if no [`Self::reparse`] has been
    /// called yet (text is empty).
    ///
    /// The Document is built on the stack inside this call so its
    /// `!Sync` arena does not leak into the surrounding `OpenDocument`.
    /// Re-parse cost is paid per call; for keystroke-rate UIs the
    /// new bumpalo pipeline absorbs this comfortably (sub-ms median
    /// on the corpus).
    pub fn with_tree<R>(&self, f: impl FnOnce(&Tree<'_>) -> R) -> Option<R> {
        if self.text.is_empty() && self.diagnostics.is_empty() {
            return None;
        }
        // Oversized documents skip semantic parsing (see `reparse`);
        // re-parsing the whole text on every hover / completion would
        // hang the editor. Degrade to `None` so those handlers return
        // nothing rather than block.
        if self.text.len() > MAX_DOCUMENT_BYTES {
            return None;
        }
        let document = Document::new(self.text.as_str());
        let tree = document.parse();
        Some(f(&tree))
    }

    /// Whether any text has been parsed yet.
    #[cfg(test)]
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.text.is_empty() && self.diagnostics.is_empty()
    }
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
        let (diags, _) = cache.reparse("");
        assert!(diags.is_empty());
    }

    #[test]
    fn oversized_text_skips_parse_and_degrades_tree() {
        let mut cache = ParseCache::default();
        let big = "a".repeat(MAX_DOCUMENT_BYTES + 1);
        let (diags, stats) = cache.reparse(&big);
        assert!(diags.is_empty(), "oversized parse must be skipped");
        assert_eq!(stats.parse_count, 0, "no segments parsed when oversized");
        assert!(
            cache.with_tree(|_| ()).is_none(),
            "with_tree must degrade to None for oversized documents",
        );
    }

    #[test]
    fn full_reparse_reports_segment_misses() {
        let mut cache = ParseCache::default();
        let (_, stats) = cache.reparse("alpha\n\nbeta\n\ngamma");
        assert_eq!(stats.cache_hits, 0, "a from-scratch parse reuses nothing");
        assert_eq!(stats.cache_misses, 3, "three paragraphs => three segments");
        assert_eq!(stats.cache_entries_after, 3);
    }

    #[test]
    fn incremental_edit_reuses_segments() {
        let mut cache = ParseCache::default();
        let old = "alpha\n\nbeta\n\ngamma";
        drop(cache.reparse(old));

        // Replace "beta" with "delta" — a plain edit inside the middle
        // segment, so the two untouched segments are reused.
        let at = old.find("beta").unwrap();
        let edit = ByteEdit::new(at..at + "beta".len(), "delta".to_owned());
        let new_text = "alpha\n\ndelta\n\ngamma";
        let (diags, stats) = cache.reparse_incremental(new_text, &[edit]);

        assert!(stats.cache_hits >= 2, "untouched segments reuse: {stats:?}");
        assert_eq!(stats.cache_misses, 1, "only the edited segment re-lexes");
        // Diagnostics must match a from-scratch parse of the new text.
        let mut fresh = ParseCache::default();
        let (want, _) = fresh.reparse(new_text);
        let as_debug = |ds: &[Diagnostic]| ds.iter().map(|d| format!("{d:?}")).collect::<Vec<_>>();
        assert_eq!(as_debug(&diags), as_debug(&want));
    }

    #[test]
    fn multi_edit_batch_falls_back_to_full() {
        let mut cache = ParseCache::default();
        drop(cache.reparse("alpha\n\nbeta\n\ngamma"));
        let edits = [
            ByteEdit::new(0..0, "x".to_owned()),
            ByteEdit::new(10..10, "y".to_owned()),
        ];
        let (_, stats) = cache.reparse_incremental("xalpha\n\nbexyta\n\ngamma", &edits);
        assert_eq!(stats.cache_hits, 0, "a multi-edit batch re-parses fully");
    }

    /// `n` blank-line-separated plain-prose paragraphs — the shape that
    /// actually exercises segment reuse (each paragraph is its own
    /// segment, no whole-document-scoped diagnostics).
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
    fn large_single_edit_reuses_all_untouched_segments() {
        let n = 50usize;
        let old = plain_paragraphs(n);
        let mut cache = ParseCache::default();
        let (_, full) = cache.reparse(&old);
        assert_eq!(
            full.cache_entries_after, n as u64,
            "one segment per paragraph"
        );

        // Insert one plain char inside the middle paragraph's body.
        let marker = "第25段落の本文";
        let at = old.find(marker).unwrap() + marker.len();
        let mut new_text = old.clone();
        new_text.insert(at, 'ぞ');
        let edit = ByteEdit::new(at..at, "ぞ".to_owned());
        let (diags, stats) = cache.reparse_incremental(&new_text, &[edit]);

        assert_eq!(
            stats.cache_misses, 1,
            "only the edited segment re-lexes: {stats:?}"
        );
        assert_eq!(
            stats.cache_hits,
            (n - 1) as u64,
            "every untouched segment is reused: {stats:?}",
        );
        let mut fresh = ParseCache::default();
        let (want, _) = fresh.reparse(&new_text);
        let as_debug = |ds: &[Diagnostic]| ds.iter().map(|d| format!("{d:?}")).collect::<Vec<_>>();
        assert_eq!(as_debug(&diags), as_debug(&want));
    }

    #[test]
    fn deliberately_wrong_edit_range_still_equals_full() {
        // The fast path is only ever *correct* because the underlying
        // `SegmentedParse` re-verifies that the edit range actually
        // transforms the cached text into the new text (byte-equality
        // guard). Feed a single edit whose range does NOT describe how the
        // new text was produced; reuse must be rejected and the result
        // must still equal a from-scratch parse.
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
