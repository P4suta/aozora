//! Owned-AST incremental re-parse engine for #237 — the sole incremental path.
//!
//! The retired Stage-A segment cache first proved — over the reference corpus —
//! *where* a document can be cut into independently-lexable spans. This engine
//! carries that insight onto the owned AST: it caches the owned lex output's
//! store-free region-find tables as a structure-sharing [`PieceSeq`] and, on an
//! edit, re-lexes only the minimal balanced region around the edit before
//! splicing that sequence.
//!
//! This module hosts the **region finder** ([`minimal_balanced_region`]), the
//! structure-sharing [`PieceSeq`], the **diagnostics-only splice**
//! ([`reparse_incremental_diagnostics_only`]), and the shared "where is it safe
//! to cut the document" cut helpers. The splice is the production incremental
//! path: it is re-exported from the crate root as the **unstable**
//! [`crate::reparse_incremental_diagnostics_only`] and consumed by the LSP's
//! debounced diagnostics (#237 Stage B'3). It is internal-unit-tested and proven
//! byte-identical to a full re-parse by the `corpus_incremental_merge`
//! differential gate.
//!
//! All coordinates here are **sanitized-source** byte offsets (the space every
//! [`OwnedLexOutput::source_span`](crate::SourceNodeOwned::source_span) and
//! [`OwnedLexOutput::pairs`](crate::OwnedLexOutput::pairs) indexes); the
//! raw↔sanitized bridge belongs to a later wiring PR. A cut is admitted only
//! where the block-container depth is zero and no resolved delimiter pair
//! straddles it — see [`structurally_safe`].

use core::ops::Range;
use std::borrow::Cow;
use std::sync::Arc;

use crate::splice::{RegionRole, classify_node_ref};
use crate::{
    CoupledKind, Diagnostic, Document, NodeRefOwned, OwnedLexOutput, PairLink, SourceNodeOwned,
    SpliceSafety,
};

/// Read-only byte view of a **sanitized** buffer the incremental engine cuts,
/// scans, and re-lexes against.
///
/// Abstracted so the engine is generic over the backing store. Today the only
/// impl is `&str` (the cached `String`'s slice), which is byte-for-byte the
/// prior direct-`&str` engine; a later PR adds a `ropey::RopeSlice` impl so the
/// cache can hold a rope without copying it to a flat `String` per edit.
///
/// Every method mirrors the corresponding `str` operation exactly: [`byte`] is
/// `as_bytes()[i]`, [`slice`] is `str::get` (returns `None` off-bounds or on a
/// non-char-boundary range). [`byte`] is the monotone scan's hot probe and must
/// be amortized `O(1)` (a direct index for `&str`; a moving cursor for a rope).
///
/// [`byte`]: SanitizedSrc::byte
/// [`slice`]: SanitizedSrc::slice
pub trait SanitizedSrc {
    /// Byte length of the buffer (`str::len`).
    fn len(&self) -> usize;

    /// Whether the buffer is empty (`self.len() == 0`).
    fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Byte at offset `i` (`as_bytes()[i]`). Amortized `O(1)` on the monotone
    /// outward scan; panics on out-of-bounds, exactly like `str` indexing.
    fn byte(&self, i: usize) -> u8;

    /// Sub-slice over byte `range`, with `str::get` semantics: `None` when the
    /// range is off-bounds or splits a UTF-8 code point.
    fn slice(&self, range: Range<usize>) -> Option<Cow<'_, str>>;

    /// Debug-only check that an incremental edit changed **only** bytes inside
    /// `edit_old`: the prefix `[0, edit_old.start)` and the suffix after the
    /// edit must be byte-identical between `self` (the cached sanitized buffer)
    /// and `new` (the edited one, whose suffix starts at `new_edit_end`).
    ///
    /// This is the incremental splice's caller precondition, not a runtime gate.
    /// `splice_prologue` used to enforce it here with an unconditional
    /// prefix/suffix `memcmp`; that was deleted primarily for **time**, not
    /// defence — on a rope source comparing `[0, edit_old.start)` is
    /// `O(prefix) = O(doc)` per edit (a `RopeSlice` equality still walks
    /// `O(min len)`), which is incompatible with the per-keystroke hot path. The
    /// default is a deliberate no-op (a backing store with no cheap external
    /// witness pays nothing); the `&str` impl restores the exact comparison
    /// under `debug_assert!` so the `&str`-backed proptests and corpus
    /// differential keep pinning it in debug builds, and a rope impl can add a
    /// no-alloc `byte_slice == byte_slice` probe. Gated `#[cfg(debug_assertions)]`
    /// so the method and its sole call vanish from release builds together.
    #[cfg(debug_assertions)]
    fn debug_assert_unchanged_outside(
        &self,
        _new: &Self,
        _edit_old: Range<usize>,
        _new_edit_end: usize,
    ) {
    }
}

impl SanitizedSrc for &str {
    fn len(&self) -> usize {
        str::len(self)
    }

    fn is_empty(&self) -> bool {
        str::is_empty(self)
    }

    fn byte(&self, i: usize) -> u8 {
        self.as_bytes()[i]
    }

    fn slice(&self, range: Range<usize>) -> Option<Cow<'_, str>> {
        self.get(range).map(Cow::Borrowed)
    }

    #[cfg(debug_assertions)]
    fn debug_assert_unchanged_outside(
        &self,
        new: &Self,
        edit_old: Range<usize>,
        new_edit_end: usize,
    ) {
        // The exact prefix/suffix comparison `splice_prologue` used to run
        // unconditionally as a `memcmp`, now a debug-only restatement of the
        // caller precondition: every byte outside `edit_old` is identical
        // between the cached buffer (`self`) and the edited one (`new`).
        // Byte-for-byte the deleted `slice(..)`-based check, kept
        // `O(prefix + suffix)` for the `&str` corpus gate and proptests, which
        // exercise this branch in debug.
        debug_assert!(
            self.slice(0..edit_old.start) == new.slice(0..edit_old.start)
                && self.slice(edit_old.end..self.len()) == new.slice(new_edit_end..new.len()),
            "incremental edit changed bytes outside edit_old: the splice \
             precondition (only edit_old's bytes differ between the cached and \
             new sanitized buffers) was violated",
        );
    }
}

/// The store-free spliceable base of the cached parse the **diagnostics-only**
/// incremental path reads.
///
/// Consumed by [`crate::reparse_incremental_diagnostics_only`] and the shared
/// `splice_prologue`: the sanitized byte source plus the [`PieceSeq`] — the
/// unified, incrementally-maintained representation of the three region-find
/// tables (`source_nodes` / `pairs` / `diagnostics`).
///
/// The LSP's per-keystroke hot path keeps only these two fields of the prior
/// parse (its `DiagBase`) and never materialises the full [`OwnedLexOutput`].
/// Both are store-independent: the prologue's region-find
/// (`minimal_balanced_region` / [`PieceSeq::structurally_safe`] / the coupling
/// guard) and the diagnostics splice read only `sanitized` and the
/// [`PieceSeq`]'s self-contained queries — a node's `source_span` and the
/// `NodeRefOwned` discriminant, never a `StrId`/`ContentRange` resolved against a
/// [`crate::NodeStore`]. A full [`OwnedLexOutput`] projects into one via
/// [`DiagBaseRef::from_cached`] (paired with a [`PieceSeq`] built over the same
/// tables by [`PieceSeq::from_contiguous`]).
///
/// Generic over the sanitized byte source `S` (a [`SanitizedSrc`]); `S` defaults
/// to `&'a str` so every existing `DiagBaseRef { sanitized: &x, .. }` literal and
/// [`DiagBaseRef::from_cached`] construction keeps compiling. `Copy` is dropped
/// (a future rope source holds a cursor and cannot be `Copy`), so the engine
/// takes the base by reference.
#[derive(Debug, Clone)]
pub struct DiagBaseRef<'a, S: SanitizedSrc = &'a str> {
    /// The cached sanitized buffer (a sanitize fixed point) — the coordinate
    /// space every piece offset indexes.
    pub sanitized: S,
    /// The incrementally-maintained region-find representation of the cached
    /// parse's `source_nodes` / `pairs` / `diagnostics` (#237 Tier 2). Spliced
    /// `O(region + log #pieces)` per edit instead of rebuilt, so the region-find
    /// prologue and the diagnostics splice read it without re-scanning the whole
    /// buffer/tables per edit.
    pub pieces: &'a PieceSeq,
}

impl<'a> DiagBaseRef<'a> {
    /// Project a full [`OwnedLexOutput`]'s sanitized buffer plus a [`PieceSeq`]
    /// over its tables into a store-free diagnostics base.
    ///
    /// The caller owns the [`PieceSeq`] (so it can outlive this borrow). The
    /// `pieces` need not be `cached`'s own freshly-built sequence: the multi-edit
    /// base threads an *accumulated* sequence whose flattened tables are
    /// byte-identical to `cached`'s (the [`PieceSeq`] ≡ contiguous invariant), so
    /// the prologue's region-find reads either interchangeably.
    #[must_use]
    pub fn from_cached(cached: &'a OwnedLexOutput, pieces: &'a PieceSeq) -> Self {
        Self {
            sanitized: &cached.sanitized,
            pieces,
        }
    }
}

/// Precomputed acceleration structure for the region-find prologue (#237 Tier 2)
/// — the prefix-sum recurrence each `PieceIndex` wraps once per backing table.
///
/// Crate-internal (`pub(crate)`) so the in-crate byte-identity oracle proptests
/// can build one directly; in production it is no longer owned per parse.
/// [`PieceSeq`] holds one `PieceIndex` (an `Arc`-shared `RegionIndex` over a
/// backing table) per piece and answers every query through it, so the LSP
/// `DiagBase` keeps a [`PieceSeq`] and splices it incrementally instead of
/// rebuilding this each edit.
///
/// Built in a single `O(N + P + D)` pass (plus small `O(P log P + D log D)`
/// sorts of the few pairs/diagnostics) over the three store-free tables the
/// prologue reads — the same `O(N)` cost the base copy already pays each edit.
/// Replaces two `O(doc)` per-edit scans with `O(log n)`/`O(1)` lookups:
///
/// - the whole-buffer `candidate_boundaries` byte scan plus its `O(#cuts · N)`
///   `structurally_safe` calls become an outward scan with `O(log n)` safety
///   probes (see `RegionIndex::structurally_safe`);
/// - the diagnostic-straddle scan becomes two `O(log n)` probes
///   (`RegionIndex::diag_straddles`).
///
/// (The whole-document-scoped / unbalanced-delimiter / coupling guards are no
/// longer RegionIndex's: they moved to per-piece `PieceFlags`, because a split
/// piece needs the OR over its *live range*, not the whole backing table.)
///
/// Every recurrence mirrors its linear oracle exactly; the
/// `corpus_incremental_merge` differential gate plus the oracle proptests pin
/// byte-identity.
#[derive(Debug)]
pub(crate) struct RegionIndex {
    /// Clamped LIFO block-container depth **after** each `source_node`
    /// (parallel to `source_nodes`, which is sorted by `source_span.start`).
    depth_prefix: Vec<i32>,
    /// Running max of `source_span.end` over `source_nodes[0..=i]` — the
    /// straddle test for [`RegionIndex::structurally_safe`].
    max_end_prefix: Vec<u32>,
    /// `pair.close.end`, sorted ascending — the search key for the pair-straddle
    /// `partition_point`. Built internally so the query never depends on the
    /// input slice's order.
    pair_close_end: Vec<u32>,
    /// Suffix min of `pair.open.start` over the close-end-sorted pair order;
    /// length `P + 1` (`[P] = u32::MAX` sentinel). `pair_suffix_min_open[j] < off`
    /// means some pair with `close.end > off` opens before `off` (straddles).
    pair_suffix_min_open: Vec<u32>,
    /// Diagnostic `span.start`, sorted ascending — search key for the straddle
    /// probes. Built internally so the query is independent of the diagnostics'
    /// (pipeline-stage vs position-sorted) order.
    diag_start_sorted: Vec<u32>,
    /// Running max of `span.end` over the start-sorted diagnostics — the
    /// straddle test for [`RegionIndex::diag_straddles`].
    diag_max_end_prefix: Vec<u32>,
}

impl RegionIndex {
    /// Build the index in one pass over the store-free tables. Mirrors the
    /// linear recurrence of `structurally_safe` exactly.
    #[must_use]
    pub(crate) fn build(
        nodes: &[SourceNodeOwned],
        pairs: &[PairLink],
        diagnostics: &[Diagnostic],
    ) -> Self {
        // Node prefixes (nodes are sorted by `source_span.start`).
        let mut depth_prefix = Vec::with_capacity(nodes.len());
        let mut max_end_prefix = Vec::with_capacity(nodes.len());
        let mut depth: i32 = 0;
        let mut max_end: u32 = 0;
        for sn in nodes {
            match sn.node {
                NodeRefOwned::BlockOpen(_) => depth += 1,
                NodeRefOwned::BlockClose(_) => depth = (depth - 1).max(0),
                _ => {}
            }
            depth_prefix.push(depth);
            max_end = max_end.max(sn.source_span.end);
            max_end_prefix.push(max_end);
        }

        // Pair index, sorted by `close.end` ascending (a self-contained copy, so
        // the query never depends on the input slice's order). `pairs` are few.
        let mut pair_order: Vec<&PairLink> = pairs.iter().collect();
        pair_order.sort_by_key(|p| p.close.end);
        let pair_close_end: Vec<u32> = pair_order.iter().map(|p| p.close.end).collect();
        let mut pair_suffix_min_open = vec![u32::MAX; pair_order.len() + 1];
        for i in (0..pair_order.len()).rev() {
            pair_suffix_min_open[i] = pair_suffix_min_open[i + 1].min(pair_order[i].open.start);
        }

        // Diagnostic index, sorted by `span.start` ascending (self-contained).
        let mut diag_order: Vec<&Diagnostic> = diagnostics.iter().collect();
        diag_order.sort_by_key(|d| d.span().start);
        let diag_start_sorted: Vec<u32> = diag_order.iter().map(|d| d.span().start).collect();
        let mut diag_max_end_prefix = Vec::with_capacity(diag_order.len());
        let mut diag_max_end: u32 = 0;
        for d in &diag_order {
            diag_max_end = diag_max_end.max(d.span().end);
            diag_max_end_prefix.push(diag_max_end);
        }

        Self {
            depth_prefix,
            max_end_prefix,
            pair_close_end,
            pair_suffix_min_open,
            diag_start_sorted,
            diag_max_end_prefix,
        }
    }

