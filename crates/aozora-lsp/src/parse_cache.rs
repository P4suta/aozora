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
//! splice ([`aozora::reparse_incremental_diagnostics_only_in`]) on a single edit
//! and full-parses otherwise (#237 Tier 1).
//!
//! # Incremental sanitized rope (#237 Tier 2, Mechanism B)
//!
//! The cache holds both the raw source and the sanitized buffer as
//! [`ropey::Rope`]s. The per-keystroke hot path **splices the sanitized rope
//! incrementally** instead of re-running the `O(doc)` `sanitize` pass: a single
//! raw-coordinate edit is mapped to a sanitized-coordinate edit by a
//! raw↔sanitized **line correspondence** (the rope's `O(log n)` line metrics
//! absorb CRLF folding and decorative-rule isolation-blank insertion), the
//! sanitized rope is spliced, and the engine is fed zero-copy `RopeSrc` views of
//! the cached and edited buffers. A wide trigger gate declines any edit whose
//! raw→sanitized mapping is not byte-local, and a windowed re-sanitize verifies
//! every accepted splice against the raw text as an independent ground truth
//! before committing, so the result is always byte-identical to a full parse.
//!
//! # Lazy tree (#237 Tier 1)
//!
//! A consumer trace established that the per-keystroke hot path (debounced
//! `publishDiagnostics`) reads only [`ParseCache::diagnostics`]; the full
//! [`Tree`] (via [`ParseCache::with_tree`]) is needed only by the rare F2
//! rename gesture. So the cache keeps a **store-free** `DiagBase` (sanitized
//! text + diagnostics + the maintained [`PieceSeq`] the next edit's region-find
//! needs) that the hot path splices in `O(region + #pieces)`, and
//! materialises the full `O(doc)` [`OwnedLexOutput`] **lazily** — only when
//! [`ParseCache::with_tree`] is actually called — memoised in a [`OnceLock`]
//! (seeded eagerly by a full parse so a structural request right after one is
//! instant, and invalidated on every incremental splice).

use std::cmp::Ordering;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use aozora::pipeline::lexer::sanitize::{is_rule_line_trimmed, sanitize};
use aozora::{DiagBaseRef, DiagSplice, Diagnostic, Document, OwnedLexOutput, PieceSeq, Tree};
use ropey::{Rope, RopeSlice};
use tracing::field::Empty as TracingEmpty;

use crate::rope_src::RopeSrc;
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
/// lazy tree and re-grounds the sanitized rope, the `bom`/`isolation_lines`
/// side-tables, and the splice base on a fresh `sanitize` (#249, the
/// periodic-compaction bound).
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

/// Document-level `sanitize` properties recomputed by each full parse from the
/// raw text. Each is a hard incremental decline: a `true` flag means the raw
/// text is **not** a sanitize fixed point in a way the line-correspondence map
/// cannot model, so every edit full-parses until the next full re-ground.
#[derive(Debug, Default, Clone, Copy)]
struct DocFlags {
    /// A lone `\r` (not part of `\r\n`) — sanitize folds it to `\n`, which would
    /// change the line count and break the raw↔sanitized line map globally.
    has_lone_cr: bool,
    /// A `〔` anywhere — an accent-decomposition span makes raw ≠ sanitized
    /// inside it (and a silent `` ` `` / `'` marker can decompose), so the
    /// within-line byte-identity the map relies on can fail.
    has_tortoise: bool,
    /// A raw `U+E001..U+E004` sentinel — sanitize rewrites it to `U+FFFD`, so
    /// raw ≠ sanitized at that byte (and the collision diagnostic must survive).
    has_pua: bool,
}

/// The store-free spliceable base of the most recent parse: exactly the fields
/// the diagnostics-only hot path
/// ([`aozora::reparse_incremental_diagnostics_only_in`]) reads from the prior
/// parse and produces for the next one, plus the raw↔sanitized line-map
/// side-tables. Kept across edits so the per-keystroke path never has to
/// materialise the full [`OwnedLexOutput`] or re-run `O(doc)` `sanitize`.
#[derive(Debug)]
struct DiagBase {
    /// Sanitized buffer of the most recent parse (a sanitize fixed point) — the
    /// coordinate space the splice and the next region-find operate in. A
    /// [`Rope`] so the hot path can splice it in `O(log n)` (COW clone +
    /// `remove`/`insert`) rather than rebuilding a flat `String`.
    sanitized: Rope,
    /// Diagnostics, position-sorted at store time so [`ParseCache::diagnostics`]
    /// reads are O(1) and byte-identical to a full parse. Flattened from
    /// `pieces` after each splice (the maintained sequence is the source of
    /// truth) and re-sorted into positional order for the editor surface.
    diagnostics: Vec<Diagnostic>,
    /// The maintained region-find representation (#237 Tier 2): the parse's
    /// `source_nodes` / `pairs` / `diagnostics` as a structure-sharing
    /// [`PieceSeq`]. The next edit's region-find reads it directly and the hot
    /// path splices it `O(region + #pieces)`, replacing the per-edit whole-table
    /// re-materialization + `RegionIndex` rebuild.
    pieces: PieceSeq,
    /// Byte length of the leading `U+FEFF` BOM run that `sanitize` stripped.
    /// The raw→sanitized map subtracts it for an edit on raw line 0.
    bom: u32,
    /// Sorted raw line indices before which decorative-rule isolation inserted a
    /// blank line. The map adds, for an edit on raw line `L`, the count of
    /// entries `<= L` (the sanitized line is shifted down by that many blanks).
    /// Invariant across a single in-line accept (a rule line is itself a trigger
    /// the gate declines), recomputed on every full parse.
    isolation_lines: Vec<u32>,
    /// Document-level sanitize flags; any set flag declines incremental.
    flags: DocFlags,
}

