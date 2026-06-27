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
//! [`ParseCache::reparse`] performs a **full** parse;
//! [`ParseCache::reparse_incremental`] takes the **diagnostics-only** incremental
//! splice ([`aozora::reparse_incremental_diagnostics_only`]) on a single edit
//! and full-parses otherwise (#237 Tier 1).
//!
//! # Lazy tree (#237 Tier 1)
//!
//! A consumer trace established that the per-keystroke hot path (debounced
//! `publishDiagnostics`) reads only [`ParseCache::diagnostics`]; the full
//! [`Tree`] (via [`ParseCache::with_tree`]) is needed only by the rare F2
//! rename gesture. So the cache keeps a **store-free** `DiagBase` (sanitized
//! text + diagnostics + the `source_nodes`/`pairs` the next edit's region-find
//! needs) that the hot path splices in `O(region + #diagnostics)`, and
//! materialises the full `O(doc)` [`OwnedLexOutput`] **lazily** — only when
//! [`ParseCache::with_tree`] is actually called — memoised in a [`OnceLock`]
//! (seeded eagerly by a full parse so a structural request right after one is
//! instant, and invalidated on every incremental splice).

use std::cmp::Ordering;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use aozora::pipeline::has_long_rule_line;
use aozora::pipeline::lexer::sanitize::sanitize;
use aozora::{
    DiagBaseRef, DiagSplice, Diagnostic, Document, OwnedLexOutput, PairLink, RegionIndex,
    SourceNodeOwned, Tree,
};
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

/// Force a fresh full parse after this many consecutive incremental splices.
///
/// The diagnostics-only hot path never clones a node store, so the dead-entry
/// growth the owned splice had is gone; but a long run of incremental splices
/// leaves [`ParseCache`] with no materialised tree, so the first structural
/// request (rename) after many keystrokes would pay one full `O(doc)` parse.
/// Forcing a full parse every `MAX_SPLICES_BEFORE_FULL` splices re-seeds the
/// lazy tree and bounds the size of any one re-lexed region's drift, keeping
/// both the splice base and a fresh tree in hand (#249, the periodic-compaction
/// bound).
pub(crate) const MAX_SPLICES_BEFORE_FULL: u32 = 64;

/// Per-call statistics emitted by [`ParseCache::reparse`] /
/// [`ParseCache::reparse_incremental`].
///
/// The caller (typically the LSP backend's `OpenDocument`) feeds these
/// into the per-document `Metrics` so parse latency and cache fields are
/// observable from a third party reading the log. A **full** parse reports
/// `cache_hits == 0` and `cache_misses == 1` for a parse that ran
/// (`cache_misses == 0` when the parse was skipped). An **incremental splice**
/// (#237 Stage B'3, [`ParseCache::reparse_incremental`]) reports `cache_hits` =
/// the number of source nodes reused from the prior parse and `cache_misses` =
/// the number of nodes the re-lexed region produced.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ReparseStats {
    pub parse_count: u64,
    pub cache_hits: u64,
    pub cache_misses: u64,
    pub cache_entries_after: u64,
    pub cache_bytes_estimate: u64,
    pub latency_us: u64,
}

/// The store-free spliceable base of the most recent parse: exactly the fields
/// the diagnostics-only hot path
/// ([`aozora::reparse_incremental_diagnostics_only`]) reads from the prior
/// parse and produces for the next one. Kept across edits so the per-keystroke
/// path never has to materialise the full [`OwnedLexOutput`].
#[derive(Debug, Default)]
struct DiagBase {
    /// Sanitized buffer of the most recent parse (a sanitize fixed point) — the
    /// coordinate space the splice and the next region-find operate in.
    sanitized: String,
    /// Diagnostics, position-sorted at store time so [`ParseCache::diagnostics`]
    /// reads are O(1) and byte-identical to a full parse.
    diagnostics: Vec<Diagnostic>,
    /// Source-node side-table the next edit's region-find consumes (it reads
    /// only `source_span` + the node discriminant, never the store).
    source_nodes: Vec<SourceNodeOwned>,
    /// Resolved delimiter pairs the next edit's region-find consumes
    /// (store-free offsets).
    pairs: Vec<PairLink>,
    /// Region-find acceleration over the three tables above (#237 Tier 2), built
    /// in the same `O(N)` pass that assembles them so the next edit's prologue
    /// runs `O(region + log n)` instead of re-scanning the whole buffer/tables.
    index: RegionIndex,
}