    /// `O(log n)` equivalent of [`structurally_safe`]: whether a cut at `off`
    /// keeps every block container and resolved pair whole. `nodes` must be the
    /// same slice the index was built over (sorted by `source_span.start`).
    #[must_use]
    fn structurally_safe(&self, nodes: &[SourceNodeOwned], off: u32) -> bool {
        let k = nodes.partition_point(|sn| sn.source_span.start < off);
        // A classified span with `start < off` whose `end > off` straddles `off`.
        let straddle = k > 0 && self.max_end_prefix[k - 1] > off;
        // Block-container depth at `off` (after all nodes with `start < off`).
        let depth = if k == 0 { 0 } else { self.depth_prefix[k - 1] };
        // Some resolved pair with `close.end > off` opens before `off`.
        let j = self.pair_close_end.partition_point(|&ce| ce <= off);
        let pair_straddle = self.pair_suffix_min_open[j] < off;
        !straddle && depth == 0 && !pair_straddle
    }

    /// Whether any diagnostic straddles boundary `b` (`span.start < b < span.end`)
    /// — one `O(log n)` probe of the start-sorted diagnostics.
    #[must_use]
    fn diag_straddles(&self, b: u32) -> bool {
        let k = self.diag_start_sorted.partition_point(|&s| s < b);
        k > 0 && self.diag_max_end_prefix[k - 1] > b
    }
}

// ============================================================================
// #237 Tier 2 (PR-1) — structure-sharing piece-sequence.
//
// `PieceSeq` is the unified, truly-incremental representation of the three
// store-free region-find tables (`source_nodes` / `pairs` / `diagnostics`),
// keyed by the engine's *structurally-safe cuts* — the only places
// `splice_prologue` ever divides a document. It is **additive and unwired** in
// this PR: the production engines still run `RegionIndex::build` + whole-table
// re-materialization each edit, exactly as before, so behaviour is unchanged.
// PR-2 onward reads the engines through this type. The `PieceSeq ≡ contiguous`
// adversarial proptest (see `oracle_proptests`) pins every query byte-identical
// to the linear oracle over the flattened tables across multi-edit splice runs.
// ============================================================================

/// Per-piece acceleration over an immutable *backing* table triple — today's
/// [`RegionIndex`] recurrence reused verbatim, built **once** per backing table
/// and shared (behind an [`Arc`]) by every piece that views a slice of it,
/// never rebuilt after a cut.
///
/// `PieceIndex` is the cut-geometry specialisation the #237 Tier 2 design calls
/// for. Because every piece boundary is a structurally-safe cut (block-container
/// depth zero; no node span, resolved pair, or diagnostic straddling it), the
/// whole-backing recurrence answers each *within-piece* query directly: the
/// boundary invariant makes the absolute and piece-local values coincide for the
/// depth / span-straddle / pair-straddle / diagnostic-straddle predicates (read
/// at the backing-local offset the owning [`Piece`]'s `san_shift` derives).
/// Wrapping [`RegionIndex`] keeps the build recurrence and the query primitives a
/// single proven implementation; the wrapper exists only to name the per-piece
/// role.
#[derive(Debug)]
struct PieceIndex {
    /// The reused whole-backing acceleration. Every query reads its prefix-sum
    /// arrays at a *backing-local* offset (`queried_offset - piece.san_shift`).
    region: RegionIndex,
}

impl PieceIndex {
    /// Build the index once over a backing `nodes` / `pairs` / `diagnostics`
    /// triple. Verbatim [`RegionIndex::build`]; see it for the recurrence.
    fn build(nodes: &[SourceNodeOwned], pairs: &[PairLink], diagnostics: &[Diagnostic]) -> Self {
        Self {
            region: RegionIndex::build(nodes, pairs, diagnostics),
        }
    }
}

/// The three region-find guard predicates, OR-reduced over a piece's **live
/// range** (not its whole backing table — a split piece shares an index whose
/// flags also cover dropped neighbours, so the document-level OR must be taken
/// over the live ranges instead).
#[derive(Debug, Clone, Copy)]
struct PieceFlags {
    /// Any diagnostic in range is [`is_whole_document_scoped`].
    has_whole_doc_scoped_diag: bool,
    /// Any diagnostic in range is an unbalanced-delimiter half
    /// ([`Diagnostic::UnclosedBracket`] / [`Diagnostic::UnmatchedClose`]).
    has_unbalanced_delimiter: bool,
    /// Any node in range [`node_forbids_region_reuse`].
    has_coupled_node: bool,
}

impl PieceFlags {
    /// Compute the flags over a piece's live `nodes` / `diags` slices.
    fn compute(nodes: &[SourceNodeOwned], diags: &[Diagnostic]) -> Self {
        Self {
            has_whole_doc_scoped_diag: diags.iter().any(is_whole_document_scoped),
            has_unbalanced_delimiter: diags.iter().any(|d| {
                matches!(
                    d,
                    Diagnostic::UnclosedBracket { .. } | Diagnostic::UnmatchedClose { .. }
                )
            }),
            has_coupled_node: nodes.iter().any(|sn| node_forbids_region_reuse(sn.node)),
        }
    }
}

/// One structurally-safe-bounded view of an immutable backing table triple.
///
/// Views `nodes[n_range]` / `pairs[p_range]` / `diags[d_range]` of a backing
/// triple shared (via [`Arc`]) with every sibling piece cut from the same
/// parse, plus the [`Arc`]-shared [`PieceIndex`] over that whole backing.
/// `san_data` is the piece's byte span in **backing** (original-parse)
/// sanitized coordinates; `san_shift` lifts backing coordinates to the piece's
/// **current** position in the live document — a per-piece scalar so a suffix
/// shift is `O(1)`, never a per-node rewrite. It is added on read; comparisons
/// against the index run in backing-local coordinates. A `split` is `O(log len)`
/// (range endpoints by binary search; data shared).
#[derive(Debug, Clone)]
struct Piece {
    /// Whole backing source-node table (sorted by `source_span.start`), shared.
    nodes: Arc<[SourceNodeOwned]>,
    /// This piece's live index range into [`nodes`](Self::nodes).
    n_range: Range<usize>,
    /// Whole backing pair table (sorted by `close.end`), shared.
    pairs: Arc<[PairLink]>,
    /// This piece's live index range into [`pairs`](Self::pairs).
    p_range: Range<usize>,
    /// Whole backing diagnostic table (position-sorted by span), shared.
    diags: Arc<[Diagnostic]>,
    /// This piece's live index range into [`diags`](Self::diags).
    d_range: Range<usize>,
    /// Byte span this piece covers in backing sanitized coordinates.
    san_data: Range<u32>,
    /// `current_san = backing_san + san_shift`.
    san_shift: i64,
    /// Block-container nesting depth entering this piece (the global depth at its
    /// start boundary). Zero for every piece cut at a depth-zero blank-line cut
    /// or the document start; nonzero only for a re-lexed piece appended at the
    /// `len` boundary of a globally-unbalanced document (an unclosed
    /// `［＃ここから…］`).
    ///
    /// The global depth is `base_depth + local_depth` with **no clamping**, which
    /// is true only because of two reachability facts, not for an arbitrary
    /// balanced sequence: (a) a re-lexed piece is *strictly* Dyck-balanced
    /// ([`relexed_is_balanced`] gates the prologue), so it has no leading stray
    /// `BlockClose` that would clamp locally but not globally; and (b) a piece
    /// with `base_depth > 0` is never [`narrow`](Self::narrow)ed, because
    /// [`PieceSeq::structurally_safe`] gates every cut to a `base_depth == 0`
    /// piece. So the only load-bearing consequence is `base_depth + local == 0`
    /// iff both summands are zero — exact nonzero depths never feed a query.
    base_depth: i32,
    /// Region-find guard predicates over this piece's live range.
    flags: PieceFlags,
    /// Shared whole-backing acceleration.
    idx: Arc<PieceIndex>,
}

impl Piece {
    /// A new piece sharing this piece's backing tables and index, narrowed to
    /// the current-coordinate sub-span `[a, b)`. Both `a` and `b` must be
    /// structurally-safe cuts inside `[current_lo, current_hi]`, so every node /
    /// pair / diagnostic partitions cleanly to one side (none straddles a safe
    /// cut). `O(log len)`.
    fn narrow(&self, a: u32, b: u32) -> Self {
        let data_a = to_backing(a, self.san_shift);
        let data_b = to_backing(b, self.san_shift);
        // Nodes / diagnostics partition by start, pairs by `close.end`; a safe
        // cut is straddled by none, so each predicate cleanly separates the two
        // sides. Partitioning the whole backing array is sound because
        // `[data_a, data_b)` lies within this piece's existing backing range.
        let n_start = self
            .nodes
            .partition_point(|sn| sn.source_span.start < data_a);
        let n_end = self
            .nodes
            .partition_point(|sn| sn.source_span.start < data_b);
        let p_start = self.pairs.partition_point(|pl| pl.close.end <= data_a);
        let p_end = self.pairs.partition_point(|pl| pl.close.end <= data_b);
        let d_start = self.diags.partition_point(|d| d.span().start < data_a);
        let d_end = self.diags.partition_point(|d| d.span().start < data_b);
        let flags = PieceFlags::compute(&self.nodes[n_start..n_end], &self.diags[d_start..d_end]);
        Self {
            nodes: Arc::clone(&self.nodes),
            n_range: n_start..n_end,
            pairs: Arc::clone(&self.pairs),
            p_range: p_start..p_end,
            diags: Arc::clone(&self.diags),
            d_range: d_start..d_end,
            san_data: data_a..data_b,
            san_shift: self.san_shift,
            base_depth: self.base_depth,
            flags,
            idx: Arc::clone(&self.idx),
        }
    }
}

/// Cumulative aggregate at a piece boundary: the current-coordinate sanitized
/// offset at the piece's start, plus the running node count. A [`PieceSeq`]'s
/// `cum` holds one per piece *plus a trailing sentinel* whose `base_san` is the
/// document's total sanitized length and whose `cum_count` is the total node
/// count.
#[derive(Debug, Clone, Copy)]
struct PieceAgg {
    /// Current-coordinate sanitized offset at the piece's start.
    base_san: u32,
    /// Node count across all earlier pieces.
    cum_count: u32,
}

/// A structure-sharing sequence of pieces, keyed by structurally-safe cuts.
///
/// The #237 Tier 2 unified, truly incremental representation of a parse's three
/// store-free region-find tables (`source_nodes`, `pairs`, `diagnostics`).
///
/// **UNSTABLE — not subject to semver until v0.5.0** (the #237 incremental
/// API). This is the maintained representation the production diagnostics-only
/// engine splices on every edit: the per-keystroke hot path replaces the
/// per-edit whole-table re-materialization + `RegionIndex::build` with
/// `O(region + log #pieces)` piece splicing.
///
/// Each piece views a slice of an immutable backing table (the cached parse or a
/// region re-lex) through a shared [`Arc`]; a [`splice`](Self::splice) divides
/// the pieces overlapping the edited region at its structurally-safe boundaries
/// (`O(1)` view adjustments), drops the middle, inserts one piece for the
/// re-lex, and shifts the suffix by a per-piece scalar. Backing tables store
/// each entry's *original* absolute offset; reads add the piece's scalar shift,
/// which keeps every piece's index byte-identical to a freshly built
/// `RegionIndex`. Queries binary-search the piece containing the offset
/// (`O(log #pieces)`) and answer from its shared index.
#[derive(Debug, Clone)]
pub struct PieceSeq {
    /// Current-order pieces; non-empty (`from_contiguous` makes one, `splice`
    /// keeps at least the re-lexed piece).
    pieces: Vec<Piece>,
    /// `pieces.len() + 1` cumulative aggregates (one per piece + a trailing
    /// totals sentinel), ascending in `base_san`.
    cum: Vec<PieceAgg>,
}

impl PieceSeq {
    /// Build a single-piece sequence over a contiguous parse's store-free
    /// tables — the region-find base a cached parse is wrapped in. `san_len`
    /// is the parse's sanitized byte length (the tables alone do not carry the
    /// trailing plain-text length). Diagnostics are stored position-sorted so a
    /// later `splice` can divide them at a safe cut by a clean partition.
    #[must_use]
    pub fn from_contiguous(
        nodes: &[SourceNodeOwned],
        pairs: &[PairLink],
        diagnostics: &[Diagnostic],
        san_len: u32,
    ) -> Self {
        let mut diags = diagnostics.to_vec();
        sort_diags(&mut diags);
        let idx = Arc::new(PieceIndex::build(nodes, pairs, &diags));
        let flags = PieceFlags::compute(nodes, &diags);
        let (n, p, d) = (nodes.len(), pairs.len(), diags.len());
        let piece = Piece {
            nodes: Arc::from(nodes),
            n_range: 0..n,
            pairs: Arc::from(pairs),
            p_range: 0..p,
            diags: Arc::from(diags),
            d_range: 0..d,
            san_data: 0..san_len,
            san_shift: 0,
            base_depth: 0,
            flags,
            idx,
        };
        Self::finalize(vec![piece])
    }

    /// Materialise the three current-coordinate tables (each piece's entries
    /// shifted by its scalar). Reproduces a contiguous parse's tables exactly:
    /// `source_nodes` ascending by `start`, `pairs` ascending by `close.end`,
    /// and `diagnostics` position-sorted. The bridge by which the corpus
    /// differential gate keeps reading the engine `&str`-first.
    #[must_use]
    pub fn flatten(&self) -> (Vec<SourceNodeOwned>, Vec<PairLink>, Vec<Diagnostic>) {
        let total = self.cum.last().map_or(0, |agg| agg.cum_count) as usize;
        let mut nodes = Vec::with_capacity(total);
        let mut pairs = Vec::new();
        for piece in &self.pieces {
            for sn in &piece.nodes[piece.n_range.start..piece.n_range.end] {
                nodes.push(SourceNodeOwned {
                    source_span: sn.source_span.shifted(piece.san_shift),
                    node: sn.node,
                });
            }
            for pl in &piece.pairs[piece.p_range.start..piece.p_range.end] {
                pairs.push(PairLink {
                    kind: pl.kind,
                    open: pl.open.shifted(piece.san_shift),
                    close: pl.close.shifted(piece.san_shift),
                });
            }
        }
        (nodes, pairs, self.collect_diagnostics())
    }