impl DiagBase {
    /// The sanitized line index of raw line `raw_line`'s content start: the raw
    /// line index plus the number of decorative-rule isolation blanks inserted
    /// at or before it.
    fn san_line(&self, raw_line: usize) -> usize {
        raw_line
            + self
                .isolation_lines
                .partition_point(|&r| (r as usize) <= raw_line)
    }
}

/// Per-document state holder for the LSP backend.
///
/// Keeps the store-free `DiagBase` of the most recent parse so the
/// `publishDiagnostics` hot path answers in O(1) and splices the sanitized rope
/// incrementally in `O(region)`, plus a lazily-materialised full
/// [`OwnedLexOutput`] so the rare structural request (rename) can still get a
/// borrowed [`Tree`] via [`Self::with_tree`] (#237 Tier 1).
#[derive(Debug, Default)]
pub struct ParseCache {
    /// Latest raw source text as a [`Rope`]. Owned so reads don't borrow back
    /// into the parent `OpenDocument`; the splice maps edits against it, and the
    /// rare structural path flattens it to `&str` for the borrowed [`Tree`].
    raw: Rope,
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
        self.reparse_full(&Rope::from(text))
    }

    /// Re-parse the post-edit `raw` rope after `edits`, taking the
    /// **diagnostics-only** incremental splice
    /// ([`aozora::reparse_incremental_diagnostics_only_in`]) when the single
    /// edit can be proven byte-identical to a full parse, and falling back to a
    /// full parse of `raw` otherwise. Reports `cache_hits` = reused nodes,
    /// `cache_misses` = re-lexed nodes on the fast path (#237 Tier 1/2).
    ///
    /// On the fast path this never builds an [`OwnedLexOutput`] and never runs
    /// `O(doc)` `sanitize`: it splices the sanitized [`Rope`], stores the new
    /// `DiagBase`, and invalidates the lazy [`Self::with_tree`] cache, so the
    /// per-keystroke cost is `O(region + #diagnostics)` rather than `O(doc)`.
    ///
    /// The result is **always** identical to a from-scratch parse of `raw`: the
    /// trigger gate declines any edit whose raw→sanitized mapping is not
    /// byte-local, a windowed re-sanitize verifies every accepted splice against
    /// the raw text, and the engine independently declines anything it cannot
    /// prove local — so the LSP can never desync (at worst it full-parses).
    ///
    /// The fast path applies only when **all** of these hold (else full parse):
    ///
    /// 1. `edits` is exactly one [`ByteEdit`].
    /// 2. A prior parse exists, fewer than `MAX_SPLICES_BEFORE_FULL` splices
    ///    have run since the last full parse, and `raw` is non-empty and within
    ///    `MAX_DOCUMENT_BYTES`.
    /// 3. The cached document is a clean sanitize fixed point modulo BOM strip /
    ///    CRLF fold / decorative-rule isolation (no lone `\r`, no `〔`, no raw
    ///    PUA sentinel).
    /// 4. The edit carries no structural byte (no line terminator, `〔` / `〕`,
    ///    or raw PUA) and neither creates nor destroys a decorative-rule line.
    /// 5. The raw→sanitized line-correspondence map, the rope splice, the engine
    ///    splice, and the windowed re-sanitize all succeed.
    pub fn reparse_incremental(
        &mut self,
        raw: &Rope,
        edits: &[ByteEdit],
    ) -> (Vec<Diagnostic>, ReparseStats) {
        let started_at = Instant::now();

        let splice = if let [edit] = edits {
            self.try_incremental(raw, edit)
        } else {
            None
        };

        let Some((new_sanitized, splice)) = splice else {
            // Any precondition miss or a splice decline (`None`) → full parse of
            // the post-edit raw, which re-seeds the lazy tree, recomputes the
            // line-map side-tables, and resets `splices_since_full`.
            return self.reparse_full(raw);
        };

        // Flatten this edit's diagnostics from the maintained `PieceSeq` (`O(D)`,
        // not the `O(N)` node table) and re-sort into the LSP's full positional
        // order — exactly as `reparse_full` does at store time, so the stored
        // slice is byte-identical to a full parse's.
        let mut diagnostics = splice.pieces.collect_diagnostics();
        diagnostics.sort_by(diagnostic_order);
        let cache_hits = splice.reused_nodes;
        let cache_misses = splice.relexed_nodes;
        let latency_us = duration_as_us(started_at.elapsed());

        // Commit. `bom` / `isolation_lines` / `flags` are invariant across a
        // single in-line accept (the trigger gate proves the edit neither
        // touches the BOM run nor changes any line's rule/blank status), so they
        // carry from the prior base unchanged. A splice implies a prior base
        // (`try_incremental` declines without one), so the `else` is unreachable;
        // it full-parses rather than panicking if that invariant ever breaks.
        let Some(prior) = self.base.take() else {
            return self.reparse_full(raw);
        };
        let new_len = raw.len_bytes();
        self.raw = raw.clone();
        let returned = diagnostics.clone();
        self.base = Some(DiagBase {
            sanitized: new_sanitized,
            diagnostics,
            pieces: splice.pieces,
            bom: prior.bom,
            isolation_lines: prior.isolation_lines,
            flags: prior.flags,
        });
        // Invalidate the lazily-materialised tree: the next structural request
        // full-parses the new text once and memoises it.
        self.tree = OnceLock::new();
        self.splices_since_full += 1;

        let stats = ReparseStats {
            parse_count: 1,
            cache_hits,
            cache_misses,
            cache_entries_after: 1,
            cache_bytes_estimate: u64::try_from(new_len).unwrap_or(u64::MAX),
            latency_us,
        };
        (returned, stats)
    }

    /// Attempt the incremental splice for the single edit `edit` producing the
    /// post-edit `new_raw`. Returns `(new_sanitized, splice)` on success, or
    /// `None` (→ the caller full-parses) for any precondition miss.
    fn try_incremental(&self, new_raw: &Rope, edit: &ByteEdit) -> Option<(Rope, DiagSplice)> {
        let prior = self.base.as_ref()?;
        if self.splices_since_full >= MAX_SPLICES_BEFORE_FULL {
            return None;
        }
        incremental_splice(prior, &self.raw, new_raw, edit)
    }

    /// Core full re-parse of the post-edit `raw` rope. Derives the store-free
    /// `DiagBase` (diagnostics position-sorted) plus the raw↔sanitized line-map
    /// side-tables (`bom` / `isolation_lines` / `flags`), eagerly seeds the lazy
    /// [`Self::with_tree`] cache with the full output, stores the raw rope, and
    /// reports per-call statistics. This is the periodic-compaction point (#249).
    #[tracing::instrument(
        level = "debug",
        skip_all,
        fields(
            text_bytes = raw.len_bytes(),
            latency_us = TracingEmpty,
        ),
    )]
    fn reparse_full(&mut self, raw: &Rope) -> (Vec<Diagnostic>, ReparseStats) {
        let started_at = Instant::now();

        // A full parse re-seeds the lazy tree and resets the splice counter,
        // regardless of which return path runs below (#249).
        self.splices_since_full = 0;

        // Skip the O(n) parse for empty or oversized documents (see
        // `MAX_DOCUMENT_BYTES`). Store the raw rope so size checks stay
        // consistent, store no base / no tree, and report a zero-parse reparse.
        let len = raw.len_bytes();
        if len == 0 || len > MAX_DOCUMENT_BYTES {
            self.raw = raw.clone();
            self.base = None;
            self.tree = OnceLock::new();
            let stats = ReparseStats {
                cache_bytes_estimate: u64::try_from(len).unwrap_or(u64::MAX),
                latency_us: duration_as_us(started_at.elapsed()),
                ..ReparseStats::default()
            };
            return (Vec::new(), stats);
        }

        let text = raw.to_string();
        let mut out = Document::new(text.as_str()).parse_owned();
        // `OwnedLexOutput.diagnostics` are in pipeline-stage order; the LSP
        // surface expects them position-sorted. Sort once here at store time so
        // every read is byte-identical and O(1).
        out.diagnostics.sort_by(diagnostic_order);

        let diagnostics = out.diagnostics.clone();
        let latency_us = duration_as_us(started_at.elapsed());

        self.raw = raw.clone();
        // Derive the line-map side-tables from the raw text, then the store-free
        // splice base, then seed the lazy tree with the full output itself (so a
        // structural request right after a full parse is instant).
        let bom = leading_bom_bytes(&text);
        let isolation_lines = isolation_line_indices(text.get(bom as usize..).unwrap_or(&text));
        let flags = scan_doc_flags(&text);
        let pieces = PieceSeq::from_contiguous(
            &out.source_nodes,
            &out.pairs,
            &out.diagnostics,
            out.sanitized_len,
        );
        self.base = Some(DiagBase {
            sanitized: Rope::from(out.sanitized.as_str()),
            diagnostics: out.diagnostics.clone(),
            pieces,
            bom,
            isolation_lines,
            flags,
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
            cache_bytes_estimate: u64::try_from(len).unwrap_or(u64::MAX),
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

    /// Borrow the spliced sanitized buffer of the most recent parse, or `None`
    /// when no base is stored (never parsed / empty / oversized). Exposed for
    /// the crate's own end-to-end tests, which assert it stays byte-identical to
    /// a full `sanitize` after every incremental splice.
    #[cfg(feature = "internals")]
    #[must_use]
    pub fn sanitized(&self) -> Option<&Rope> {
        self.base.as_ref().map(|b| &b.sanitized)
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
        // The rare structural path flattens the raw rope to a `&str` for the
        // parse + the borrowed tree's source. `O(doc)`, off the hot path.
        let text = self.raw.to_string();
        let tree = self
            .tree
            .get_or_init(|| Document::new(text.as_str()).parse_owned());
        Some(f(&Tree::view(&text, tree)))
    }

    /// Whether any text has been parsed yet.
    #[cfg(test)]
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.raw.len_bytes() == 0 && self.base.is_none()
    }
}

/// Build the diagnostics-only splice for the single edit `edit` (in **old /
/// cached** coordinates) producing the post-edit `new_raw`, by mapping the raw
/// edit to a sanitized-coordinate edit, splicing the cached sanitized rope, and
/// running the engine over zero-copy rope views — **without a full re-parse**.
///
/// Returns `(new_sanitized, splice)` on success, or `None` for any edit that
/// cannot be proven byte-local (the caller then full-parses, trivially correct).
fn incremental_splice(
    prior: &DiagBase,
    old_raw: &Rope,
    new_raw: &Rope,
    edit: &ByteEdit,
) -> Option<(Rope, DiagSplice)> {
    let (a, b) = (edit.range.start, edit.range.end);
    let n = &edit.new_text;

    // G0 size / fixed-point clauses. Empty / oversized post-edit doc, or a
    // cached doc that is not a sanitize fixed point in a way the line map cannot
    // model (lone CR / 〔…〕 accent / raw PUA), → decline.
    let new_len = new_raw.len_bytes();
    if new_len == 0 || new_len > MAX_DOCUMENT_BYTES {
        return None;
    }
    if prior.flags.has_lone_cr || prior.flags.has_tortoise || prior.flags.has_pua {
        return None;
    }
    // The edit is in old (cached) coordinates; it must be in bounds of `old_raw`.
    if a > b || b > old_raw.len_bytes() {
        return None;
    }
    // G1 — strict `a > bom`. An insert at `a <= bom` could extend the leading
    // BOM run that sanitize strips, desyncing the line map.
    if a <= prior.bom as usize {
        return None;
    }
    // Trigger gate: structural bytes in the edit, or a rule/blank-status toggle.
    if edit_triggers_decline(prior, old_raw, new_raw, edit) {
        return None;
    }

    // Raw→sanitized line-correspondence map. `a` and `b` share a raw line (the
    // gate rejects line terminators in the edit), so within the line raw == san
    // and the sanitized edit width equals the raw edit width.
    let raw_line = old_raw.byte_to_line(a);
    let SanEdit { a_san, b_san_old } = map_edit(prior, old_raw, a, b)?;

    // Rope splice: COW-clone the cached sanitized rope and replace the mapped
    // range with `n` (which carries no trigger byte ⇒ its raw image equals its
    // sanitized image).
    let mut new_san = prior.sanitized.clone();
    let (ca, cb) = (new_san.byte_to_char(a_san), new_san.byte_to_char(b_san_old));
    new_san.remove(ca..cb);
    new_san.insert(ca, n);

    // O(1) structural invariant (not a divergence detector): the sanitized delta
    // equals the raw delta by construction — `b_san_old == a_san + (b - a)` and
    // the splice removes exactly that span, so for char-aligned endpoints this
    // never fires. It guards only the degenerate mid-codepoint `a_san`/`b_san_old`
    // case (which cannot arise here); the windowed re-sanitize below is the real
    // position check.
    if new_san.len_bytes() != prior.sanitized.len_bytes() + n.len() - (b - a) {
        return None;
    }

    // Engine: diagnostics-only splice over zero-copy rope views, in OLD
    // sanitized coordinates (the engine derives the new end from the delta).
    let splice = {
        let base_ref = DiagBaseRef {
            sanitized: RopeSrc::new(prior.sanitized.byte_slice(..)),
            pieces: &prior.pieces,
        };
        let new_src = RopeSrc::new(new_san.byte_slice(..));
        aozora::reparse_incremental_diagnostics_only_in(&base_ref, &new_src, a_san..b_san_old)?
    };

    // Windowed release re-sanitize — the sole release-time check against the raw
    // text as an independent ground truth (catches a rule break, a
    // predecessor-blank toggle, or a gross offset error the engine's region
    // idempotence cannot, because plain text is a sanitize fixed point anywhere).
    if !windowed_resanitize_ok(prior, new_raw, &new_san, raw_line) {
        return None;
    }
    // Strongest pin, debug only: the whole spliced buffer equals a full
    // sanitize of the new raw text.
    #[cfg(debug_assertions)]
    debug_assert_eq!(
        new_san.to_string(),
        sanitize(&new_raw.to_string()).text.as_ref(),
        "spliced sanitized rope diverged from a full sanitize of the new raw text",
    );

    Some((new_san, splice))
}

/// A sanitized-coordinate edit derived from a raw-coordinate one: replace
/// `a_san..b_san_old` in the cached sanitized buffer.
struct SanEdit {
    a_san: usize,
    b_san_old: usize,
}

/// Map the raw-coordinate edit range `a..b` (both on the same raw line) to the
/// cached sanitized buffer's coordinates via the raw↔sanitized line map.
/// Returns `None` if the mapping lands out of bounds.
fn map_edit(prior: &DiagBase, old_raw: &Rope, a: usize, b: usize) -> Option<SanEdit> {
    let raw_line = old_raw.byte_to_line(a);
    let within = a - old_raw.line_to_byte(raw_line);
    let san_line = prior.san_line(raw_line);
    if san_line > prior.sanitized.len_lines() {
        return None;
    }
    let line_start = prior.sanitized.line_to_byte(san_line);
    // Raw line 0 carries the BOM that sanitize stripped; `within` counts those
    // bytes, so subtract them. Later lines have no BOM.
    let bom_adj = if raw_line == 0 { prior.bom as usize } else { 0 };
    let a_san = (line_start + within).checked_sub(bom_adj)?;
    let b_san_old = a_san + (b - a);
    (b_san_old <= prior.sanitized.len_bytes()).then_some(SanEdit { a_san, b_san_old })
}

/// Whether the edit must decline because it carries a structural byte or toggles
/// a decorative-rule / blank-line status the line map cannot absorb. Scans both
/// the removed (old) and inserted (new) bytes.
fn edit_triggers_decline(
    prior: &DiagBase,
    old_raw: &Rope,
    new_raw: &Rope,
    edit: &ByteEdit,
) -> bool {
    let (a, b) = (edit.range.start, edit.range.end);
    let n = &edit.new_text;
    // T1 (line terminator — moves a line boundary), T2 (`〔` / `〕` — an accent
    // span open/close), T3 (raw PUA in the insert — rewrites to U+FFFD).
    let removed = old_raw.byte_slice(a..b);
    if rope_slice_has(removed, |c| matches!(c, '\n' | '\r' | '〔' | '〕')) {
        return true;
    }
    if n.contains(['\n', '\r', '〔', '〕']) || contains_raw_pua(n) {
        return true;
    }
    // R1 / R2 compare *line content* (a byte-scan of the edit is unsound: e.g.
    // deleting `a` from `a----------` creates a rule with no `-` in the removed
    // bytes). The edit lies within one raw line (T1 above), so old and new
    // differ only on raw line `raw_line`.
    let raw_line = old_raw.byte_to_line(a);
    let old_line = trimmed_line(old_raw, raw_line, prior.bom);
    let new_line = trimmed_line(new_raw, raw_line, prior.bom);
    if is_rule_line_trimmed(&old_line) != is_rule_line_trimmed(&new_line) {
        return true; // R1: the edit creates or destroys a decorative-rule line
    }
    // R2: toggling line `raw_line`'s blankness flips whether the *next* line, if
    // it is a rule, gets an isolation blank — mutating `isolation_lines`.
    old_line.is_empty() != new_line.is_empty() && next_line_is_rule(new_raw, raw_line + 1)
}

/// Whether a windowed re-sanitize of the post-edit raw text around the edit
/// reproduces the spliced sanitized buffer's matching region. The window is the
/// edited raw line `raw_line` plus the next line (so the next line's
/// isolation — which depends on this line's blank status — is verified, seeded
/// by this line); the edited line's own preceding isolation blank is invariant
/// (R1) and excluded by starting the comparison at the line's content.
fn windowed_resanitize_ok(
    prior: &DiagBase,
    new_raw: &Rope,
    new_san: &Rope,
    raw_line: usize,
) -> bool {
    let line_count = new_raw.len_lines();
    let w_hi = (raw_line + 1).min(line_count.saturating_sub(1));

    // Raw window [content of `raw_line` .. end of line `w_hi`].
    let raw_start = new_raw.line_to_byte(raw_line);
    let raw_end = if w_hi + 1 < line_count {
        new_raw.line_to_byte(w_hi + 1)
    } else {
        new_raw.len_bytes()
    };
    let raw_window = String::from(new_raw.byte_slice(raw_start..raw_end));
    let san_window = sanitize(&raw_window);
    if !san_window.diagnostics.is_empty() {
        return false; // a fresh sanitize diagnostic the splice cannot account for
    }

    // Spliced-buffer region for the same raw lines: from raw line `raw_line`'s
    // content start to the end of line `w_hi`'s content (one sanitized line past
    // `w_hi`'s content, which excludes any isolation blank belonging to the line
    // after `w_hi`).
    let san_lines = new_san.len_lines();
    let lo_line = prior.san_line(raw_line);
    let hi_line = prior.san_line(w_hi) + 1;
    if lo_line > san_lines || hi_line > san_lines {
        return false;
    }
    let sb_lo = new_san.line_to_byte(lo_line);
    let sb_end = new_san.line_to_byte(hi_line);
    sb_lo <= sb_end && new_san.byte_slice(sb_lo..sb_end) == san_window.text.as_ref()
}

/// Whether any char of `slice` satisfies `pred`. Used on the small removed-edit
/// span, so the per-char walk is cheap.
fn rope_slice_has(slice: RopeSlice<'_>, pred: impl Fn(char) -> bool) -> bool {
    slice.chars().any(pred)
}

/// The trimmed content of raw line `line_idx`, with the leading BOM stripped on
/// line 0 (so the rule/blank checks match what `sanitize` sees). `str::trim`
/// removes the trailing `\r`, matching the CRLF fold.
fn trimmed_line(raw: &Rope, line_idx: usize, bom: u32) -> String {
    let line = String::from(raw.line(line_idx));
    let body = if line_idx == 0 {
        line.get(bom as usize..).unwrap_or_default()
    } else {
        line.as_str()
    };
    body.trim().to_owned()
}

/// Whether raw line `line_idx` (if it exists) is a decorative-rule line.
fn next_line_is_rule(raw: &Rope, line_idx: usize) -> bool {
    line_idx < raw.len_lines() && is_rule_line_trimmed(String::from(raw.line(line_idx)).trim())
}

/// Whether `s` contains a raw PUA sentinel `U+E001..U+E004` (`EE 80 81..84`),
/// which `sanitize` rewrites to `U+FFFD`.
fn contains_raw_pua(s: &str) -> bool {
    s.as_bytes()
        .windows(3)
        .any(|w| w[0] == 0xEE && w[1] == 0x80 && (0x81..=0x84).contains(&w[2]))
}

/// Byte length of the leading `U+FEFF` BOM run that `sanitize` strips (each BOM
/// is 3 UTF-8 bytes).
fn leading_bom_bytes(s: &str) -> u32 {
    let mut rest = s;
    let mut n = 0u32;
    while let Some(r) = rest.strip_prefix('\u{FEFF}') {
        rest = r;
        n += 3;
    }
    n
}

/// Sorted raw line indices before which decorative-rule isolation inserts a
/// blank line. Mirrors `isolate_decorative_rules`'s `prev_nonblank` bookkeeping
/// (a rule line that follows a visible line); `stripped` is the raw text with
/// the leading BOM removed. `split('\n')` line indices match the rope's line
/// metrics (ropey counts only `\n`; lone-CR docs are declined upstream), and
/// `str::trim` folds the trailing `\r`.
fn isolation_line_indices(stripped: &str) -> Vec<u32> {
    let mut out = Vec::new();
    let mut prev_nonblank = false;
    for (idx, line) in stripped.split('\n').enumerate() {
        let trimmed = line.trim();
        if is_rule_line_trimmed(trimmed) && prev_nonblank {
            out.push(u32::try_from(idx).unwrap_or(u32::MAX));
        }
        prev_nonblank = !trimmed.is_empty();
    }
    out
}

/// Scan the raw text for the document-level sanitize-fixed-point flags.
fn scan_doc_flags(raw: &str) -> DocFlags {
    let bytes = raw.as_bytes();
    let has_lone_cr = bytes
        .iter()
        .enumerate()
        .any(|(i, &c)| c == b'\r' && bytes.get(i + 1) != Some(&b'\n'));
    DocFlags {
        has_lone_cr,
        has_tortoise: raw.contains('〔'),
        has_pua: contains_raw_pua(raw),
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

    /// Re-parse the post-edit `text` (built by the caller) after one `edit`,
    /// routing the raw text through the rope `ParseCache` takes.
    fn reparse_incremental_str(
        cache: &mut ParseCache,
        text: &str,
        edit: ByteEdit,
    ) -> (Vec<Diagnostic>, ReparseStats) {
        cache.reparse_incremental(&Rope::from(text), &[edit])
    }

    /// The stored sanitized buffer as a `String` (in-crate tests read the
    /// private base directly).
    fn stored_sanitized(cache: &ParseCache) -> String {
        cache
            .base
            .as_ref()
            .expect("stored base")
            .sanitized
            .to_string()
    }

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
        let (diags, stats) = reparse_incremental_str(&mut cache, &new_text, edit);

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
        let (diags, stats) = reparse_incremental_str(&mut cache, &big, edit);
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
            let (_, stats) = reparse_incremental_str(&mut cache, &new_text, edit);
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
        let (_, stats) = reparse_incremental_str(&mut cache, &new_text, edit);
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
        let (_, stats) =
            cache.reparse_incremental(&Rope::from("xalpha\n\nbexyta\n\ngamma"), &edits);
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
    fn large_single_edit_splices_and_matches() {
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
        let (diags, stats) = reparse_incremental_str(&mut cache, &new_text, edit);

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
        // full-parses the passed text — the result must equal a from-scratch
        // parse regardless.
        let mut cache = ParseCache::default();
        drop(cache.reparse("alpha\n\nbeta\n\ngamma"));
        let new_text = "alpha\n\nbeta edited\n\ngamma";
        let bogus = ByteEdit::new(0..0, "zzz".to_owned());
        let (diags, _) = reparse_incremental_str(&mut cache, new_text, bogus);
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
        // (CRLF), so the rope line map maps the edit; the prefix ruby is reused.
        let mut cache = ParseCache::default();
        let old = "｜青空《あおぞら》のした\r\n\r\nかきくけこ\r\n\r\nさしすせそ\r\n";
        drop(cache.reparse(old));

        let (new_raw, edit) = interior_insert(old, "くけこ", "も");
        let (diags, stats) = reparse_incremental_str(&mut cache, &new_raw, edit);

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
        let (_, stats) = reparse_incremental_str(&mut cache, &new_raw, edit);
        assert!(stats.cache_hits > 0, "CRLF fast path must fire: {stats:?}");

        let spliced_san = stored_sanitized(&cache);
        let spliced_diags = diag_debug(cache.diagnostics());
        let spliced_html = html_of(&cache).expect("spliced html");

        let mut fresh = ParseCache::default();
        drop(fresh.reparse(&new_raw));
        let full_san = stored_sanitized(&fresh);
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
        let (diags, stats) = reparse_incremental_str(&mut cache, &new_raw, edit);

        assert!(
            stats.cache_hits > 0,
            "BOM+CRLF fast path must reuse the prefix ruby: {stats:?}",
        );
        let mut fresh = ParseCache::default();
        let (want, _) = fresh.reparse(&new_raw);
        assert_eq!(diag_debug(&diags), diag_debug(&want));
        assert_eq!(stored_sanitized(&cache), stored_sanitized(&fresh));
        // The lazily-materialised spliced tree renders identically to a fresh
        // full parse (both full-parse the same stored text).
        assert_eq!(html_of(&cache), html_of(&fresh));
    }

    #[test]
    fn accent_doc_declines_and_matches_full() {
        // A CRLF doc whose `〔…〕` span carries an accent digraph: the document
        // carries a tortoiseshell bracket, so `has_tortoise` declines every
        // incremental edit and the cache full-parses — with the decomposition
        // and its Note intact.
        let mut cache = ParseCache::default();
        let old = "じょぶんです。\r\n\r\n〔oraison fune`bre〕\r\n\r\nまつびです。\r\n";
        drop(cache.reparse(old));
        let (new_raw, edit) = interior_insert(old, "まつび", "も");
        let (diags, stats) = reparse_incremental_str(&mut cache, &new_raw, edit);

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
    fn decorative_rule_doc_incremental_hits() {
        // A CRLF doc with a >=10-char decorative rule line (the real aozora-bunko
        // header shape). Rule isolation inserts a blank line silently, but the
        // line-correspondence map accounts for it via `isolation_lines`: a body
        // edit far from the rule fast-paths, reusing the leading ruby node, and
        // stays byte-identical to a full parse.
        let mut cache = ParseCache::default();
        let old = "｜青空《あおぞら》\r\n----------\r\n\r\nほんぶんです。\r\n";
        drop(cache.reparse(old));
        let (new_raw, edit) = interior_insert(old, "ほんぶん", "も");
        let (diags, stats) = reparse_incremental_str(&mut cache, &new_raw, edit);

        assert!(
            stats.cache_hits > 0,
            "rule doc must fast-path and reuse the prefix ruby: {stats:?}",
        );
        let mut fresh = ParseCache::default();
        let (want, _) = fresh.reparse(&new_raw);
        assert_eq!(diag_debug(&diags), diag_debug(&want));
        assert_eq!(stored_sanitized(&cache), stored_sanitized(&fresh));
    }

    #[test]
    fn rule_creation_declines_and_matches_full() {
        // Growing a 9-dash line (not a rule) to 10 dashes turns it into a
        // decorative rule: the R1 trigger (is-rule status flips) declines, and
        // the full parse — which inserts the isolation blank — matches.
        let mut cache = ParseCache::default();
        let old = "まえがき。\r\n---------\r\n\r\nほんぶんです。\r\n";
        drop(cache.reparse(old));
        // Insert one more '-' into the 9-dash run, making it a 10-dash rule.
        let (new_raw, edit) = interior_insert(old, "---------", "-");
        let (diags, stats) = reparse_incremental_str(&mut cache, &new_raw, edit);

        assert_eq!(
            stats.cache_hits, 0,
            "turning a line into a decorative rule must decline: {stats:?}",
        );
        let mut fresh = ParseCache::default();
        let (want, _) = fresh.reparse(&new_raw);
        assert_eq!(diag_debug(&diags), diag_debug(&want));
    }

    #[test]
    fn replace_midword_kana_hits() {
        // Replacing a kana run with one that shares UTF-8 lead bytes used to need
        // a char-boundary snap in the old byte-diff path; the line map derives
        // the edit range exactly from the raw coordinates, so it splices
        // byte-identically with no snapping. CRLF, leading ruby reused.
        let mut cache = ParseCache::default();
        let old = "｜青空《あおぞら》\r\n\r\nぁあぃ\r\n\r\nさしすせそ\r\n";
        drop(cache.reparse(old));
        let at = old.find("ぁあぃ").expect("needle");
        let edit = ByteEdit::new(at..at + "ぁあぃ".len(), "あぁい".to_owned());
        let mut new_raw = String::with_capacity(old.len());
        new_raw.push_str(&old[..at]);
        new_raw.push_str("あぁい");
        new_raw.push_str(&old[at + "ぁあぃ".len()..]);
        let (diags, stats) = reparse_incremental_str(&mut cache, &new_raw, edit);

        assert!(
            stats.cache_hits > 0,
            "must fast-path, reusing the ruby: {stats:?}"
        );
        let mut fresh = ParseCache::default();
        let (want, _) = fresh.reparse(&new_raw);
        assert_eq!(diag_debug(&diags), diag_debug(&want));
        assert_eq!(stored_sanitized(&cache), stored_sanitized(&fresh));
    }

    #[test]
    fn pua_doc_declines_preserves_diagnostic() {
        // A CRLF doc carrying a raw U+E001 sentinel: `has_pua` declines every
        // incremental edit, so the full parse keeps the SourceContainsPua
        // diagnostic that a region re-lex would have dropped.
        let mut cache = ParseCache::default();
        let old = "まえがき。\r\n\r\nあ\u{E001}い\r\n\r\nまつびです。\r\n";
        drop(cache.reparse(old));
        let (new_raw, edit) = interior_insert(old, "まつび", "も");
        let (diags, stats) = reparse_incremental_str(&mut cache, &new_raw, edit);

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

    // ---- line-correspondence map unit checks ------------------------------

    #[test]
    fn isolation_line_indices_records_rule_lines() {
        // A rule line that follows a visible line is isolated; one that follows a
        // blank line (or opens the document) is not.
        assert_eq!(isolation_line_indices("本文\n----------\n後"), vec![1]);
        assert_eq!(
            isolation_line_indices("----------\n後"),
            Vec::<u32>::new(),
            "a rule at document start is not isolated",
        );
        assert_eq!(
            isolation_line_indices("本文\n\n----------\n後"),
            Vec::<u32>::new(),
            "a rule already preceded by a blank line is not isolated",
        );
    }

    #[test]
    fn leading_bom_bytes_counts_stacked_boms() {
        assert_eq!(leading_bom_bytes("hello"), 0);
        assert_eq!(leading_bom_bytes("\u{FEFF}hi"), 3);
        assert_eq!(leading_bom_bytes("\u{FEFF}\u{FEFF}hi"), 6);
        assert_eq!(
            leading_bom_bytes("a\u{FEFF}b"),
            0,
            "interior BOM is not leading"
        );
    }

    #[test]
    fn contains_raw_pua_detects_only_sentinels() {
        assert!(contains_raw_pua("x\u{E001}y"));
        assert!(contains_raw_pua("\u{E004}"));
        assert!(
            !contains_raw_pua("\u{E000}\u{E005}"),
            "neighbours are not sentinels"
        );
        assert!(!contains_raw_pua("ふつうの日本語"));
    }

    #[test]
    fn windowed_resanitize_rejects_a_corrupted_splice() {
        // The windowed re-sanitize is the sole *release-time* authority that the
        // spliced sanitized rope matches `sanitize(raw)`. Pin that it can
        // actually reject: feed it a deliberately-corrupted `new_san` (one byte
        // wrong inside the window, same length so the O(1) tripwire is blind) and
        // assert it returns `false`, plus a correct splice returns `true`.
        // Without this, a regression to always-`true` would silently disarm the
        // release backstop and still pass every other test.
        let mut cache = ParseCache::default();
        let old = "本文\r\n----------\r\nほんぶん\r\n";
        drop(cache.reparse(old));
        let prior = cache.base.as_ref().expect("base");

        // An accepted plain edit: insert ASCII 'a' inside the body line (raw line 2).
        let new_raw_s = "本文\r\n----------\r\nほaんぶん\r\n";
        let new_raw = Rope::from(new_raw_s);
        let correct = sanitize(new_raw_s).text.into_owned();
        let correct_rope = Rope::from(correct.as_str());
        assert!(
            windowed_resanitize_ok(prior, &new_raw, &correct_rope, 2),
            "a correct splice must pass the windowed check",
        );

        // Corrupt one byte inside the windowed region (the unique inserted 'a'),
        // keeping the length identical so only the windowed compare can catch it.
        let wrong = correct.replacen('a', "b", 1);
        assert_ne!(wrong, correct, "the corruption must change a byte");
        let wrong_rope = Rope::from(wrong.as_str());
        assert!(
            !windowed_resanitize_ok(prior, &new_raw, &wrong_rope, 2),
            "a corrupted splice must be rejected by the windowed check",
        );
    }

    #[test]
    fn edit_triggers_decline_fires_on_isolation_toggle() {
        // R2: an edit that toggles a line's blank/non-blank status immediately
        // before a decorative-rule line creates (or destroys) that rule's
        // isolation blank, mutating `isolation_lines` — which an in-line splice
        // cannot absorb, so the trigger gate must decline. A byte-scan of the
        // insert (`X`, no `-`) would miss it; the line-content comparison catches
        // it. Pinned directly so a regression to always-`false` is caught (the
        // windowed check would also decline, hiding it from the e2e tests).
        let mut cache = ParseCache::default();
        // A blank line sits directly before the rule, so the rule is NOT isolated.
        let old = "本文\r\n\r\n----------\r\nあと\r\n";
        drop(cache.reparse(old));
        let prior = cache.base.as_ref().expect("base");
        let old_raw = Rope::from(old);

        // Insert a non-blank char into the blank line (raw line 1): it gains
        // content, so the following rule would now gain an isolation blank.
        let line1 = old_raw.line_to_byte(1);
        let edit = ByteEdit::new(line1..line1, "X".to_owned());
        let new_raw = Rope::from("本文\r\nX\r\n----------\r\nあと\r\n");
        assert!(
            edit_triggers_decline(prior, &old_raw, &new_raw, &edit),
            "toggling a blank line before a rule must decline (R2)",
        );

        // Control: the same blank→non-blank toggle when the next line is NOT a
        // rule carries no isolation change, so the gate must not trigger.
        let mut c2 = ParseCache::default();
        let old2 = "本文\r\n\r\nあと\r\n";
        drop(c2.reparse(old2));
        let p2 = c2.base.as_ref().expect("base");
        let old_raw2 = Rope::from(old2);
        let line1b = old_raw2.line_to_byte(1);
        let edit2 = ByteEdit::new(line1b..line1b, "X".to_owned());
        let new_raw2 = Rope::from("本文\r\nX\r\nあと\r\n");
        assert!(
            !edit_triggers_decline(p2, &old_raw2, &new_raw2, &edit2),
            "a blank toggle with no following rule must not trigger R2",
        );
    }

    #[test]
    fn two_rule_doc_interior_edit_hits_and_matches_full() {
        // A document with TWO isolated decorative rules, both before the edit, so
        // the edited body line's `san_line` shifts by a multi-entry
        // `partition_point` (count 2). A blank-line-delimited body paragraph after
        // both rules keeps the re-lex region off the rules, so it still fast-paths
        // — and the windowed check runs over an interior isolation blank. Pins the
        // >=2-isolation accept path the randomized e2e generator does not reach.
        let mut cache = ParseCache::default();
        // A leading ruby node sits in the reused prefix, so a successful splice
        // reports a non-zero reuse count (plain text alone yields no source node).
        let old = "｜序文《じょぶん》\r\n----------\r\n\r\nちゅうかん\r\n----------\r\n\r\nなかほん\r\n\r\nまつび\r\n";
        drop(cache.reparse(old));
        // Insert mid-word inside the body paragraph that follows both rules.
        let (new_raw, edit) = interior_insert(old, "かほん", "X");
        let (diags, stats) = reparse_incremental_str(&mut cache, &new_raw, edit);

        assert!(
            stats.cache_hits > 0,
            "a plain edit past two isolated rules must fast-path: {stats:?}",
        );
        let mut fresh = ParseCache::default();
        let (want, _) = fresh.reparse(&new_raw);
        assert_eq!(diag_debug(&diags), diag_debug(&want));
        assert_eq!(stored_sanitized(&cache), stored_sanitized(&fresh));
    }
}