impl DiagBase {
    /// Borrow this base as the lightweight [`DiagBaseRef`] the diagnostics-only
    /// engine takes.
    fn as_diag_ref(&self) -> DiagBaseRef<'_> {
        DiagBaseRef {
            sanitized: &self.sanitized,
            source_nodes: &self.source_nodes,
            pairs: &self.pairs,
            diagnostics: &self.diagnostics,
            index: &self.index,
        }
    }
}

/// Per-document state holder for the LSP backend.
///
/// Keeps the store-free `DiagBase` of the most recent parse so the
/// `publishDiagnostics` hot path answers in O(1) and splices incrementally in
/// `O(region)`, plus a lazily-materialised full [`OwnedLexOutput`] so the rare
/// structural request (rename) can still get a borrowed [`Tree`] via
/// [`Self::with_tree`] (#237 Tier 1).
#[derive(Debug, Default)]
pub struct ParseCache {
    /// Latest source text. Owned so reads don't have to borrow back
    /// into the parent `OpenDocument`, and so the borrowed [`Tree`] view
    /// handed out by [`Self::with_tree`] can borrow it alongside the lazily
    /// materialised output.
    text: String,
    /// Store-free diagnostics base of the most recent parse. `None` until the
    /// first parse, or when the document is empty / oversized (those store no
    /// base, so [`Self::with_tree`] degrades to `None`).
    base: Option<DiagBase>,
    /// Lazily-materialised full owned output for structural requests
    /// ([`Self::with_tree`]). Seeded eagerly by [`Self::reparse_full`] (so a
    /// rename right after a full parse is instant) and reset to an empty
    /// [`OnceLock`] on every incremental splice (so the next structural request
    /// full-parses the current text once and memoises it).
    tree: OnceLock<OwnedLexOutput>,
    /// Consecutive incremental splices since the last full parse. Once this
    /// reaches [`MAX_SPLICES_BEFORE_FULL`] the next reparse forces a full parse
    /// (re-seeding the lazy tree) and resets it to `0` (#249).
    splices_since_full: u32,
}

impl ParseCache {
    /// Re-parse `text` from scratch. Returns the diagnostics plus per-call
    /// statistics (`cache_hits == 0` — every parse re-lexes the whole
    /// document under the current foundation).
    pub fn reparse(&mut self, text: &str) -> (Vec<Diagnostic>, ReparseStats) {
        self.reparse_full(text)
    }