    /// Materialise just the diagnostics (each piece's entries shifted by its
    /// scalar), in piece order. Because pieces are ascending and each piece's
    /// diagnostics are position-sorted with **none straddling a piece boundary**,
    /// the concatenation is already globally ascending by span start. `O(D)` — it
    /// walks only the diagnostic ranges, not the `O(N)` node table, so the
    /// per-keystroke hot path flattens diagnostics for the LSP surface without
    /// re-materialising the maintained `source_nodes`/`pairs`.
    ///
    /// The LSP re-sorts the result by its full positional order (`(start, end)`
    /// then a debug-string tiebreak), so two diagnostics sharing a span order
    /// identically to a full parse; the in-piece `(start, end)` sort already
    /// agrees except for that tiebreak.
    #[must_use]
    pub fn collect_diagnostics(&self) -> Vec<Diagnostic> {
        let mut diags = Vec::new();
        for piece in &self.pieces {
            for d in &piece.diags[piece.d_range.start..piece.d_range.end] {
                diags.push(d.clone().shifted(piece.san_shift));
            }
        }
        diags
    }

    /// Replace the structurally-safe `region` (current sanitized coordinates,
    /// both endpoints safe cuts) with one piece built from the region re-lex's
    /// tables, shifting the suffix by the sanitized length delta `d_san`. The
    /// new sequence's flattened tables equal a full re-parse's whenever the
    /// caller's `region` / re-lex satisfy the engine's splice preconditions
    /// (exactly what `splice_prologue` guarantees). `O(region + #pieces)`.
    ///
    /// Backing tables are immutable, so the prefix and untouched suffix pieces
    /// are carried by `Arc` share; only the pieces straddling a region endpoint
    /// are re-viewed (`narrow`), and the suffix moves by a per-piece scalar
    /// `san_shift` — no per-node coordinate rewrite.
    ///
    /// # Panics
    ///
    /// Panics if the region geometry is internally inconsistent — `region.end <
    /// region.start`, or `d_san` deletes more than the region's bytes (a
    /// negative re-lexed length). Both are caller-precondition violations the
    /// engine never produces.
    #[must_use]
    #[allow(
        clippy::too_many_arguments,
        reason = "the five inputs are irreducible: the region plus its three parallel re-lexed tables plus the sanitized length delta; bundling them into an ad-hoc struct would move the arity without adding meaning, and keeping raw tables decouples PieceSeq from OwnedLexOutput"
    )]
    pub fn splice(
        &self,
        region: Range<u32>,
        relexed_nodes: &[SourceNodeOwned],
        relexed_pairs: &[PairLink],
        relexed_diags: &[Diagnostic],
        d_san: i64,
    ) -> Self {
        let mut rdiags = relexed_diags.to_vec();
        sort_diags(&mut rdiags);
        let ridx = Arc::new(PieceIndex::build(relexed_nodes, relexed_pairs, &rdiags));
        let flags = PieceFlags::compute(relexed_nodes, &rdiags);
        let region_len = region.end.saturating_sub(region.start);
        let new_region_san_len = backing_len(region_len, d_san);
        // Container depth entering the region. Zero unless the region starts at
        // the `len` boundary of a globally-unbalanced document (an append after
        // an unclosed `［＃ここから…］`), where the re-lexed piece nests inside it.
        let base_depth = self.container_depth_at(region.start);
        let (rn, rp, rd) = (relexed_nodes.len(), relexed_pairs.len(), rdiags.len());
        let relexed_piece = Piece {
            nodes: Arc::from(relexed_nodes),
            n_range: 0..rn,
            pairs: Arc::from(relexed_pairs),
            p_range: 0..rp,
            diags: Arc::from(rdiags),
            d_range: 0..rd,
            san_data: 0..new_region_san_len,
            san_shift: i64::from(region.start),
            base_depth,
            flags,
            idx: ridx,
        };

        let mut left: Vec<Piece> = Vec::new();
        let mut right: Vec<Piece> = Vec::new();
        for (i, piece) in self.pieces.iter().enumerate() {
            let lo = self.cum[i].base_san;
            let hi = self.cum[i + 1].base_san;
            if hi <= region.start {
                left.push(piece.clone());
            } else if lo >= region.end {
                right.push(piece.clone());
            } else {
                if lo < region.start {
                    left.push(piece.narrow(lo, region.start));
                }
                if hi > region.end {
                    right.push(piece.narrow(region.end, hi));
                }
            }
        }
        // The suffix moves by the sanitized delta — only `san_shift` is touched.
        for piece in &mut right {
            piece.san_shift += d_san;
        }

        let mut pieces = left;
        pieces.push(relexed_piece);
        pieces.extend(right);
        Self::finalize(pieces)
    }

    /// Whether a cut at current-coordinate sanitized offset `off` keeps every
    /// block container and resolved pair whole — the piece-local form of the
    /// `RegionIndex` structural-safety query.
    #[must_use]
    pub fn structurally_safe(&self, off: u32) -> bool {
        let piece = &self.pieces[self.find_piece(off)];
        // A piece is internally balanced, so its local depth never dips below
        // zero; the global depth `base_depth + local_depth` is therefore zero
        // iff both are. A nonzero `base_depth` means the cut sits inside an open
        // container, so no offset in the piece is safe.
        piece.base_depth == 0
            && piece
                .idx
                .region
                .structurally_safe(&piece.nodes, to_backing(off, piece.san_shift))
    }

    /// Block-container nesting depth at current-coordinate offset `off`: the
    /// containing piece's `base_depth` plus its local clamped depth. Used to seed
    /// a re-lexed piece's `base_depth` when the region starts inside an open
    /// container.
    fn container_depth_at(&self, off: u32) -> i32 {
        let piece = &self.pieces[self.find_piece(off)];
        let data_off = to_backing(off, piece.san_shift);
        let k = piece
            .nodes
            .partition_point(|sn| sn.source_span.start < data_off);
        let local = if k == 0 {
            0
        } else {
            piece.idx.region.depth_prefix[k - 1]
        };
        piece.base_depth + local
    }

    /// Whether any diagnostic straddles current-coordinate boundary `b`
    /// (`span.start < b < span.end`) — the piece-local form of the
    /// `RegionIndex` diagnostic-straddle query.
    ///
    /// Delegates to the containing piece's shared `RegionIndex`, which scans
    /// its *whole* backing diagnostic table — including diagnostics now dropped
    /// into sibling pieces. That is exact only under the invariant **no
    /// diagnostic straddles a structurally-safe cut** (the same invariant that
    /// lets [`flatten`](Self::flatten) partition `d_range` cleanly, and that
    /// `splice_prologue` enforces by declining any edit whose cached diagnostic
    /// straddles a region boundary). A future diagnostic class that spanned a
    /// blank-line cut would break it; the `PieceSeq ≡ contiguous` proptest pins
    /// today's classes, and the `debug_assert` in `finalize` fails loudly if a
    /// live diagnostic ever straddles a piece boundary.
    #[must_use]
    pub fn diag_straddles(&self, b: u32) -> bool {
        if b > self.total_san_len() {
            return false;
        }
        let piece = &self.pieces[self.find_piece(b)];
        piece
            .idx
            .region
            .diag_straddles(to_backing(b, piece.san_shift))
    }

    /// Whether any live piece carries a whole-document-scoped diagnostic.
    #[must_use]
    pub fn has_whole_doc_scoped_diag(&self) -> bool {
        self.pieces
            .iter()
            .any(|p| p.flags.has_whole_doc_scoped_diag)
    }

    /// Whether any live piece carries an unbalanced-delimiter diagnostic.
    #[must_use]
    pub fn has_unbalanced_delimiter(&self) -> bool {
        self.pieces.iter().any(|p| p.flags.has_unbalanced_delimiter)
    }

    /// Whether any live piece carries a region-reuse-forbidding (text-coupled /
    /// opaque) node.
    #[must_use]
    pub fn has_coupled_node(&self) -> bool {
        self.pieces.iter().any(|p| p.flags.has_coupled_node)
    }

    /// Total current sanitized byte length (the trailing aggregate's
    /// `base_san`).
    #[must_use]
    pub fn total_san_len(&self) -> u32 {
        self.cum.last().map_or(0, |agg| agg.base_san)
    }

    /// Total node count across every live piece (the trailing aggregate's
    /// `cum_count`) — the spliced output's `source_nodes` length without
    /// materialising it. The diagnostics-only splice derives its reuse count as
    /// `node_count − relexed_nodes`.
    #[must_use]
    pub fn node_count(&self) -> u64 {
        u64::from(self.cum.last().map_or(0, |agg| agg.cum_count))
    }

    /// Collapse the accumulated pieces back into a single backing triple,
    /// freeing the dead middle ranges that spliced-away pieces still retain in
    /// their shared `Arc` backing, and bounding piece count + per-query cost.
    ///
    /// `O(N)`: flatten the live tables and build one fresh `PieceIndex` — **no
    /// re-lex, no sanitize**. The result is query-equivalent to `self` by
    /// construction: [`flatten`](Self::flatten) reproduces the live tables and
    /// [`from_contiguous`](Self::from_contiguous) over them answers every query
    /// identically (the same `PieceSeq ≡ contiguous` proptest that pins the
    /// single-piece base case). This is the structure-only replacement for the
    /// LSP cache's periodic forced full re-parse (#249): the incremental base
    /// never re-derives itself from source, it compacts.
    #[must_use]
    pub fn compact(&self) -> Self {
        let (nodes, pairs, diags) = self.flatten();
        Self::from_contiguous(&nodes, &pairs, &diags, self.total_san_len())
    }

    /// Number of live pieces. A fresh parse or a [`compact`](Self::compact) is
    /// one piece; each accepted splice adds at most two before the next compact.
    #[must_use]
    pub fn piece_count(&self) -> usize {
        self.pieces.len()
    }

    /// Index of the piece to answer a query at current-coordinate offset `off`
    /// (`O(log #pieces)`): the piece **ending** at `off` when `off` is a piece
    /// boundary (so its backing holds every node with `start < off`), else the
    /// piece strictly containing `off`; for `off == 0`, the first piece.
    ///
    /// Selecting the piece that *ends* at a boundary — not the one that starts
    /// there — is load-bearing: the safe-cut boundaries `0` / `len` need not be
    /// container-depth zero (an unclosed `［＃ここから…］` leaves depth nonzero at
    /// `len`), so the depth at `off` must be read from the piece that accumulated
    /// the preceding nodes, and an empty boundary piece (a zero-width re-lex) is
    /// skipped. The pair/diagnostic/normalized answers are identical from either
    /// side of a safe cut, so this choice is uniformly correct.
    fn find_piece(&self, off: u32) -> usize {
        let i = self.cum.partition_point(|agg| agg.base_san < off);
        i.saturating_sub(1).min(self.pieces.len() - 1)
    }

    /// Rebuild `cum` over a finished piece list: the running current-coordinate
    /// sanitized offset and node count at each piece boundary, plus a trailing
    /// totals sentinel.
    fn finalize(pieces: Vec<Piece>) -> Self {
        let mut cum = Vec::with_capacity(pieces.len() + 1);
        let mut base_san: u32 = 0;
        let mut cum_count: u32 = 0;
        for piece in &pieces {
            // The safe-cut invariant `diag_straddles` relies on: every live
            // diagnostic lies wholly inside its piece's backing span (none
            // straddles a piece boundary). Checked once per construction so a
            // future spanning-diagnostic class fails loudly here instead of
            // silently diverging from `flatten` at query time.
            #[cfg(debug_assertions)]
            for d in &piece.diags[piece.d_range.start..piece.d_range.end] {
                debug_assert!(
                    d.span().start >= piece.san_data.start && d.span().end <= piece.san_data.end,
                    "diagnostic {:?} straddles piece backing span {:?}",
                    d.span(),
                    piece.san_data,
                );
            }
            cum.push(PieceAgg {
                base_san,
                cum_count,
            });
            let san_len = piece.san_data.end - piece.san_data.start;
            base_san += san_len;
            cum_count += u32::try_from(piece.n_range.len()).expect("a piece's node count fits u32");
        }
        cum.push(PieceAgg {
            base_san,
            cum_count,
        });
        Self { pieces, cum }
    }
}

/// Convert a current-coordinate offset to its backing-table coordinate by
/// removing a piece's `san_shift`. The callers only ever pass an offset at or
/// after the piece's san base, so the subtraction never underflows.
fn to_backing(off: u32, san_shift: i64) -> u32 {
    shift_u32(off, -san_shift).expect("a queried offset lies at or after its piece's san base")
}

/// A re-lexed region's backing sanitized length: the old region length shifted
/// by `d_san`. Never negative for an edit the engine admits.
fn backing_len(region_len: u32, d_san: i64) -> u32 {
    shift_u32(region_len, d_san).expect("a re-lexed region length is non-negative")
}

/// Sort a diagnostics slice into position order (by span `start`, then `end`),
/// so a [`PieceSeq`] can store them contiguously and divide them at a safe cut
/// by a clean `partition_point`. A safe cut is straddled by no diagnostic, so
/// `start` alone separates the two sides; `end` is a deterministic tiebreak.
fn sort_diags(diags: &mut [Diagnostic]) {
    diags.sort_by(|a, b| {
        let (sa, sb) = (a.span(), b.span());
        (sa.start, sa.end).cmp(&(sb.start, sb.end))
    });
}

/// The result of a successful **diagnostics-only** incremental splice.
///
/// **UNSTABLE — not subject to semver until v0.5.0** (the #237 incremental API).
///
/// Returned by [`crate::reparse_incremental_diagnostics_only`]: the spliced
/// [`PieceSeq`] — the *next* edit's region-find base, from which the LSP also
/// flattens this edit's diagnostics ([`PieceSeq::collect_diagnostics`]) — plus
/// the reuse accounting the LSP reports as cache hits/misses. No
/// [`OwnedLexOutput`] (no `normalized`, `registry`, `container_pairs`, or
/// [`crate::NodeStore`]) is built — that is the whole point of the hot path.
///
/// The re-lexed region's piece carries the **re-lexed** sub-output's
/// `NodeRefOwned` handles verbatim (no store graft). This is sound only because
/// the next edit's region-find reads those handles store-independently — see
/// [`crate::reparse_incremental_diagnostics_only`] for the pinned invariant.
#[derive(Debug)]
pub struct DiagSplice {
    /// The spliced region-find representation (prefix pieces shared by `Arc` ++
    /// one re-lexed piece ++ shifted suffix pieces), byte-identical on every
    /// query and on its flattened tables to a full parse's. The LSP stores it as
    /// the next edit's base and flattens its diagnostics for the editor surface.
    pub pieces: PieceSeq,
    /// Number of `cached` source nodes carried unchanged (prefix + suffix),
    /// derived as `pieces.node_count() − relexed_nodes`.
    pub reused_nodes: u64,
    /// Number of source nodes the isolated region re-lex produced.
    pub relexed_nodes: u64,
}

