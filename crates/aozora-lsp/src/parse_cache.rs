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

use std::time::{Duration, Instant};

use aozora::{Diagnostic, Document, Tree};
use tracing::field::Empty as TracingEmpty;

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

/// Per-call statistics emitted by [`ParseCache::reparse`].
///
/// The caller (typically the LSP backend's `OpenDocument`) feeds these
/// into the per-document `Metrics` so parse latency
/// and cache fields are observable from a third party reading the
/// log. `cache_hits` / `cache_misses` are set to `0` / `1` for every
/// call — every reparse is a "miss" under the whole-document model.
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
}

impl ParseCache {
    /// Re-parse `text`. Returns the diagnostics produced by the parse
    /// plus per-call statistics.
    #[tracing::instrument(
        level = "debug",
        skip_all,
        fields(
            text_bytes = text.len(),
            latency_us = TracingEmpty,
        ),
    )]
    pub fn reparse(&mut self, text: &str) -> (Vec<Diagnostic>, ReparseStats) {
        let started_at = Instant::now();

        // Skip the O(n) parse for oversized documents (see
        // `MAX_DOCUMENT_BYTES`). Store the text so size checks stay
        // consistent, leave diagnostics empty — the backend publishes a
        // single "too large" notice in their place — and report a
        // zero-segment reparse so metrics don't count phantom work.
        if text.len() > MAX_DOCUMENT_BYTES {
            text.clone_into(&mut self.text);
            self.diagnostics.clear();
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

        let document = Document::new(text);
        let diagnostics: Vec<Diagnostic> = document.parse().diagnostics().to_vec();
        let latency_us = duration_as_us(started_at.elapsed());

        text.clone_into(&mut self.text);
        self.diagnostics.clone_from(&diagnostics);

        let bytes_estimate = u64::try_from(text.len()).unwrap_or(u64::MAX);
        let stats = ReparseStats {
            parse_count: 1,
            cache_hits: 0,
            cache_misses: 1,
            cache_entries_after: 1,
            cache_bytes_estimate: bytes_estimate,
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

    /// Install diagnostics produced by an out-of-band parse (the
    /// debounced background task). Replaces the prior diagnostic vector
    /// wholesale; the caller has verified this parse matches the current
    /// text version.
    pub fn set_diagnostics(&mut self, diagnostics: Vec<Diagnostic>) {
        self.diagnostics = diagnostics;
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
}