    /// Re-parse `text` after `edits`, taking the **diagnostics-only** incremental
    /// splice ([`aozora::reparse_incremental_diagnostics_only`]) when it can be
    /// proven byte-identical to a full parse, and falling back to a full parse
    /// otherwise. Reports `cache_hits` = reused nodes, `cache_misses` = re-lexed
    /// nodes on the fast path (#237 Tier 1).
    ///
    /// On the fast path this never builds an [`OwnedLexOutput`]: it stores the
    /// spliced `DiagBase` and invalidates the lazy [`Self::with_tree`] cache,
    /// so the per-keystroke cost is `O(region + #diagnostics)` rather than
    /// `O(doc)`.
    ///
    /// The result is **always** identical to a from-scratch parse of `text`: the
    /// splice itself returns `None` for anything it cannot prove local, so the
    /// LSP can never desync — at worst it full-parses.
    ///
    /// The fast path applies only when **all** of these hold (else full parse):
    ///
    /// 1. `edits` is exactly one [`ByteEdit`].
    /// 2. A prior parse exists (a stored owned output).
    /// 3. The source-coordinate edit can be expressed as a sanitized-coordinate
    ///    edit:
    ///    - **LF-clean** (`output.sanitized == self.text`): source == sanitized,
    ///      so the source-coordinate edit equals the sanitized-coordinate edit
    ///      and `text` equals the new sanitized text — spliced directly.
    ///    - **BOM + CRLF** (`output.sanitized != self.text`): the source edit is
    ///      mapped through `san_offset` to sanitized coordinates and spliced
    ///      against the true `sanitize(text)` (see `try_incremental_crlf`).
    ///      The "LF-clean only" limitation is therefore lifted for real
    ///      aozora-bunko files (BOM-prefixed, CRLF line endings). The CRLF path
    ///      still declines (→ full parse) when sanitize would do more than
    ///      BOM-strip + CRLF→LF: an accent decomposition or PUA-collision (both
    ///      emit a sanitize diagnostic a region re-lex cannot reproduce, so the
    ///      diagnostic could be lost), a decorative-rule line (silent
    ///      offset-changing blank-line insertion), or an edit that splits a CRLF
    ///      pair / lands inside the stripped BOM (unmappable offset).
    /// 4. Fewer than `MAX_SPLICES_BEFORE_FULL` splices since the last full
    ///    parse (the dead-entry capacity bound, #249).
    /// 5. The text is non-empty and within `MAX_DOCUMENT_BYTES` (mirrors the
    ///    full-parse guard).
    ///
    /// Correctness in every case rests on [`aozora::reparse_incremental_owned`]
    /// independently validating that the mapped sanitized edit transforms the
    /// cached sanitized text into the true `sanitize(text)`: a bad mapping can
    /// only make it return `None` (→ full parse), never a wrong splice.
    pub fn reparse_incremental(
        &mut self,
        text: &str,
        edits: &[ByteEdit],
    ) -> (Vec<Diagnostic>, ReparseStats) {
        let started_at = Instant::now();

        // Fast-path precondition gate. The clauses shared by both branches
        // (single edit, prior present, splice-count bound, non-empty /
        // non-oversize) are necessary for the sanitized-coordinate splice
        // contract; any miss falls back to a full parse (trivially correct).
        // Each branch yields `(new_sanitized, splice)`: the new sanitized buffer
        // becomes the next base, and the splice carries the spliced diagnostics
        // + the store-free tables.
        let splice = if let [edit] = edits {
            match self.base.as_ref() {
                Some(prior)
                    if self.splices_since_full < MAX_SPLICES_BEFORE_FULL
                        && !text.is_empty()
                        && text.len() <= MAX_DOCUMENT_BYTES =>
                {
                    if prior.sanitized == self.text {
                        // LF-clean fixed point: source == sanitized, so the
                        // source-coordinate edit is already in sanitized
                        // coordinates and `text` is already the new sanitized
                        // text. Splice directly.
                        aozora::reparse_incremental_diagnostics_only(
                            prior.as_diag_ref(),
                            text,
                            edit.range.clone(),
                        )
                        .map(|d| (text.to_owned(), d))
                    } else {
                        // BOM + CRLF source: map the source edit to sanitized
                        // coordinates and splice against the true sanitize(text).
                        self.try_incremental_crlf(prior, text, edit)
                    }
                }
                _ => None,
            }
        } else {
            None
        };

        let Some((new_sanitized, mut splice)) = splice else {
            // Any precondition miss or a splice decline (`None`) → full parse,
            // which re-seeds the lazy tree and resets `splices_since_full`.
            return self.reparse_full(text);
        };

        // The splice does not guarantee globally position-sorted diagnostics
        // (it concatenates prefix/region/suffix slices); the LSP surface needs
        // them sorted, exactly as `reparse_full` does at store time.
        splice.diagnostics.sort_by(diagnostic_order);
        let diagnostics = splice.diagnostics.clone();
        // Build the next edit's region-find index over the spliced tables — the
        // same O(N) pass that already assembled them (free relative to the base
        // maintenance), measured before `latency_us` so it counts toward the
        // per-edit cost.
        let index = RegionIndex::build(&splice.source_nodes, &splice.pairs, &splice.diagnostics);
        let latency_us = duration_as_us(started_at.elapsed());

        text.clone_into(&mut self.text);
        self.base = Some(DiagBase {
            sanitized: new_sanitized,
            diagnostics: splice.diagnostics,
            source_nodes: splice.source_nodes,
            pairs: splice.pairs,
            index,
        });
        // Invalidate the lazily-materialised tree: the next structural request
        // full-parses the new text once and memoises it.
        self.tree = OnceLock::new();
        self.splices_since_full += 1;

        let stats = ReparseStats {
            parse_count: 1,
            cache_hits: splice.reused_nodes,
            cache_misses: splice.relexed_nodes,
            cache_entries_after: 1,
            cache_bytes_estimate: u64::try_from(text.len()).unwrap_or(u64::MAX),
            latency_us,
        };
        (diagnostics, stats)
    }