/// The store-independent prologue the diagnostics-only splice
/// ([`crate::reparse_incremental_diagnostics_only`]) builds on: the validated
/// region plus the isolated region re-lex. Produced by [`splice_prologue`]
/// (steps 1–5).
struct Prologue {
    /// Minimal balanced region (sanitized coordinates) the edit re-lexes.
    region: Range<u32>,
    /// The isolated re-lex of the edited region (a full [`OwnedLexOutput`] of
    /// the region slice — `O(region)`, not `O(doc)`).
    relexed: OwnedLexOutput,
    /// Sanitized suffix shift (`new_len - old_len` over the edit), equal to the
    /// region re-lex's length delta.
    d_san: i64,
}

/// Whether `s` carries document structure that an incremental segment re-lex
/// must not silently absorb: a line terminator (could move a blank-line
/// boundary) or a directive bracket `［` / `］` (could open or close a container
/// or forward reference, both whole-document-scoped concerns — corrupting a
/// `［…］` close re-pairs inline containers document-wide).
pub(crate) fn carries_structure(s: &str) -> bool {
    s.bytes().any(|b| b == b'\n' || b == b'\r') || s.contains('［') || s.contains('］')
}

/// Whether byte offset `pos` lies inside an unterminated `［…］` directive in
/// `src` — a `［` precedes `pos` on its line with no closing `］` between.
///
/// Editing inside a directive can re-pair an inline container (e.g. a warichu
/// `［＃割り注］…［＃割り注終わり］`) document-wide without emitting any local
/// diagnostic: corrupting the `［＃割り注終わり］` close leaves the warichu open,
/// so text far downstream is reinterpreted. The isolated region re-lex cannot
/// see the matching open in the reused prefix, so the splice must decline.
pub(crate) fn inside_directive(src: &str, pos: usize) -> bool {
    let head = &src[..pos.min(src.len())];
    let Some(open) = head.rfind('［') else {
        return false;
    };
    let closed = head.rfind('］').is_some_and(|close| close > open);
    let line_broken = head.rfind('\n').is_some_and(|nl| nl > open);
    !closed && !line_broken
}

/// `value + delta` clamped into `u32`, or `None` on under/overflow.
pub(crate) fn shift_u32(value: u32, delta: i64) -> Option<u32> {
    u32::try_from(i64::from(value) + delta).ok()
}

/// Whether a diagnostic's classification depends on the whole document and so
/// cannot be reliably computed from an isolated segment.
///
/// These are the parser's document-global checks, which a segment can get
/// wrong in *either* direction (a real diagnostic missed, or a phantom
/// invented), so they are never trusted per-segment and are taken wholesale
/// from the whole-document parse:
///
/// - **Forward-reference resolution** — bouten target ambiguity
///   ([`Diagnostic::BoutenTargetAmbiguous`], look-back
///   `source[..directive]`), 縦中横 target resolution
///   ([`Diagnostic::TcyTargetNotFound`]), standalone-gaiji forward resolution
///   ([`Diagnostic::UnresolvedGaiji`]), and directive recognition that
///   depends on a matching partner
///   ([`Diagnostic::UnrecognisedContainerDirective`]).
/// - **Container / kanbun / end-of-document pairing** — bracketed kaeriten
///   (返り点) whose partner may sit in a later segment
///   ([`Diagnostic::BracketedKaeritenNoPair`]), kaeriten whose enclosing
///   漢文 context spans segments ([`Diagnostic::KaeritenOutsideKanbun`]), and
///   container-close family mismatches
///   ([`Diagnostic::MismatchedContainerClose`],
///   [`Diagnostic::MismatchedBoutenContainer`]). Block-directive
///   classification is itself context-dependent — a deeply-nested
///   heading/indent structure (e.g. 論語-style repeated `中見出し` blocks) can
///   be classified as a container only with the whole-document context, so a
///   segment re-lexed in isolation pairs its closes differently and invents a
///   phantom mismatch.
///
/// This is the single authority for which diagnostics the incremental splice
/// cannot reproduce from an isolated re-lexed region; any such diagnostic in
/// the cached or re-lexed output makes
/// [`crate::reparse_incremental_diagnostics_only`] fall back to a full parse.
/// The `corpus_incremental_merge` differential gate proves the splice is
/// byte-identical to a full parse over the reference corpus, so a missing class
/// here surfaces there as a divergence.
pub(crate) fn is_whole_document_scoped(diagnostic: &Diagnostic) -> bool {
    matches!(
        diagnostic,
        Diagnostic::BoutenTargetAmbiguous { .. }
            | Diagnostic::TcyTargetNotFound { .. }
            | Diagnostic::UnresolvedGaiji { .. }
            | Diagnostic::UnrecognisedContainerDirective { .. }
            | Diagnostic::BracketedKaeritenNoPair { .. }
            | Diagnostic::KaeritenOutsideKanbun { .. }
            | Diagnostic::MismatchedContainerClose { .. }
            | Diagnostic::MismatchedBoutenContainer { .. }
    )
}

/// Whether byte offset `j` is a candidate blank-line boundary in `bytes`: the
/// offset of an empty line that follows another line. Cutting there keeps a
/// CRLF (`\r\n`) terminator intact and starts the next segment on a blank line,
/// matching the whole-document decorative-rule isolation context. The single
/// predicate the whole-buffer oracle [`candidate_boundaries`] and the
/// production outward scan ([`minimal_balanced_region`]) both read, so they
/// cannot drift.
fn is_blank_line_boundary<S: SanitizedSrc + ?Sized>(src: &S, j: usize) -> bool {
    let len = src.len();
    j >= 1
        && j < len
        && src.byte(j - 1) == b'\n'
        && (src.byte(j) == b'\n'
            || (src.byte(j) == b'\r' && j + 1 < len && src.byte(j + 1) == b'\n'))
}

/// Candidate blank-line boundaries on the source, ascending: every byte offset
/// where [`is_blank_line_boundary`] holds. Retained as the whole-buffer oracle
/// the outward-scan region finder is proptest-pinned against; the production
/// path scans outward from the edit instead of materialising this whole vector.
#[cfg_attr(
    not(test),
    allow(
        dead_code,
        reason = "whole-buffer oracle for the outward-scan region-find proptest"
    )
)]
pub(crate) fn candidate_boundaries(source: &str) -> Vec<usize> {
    (1..source.len())
        .filter(|&j| is_blank_line_boundary(&source, j))
        .collect()
}

/// Whether a cut at sanitized offset `san_off` keeps every block container
/// and resolved delimiter pair whole.
///
/// Linear oracle retained for the property tests that pin
/// [`RegionIndex::structurally_safe`] byte-identical to it at every offset; the
/// production region finder uses the `O(log n)` indexed form. (`dead_code`
/// allowed outside `cfg(test)` because only the proptests call it now.)
#[cfg_attr(
    not(test),
    allow(
        dead_code,
        reason = "linear oracle for the RegionIndex::structurally_safe proptest"
    )
)]
pub(crate) fn structurally_safe(
    san_off: u32,
    nodes: &[SourceNodeOwned],
    pairs: &[PairLink],
) -> bool {
    // Block-container nesting depth, via the same lenient LIFO the
    // normalizer uses (a stray close on an empty stack is ignored). Reject
    // the cut if a classified span strictly contains it, or depth is
    // non-zero at it.
    let mut depth: i32 = 0;
    for sn in nodes {
        if sn.source_span.start >= san_off {
            break; // nodes are sorted by source_span.start
        }
        if sn.source_span.end > san_off {
            return false; // a classified span straddles the cut
        }
        match sn.node {
            NodeRefOwned::BlockOpen(_) => depth += 1,
            NodeRefOwned::BlockClose(_) => depth = (depth - 1).max(0),
            _ => {}
        }
    }
    if depth != 0 {
        return false;
    }
    // No resolved delimiter pair straddles the cut.
    !pairs
        .iter()
        .any(|pair| pair.open.start < san_off && pair.close.end > san_off)
}

/// The minimal region of `cached`'s sanitized buffer that must be re-lexed to
/// absorb an edit spanning sanitized byte range `edit`, bounded by
/// structurally-safe blank-line cuts. The returned range is in SANITIZED
/// coordinates, contains `edit`, and both endpoints are safe cut points
/// (document start/end, or a structurally-safe blank-line boundary), so the
/// region can be re-lexed in isolation without inventing phantom
/// unclosed/unmatched brackets or wrongly nesting a container.
///
/// Returns `None` when no sub-document benefit is provable:
/// - `cached` carries any whole-document-scoped diagnostic (forward-reference /
///   container pairing), which any edit can perturb beyond the region;
/// - `edit` is out of bounds (start > end, or end > sanitized length);
/// - the minimal safe region is the whole document (no interior safe cut).
pub(crate) fn minimal_balanced_region<S: SanitizedSrc>(
    base: &DiagBaseRef<'_, S>,
    edit: Range<usize>,
) -> Option<Range<u32>> {
    // O(1) flag read (was an O(#diags) scan): any whole-document-scoped
    // diagnostic perturbs beyond the region, so decline.
    if base.pieces.has_whole_doc_scoped_diag() {
        return None;
    }
    let san = &base.sanitized;
    let len = u32::try_from(san.len()).ok()?;
    if edit.start > edit.end || edit.end > san.len() {
        return None;
    }
    let es = u32::try_from(edit.start).ok()?;
    let ee = u32::try_from(edit.end).ok()?;

    // Outward scan (was a whole-buffer candidate scan + O(#cuts·N) safety
    // checks). The safe-cut set is the document ends `{0, len}` plus every
    // structurally-safe blank-line boundary; `region_start` is the greatest cut
    // `<= es`, `region_end` the least cut `>= ee`. Scan outward from each edit
    // endpoint and stop at the first cut — `0` always terminates the left scan,
    // `len` the right one. Each crossed blank-line boundary costs one `O(log n)`
    // safety probe; plain bytes are skipped in `O(1)`.
    let is_cut = |j: u32| -> bool {
        j == 0
            || j == len
            || (is_blank_line_boundary(san, j as usize) && base.pieces.structurally_safe(j))
    };
    let region_start = (0..=es)
        .rev()
        .find(|&j| is_cut(j))
        .expect("0 is always a cut");
    let region_end = (ee..=len)
        .find(|&j| is_cut(j))
        .expect("len is always a cut");

    if region_start == 0 && region_end == len {
        return None; // whole document — no benefit
    }
    Some(region_start..region_end)
}

/// Whether `node` makes the region unsafe to reuse incrementally, so the
/// splice must fall back to a full parse. Two cases:
///
/// - **Text-coupled**: a construct whose resolution depends on a whole-document
///   text search (forward reference, heading hint, margin note), so a plain
///   edit inside a re-lexed region can perturb a partner that sits in the
///   reused prefix/suffix. Container coupling is excluded — it is bounded by
///   [`structurally_safe`]'s container-depth check, which keeps every
///   `［＃ここから…］`/`［＃ここで…終わり］` pair whole within one region.
/// - **Opaque**: a node [`classify_node_ref`] does not understand (a future
///   variant declined for safety by the #202 splice model). The splice cannot
///   reason about its coupling, so it declines rather than risk a silent
///   divergence — keeping this guard correct by construction as the node set
///   grows.
/// - **Context-sensitive diagnostic or classification**: a `Direct`-classified
///   node whose *whole-document-scoped* diagnostic — or whose very
///   classification — depends on text **outside** the node's own region, so a
///   cross-region edit can flip it without touching the node, and the region
///   re-lex (which does not contain the node) cannot reproduce it. A
///   **reclaimed forward reference** (`X［＃「X」に傍点］`) emits
///   `BoutenTargetAmbiguous` from a look-back over the *whole* prefix, so
///   duplicating the target word in any earlier region makes a full parse
///   ambiguous while the splice keeps the cached unambiguous node. A
///   **kaeriten** (`［＃レ］`) emits `KaeritenOutsideKanbun` from a kana-prose
///   window that spans ±12 chars across blank-line boundaries, so an edit in an
///   adjacent region can flip it. (The `Coupled(ForwardReference)` *referenced*
///   forward is already declined above; `Reclaimed` is `Direct` because its
///   rendered bytes are self-contained, but its *diagnostic* is not.) A
///   **self-contained forward reference** (`［＃「X」は太字］` with no earlier
///   `X`) is `Direct` for the same self-contained-bytes reason, but its very
///   *classification* is a whole-prefix predicate (target absence):
///   introducing an earlier `X` in another region flips a full parse to
///   `Reclaimed`/`Referenced` and would resurrect the #228 double-render across
///   the splice boundary, which the node-free region re-lex cannot see — so it
///   is declined here too. A **self-contained forward heading**
///   (`［＃「X」は中見出し］` with no earlier `X`) shares this whole-prefix
///   classification — an earlier `X` flips it to a referent-bearing hint — and
///   is declined for the same reason.
///
/// Single-sources the classification through [`classify_node_ref`] (the #202
/// splice authority) so this region-reuse guard and the #202 splice cannot
/// drift.
fn node_forbids_region_reuse(node: NodeRefOwned) -> bool {
    let (role, safety) = classify_node_ref(node);
    matches!(
        safety,
        SpliceSafety::Coupled(
            CoupledKind::ForwardReference | CoupledKind::HeadingHint | CoupledKind::MarginNote
        ) | SpliceSafety::Opaque
    ) || matches!(
        role,
        RegionRole::ForwardReclaimed
            | RegionRole::ForwardSelfContained
            | RegionRole::HeadingSelfContained
            | RegionRole::Kaeriten
    )
}