    /// Attempt the incremental splice for a non-fixed-point (BOM + CRLF) source.
    ///
    /// `new_raw` is the new document text (source coordinates); `prior` is the
    /// cached owned output whose `sanitized` is `sanitize(self.text)`; `edit`
    /// describes the change applied to `self.text` (the **old** raw source) to
    /// obtain `new_raw`, so its `range` is in old-raw coordinates.
    ///
    /// Returns the splice, or `None` (→ the caller full-parses) when the source
    /// is not a pure BOM-strip + CRLF→LF case or the edit cannot be mapped:
    ///
    /// - **Simple-doc gate.** If `sanitize(new_raw)` emits any diagnostic
    ///   (accent decomposition inside `〔…〕`, or a PUA-sentinel collision), we
    ///   decline: a region re-lex works on already-sanitized text and cannot
    ///   reproduce that sanitize-stage diagnostic, so a splice could silently
    ///   drop it. If `new_raw` carries a decorative-rule line, we decline too —
    ///   rule isolation inserts a blank line (offset-changing) **without** a
    ///   diagnostic, which [`san_offset`] does not model.
    /// - **Offset map.** Both edit endpoints are mapped from old-raw to
    ///   old-sanitized coordinates against `self.text` via [`san_offset`], which
    ///   returns `None` for an endpoint inside the stripped BOM or splitting a
    ///   CRLF pair.
    ///
    /// The splice runs against `sanitize(new_raw).text` — the **true** sanitize
    /// of the new source — so
    /// [`aozora::reparse_incremental_diagnostics_only`]'s own validation
    /// backstops any mapping error with a decline, never a wrong splice.
    ///
    /// Returns `(new_sanitized, splice)` on success: the new sanitized buffer
    /// (which becomes the next `DiagBase::sanitized`) and the diagnostics-only
    /// splice.
    fn try_incremental_crlf(
        &self,
        prior: &DiagBase,
        new_raw: &str,
        edit: &ByteEdit,
    ) -> Option<(String, DiagSplice)> {
        // Simple-doc gate: only a pure BOM-strip + CRLF→LF transformation is
        // offset-modelled by `san_offset`. Any sanitize diagnostic (accent /
        // PUA) would be unreproducible by a region re-lex; a decorative rule
        // line is a silent offset-changing edit.
        let san_out = sanitize(new_raw);
        if !san_out.diagnostics.is_empty() {
            return None;
        }
        if has_long_rule_line(new_raw) {
            return None;
        }

        // Map the edit endpoints from old-raw to old-sanitized coordinates
        // (against `self.text`, the old raw source the edit describes).
        let start = san_offset(&self.text, edit.range.start)?;
        let end = san_offset(&self.text, edit.range.end)?;

        // Splice against the true sanitize(new_raw) (reusing the buffer already
        // computed for the gate — never sanitize twice).
        let splice = aozora::reparse_incremental_diagnostics_only(
            prior.as_diag_ref(),
            &san_out.text,
            start..end,
        )?;
        Some((san_out.text.into_owned(), splice))
    }

    /// Core full re-parse. Derives the store-free `DiagBase` (with diagnostics
    /// sorted into position order) and eagerly seeds the lazy [`Self::with_tree`]
    /// cache with the full output, stores the text, and reports per-call
    /// statistics. This is the periodic-compaction point (#249).
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

        // A full parse re-seeds the lazy tree and resets the splice counter,
        // regardless of which return path runs below (#249).
        self.splices_since_full = 0;

        // Skip the O(n) parse for empty or oversized documents (see
        // `MAX_DOCUMENT_BYTES`). Store the text so size checks stay
        // consistent, store no base / no tree — the backend publishes a single
        // "too large" notice for oversized text, and empty text has nothing
        // to surface — and report a zero-parse reparse so metrics don't count
        // phantom work. With no stored base, `with_tree` degrades to `None`.
        if text.is_empty() || text.len() > MAX_DOCUMENT_BYTES {
            text.clone_into(&mut self.text);
            self.base = None;
            self.tree = OnceLock::new();
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
        // Derive the store-free splice base from the full output, then seed the
        // lazy tree with the full output itself (so a structural request right
        // after a full parse is instant).
        let index = RegionIndex::build(&out.source_nodes, &out.pairs, &out.diagnostics);
        self.base = Some(DiagBase {
            sanitized: out.sanitized.clone(),
            diagnostics: out.diagnostics.clone(),
            source_nodes: out.source_nodes.clone(),
            pairs: out.pairs.clone(),
            index,
        });
        self.tree = OnceLock::new();
        // The lock is freshly empty, so `set` always succeeds; the only error
        // is "already initialised" (impossible here), which would hand `out`
        // back — drop it explicitly rather than bind-and-drop the `Result`.
        drop(self.tree.set(out));

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
        self.base.as_ref().map_or(&[][..], |b| &b.diagnostics)
    }