/// Whether a re-lexed region's own container nesting is balanced: a lenient
/// LIFO depth over its `BlockOpen`/`BlockClose` source nodes that never goes
/// negative and returns to zero. An unbalanced region would have paired a
/// container across the former cut boundary, so an isolated re-lex would nest
/// it differently than the whole document does.
fn relexed_is_balanced(nodes: &[SourceNodeOwned]) -> bool {
    let mut depth: i32 = 0;
    for sn in nodes {
        match sn.node {
            NodeRefOwned::BlockOpen(_) => depth += 1,
            NodeRefOwned::BlockClose(_) => {
                depth -= 1;
                if depth < 0 {
                    return false;
                }
            }
            _ => {}
        }
    }
    depth == 0
}

/// The store-independent prologue the diagnostics-only splice builds on (steps
/// 1–5): find the minimal balanced region around the edit, validate the edit,
/// re-lex the region in isolation, and run the self-containment + cross-region
/// text-coupling guards. Returns the validated [`Prologue`] (region, the region
/// re-lex, and the sanitized suffix shift), or `None` for any edit that cannot
/// be proven local.
///
/// Reads only the store-free [`DiagBaseRef`] of the cached parse, so the
/// diagnostics-only hot path can run it without ever materialising the full
/// [`OwnedLexOutput`]. [`crate::reparse_incremental_diagnostics_only`] builds on
/// this single helper.
///
/// Returns `None` (the caller then full-parses, trivially correct) when:
///
/// - [`minimal_balanced_region`] declines (whole-document-scoped diagnostic in
///   `cached`, out-of-bounds edit, or no interior safe cut);
/// - `cached` carries a globally-unbalanced delimiter
///   ([`Diagnostic::UnclosedBracket`] / [`Diagnostic::UnmatchedClose`]) whose
///   open/close half swallows or strays across region boundaries;
/// - the edited bytes carry document structure ([`carries_structure`]) or sit
///   inside an open `［…］` directive; that the edit changes only bytes inside
///   `edit_old` (truly transforming `cached.sanitized` into `new_sanitized`) is
///   instead a caller precondition, checked in debug by the sanitized source's
///   `debug_assert_unchanged_outside` rather than gated at runtime;
/// - the region slice is not a sanitize fixed point in isolation;
/// - the re-lexed region is not self-contained: it carries a
///   whole-document-scoped diagnostic, an unclosed/unmatched delimiter half, or
///   unbalanced container nesting;
/// - any node in `cached` or the re-lexed region forbids region reuse
///   ([`node_forbids_region_reuse`]) — a text-coupled construct (forward
///   reference / heading hint / margin note resolving by whole-document text
///   search) or an opaque node the splice cannot reason about;
/// - a cached diagnostic straddles a region boundary, or any offset arithmetic
///   overflows.
fn splice_prologue<S: SanitizedSrc>(
    base: &DiagBaseRef<'_, S>,
    new_sanitized: &S,
    edit_old: Range<usize>,
) -> Option<Prologue> {
    // 1. Minimal balanced region (sanitized coordinates).
    let region = minimal_balanced_region(base, edit_old.clone())?;
    let r_start = region.start as usize;

    // A globally-unbalanced delimiter in `cached` — an unclosed open
    // (`UnclosedBracket`) or a stray close (`UnmatchedClose`) — makes the whole
    // document's classification depend on a span that crosses region
    // boundaries: an unclosed `《` swallows every following `《…》` so the
    // whole-document parse classifies no ruby there, yet a region re-lexed in
    // isolation (balanced on its own) would invent them. These are not in the
    // whole-document-scoped diagnostic set (they have no partner span to pair),
    // so guard them explicitly here — O(1) flag read (was an O(#diags) scan).
    if base.pieces.has_unbalanced_delimiter() {
        return None;
    }

    // 2. Edit validation — `edit_old` must be in bounds and the edited bytes
    //    must touch no document structure. That the edit changes *only* bytes
    //    inside `edit_old` (so it truly transforms `cached.sanitized` into
    //    `new_sanitized`) is the caller's precondition: it is restated in debug
    //    by `debug_assert_unchanged_outside` below, not gated at runtime, so a
    //    genuinely malformed edit is a caller bug rather than a silent
    //    full-parse fallback (see that call's comment for why).
    let old_source = &base.sanitized;
    if edit_old.start > edit_old.end || edit_old.end > old_source.len() {
        return None;
    }
    let edit_delta =
        i64::try_from(new_sanitized.len()).ok()? - i64::try_from(old_source.len()).ok()?;
    let new_edit_end = usize::try_from(i64::try_from(edit_old.end).ok()? + edit_delta).ok()?;
    if new_edit_end > new_sanitized.len() {
        return None;
    }
    // The bytes outside `edit_old` must be byte-identical between the cached and
    // new sanitized buffers (the prefix `[0, edit_old.start)` plus the suffix
    // after the edit). This used to be an unconditional prefix/suffix `memcmp`
    // here; it is gone primarily for **time**, not defence — on the rope source
    // a later PR introduces, slicing `[0, edit_old.start)` is `O(prefix) =
    // O(doc)` per edit (a `RopeSlice` equality still walks `O(min len)`), which
    // is incompatible with the per-keystroke hot path. The check now lives as
    // the caller's precondition: in debug it is restated by
    // `debug_assert_unchanged_outside` (a full `memcmp` for `&str`, byte-
    // identical to the deleted code; a no-alloc probe for a rope), and the
    // production LSP caller guarantees it by deriving `edit_old` from the
    // real sanitized diff.
    #[cfg(debug_assertions)]
    old_source.debug_assert_unchanged_outside(new_sanitized, edit_old.clone(), new_edit_end);
    let old_slice = old_source.slice(edit_old.clone())?;
    let new_slice = new_sanitized.slice(edit_old.start..new_edit_end)?;
    if carries_structure(&old_slice) || carries_structure(&new_slice) {
        return None;
    }
    // An edit *inside* a `［…］` directive carries no structural byte itself yet
    // can re-pair an inline container across the whole document (the region
    // re-lex sees only the corrupted close, never the matching open in the
    // reused prefix). The prefix up to `edit_old.start` is byte-identical
    // between old and new, so checking `old_source` covers both. `inside_directive`
    // reads only `src[..pos]`, so the head slice with `pos == head.len()` is
    // byte-identical to passing the whole buffer with `pos == edit_old.start`.
    let head = old_source.slice(0..edit_old.start)?;
    if inside_directive(&head, head.len()) {
        return None;
    }

    // Decline if any cached diagnostic straddles a region boundary: it is
    // neither reproduced by the isolated region re-lex nor safely shiftable. A
    // span is bad iff it straddles `region.start` or `region.end` (proven:
    // `!(prefix || suffix || inside)` ⇔ `start < rs < end` ∨ `start < re < end`),
    // so two O(log n) probes replace the O(#diags) scan.
    if base.pieces.diag_straddles(region.start) || base.pieces.diag_straddles(region.end) {
        return None;
    }

    // 3. Re-lex the region in isolation. The region slice is already sanitized
    //    (a slice of the sanitized fixed point `new_sanitized`); if re-sanitize
    //    changes it, bail conservatively.
    let new_r_end = usize::try_from(i64::from(region.end) + edit_delta).ok()?;
    let new_region_src = new_sanitized.slice(r_start..new_r_end)?;
    let relexed = Document::new(&*new_region_src).parse_owned();
    if relexed.sanitized != *new_region_src {
        return None;
    }

    // 4. Region self-containment — so the isolated re-lex equals an in-context
    //    re-lex.
    if relexed.diagnostics.iter().any(is_whole_document_scoped) {
        return None;
    }
    if relexed.diagnostics.iter().any(|d| {
        matches!(
            d,
            Diagnostic::UnclosedBracket { .. } | Diagnostic::UnmatchedClose { .. }
        )
    }) {
        return None;
    }
    if !relexed_is_balanced(&relexed.source_nodes) {
        return None;
    }

    // 5. Cross-region text-coupling fallback. Any forward reference / heading
    //    hint / margin note anywhere (cached or relexed) resolves by
    //    whole-document text search, so a plain region edit could perturb a
    //    partner in the reused prefix/suffix. Conservative — a later PR can
    //    narrow this to the affected partners. The base side is an O(1) flag
    //    read; only the re-lexed side (O(region)) is still scanned.
    if base.pieces.has_coupled_node()
        || relexed
            .source_nodes
            .iter()
            .any(|sn| node_forbids_region_reuse(sn.node))
    {
        return None;
    }

    // The sanitized suffix shift equals `relexed.sanitized_len - region_len`,
    // which is exactly `edit_delta` because `new_r_end = r_end + edit_delta`.
    Some(Prologue {
        region,
        relexed,
        d_san: edit_delta,
    })
}

/// **UNSTABLE — not subject to semver until v0.5.0.**
///
/// The LSP's per-keystroke hot path: compute the spliced diagnostics for the
/// edited text `new_sanitized` (a sanitized fixed point) from the store-free
/// [`DiagBaseRef`] of the prior parse and the single sanitized-coordinate edit
/// `edit_old`, **without building an [`OwnedLexOutput`]**.
///
/// A consumer trace established that the debounced `publishDiagnostics` path
/// reads only `cache.diagnostics()`; the full tree (`with_tree`) is needed only
/// by the rare F2 rename gesture. So this engine runs the shared
/// [`splice_prologue`] (`O(region)` re-lex) and then **only** the diagnostics
/// splice plus the store-free `source_nodes`/`pairs` splice the *next* edit's
/// region-find consumes — never the `O(doc)` work a full parse does (the
/// normalized/sanitized string build, the store + registry Eytzinger table, the
/// container-pairs table). The result is `O(region + #diagnostics)`.
///
/// Its `diagnostics` flatten from the spliced [`PieceSeq`] over the shared
/// [`splice_prologue`], which the `corpus_incremental_merge` differential gate
/// pins byte-identical to a full parse over the reference corpus.
///
/// # Store-free-base soundness
///
/// The region's nodes carry the **re-lexed** sub-output's [`NodeRefOwned`]
/// handles verbatim — **no store graft, no store clone**. A `NodeRefOwned` that
/// embeds a `StrId`/`ContentRange` resolves only against the store it was built
/// in (here, the re-lex's transient store, which is dropped). This is sound
/// **only** because the resulting [`DiagSplice::pieces`] (the maintained
/// sequence's nodes/pairs) are consumed exclusively by the next edit's
/// region-find ([`minimal_balanced_region`] → [`PieceSeq::structurally_safe`],
/// [`relexed_is_balanced`] via the prologue, and the
/// [`node_forbids_region_reuse`] coupling guard),
/// every one of which reads only a node's `source_span` (`Copy`,
/// store-independent) and the `NodeRefOwned` **discriminant** (`BlockOpen` /
/// `BlockClose` / `Inline` / …) — and never resolves a `StrId`/`ContentRange`
/// against a store. The `debug_assert` below pins this invariant: a future
/// change that resolves a region node's payload here must instead graft it into
/// a real store rather than read it off the transient re-lex store this
/// `relexed` is about to drop.
#[must_use]
pub(crate) fn reparse_incremental_diagnostics_only<S: SanitizedSrc>(
    base: &DiagBaseRef<'_, S>,
    new_sanitized: &S,
    edit_old: Range<usize>,
) -> Option<DiagSplice> {
    // Steps 1–5 (store-independent), via the shared prologue.
    let Prologue {
        region,
        relexed,
        d_san,
    } = splice_prologue(base, new_sanitized, edit_old)?;

    // Invariant pin (store-free base, see the doc above): the region nodes are
    // carried with the re-lex's own `NodeRefOwned` handles and never resolved
    // against a store on this path. If a future change needs a region node's
    // resolved payload here, it must graft into a real store, not read it off
    // the transient re-lex store this `relexed` is about to drop. This assert
    // trips loudly if such a resolution is wired in by accident: the region
    // nodes must stay parallel to the re-lex registry.
    debug_assert_eq!(
        relexed.registry.len(),
        relexed.source_nodes.len(),
        "diagnostics-only path carries re-lex node handles verbatim (no store graft); \
         resolving a region node's payload here is unsound — graft into a store instead",
    );

    // Splice the maintained `PieceSeq` in `O(region + #pieces)`: the prefix and
    // shifted suffix pieces are carried by `Arc` share (no per-node rewrite), the
    // edited region becomes one re-lexed piece, and `cum` is patched. This is the
    // whole point of the hot path — no `source_nodes`/`pairs`/`diagnostics`
    // whole-table re-materialization, no `RegionIndex` rebuild. The result's
    // flattened tables and every query are byte-identical to a full parse's: the
    // `PieceSeq ≡ contiguous` adversarial proptest over multi-edit splice runs,
    // plus the `corpus_incremental_merge` differential gate, pin this.
    let pieces = base.pieces.splice(
        region,
        &relexed.source_nodes,
        &relexed.pairs,
        &relexed.diagnostics,
        d_san,
    );

    // Reuse accounting (the LSP surface reports these as cache hits/misses): the
    // re-lexed piece's nodes are the misses; every other node in the spliced
    // sequence (the `Arc`-shared prefix + shifted suffix) is reused.
    let relexed_nodes = u64::try_from(relexed.source_nodes.len()).ok()?;
    let reused_nodes = pieces.node_count().saturating_sub(relexed_nodes);

    Some(DiagSplice {
        pieces,
        reused_nodes,
        relexed_nodes,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Document;

    /// Parse `src` to a real owned lex output.
    fn owned(src: &str) -> OwnedLexOutput {
        Document::new(src).parse_owned()
    }

    /// A [`PieceSeq`] over a cached output's store-free tables — the single-piece
    /// base the region-find prologue reads.
    fn pieces_of(cached: &OwnedLexOutput) -> PieceSeq {
        PieceSeq::from_contiguous(
            &cached.source_nodes,
            &cached.pairs,
            &cached.diagnostics,
            cached.sanitized_len,
        )
    }

    /// The full ascending safe-cut set the region finder works over, for
    /// asserting endpoint membership.
    fn safe_cuts(cached: &OwnedLexOutput) -> Vec<u32> {
        let len = u32::try_from(cached.sanitized.len()).unwrap();
        let mut cuts = vec![0u32];
        for b in candidate_boundaries(&cached.sanitized) {
            let b = u32::try_from(b).unwrap();
            if b != 0 && b != len && structurally_safe(b, &cached.source_nodes, &cached.pairs) {
                cuts.push(b);
            }
        }
        cuts.push(len);
        cuts
    }

    fn assert_endpoint_safe(cached: &OwnedLexOutput, off: u32) {
        let len = u32::try_from(cached.sanitized.len()).unwrap();
        assert!(
            off == 0 || off == len || structurally_safe(off, &cached.source_nodes, &cached.pairs),
            "endpoint {off} must be a safe cut (0/len or structurally safe)"
        );
    }

    #[test]
    fn edit_inside_paragraph_shrinks_below_whole_doc() {
        // Three blank-line-separated paragraphs, all plain text.
        let src = "あいうえお\n\nかきくけこ\n\nさしすせそ\n";
        let cached = owned(src);
        let pieces = pieces_of(&cached);
        let len = u32::try_from(cached.sanitized.len()).unwrap();
        // Edit inside the middle paragraph "かきくけこ".
        let mid = src.find("かきくけこ").unwrap();
        let edit = mid..mid + "かき".len();
        let region =
            minimal_balanced_region(&DiagBaseRef::from_cached(&cached, &pieces), edit.clone())
                .expect("interior region");
        // Strictly smaller than the whole document.
        assert!(
            region.start > 0 || region.end < len,
            "region {region:?} must be strictly smaller than 0..{len}"
        );
        assert_ne!(region, 0..len);
        // Contains the edit.
        assert!(region.start as usize <= edit.start && region.end as usize >= edit.end);
        // Both endpoints are safe cuts.
        assert_endpoint_safe(&cached, region.start);
        assert_endpoint_safe(&cached, region.end);
        let cuts = safe_cuts(&cached);
        assert!(cuts.contains(&region.start), "start in safe-cut set");
        assert!(cuts.contains(&region.end), "end in safe-cut set");
    }

    #[test]
    fn single_paragraph_has_no_interior_cut() {
        let src = "あいうえおかきくけこ\n";
        let cached = owned(src);
        let pieces = pieces_of(&cached);
        let edit = 3..6;
        assert_eq!(
            minimal_balanced_region(&DiagBaseRef::from_cached(&cached, &pieces), edit),
            None
        );
    }

    #[test]
    fn whole_document_scoped_diagnostic_yields_none() {
        // An unresolved standalone gaiji reference is whole-document-scoped.
        let src = "前の段落\n\n※［＃存在しない外字、第1水準1-2-3］\n\n後の段落\n";
        let cached = owned(src);
        let pieces = pieces_of(&cached);
        assert!(
            cached.diagnostics.iter().any(is_whole_document_scoped),
            "fixture must carry a whole-document-scoped diagnostic, got {:?}",
            cached.diagnostics
        );
        // Region declines regardless of where the edit sits.
        assert_eq!(
            minimal_balanced_region(&DiagBaseRef::from_cached(&cached, &pieces), 0..1),
            None
        );
        let mid = src.find("後の段落").unwrap();
        assert_eq!(
            minimal_balanced_region(&DiagBaseRef::from_cached(&cached, &pieces), mid..mid + 3),
            None
        );
    }

    #[test]
    fn out_of_bounds_edit_yields_none() {
        let src = "あいうえお\n\nかきくけこ\n";
        let cached = owned(src);
        let pieces = pieces_of(&cached);
        let len = cached.sanitized.len();
        // end past sanitized length.
        assert_eq!(
            minimal_balanced_region(&DiagBaseRef::from_cached(&cached, &pieces), 0..len + 10),
            None
        );
        // start > end (built without a literal reversed range to satisfy the
        // reversed_empty_ranges lint).
        let reversed = Range {
            start: 5usize,
            end: 2,
        };
        assert_eq!(
            minimal_balanced_region(&DiagBaseRef::from_cached(&cached, &pieces), reversed),
            None
        );
    }

    #[test]
    fn edit_spanning_blank_line_widens_to_both_paragraphs() {
        let src = "あいうえお\n\nかきくけこ\n\nさしすせそ\n";
        let cached = owned(src);
        let pieces = pieces_of(&cached);
        // Edit straddles the blank line between paragraph 1 and 2.
        let p1 = src.find("うえお").unwrap();
        let p2_end = src.find("かきく").unwrap() + "かきく".len();
        let edit = p1..p2_end;
        let region =
            minimal_balanced_region(&DiagBaseRef::from_cached(&cached, &pieces), edit.clone())
                .expect("region");
        // Must contain the whole straddled range, hence both flanking paragraphs.
        assert!(region.start as usize <= edit.start);
        assert!(region.end as usize >= edit.end);
        // Region must include the entire first paragraph text and the second.
        assert!((region.start as usize) <= src.find("あいうえお").unwrap());
        assert!((region.end as usize) >= src.find("かきくけこ").unwrap() + "かきくけこ".len());
        assert_endpoint_safe(&cached, region.start);
        assert_endpoint_safe(&cached, region.end);
    }

    #[test]
    fn crlf_source_region_is_in_sanitized_coordinates() {
        // CRLF source (as real Aozora Bunko files are). Sanitize strips the
        // \r, so sanitized offsets are smaller than raw offsets.
        let src = "あいうえお\r\n\r\nかきくけこ\r\n\r\nさしすせそ\r\n";
        let cached = owned(src);
        let pieces = pieces_of(&cached);
        let san = &cached.sanitized;
        assert!(!san.contains('\r'), "sanitized buffer drops CR");
        let len = u32::try_from(san.len()).unwrap();
        // Edit the middle paragraph in SANITIZED coordinates.
        let mid = san.find("かきくけこ").unwrap();
        let edit = mid..mid + "かき".len();
        let region =
            minimal_balanced_region(&DiagBaseRef::from_cached(&cached, &pieces), edit.clone())
                .expect("region");
        assert_ne!(region, 0..len);
        assert!(region.start as usize <= edit.start && region.end as usize >= edit.end);
        assert_endpoint_safe(&cached, region.start);
        assert_endpoint_safe(&cached, region.end);
        // The region must not reach into the third paragraph.
        let p3 = san.find("さしすせそ").unwrap();
        assert!(
            (region.end as usize) <= p3,
            "region {region:?} must not include paragraph 3 at {p3}"
        );
    }

    #[test]
    fn crlf_single_paragraph_yields_none() {
        let src = "あいうえおかきくけこ\r\n";
        let cached = owned(src);
        let pieces = pieces_of(&cached);
        assert_eq!(
            minimal_balanced_region(&DiagBaseRef::from_cached(&cached, &pieces), 0..3),
            None
        );
    }

    #[test]
    fn empty_document_yields_none() {
        let cached = owned("");
        let pieces = pieces_of(&cached);
        assert_eq!(cached.sanitized.len(), 0, "empty source sanitizes to empty");
        // Zero-width edit on the empty document: the region is 0..0 == 0..len,
        // i.e. the whole (empty) document, so there is no sub-document benefit.
        let zero = 0usize;
        assert_eq!(
            minimal_balanced_region(&DiagBaseRef::from_cached(&cached, &pieces), zero..zero),
            None
        );
    }

    #[test]
    fn boundary_landing_edits_return_empty_safe_regions() {
        // A zero-width edit at a document end or exactly on an interior safe
        // cut yields the minimal empty region pinned to that offset — a genuine
        // cut, so the PR3 splice can re-lex the inserted text from a clean
        // boundary. (Edits go through a variable to avoid the
        // `reversed_empty_ranges` lint on literal equal-bound ranges.)
        let src = "あいうえお\n\nかきくけこ\n\nさしすせそ\n";
        let cached = owned(src);
        let pieces = pieces_of(&cached);
        let len = u32::try_from(cached.sanitized.len()).unwrap();
        let at_offset = |at: usize| {
            minimal_balanced_region(&DiagBaseRef::from_cached(&cached, &pieces), at..at)
        };

        // Document start and end.
        assert_eq!(at_offset(0), Some(Range { start: 0, end: 0 }));
        assert_eq!(
            at_offset(len as usize),
            Some(Range {
                start: len,
                end: len
            }),
        );

        // Exactly on an interior safe cut (a blank-line boundary).
        let interior = safe_cuts(&cached)
            .into_iter()
            .find(|&c| c != 0 && c != len)
            .expect("a three-paragraph doc has an interior safe cut");
        assert_eq!(
            at_offset(interior as usize),
            Some(Range {
                start: interior,
                end: interior,
            }),
        );
    }

    // ---- shared decline tests (diagnostics-only engine) ----

    /// Apply a single-region edit (`replacement` swapped for `edit`) to `san`.
    fn apply_edit(san: &str, edit: Range<usize>, replacement: &str) -> String {
        let mut out = String::with_capacity(san.len() + replacement.len());
        out.push_str(&san[..edit.start]);
        out.push_str(replacement);
        out.push_str(&san[edit.end..]);
        out
    }

    #[test]
    fn breaking_a_gaiji_declines() {
        // A resolvable standalone gaiji in the middle paragraph; an edit that
        // mangles its 面区点 tail makes the isolated re-lex emit a
        // whole-document-scoped UnresolvedGaiji, so the splice declines.
        let cached = owned("前の段落\n\n※［＃ばける、第3水準1-15-94］\n\n後の段落\n");
        assert!(
            !cached.diagnostics.iter().any(is_whole_document_scoped),
            "fixture's gaiji resolves cleanly: {:?}",
            cached.diagnostics,
        );
        let san = cached.sanitized.clone();
        // Replace the 面区点 tail (no bracket / newline touched) with an
        // out-of-range value the resolver cannot map.
        let at = san.find("1-15-94").expect("menkuten tail");
        let edit = at..at + "1-15-94".len();
        let new_san = apply_edit(&san, edit.clone(), "9-99-99");
        let full = owned(&new_san);
        assert_eq!(full.sanitized, new_san, "edit is a sanitize fixed point");
        assert!(
            full.diagnostics.iter().any(is_whole_document_scoped),
            "the mangled gaiji is whole-document-scoped in a full parse: {:?}",
            full.diagnostics,
        );

        let pieces = pieces_of(&cached);
        let base = DiagBaseRef::from_cached(&cached, &pieces);
        assert!(
            reparse_incremental_diagnostics_only(&base, &new_san.as_str(), edit).is_none(),
            "a broken gaiji must decline to the full-parse fallback",
        );
    }

    #[test]
    fn inserted_lone_open_bracket_declines() {
        // Inserting a lone 《 into a paragraph leaves an unclosed ruby bracket;
        // the isolated re-lex reports UnclosedBracket, so the splice declines.
        let cached = owned("前の段落\n\nかきくけこ\n\n後の段落\n");
        let san = cached.sanitized.clone();
        let at = san.find("くけこ").expect("middle paragraph");
        let edit = at..at;
        let new_san = apply_edit(&san, edit.clone(), "《");
        // `《` carries no document structure, so it passes the edit guard and is
        // declined by the unclosed-bracket self-containment check instead.
        assert!(!carries_structure("《"), "《 is not a structural byte");

        let pieces = pieces_of(&cached);
        let base = DiagBaseRef::from_cached(&cached, &pieces);
        assert!(
            reparse_incremental_diagnostics_only(&base, &new_san.as_str(), edit).is_none(),
            "an unclosed delimiter half must decline",
        );
    }

    #[test]
    fn inside_directive_detects_open_bracket_context() {
        // Inside an open ［…］ on the same line (a corruptible directive close).
        assert!(inside_directive(
            "本文［＃割り注終わり］",
            "本文［＃割り注".len()
        ));
        // After the closing ］ — not inside.
        assert!(!inside_directive("本文［＃注］後", "本文［＃注］".len()));
        // No directive bracket at all.
        assert!(!inside_directive("ただの本文", "ただの".len()));
        // A line break after the ［ ends the directive's line.
        assert!(!inside_directive(
            "本文［＃壊れ\n次行",
            "本文［＃壊れ\n".len()
        ));
        // At/just before the ［ — not yet inside.
        assert!(!inside_directive("本文［＃注］", "本文".len()));
    }

    #[test]
    fn edit_inside_directive_close_declines() {
        // Editing inside a `［＃割り注終わり］` close keyword corrupts the
        // directive; the matching open lives in the reused prefix so the region
        // re-lex cannot see the breakage. `inside_directive` must decline even
        // though the inserted byte carries no structure of its own. (Regression
        // for the corpus divergence #284 surfaced: 折口春洋/島の便り.txt.)
        let cached = owned(
            "序文の段落です。\n\n本文［＃割り注］注記の文字［＃割り注終わり］続き\n\n末尾の段落。\n",
        );
        let san = cached.sanitized.clone();
        let close = san.find("割り注終わり").expect("warichu close");
        let at = close + "割り".len(); // inside the close keyword
        assert!(
            inside_directive(&san, at),
            "edit must be inside the directive"
        );
        let edit = at..at;
        let new_san = apply_edit(&san, edit.clone(), "x");
        assert!(!carries_structure("x"), "x is not a structural byte");
        let pieces = pieces_of(&cached);
        let base = DiagBaseRef::from_cached(&cached, &pieces);
        assert!(
            reparse_incremental_diagnostics_only(&base, &new_san.as_str(), edit).is_none(),
            "an edit inside a ［…］ directive must decline",
        );
    }

    #[test]
    fn forward_reference_doc_declines_via_coupling() {
        // A clean forward bouten reference (［＃「青空」に傍点］ pointing back at
        // the earlier 青空) is text-coupled: editing an unrelated paragraph could
        // still perturb the reference, so the splice declines even though the
        // edit itself is local.
        let cached = owned("彼は青空を見た\n\n［＃「青空」に傍点］\n\n後の段落\n");
        assert!(
            !cached.diagnostics.iter().any(is_whole_document_scoped),
            "the forward reference resolves unambiguously: {:?}",
            cached.diagnostics,
        );
        assert!(
            cached
                .source_nodes
                .iter()
                .any(|sn| node_forbids_region_reuse(sn.node)),
            "fixture must carry a text-coupled node, got {:?}",
            cached
                .source_nodes
                .iter()
                .map(|sn| sn.node)
                .collect::<Vec<_>>(),
        );
        let san = cached.sanitized.clone();
        let at = san.find("後の段落").expect("trailing paragraph") + "後の".len();
        let edit = at..at;
        let new_san = apply_edit(&san, edit.clone(), "ね");

        let pieces = pieces_of(&cached);
        let base = DiagBaseRef::from_cached(&cached, &pieces);
        assert!(
            reparse_incremental_diagnostics_only(&base, &new_san.as_str(), edit).is_none(),
            "a document with a forward reference must decline (text coupling)",
        );
    }

    #[test]
    fn reclaimed_forward_bouten_doc_declines() {
        // A *reclaimed* forward bouten (青空［＃「青空」に傍点］, target adjacent)
        // is rendered self-contained (Direct), but its BoutenTargetAmbiguous
        // diagnostic looks back over the whole prefix. Duplicating the target in
        // an earlier region makes a full parse ambiguous while a naive splice
        // keeps the cached unambiguous node — so the node must forbid region
        // reuse. (Regression for the verify finding on #284.)
        let cached = owned("むかし。\n\n青空［＃「青空」に傍点］\n");
        let san = cached.sanitized.clone();
        assert!(
            !cached.diagnostics.iter().any(is_whole_document_scoped),
            "the reclaimed forward resolves unambiguously: {:?}",
            cached.diagnostics,
        );
        let at = san.find("むかし").expect("first paragraph") + "むかし".len();
        let edit = at..at;
        let new_san = apply_edit(&san, edit.clone(), "青空");
        // The duplicated target genuinely makes the full parse ambiguous.
        let full = Document::new(new_san.as_str()).parse_owned();
        assert!(
            full.diagnostics
                .iter()
                .any(|d| matches!(d, Diagnostic::BoutenTargetAmbiguous { .. })),
            "the duplicated target must make the full parse ambiguous: {:?}",
            full.diagnostics,
        );
        let pieces = pieces_of(&cached);
        let base = DiagBaseRef::from_cached(&cached, &pieces);
        assert!(
            reparse_incremental_diagnostics_only(&base, &new_san.as_str(), edit).is_none(),
            "a reclaimed forward bouten must forbid region reuse",
        );
    }

    /// E1-1: a no-referent forward ([`ForwardOrigin::SelfContained`]) is
    /// `Direct` (its rendered bytes are self-contained), but its very
    /// *classification* is a whole-prefix predicate (target absence): a distant
    /// edit that introduces an earlier copy of the target flips a full parse to
    /// `Reclaimed`/`Referenced` and would resurrect the #228 double-render
    /// across the splice boundary. So its region must not be reused — like the
    /// reclaimed case above, but for classification rather than diagnostic.
    /// Constructed directly; the source-driven decline test arrives with the
    /// producer in E1-2/E1-3.
    #[test]
    fn self_contained_forward_forbids_region_reuse() {
        use aozora_syntax::alloc_owned::OwnedAllocator;
        use aozora_syntax::{ForwardAttr, ForwardOrigin};

        let mut a = OwnedAllocator::new();
        let t = a.content_plain("X");
        let node = a.forward_format(ForwardAttr::Bold, t, ForwardOrigin::SelfContained);
        assert!(
            node_forbids_region_reuse(NodeRefOwned::Inline(node)),
            "SelfContained must forbid region reuse: its classification depends \
             on the whole preceding prefix",
        );
    }

    /// E1-4: a self-contained heading hint's classification is a whole-prefix
    /// predicate (target absence), like the forward case above, so its region
    /// must not be reused — an earlier 序章 in a far region would flip a full
    /// parse to a referent-bearing hint.
    #[test]
    fn self_contained_heading_forbids_region_reuse() {
        use aozora_syntax::alloc_owned::OwnedAllocator;
        use aozora_syntax::{HeadingKind, HeadingStyle};

        let mut a = OwnedAllocator::new();
        let node = a.heading_hint(HeadingKind::Medium, HeadingStyle::Standard, "序章", true);
        assert!(
            node_forbids_region_reuse(NodeRefOwned::Inline(node)),
            "self-contained heading hint must forbid region reuse",
        );
    }

    #[test]
    fn self_contained_forward_doc_declines() {
        // E1-2 producer end-to-end: a no-referent ［＃「強調」は太字］ (no earlier
        // 強調) parses as a self-contained forward — Direct, but its very
        // classification is a whole-prefix predicate. Introducing an earlier 強調
        // in a far region flips a full parse to a referent-present forward; a
        // naive splice would keep the cached self-contained node *and* the new
        // upstream copy (the #228 double-render). So the node forbids region
        // reuse and the incremental reparse must decline.
        let cached = owned("むかし。\n\n本文［＃「強調」は太字］\n");
        let san = cached.sanitized.clone();
        assert!(
            cached
                .source_nodes
                .iter()
                .any(|sn| node_forbids_region_reuse(sn.node)),
            "fixture must carry the self-contained forward node, got {:?}",
            cached
                .source_nodes
                .iter()
                .map(|sn| sn.node)
                .collect::<Vec<_>>(),
        );
        let at = san.find("むかし").expect("first paragraph") + "むかし".len();
        let edit = at..at;
        let new_san = apply_edit(&san, edit.clone(), "強調");
        let pieces = pieces_of(&cached);
        let base = DiagBaseRef::from_cached(&cached, &pieces);
        assert!(
            reparse_incremental_diagnostics_only(&base, &new_san.as_str(), edit).is_none(),
            "a self-contained forward must forbid region reuse",
        );
    }

    #[test]
    fn kaeriten_doc_declines() {
        // A kaeriten (［＃（レ）］) emits KaeritenOutsideKanbun from a kana-prose
        // window that spans ±12 chars across a blank-line boundary, so flipping
        // an adjacent region kanji→kana makes a full parse emit it while a splice
        // (whose re-lexed region holds no kaeriten) drops it. The node must
        // forbid region reuse.
        let cached = owned("あいうえお［＃（レ）］\n\n漢字漢字漢字漢字漢\n");
        let san = cached.sanitized.clone();
        assert!(
            !cached.diagnostics.iter().any(is_whole_document_scoped),
            "the kaeriten in kanji context is clean: {:?}",
            cached.diagnostics,
        );
        let p2 = san.find("漢字漢字漢字漢字漢").expect("kanji paragraph");
        let edit = p2..p2 + "漢字漢字漢字漢字漢".len();
        let new_san = apply_edit(&san, edit.clone(), "かきくけこかきくけこ");
        let full = Document::new(new_san.as_str()).parse_owned();
        assert!(
            full.diagnostics
                .iter()
                .any(|d| matches!(d, Diagnostic::KaeritenOutsideKanbun { .. })),
            "the kana flip must make the full parse emit KaeritenOutsideKanbun: {:?}",
            full.diagnostics,
        );
        let pieces = pieces_of(&cached);
        let base = DiagBaseRef::from_cached(&cached, &pieces);
        assert!(
            reparse_incremental_diagnostics_only(&base, &new_san.as_str(), edit).is_none(),
            "a document with a kaeriten must forbid region reuse",
        );
    }
}

/// Property tests pinning every `RegionIndex` query byte-identical to the linear
/// oracle it replaces, over randomly-assembled aozora-shaped documents (#237
/// Tier 2). The oracles (`structurally_safe`, `candidate_boundaries`) and a
/// whole-buffer reference region finder are the ground truth; the
/// indexed/outward-scan production forms must match them exactly, which (with the
/// corpus differential gate) is the byte-identity guarantee.
#[cfg(test)]
mod oracle_proptests {
    use core::iter::once;
    use core::mem::{discriminant, swap};
    use core::ops::Range;

    use proptest::collection::vec as prop_vec;
    use proptest::prelude::*;

    use super::{
        DiagBaseRef, OwnedLexOutput, PieceSeq, Prologue, RegionIndex, candidate_boundaries,
        is_whole_document_scoped, minimal_balanced_region, node_forbids_region_reuse,
        splice_prologue, structurally_safe,
    };
    use crate::{Diagnostic, Document, PairLink, SourceNodeOwned};

    /// Parse a generated document to its owned lex output.
    fn owned(src: &str) -> OwnedLexOutput {
        Document::new(src).parse_owned()
    }

    /// A [`RegionIndex`] over a cached output's store-free tables.
    fn idx_of(cached: &OwnedLexOutput) -> RegionIndex {
        RegionIndex::build(&cached.source_nodes, &cached.pairs, &cached.diagnostics)
    }

    /// A [`PieceSeq`] over a cached output's store-free tables (the single-piece
    /// region-find base the prologue reads).
    fn pieces_of(cached: &OwnedLexOutput) -> PieceSeq {
        PieceSeq::from_contiguous(
            &cached.source_nodes,
            &cached.pairs,
            &cached.diagnostics,
            cached.sanitized_len,
        )
    }

    /// The pre-Tier-2 whole-buffer region finder, retained verbatim as the
    /// reference the outward-scan [`minimal_balanced_region`] must match: all
    /// structurally-safe blank-line cuts (plus document ends), then the greatest
    /// cut `<= es` and least cut `>= ee`.
    fn reference_region(cached: &OwnedLexOutput, edit: Range<usize>) -> Option<Range<u32>> {
        if cached.diagnostics.iter().any(is_whole_document_scoped) {
            return None;
        }
        let san = &cached.sanitized;
        let len = u32::try_from(san.len()).ok()?;
        if edit.start > edit.end || edit.end > san.len() {
            return None;
        }
        let es = u32::try_from(edit.start).ok()?;
        let ee = u32::try_from(edit.end).ok()?;
        let mut cuts = vec![0_u32];
        for b in candidate_boundaries(san) {
            let b = u32::try_from(b).expect("boundary fits u32");
            if b != 0 && b != len && structurally_safe(b, &cached.source_nodes, &cached.pairs) {
                cuts.push(b);
            }
        }
        cuts.push(len);
        let region_start = cuts.iter().copied().filter(|&c| c <= es).max()?;
        let region_end = cuts.iter().copied().filter(|&c| c >= ee).min()?;
        if region_start == 0 && region_end == len {
            return None;
        }
        Some(region_start..region_end)
    }

    /// The linear diagnostic-straddle predicate the prologue used before the
    /// indexed probes: a cached diagnostic is bad iff it is neither fully before,
    /// fully after, nor fully inside the region.
    fn any_diag_straddles(diags: &[Diagnostic], region: Range<u32>) -> bool {
        diags.iter().any(|d| {
            let span = d.span();
            let prefix = span.end <= region.start;
            let suffix = span.start >= region.end;
            let inside = span.start >= region.start && span.end <= region.end;
            !(prefix || suffix || inside)
        })
    }

    /// Randomly-assembled aozora-shaped text: plain kana, blank-line breaks
    /// (LF and CRLF), ruby, lone delimiter halves, block containers, standalone
    /// blocks, gaiji, and a forward reference — exercising nodes, pairs,
    /// container depth, and every diagnostic-class flag.
    fn doc_strategy() -> impl Strategy<Value = String> {
        let fragment = prop_oneof![
            Just("あ"),
            Just("い"),
            Just("ん。"),
            Just("\n"),
            Just("\n\n"),
            Just("\r\n\r\n"),
            Just("｜漢字《かんじ》"),
            Just("《"),
            Just("》"),
            Just("［＃ここから２字下げ］\n"),
            Just("［＃ここで字下げ終わり］\n"),
            Just("［＃改ページ］"),
            Just("※［＃ばける、第3水準1-15-94］"),
            Just("※［＃存在しない外字、第1水準1-2-3］"),
            Just("［＃「漢字」に傍点］"),
        ];
        prop_vec(fragment, 0..10).prop_map(|parts| parts.concat())
    }

    proptest! {
        /// `RegionIndex::structurally_safe` equals the linear `structurally_safe`
        /// at *every* offset of the document.
        #[test]
        fn structurally_safe_indexed_matches_oracle(doc in doc_strategy()) {
            let cached = owned(&doc);
            let index = idx_of(&cached);
            let len = u32::try_from(cached.sanitized.len()).expect("len fits u32");
            for off in 0..=len {
                prop_assert_eq!(
                    index.structurally_safe(&cached.source_nodes, off),
                    structurally_safe(off, &cached.source_nodes, &cached.pairs),
                    "structurally_safe mismatch at {} in {:?}", off, doc,
                );
            }
        }

        /// The outward-scan `minimal_balanced_region` equals the whole-buffer
        /// reference for arbitrary edits.
        #[test]
        fn outward_scan_region_matches_reference(
            doc in doc_strategy(),
            a in 0_usize..256,
            b in 0_usize..256,
        ) {
            let cached = owned(&doc);
            let pieces = pieces_of(&cached);
            let span = cached.sanitized.len() + 1;
            let es = a % span;
            let ee = b % span;
            let edit = Range { start: es, end: ee };
            prop_assert_eq!(
                minimal_balanced_region(
                    &DiagBaseRef::from_cached(&cached, &pieces),
                    edit.clone(),
                ),
                reference_region(&cached, edit.clone()),
                "region mismatch for edit {:?} in {:?}", edit, doc,
            );
        }

        /// The two `O(log n)` diagnostic-straddle probes equal the linear
        /// not-prefix/suffix/inside scan for an arbitrary region.
        #[test]
        fn diag_straddle_probes_match_oracle(
            doc in doc_strategy(),
            a in 0_usize..256,
            b in 0_usize..256,
        ) {
            let cached = owned(&doc);
            let index = idx_of(&cached);
            let span = cached.sanitized.len() + 1;
            let mut rs = u32::try_from(a % span).expect("fits u32");
            let mut re = u32::try_from(b % span).expect("fits u32");
            if rs > re {
                swap(&mut rs, &mut re);
            }
            prop_assert_eq!(
                index.diag_straddles(rs) || index.diag_straddles(re),
                any_diag_straddles(&cached.diagnostics, rs..re),
                "diag straddle mismatch for {}..{} in {:?}", rs, re, doc,
            );
        }

        /// The single-piece `PieceSeq` flag predicates equal the `any()` scans
        /// they replace (the per-live-range `PieceFlags` over a multi-piece
        /// sequence are pinned by `verify_seq_matches` in the splice proptests).
        #[test]
        fn flags_match_any_scans(doc in doc_strategy()) {
            let cached = owned(&doc);
            let seq = pieces_of(&cached);
            prop_assert_eq!(
                seq.has_whole_doc_scoped_diag(),
                cached.diagnostics.iter().any(is_whole_document_scoped),
                "has_whole_doc_scoped_diag mismatch in {:?}", doc,
            );
            prop_assert_eq!(
                seq.has_unbalanced_delimiter(),
                cached.diagnostics.iter().any(|d| matches!(
                    d,
                    Diagnostic::UnclosedBracket { .. } | Diagnostic::UnmatchedClose { .. }
                )),
                "has_unbalanced_delimiter mismatch in {:?}", doc,
            );
            prop_assert_eq!(
                seq.has_coupled_node(),
                cached.source_nodes.iter().any(|sn| node_forbids_region_reuse(sn.node)),
                "has_coupled_node mismatch in {:?}", doc,
            );
        }
    }

    // ---- PieceSeq ≡ contiguous (PR-1 merge gate) ----

    /// Assert two source-node tables agree on the store-free projection the
    /// region-find consumes: byte spans and the `BlockOpen`/`BlockClose`/`Inline`
    /// discriminant (container depth).
    fn assert_nodes_eq(got: &[SourceNodeOwned], want: &[SourceNodeOwned]) {
        assert_eq!(got.len(), want.len(), "source_nodes length");
        for (g, w) in got.iter().zip(want) {
            assert_eq!(g.source_span, w.source_span, "source_span");
            assert_eq!(
                discriminant(&g.node),
                discriminant(&w.node),
                "node discriminant",
            );
        }
    }

    /// Assert two pair tables are byte-identical (kind + open/close spans).
    fn assert_pairs_eq(got: &[PairLink], want: &[PairLink]) {
        assert_eq!(got.len(), want.len(), "pairs length");
        for (g, w) in got.iter().zip(want) {
            assert_eq!(
                (g.kind, g.open, g.close),
                (w.kind, w.open, w.close),
                "pair link",
            );
        }
    }

    /// Diagnostics as a position-sorted multiset of debug strings, mirroring the
    /// corpus differential gate's comparison.
    fn sorted_debug(mut diags: Vec<Diagnostic>) -> Vec<String> {
        diags.sort_by(|a, b| {
            let (sa, sb) = (a.span(), b.span());
            (sa.start, sa.end)
                .cmp(&(sb.start, sb.end))
                .then_with(|| format!("{a:?}").cmp(&format!("{b:?}")))
        });
        diags.iter().map(|d| format!("{d:?}")).collect()
    }

    /// Assert two diagnostic tables are equal as position-sorted multisets.
    fn assert_diags_eq(got: &[Diagnostic], want: &[Diagnostic]) {
        assert_eq!(
            sorted_debug(got.to_vec()),
            sorted_debug(want.to_vec()),
            "diagnostics multiset",
        );
    }

    /// Whether any diagnostic straddles point `b` (`start < b < end`).
    fn straddles_at(diags: &[Diagnostic], b: u32) -> bool {
        diags.iter().any(|d| {
            let s = d.span();
            s.start < b && b < s.end
        })
    }

    /// Pin a `PieceSeq` byte-identical to a full re-parse `full`: flattened
    /// tables (node projection / pairs / diagnostics multiset), the sanitized
    /// total, and every region-find query at every offset against the linear
    /// oracle over the flattened tables.
    fn verify_seq_matches(seq: &PieceSeq, full: &OwnedLexOutput) {
        let san_len = u32::try_from(full.sanitized.len()).expect("len fits u32");
        let (nodes, pairs, diags) = seq.flatten();
        assert_nodes_eq(&nodes, &full.source_nodes);
        assert_pairs_eq(&pairs, &full.pairs);
        assert_diags_eq(&diags, &full.diagnostics);
        assert_eq!(seq.total_san_len(), san_len, "total sanitized length");
        for off in 0..=san_len {
            assert_eq!(
                seq.structurally_safe(off),
                structurally_safe(off, &nodes, &pairs),
                "structurally_safe at {off}",
            );
            assert_eq!(
                seq.diag_straddles(off),
                straddles_at(&full.diagnostics, off),
                "diag_straddles at {off}",
            );
        }
        assert_eq!(
            seq.has_whole_doc_scoped_diag(),
            full.diagnostics.iter().any(is_whole_document_scoped),
            "has_whole_doc_scoped_diag",
        );
        assert_eq!(
            seq.has_unbalanced_delimiter(),
            full.diagnostics.iter().any(|d| matches!(
                d,
                Diagnostic::UnclosedBracket { .. } | Diagnostic::UnmatchedClose { .. }
            )),
            "has_unbalanced_delimiter",
        );
        assert_eq!(
            seq.has_coupled_node(),
            full.source_nodes
                .iter()
                .any(|sn| node_forbids_region_reuse(sn.node)),
            "has_coupled_node",
        );
    }

    /// Plain, non-structural replacements that keep an edit a sanitize fixed
    /// point (so a splice can apply); the empty string exercises deletions.
    const SAFE_REPLACEMENTS: &[&str] = &["", "も", "やや", "ねこ", "あ", "。"];

    /// `text` with bytes `[start, end)` replaced by `repl`.
    fn apply_text_edit(text: &str, start: usize, end: usize, repl: &str) -> String {
        let mut out = String::with_capacity(text.len() + repl.len());
        out.push_str(&text[..start]);
        out.push_str(repl);
        out.push_str(&text[end..]);
        out
    }

    /// The mutable trio a multi-edit run threads through `try_safe_edit`: the
    /// piece sequence under test, its current sanitized text, and the matching
    /// full re-parse used to drive the prologue and pin the result.
    struct EditState {
        seq: PieceSeq,
        current: String,
        cached: OwnedLexOutput,
    }

    /// Try one plain edit. On a prologue-admitted, sanitize-fixed-point edit,
    /// splice the `PieceSeq`, advance the `EditState`, and pin the result against
    /// a full re-parse. Returns whether the edit was applied.
    fn try_safe_edit(state: &mut EditState, start: usize, end: usize, repl: &str) -> bool {
        let new_text = apply_text_edit(&state.current, start, end, repl);
        let full_new = owned(&new_text);
        if full_new.sanitized != new_text {
            return false; // not a sanitize fixed point — skip
        }
        // The prologue reads the *accumulated* sequence under test — faithful to
        // production, where `base.pieces` is the maintained multi-piece sequence,
        // not a freshly-rebuilt single-piece one. `state.cached.sanitized` is the
        // matching full parse's buffer (== `state.current`).
        let base = DiagBaseRef::from_cached(&state.cached, &state.seq);
        let new_str = new_text.as_str();
        let Some(Prologue {
            region,
            relexed,
            d_san,
        }) = splice_prologue(&base, &new_str, start..end)
        else {
            return false;
        };
        let spliced = state.seq.splice(
            region,
            &relexed.source_nodes,
            &relexed.pairs,
            &relexed.diagnostics,
            d_san,
        );
        state.seq = spliced;
        state.current = new_text;
        state.cached = full_new;
        verify_seq_matches(&state.seq, &state.cached);
        true
    }

    proptest! {
        /// `from_contiguous(x).flatten()` reproduces `x`, and every query equals
        /// the linear oracle (the single-piece base case).
        #[test]
        fn pieceseq_from_contiguous_round_trips(doc in doc_strategy()) {
            let cached = owned(&doc);
            let san_len = u32::try_from(cached.sanitized.len()).expect("len fits u32");
            let seq = PieceSeq::from_contiguous(
                &cached.source_nodes,
                &cached.pairs,
                &cached.diagnostics,
                san_len,
            );
            verify_seq_matches(&seq, &cached);
        }

        /// A run of safe edits splices the `PieceSeq` into a multi-piece state
        /// that stays byte-identical to a full re-parse at every step — the
        /// load-bearing `PieceSeq ≡ contiguous` adversarial gate. The base
        /// document carries containers / stray closes; the edits redistribute that
        /// structure across piece boundaries as they cut.
        ///
        /// A run whose edits all decline (no safe interior cut, or a
        /// prologue-rejected edit) only re-verifies the single-piece base — sound
        /// but vacuous. Non-vacuity is therefore *not* asserted here (a 0-admit
        /// run is a legitimate input); it is guaranteed instead by the
        /// deterministic [`pieceseq_two_paragraph_edits_build_multi_piece`] and
        /// [`pieceseq_multi_piece_with_block_container_stays_identical`], which
        /// assert both `try_safe_edit == true` and `pieces.len() > 1`, so a
        /// regression that broke splice admittance fails loudly there.
        #[test]
        fn pieceseq_multi_edit_matches_full_parse(
            doc in doc_strategy(),
            edits in prop_vec(
                (0usize..2000, 0usize..40, 0usize..SAFE_REPLACEMENTS.len()),
                0..7,
            ),
        ) {
            let cached = owned(&doc);
            let san_len = u32::try_from(cached.sanitized.len()).expect("len fits u32");
            let seq = PieceSeq::from_contiguous(
                &cached.source_nodes,
                &cached.pairs,
                &cached.diagnostics,
                san_len,
            );
            let current = cached.sanitized.clone();
            let mut state = EditState { seq, current, cached };
            verify_seq_matches(&state.seq, &state.cached);
            for (sf, lf, ri) in edits {
                let bounds: Vec<usize> = state
                    .current
                    .char_indices()
                    .map(|(i, _)| i)
                    .chain(once(state.current.len()))
                    .collect();
                let n = bounds.len();
                let si = sf % n;
                let ei = (si + lf).min(n - 1);
                let (start, end) = (bounds[si], bounds[ei]);
                let repl = SAFE_REPLACEMENTS[ri % SAFE_REPLACEMENTS.len()];
                try_safe_edit(&mut state, start, end, repl);
            }
        }
    }

    /// A deterministic multi-piece run (proptest may not always reach one):
    /// two interior edits in different paragraphs cut the document into more
    /// than one live piece, and every query still matches a full re-parse.
    #[test]
    fn pieceseq_two_paragraph_edits_build_multi_piece() {
        let cached = owned("あいうえお\n\nかきくけこ\n\nさしすせそ\n\nたちつてと\n");
        let san_len = u32::try_from(cached.sanitized.len()).expect("len fits u32");
        let seq = PieceSeq::from_contiguous(
            &cached.source_nodes,
            &cached.pairs,
            &cached.diagnostics,
            san_len,
        );
        let current = cached.sanitized.clone();
        let mut state = EditState {
            seq,
            current,
            cached,
        };
        verify_seq_matches(&state.seq, &state.cached);

        let at1 = state.current.find("きくけ").expect("second paragraph") + "き".len();
        assert!(
            try_safe_edit(&mut state, at1, at1, "も"),
            "an interior edit in paragraph 2 must splice",
        );
        let at2 = state.current.find("ちつて").expect("fourth paragraph") + "ち".len();
        assert!(
            try_safe_edit(&mut state, at2, at2, "や"),
            "an interior edit in paragraph 4 must splice",
        );

        assert!(
            state.seq.pieces.len() > 1,
            "two interior edits build a multi-piece sequence, got {} piece(s)",
            state.seq.pieces.len(),
        );
    }

    /// A deterministic multi-piece run whose document carries a **balanced block
    /// container** (`［＃ここから２字下げ］…［＃ここで字下げ終わり］`) in its own
    /// blank-line-bounded paragraph. Interior edits in the paragraphs flanking it
    /// cut the document into several live pieces, one of which holds the
    /// container's nonzero interior block depth. `verify_seq_matches` (run inside
    /// `try_safe_edit`) then pins `structurally_safe` at every offset — including
    /// inside the container, where the depth is nonzero so no cut is safe —
    /// byte-identical to a full re-parse. This deterministically exercises the
    /// depth/offset error class a piece boundary could introduce, rather than
    /// leaving the bracketed-multi-piece case to the proptest generator's chance.
    #[test]
    fn pieceseq_multi_piece_with_block_container_stays_identical() {
        let cached = owned(
            "まえがきの段落\n\n\
             ［＃ここから２字下げ］\n本文のなかみ\n［＃ここで字下げ終わり］\n\n\
             あとがきの段落\n\nむすびのことば\n",
        );
        let san_len = u32::try_from(cached.sanitized.len()).expect("len fits u32");
        let seq = PieceSeq::from_contiguous(
            &cached.source_nodes,
            &cached.pairs,
            &cached.diagnostics,
            san_len,
        );
        let current = cached.sanitized.clone();
        let mut state = EditState {
            seq,
            current,
            cached,
        };
        verify_seq_matches(&state.seq, &state.cached);

        // Edit the paragraph before the container ("えがき" is unique to it)...
        let at1 = state.current.find("えがき").expect("first paragraph") + "え".len();
        assert!(
            try_safe_edit(&mut state, at1, at1, "も"),
            "an interior edit before the container must splice",
        );
        // ...and the paragraph after it ("とがき" is unique to あとがき), so the
        // balanced container is isolated in its own interior piece.
        let at2 = state
            .current
            .find("とがき")
            .expect("after-container paragraph")
            + "と".len();
        assert!(
            try_safe_edit(&mut state, at2, at2, "や"),
            "an interior edit after the container must splice",
        );

        assert!(
            state.seq.pieces.len() > 1,
            "edits flanking a block container build a multi-piece sequence, got {} piece(s)",
            state.seq.pieces.len(),
        );
    }

    #[test]
    fn pieceseq_compact_collapses_to_one_piece_query_equivalent() {
        // Build a multi-piece sequence (edits flanking a block container, so a
        // piece carries nonzero interior depth), then `compact` it: the result
        // is a single piece whose every query is byte-identical to the
        // pre-compact sequence and to a full re-parse of the same text.
        let cached = owned(
            "まえがきの段落\n\n\
             ［＃ここから２字下げ］\n本文のなかみ\n［＃ここで字下げ終わり］\n\n\
             あとがきの段落\n\nむすびのことば\n",
        );
        let san_len = u32::try_from(cached.sanitized.len()).expect("len fits u32");
        let seq = PieceSeq::from_contiguous(
            &cached.source_nodes,
            &cached.pairs,
            &cached.diagnostics,
            san_len,
        );
        let current = cached.sanitized.clone();
        let mut state = EditState {
            seq,
            current,
            cached,
        };
        let at1 = state.current.find("えがき").expect("first paragraph") + "え".len();
        assert!(
            try_safe_edit(&mut state, at1, at1, "も"),
            "edit must splice"
        );
        let at2 = state
            .current
            .find("とがき")
            .expect("after-container paragraph")
            + "と".len();
        assert!(
            try_safe_edit(&mut state, at2, at2, "や"),
            "edit must splice"
        );
        assert!(state.seq.pieces.len() > 1, "multi-piece precondition");

        let compacted = state.seq.compact();
        assert_eq!(
            compacted.piece_count(),
            1,
            "compact collapses to a single piece",
        );
        // Query-equivalent to a full re-parse — and hence to the pre-compact
        // sequence, which `try_safe_edit` already pinned against the same parse.
        verify_seq_matches(&compacted, &state.cached);
    }
}