    /// Run `f` against a borrowed [`Tree`] over the most recent parse.
    /// Returns the closure's result, or `None` when there is no stored base
    /// — before the first [`Self::reparse`], for empty text, or for an
    /// oversized document (see `MAX_DOCUMENT_BYTES`, which skips the parse).
    ///
    /// **Lazy** (#237 Tier 1): the per-keystroke hot path stores only the
    /// store-free `DiagBase`, so the full [`OwnedLexOutput`] is materialised
    /// here on first access — `O(doc)`, but only for the rare structural request
    /// (rename) that needs the tree — and memoised in a [`OnceLock`]. A full
    /// parse seeds the lock eagerly, so a structural request immediately after
    /// one is instant; the memo is shared across the `prepare_rename` → `rename`
    /// pair and reset on every incremental splice.
    pub fn with_tree<R>(&self, f: impl FnOnce(&Tree<'_>) -> R) -> Option<R> {
        // No base ⇒ never-parsed / empty / oversized ⇒ degrade to `None`
        // (and never trigger the lazy full parse below).
        self.base.as_ref()?;
        let tree = self
            .tree
            .get_or_init(|| Document::new(self.text.as_str()).parse_owned());
        Some(f(&Tree::view(&self.text, tree)))
    }

    /// Whether any text has been parsed yet.
    #[cfg(test)]
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.text.is_empty() && self.base.is_none()
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

/// Map a raw byte offset `b` to its offset in `sanitize(raw)` for the
/// BOM-strip + CRLF→LF case (the only transformation the CRLF fast path
/// admits; richer sanitations are gated out before this is called).
///
/// The sanitize stage strips every leading `U+FEFF` (3 bytes each) and
/// rewrites `\r\n` → `\n` (−1 byte per pair) and lone `\r` → `\n`
/// (offset-neutral, 1 byte → 1 byte). So the sanitized offset is `b` minus the
/// leading-BOM bytes minus the number of CRLF pairs that lie fully before `b`.
///
/// Returns `None` for an offset that cannot be mapped:
/// - `b` lands inside the stripped leading BOM (`b < bom`); or
/// - `b` splits a `\r\n` pair (sits exactly on the `\n`), which has no
///   counterpart in the collapsed sanitized text.
fn san_offset(raw: &str, b: usize) -> Option<usize> {
    let bytes = raw.as_bytes();

    // Leading-BOM run: sanitize strips every leading `U+FEFF` (`EF BB BF`).
    let mut bom = 0usize;
    while raw[bom..].starts_with('\u{FEFF}') {
        bom += '\u{FEFF}'.len_utf8();
    }
    if b < bom {
        // The edit endpoint is inside bytes sanitize removes entirely.
        return None;
    }

    // Count CRLF pairs that lie fully before `b`; each collapses 2 → 1 byte.
    let mut crlf_pairs = 0usize;
    let mut i = bom;
    while i < b {
        if bytes[i] == b'\r' && i + 1 < bytes.len() && bytes[i + 1] == b'\n' {
            if i + 1 == b {
                // `b` sits on the `\n` of a CRLF pair — unmappable.
                return None;
            }
            // `i < b` and `i + 1 != b` ⇒ `i + 1 < b`: the pair is fully before
            // `b`, so it contributes one collapsed byte. A lone `\r` (no
            // following `\n`) is offset-neutral and falls through to `i += 1`.
            crlf_pairs += 1;
            i += 2;
            continue;
        }
        i += 1;
    }
    Some(b - bom - crlf_pairs)
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
    fn single_interior_edit_reuses_flanking_nodes() {
        // LF-clean (source == sanitized) three-paragraph doc; the first
        // paragraph carries a ruby node, the edit lands in the plain middle
        // paragraph. The re-lexed region is the middle paragraph alone, so the
        // ruby in the reused prefix is carried unchanged → a non-zero hit.
        let mut cache = ParseCache::default();
        let old = "｜青空《あおぞら》のした\n\nかきくけこ\n\nさしすせそ\n";
        drop(cache.reparse(old));

        // Insert one plain kana inside the middle paragraph "かきくけこ".
        let at = old.find("くけこ").unwrap();
        let edit = ByteEdit::new(at..at, "も".to_owned());
        let mut new_text = String::with_capacity(old.len() + "も".len());
        new_text.push_str(&old[..at]);
        new_text.push('も');
        new_text.push_str(&old[at..]);
        let (diags, stats) = cache.reparse_incremental(&new_text, &[edit]);

        assert!(
            stats.cache_hits > 0,
            "the prefix ruby must be reused on the fast path: {stats:?}",
        );
        // Diagnostics identical to a from-scratch parse of the edited text.
        let mut fresh = ParseCache::default();
        let (want, _) = fresh.reparse(&new_text);
        let as_debug = |ds: &[Diagnostic]| ds.iter().map(|d| format!("{d:?}")).collect::<Vec<_>>();
        assert_eq!(as_debug(&diags), as_debug(&want));
    }

    #[test]
    fn oversized_single_edit_skips_incremental() {
        let mut cache = ParseCache::default();
        drop(cache.reparse("｜青空《あおぞら》\n\nほん\n"));
        let big = "a".repeat(MAX_DOCUMENT_BYTES + 1);
        let edit = ByteEdit::new(0..0, big.clone());
        let (diags, stats) = cache.reparse_incremental(&big, &[edit]);
        assert!(diags.is_empty(), "oversized incremental parse is skipped");
        assert_eq!(stats.parse_count, 0, "no parse when oversized");
        assert_eq!(stats.cache_hits, 0, "no reuse for a skipped parse");
    }

    #[test]
    fn splice_run_forces_full_parse_at_capacity_bound() {
        // After MAX_SPLICES_BEFORE_FULL consecutive single LF-clean edits, the
        // next reparse must full-parse (dead-entry bound) and reset the counter.
        let mut cache = ParseCache::default();
        let mut text = "｜青空《あおぞら》のした\n\nかきくけこ\n\nさしすせそ\n".to_owned();
        drop(cache.reparse(&text));

        // Drive exactly MAX_SPLICES_BEFORE_FULL splices, each a one-char insert
        // inside the middle paragraph.
        for _ in 0..MAX_SPLICES_BEFORE_FULL {
            let at = text.find("けこ").unwrap();
            let edit = ByteEdit::new(at..at, "も".to_owned());
            let mut new_text = String::with_capacity(text.len() + "も".len());
            new_text.push_str(&text[..at]);
            new_text.push('も');
            new_text.push_str(&text[at..]);
            let (_, stats) = cache.reparse_incremental(&new_text, &[edit]);
            assert!(stats.cache_hits > 0, "each splice reuses the prefix ruby");
            text = new_text;
        }
        assert_eq!(cache.splices_since_full, MAX_SPLICES_BEFORE_FULL);

        // The next single edit is over the bound → forced full parse, counter
        // resets to zero.
        let at = text.find("けこ").unwrap();
        let edit = ByteEdit::new(at..at, "も".to_owned());
        let mut new_text = String::with_capacity(text.len() + "も".len());
        new_text.push_str(&text[..at]);
        new_text.push('も');
        new_text.push_str(&text[at..]);
        let (_, stats) = cache.reparse_incremental(&new_text, &[edit]);
        assert_eq!(
            stats.cache_hits, 0,
            "the capacity-bound reparse must be a full parse: {stats:?}",
        );
        assert_eq!(
            cache.splices_since_full, 0,
            "a full parse resets the dead-entry counter",
        );
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

        // The paragraphs are plain prose (no classified nodes), so even when the
        // splice fast path fires it reuses zero nodes; the diagnostics must
        // still equal a from-scratch parse of the edited text.
        assert_eq!(
            stats.cache_hits, 0,
            "plain prose reuses no nodes: {stats:?}"
        );
        let mut fresh = ParseCache::default();
        let (want, _) = fresh.reparse(&new_text);
        let as_debug = |ds: &[Diagnostic]| ds.iter().map(|d| format!("{d:?}")).collect::<Vec<_>>();
        assert_eq!(as_debug(&diags), as_debug(&want));
    }

    #[test]
    fn deliberately_wrong_edit_range_still_equals_full() {
        // The bogus edit range does not transform the cached text into the new
        // text (bytes outside it differ), so the splice declines and the cache
        // full-parses — the result must equal a from-scratch parse regardless.
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

    // ---- CRLF / BOM incremental fast path (#237 follow-up) -----------------

    /// Render diagnostics to a comparable `Vec<String>`.
    fn diag_debug(ds: &[Diagnostic]) -> Vec<String> {
        ds.iter().map(|d| format!("{d:?}")).collect()
    }

    /// HTML of the cache's stored output, or `None` when nothing is stored.
    #[expect(
        clippy::redundant_closure_for_method_calls,
        reason = "`Tree::to_html` as a fn item fails `with_tree`'s `for<'a> FnOnce(&Tree<'a>)` HRTB bound; the closure is required"
    )]
    fn html_of(cache: &ParseCache) -> Option<String> {
        cache.with_tree(|t| t.to_html())
    }

    /// Build `(new_raw, edit)` for inserting `ins` immediately before the first
    /// occurrence of `needle` in `old`. The `edit.range` is in `old`-raw
    /// (source) coordinates, exactly as the LSP buffer reports it.
    fn interior_insert(old: &str, needle: &str, ins: &str) -> (String, ByteEdit) {
        let at = old.find(needle).expect("needle present in old text");
        let edit = ByteEdit::new(at..at, ins.to_owned());
        let mut new_raw = String::with_capacity(old.len() + ins.len());
        new_raw.push_str(&old[..at]);
        new_raw.push_str(ins);
        new_raw.push_str(&old[at..]);
        (new_raw, edit)
    }

    #[test]
    fn crlf_interior_edit_reuses_flanking_nodes() {
        // Three CRLF paragraphs (\r\n\r\n separators); the first carries a ruby
        // node, the edit lands in the plain middle paragraph. Source != sanitized
        // (CRLF), so the BOM+CRLF branch maps the edit; the prefix ruby is reused.
        let mut cache = ParseCache::default();
        let old = "｜青空《あおぞら》のした\r\n\r\nかきくけこ\r\n\r\nさしすせそ\r\n";
        drop(cache.reparse(old));

        let (new_raw, edit) = interior_insert(old, "くけこ", "も");
        let (diags, stats) = cache.reparse_incremental(&new_raw, &[edit]);

        assert!(
            stats.cache_hits > 0,
            "the prefix ruby must be reused on the CRLF fast path: {stats:?}",
        );
        let mut fresh = ParseCache::default();
        let (want, _) = fresh.reparse(&new_raw);
        assert_eq!(diag_debug(&diags), diag_debug(&want));
    }

    #[test]
    fn crlf_incremental_byte_identical_to_full() {
        // After a CRLF splice the stored base + the lazily-materialised tree must
        // match a fresh full parse of the new raw text across every surface the
        // LSP exposes (sanitized + diagnostics from the base; HTML via the lazy
        // tree, which full-parses the stored text on first `with_tree`).
        let mut cache = ParseCache::default();
        let old = "｜青空《あおぞら》のした\r\n\r\nかきくけこ\r\n\r\nさしすせそ\r\n";
        drop(cache.reparse(old));
        let (new_raw, edit) = interior_insert(old, "くけこ", "も");
        let (_, stats) = cache.reparse_incremental(&new_raw, &[edit]);
        assert!(stats.cache_hits > 0, "CRLF fast path must fire: {stats:?}");

        let spliced_san = cache.base.as_ref().expect("stored base").sanitized.clone();
        let spliced_diags = diag_debug(cache.diagnostics());
        let spliced_html = html_of(&cache).expect("spliced html");

        let mut fresh = ParseCache::default();
        drop(fresh.reparse(&new_raw));
        let full_san = fresh.base.as_ref().expect("stored base").sanitized.clone();
        let full_diags = diag_debug(fresh.diagnostics());
        let full_html = html_of(&fresh).expect("full html");

        assert_eq!(spliced_san, full_san, "sanitized differs");
        assert_eq!(spliced_html, full_html, "rendered HTML differs");
        assert_eq!(spliced_diags, full_diags, "diagnostics differ");
    }

    #[test]
    fn bom_plus_crlf_interior_edit_hits() {
        // Leading U+FEFF BOM + CRLF line endings — the realistic aozora-bunko
        // shape. The fast path must still fire and match a full parse.
        let mut cache = ParseCache::default();
        let old = "\u{FEFF}｜青空《あおぞら》のした\r\n\r\nかきくけこ\r\n\r\nさしすせそ\r\n";
        drop(cache.reparse(old));
        let (new_raw, edit) = interior_insert(old, "くけこ", "も");
        let (diags, stats) = cache.reparse_incremental(&new_raw, &[edit]);

        assert!(
            stats.cache_hits > 0,
            "BOM+CRLF fast path must reuse the prefix ruby: {stats:?}",
        );
        let mut fresh = ParseCache::default();
        let (want, _) = fresh.reparse(&new_raw);
        assert_eq!(diag_debug(&diags), diag_debug(&want));
        let full_san = &fresh.base.as_ref().expect("stored base").sanitized;
        let spliced_san = &cache.base.as_ref().expect("stored base").sanitized;
        assert_eq!(spliced_san, full_san);
        // The lazily-materialised spliced tree renders identically to a fresh
        // full parse (both full-parse the same stored text).
        assert_eq!(html_of(&cache), html_of(&fresh));
    }

    #[test]
    fn accent_doc_declines_and_matches_full() {
        // A CRLF doc whose `〔…〕` span carries an accent digraph: sanitize emits
        // an accent-decomposition Note a region re-lex cannot reproduce, so the
        // CRLF fast path declines and full-parses — with the decomposition and
        // the Note intact.
        let mut cache = ParseCache::default();
        let old = "じょぶんです。\r\n\r\n〔oraison fune`bre〕\r\n\r\nまつびです。\r\n";
        drop(cache.reparse(old));
        let (new_raw, edit) = interior_insert(old, "まつび", "も");
        let (diags, stats) = cache.reparse_incremental(&new_raw, &[edit]);

        assert_eq!(stats.cache_hits, 0, "accent doc must decline: {stats:?}");
        let mut fresh = ParseCache::default();
        let (want, _) = fresh.reparse(&new_raw);
        assert_eq!(diag_debug(&diags), diag_debug(&want));
        let html = html_of(&cache).expect("html");
        assert!(
            html.contains("funèbre"),
            "accent decomposition must survive: {html}",
        );
    }

    #[test]
    fn decorative_rule_doc_declines() {
        // A CRLF doc with a >=10-char decorative rule line. Rule isolation
        // inserts a blank line silently (offset-changing, no diagnostic), which
        // the offset map does not model, so the fast path declines via
        // `has_long_rule_line` and full-parses identically.
        let mut cache = ParseCache::default();
        let old = "まえがき。\r\n----------\r\n\r\nほんぶんです。\r\n";
        drop(cache.reparse(old));
        let (new_raw, edit) = interior_insert(old, "ほんぶん", "も");
        let (diags, stats) = cache.reparse_incremental(&new_raw, &[edit]);

        assert_eq!(
            stats.cache_hits, 0,
            "decorative-rule doc must decline: {stats:?}",
        );
        let mut fresh = ParseCache::default();
        let (want, _) = fresh.reparse(&new_raw);
        assert_eq!(diag_debug(&diags), diag_debug(&want));
    }

    #[test]
    fn pua_doc_declines_preserves_diagnostic() {
        // A CRLF doc carrying a raw U+E001 sentinel: sanitize emits
        // SourceContainsPua, so the fast path declines and the full parse keeps
        // the diagnostic that a region re-lex would have dropped.
        let mut cache = ParseCache::default();
        let old = "まえがき。\r\n\r\nあ\u{E001}い\r\n\r\nまつびです。\r\n";
        drop(cache.reparse(old));
        let (new_raw, edit) = interior_insert(old, "まつび", "も");
        let (diags, stats) = cache.reparse_incremental(&new_raw, &[edit]);

        assert_eq!(stats.cache_hits, 0, "PUA doc must decline: {stats:?}");
        assert!(
            diags
                .iter()
                .any(|d| matches!(d, Diagnostic::SourceContainsPua { .. })),
            "the PUA diagnostic must survive the full-parse fallback: {diags:?}",
        );
        let mut fresh = ParseCache::default();
        let (want, _) = fresh.reparse(&new_raw);
        assert_eq!(diag_debug(&diags), diag_debug(&want));
    }

    // ---- san_offset unit tests --------------------------------------------

    #[test]
    fn san_offset_pure_ascii_is_identity() {
        // No BOM, no CR: sanitize is the identity, so offsets are unchanged.
        let raw = "abcdef";
        for b in 0..=raw.len() {
            assert_eq!(san_offset(raw, b), Some(b), "offset {b}");
        }
    }

    #[test]
    fn san_offset_leading_bom_drops_three() {
        // One leading BOM (3 bytes) is stripped; a byte after it shifts by −3.
        let raw = "\u{FEFF}abc";
        assert_eq!(san_offset(raw, 3), Some(0), "first real byte");
        assert_eq!(san_offset(raw, 4), Some(1));
    }

    #[test]
    fn san_offset_inside_bom_is_unmappable() {
        let raw = "\u{FEFF}abc";
        assert_eq!(san_offset(raw, 1), None, "inside the stripped BOM");
        assert_eq!(san_offset(raw, 2), None);
    }

    #[test]
    fn san_offset_counts_crlf_pairs() {
        // "a\r\nb\r\nc": each fully-elapsed CRLF pair collapses 2 → 1 byte.
        let raw = "a\r\nb\r\nc";
        assert_eq!(san_offset(raw, 0), Some(0), "before any CRLF");
        assert_eq!(san_offset(raw, 3), Some(2), "after one CRLF (b)");
        assert_eq!(san_offset(raw, 6), Some(4), "after two CRLFs (c)");
    }

    #[test]
    fn san_offset_lone_cr_is_neutral() {
        // A lone `\r` (no following `\n`) becomes `\n`: 1 byte → 1 byte, neutral.
        let raw = "a\rb";
        assert_eq!(san_offset(raw, 2), Some(2), "lone CR does not shift");
    }

    #[test]
    fn san_offset_split_crlf_is_unmappable() {
        // An offset sitting on the `\n` of a CRLF pair has no sanitized image.
        let raw = "a\r\nb";
        assert_eq!(san_offset(raw, 2), None);
    }
}
