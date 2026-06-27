//! Owned-AST incremental re-parse engine for #237 — the sole incremental path.
//!
//! The retired Stage-A segment cache first proved — over the reference corpus —
//! *where* a document can be cut into independently-lexable spans. This engine
//! carries that insight onto the owned AST: it caches the owned lex output and,
//! on an edit, re-lexes only the minimal balanced region around the edit before
//! splicing the owned node table.
//!
//! This module hosts the **region finder** ([`minimal_balanced_region`]), the
//! sanitized→normalized offset map ([`norm_offset`]), the **owned-table splice**
//! ([`reparse_incremental_owned`]), and the shared "where is it safe to cut the
//! document" cut helpers. The splice is the production incremental path: it is
//! re-exported from the crate root as the **unstable**
//! [`crate::reparse_incremental_owned`] and consumed by the LSP's debounced
//! diagnostics (#237 Stage B'3). It is internal-unit-tested and proven
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

use aozora_syntax::owned::{ContainerPair, RegistryOwned};

use crate::splice::classify_node_ref;
use crate::{
    CoupledKind, Diagnostic, Document, NodeRefOwned, NormalizedOffset, OwnedLexOutput, PairLink,
    SourceNodeOwned, SpliceSafety,
};

/// The result of a successful owned-table incremental splice.
///
/// Returned by [`crate::reparse_incremental_owned`]: the byte-identical
/// [`OwnedLexOutput`] plus the reuse accounting the LSP surface reports as cache
/// hits/misses.
///
/// `reused_nodes` counts the `cached` source nodes carried into the result
/// unchanged (the prefix before the re-lexed region plus the shifted suffix
/// after it); `relexed_nodes` counts the nodes the region re-lex produced.
/// Together they expose how much of the prior parse the splice salvaged.
#[derive(Debug)]
pub struct OwnedSplice {
    /// The spliced lex output, byte-for-byte equal to a full re-parse of the
    /// edited text on every resolved/rendered surface.
    pub output: OwnedLexOutput,
    /// Number of `cached` source nodes carried into the result unchanged
    /// (prefix `end <= region.start` plus suffix `start >= region.end`).
    pub reused_nodes: u64,
    /// Number of source nodes the isolated region re-lex produced.
    pub relexed_nodes: u64,
}

/// The store-free fields the **diagnostics-only** incremental path
/// ([`crate::reparse_incremental_diagnostics_only`]) and the shared
/// `splice_prologue` read from the cached parse.
///
/// The LSP's per-keystroke hot path keeps only these four lightweight fields of
/// the prior parse (its `DiagBase`) and never materialises the full
/// [`OwnedLexOutput`]. Every field is store-independent: the prologue's
/// region-find (`minimal_balanced_region` / `structurally_safe` / the
/// coupling guard) and the diagnostics splice read only `sanitized`,
/// `source_nodes` (`source_span` + the `NodeRefOwned` discriminant),
/// `pairs`, and `diagnostics` — none of which resolves a `StrId`/`ContentRange`
/// against a [`crate::NodeStore`]. A full [`OwnedLexOutput`] projects into one
/// via [`DiagBaseRef::with_index`] (paired with a [`RegionIndex`] built over the
/// same tables).
#[derive(Debug, Clone, Copy)]
pub struct DiagBaseRef<'a> {
    /// The cached sanitized buffer (a sanitize fixed point) — the coordinate
    /// space every `source_span`/`pairs` offset indexes.
    pub sanitized: &'a str,
    /// Source-keyed node side-table, sorted by `source_span.start`.
    pub source_nodes: &'a [SourceNodeOwned],
    /// Resolved (open, close) delimiter pairs in sanitized coordinates.
    pub pairs: &'a [PairLink],
    /// Non-fatal observations from the cached parse.
    pub diagnostics: &'a [Diagnostic],
    /// Precomputed `O(log n)`/`O(1)` acceleration over the three tables above
    /// (#237 Tier 2). Built once per parse — free during the base copy that
    /// already runs each edit — so the region-find prologue reads this instead
    /// of re-scanning the whole buffer/tables per edit.
    pub index: &'a RegionIndex,
}

impl<'a> DiagBaseRef<'a> {
    /// Project a full [`OwnedLexOutput`] plus a [`RegionIndex`] built over its
    /// `source_nodes`/`pairs`/`diagnostics` into a store-free diagnostics base.
    ///
    /// The caller owns the [`RegionIndex`] (so it can outlive this borrow); the
    /// `O(doc)` owned-path / test / bench sites are already `O(doc)`, so the
    /// extra `O(N)` build is free.
    #[must_use]
    pub fn with_index(cached: &'a OwnedLexOutput, index: &'a RegionIndex) -> Self {
        Self {
            sanitized: &cached.sanitized,
            source_nodes: &cached.source_nodes,
            pairs: &cached.pairs,
            diagnostics: &cached.diagnostics,
            index,
        }
    }
}

/// Precomputed acceleration structure for the per-edit region-find prologue
/// (#237 Tier 2 — engine-internal, but `pub` so the LSP's `DiagBase` can own one
/// and the bench can build one).
///
/// Built in a single `O(N + P + D)` pass (plus small `O(P log P + D log D)`
/// sorts of the few pairs/diagnostics) over the three store-free tables the
/// prologue reads — the same `O(N)` cost the base copy already pays each edit.
/// Replaces three `O(doc)` per-edit scans with `O(log n)`/`O(1)` lookups:
///
/// - the whole-buffer `candidate_boundaries` byte scan plus its `O(#cuts · N)`
///   `structurally_safe` calls become an outward scan with `O(log n)` safety
///   probes (see `RegionIndex::structurally_safe`);
/// - the `norm_offset` drift summation becomes one `O(log n)` lookup
///   (`RegionIndex::norm_offset`);
/// - the whole-document-scoped / unbalanced-delimiter / coupling guards become
///   `O(1)` flag reads, and the diagnostic-straddle scan two `O(log n)` probes
///   (`RegionIndex::diag_straddles`).
///
/// Every recurrence mirrors its linear oracle exactly; the
/// `corpus_incremental_merge` differential gate plus the oracle proptests pin
/// byte-identity.
///
/// `Default` is the empty index (no nodes/pairs/diagnostics); it exists only so
/// containers that hold one can derive `Default`, and is never queried.
#[derive(Debug, Default)]
pub struct RegionIndex {
    /// Clamped LIFO block-container depth **after** each `source_node`
    /// (parallel to `source_nodes`, which is sorted by `source_span.start`).
    depth_prefix: Vec<i32>,
    /// Running max of `source_span.end` over `source_nodes[0..=i]` — the
    /// straddle test for [`RegionIndex::structurally_safe`].
    max_end_prefix: Vec<u32>,
    /// Inclusive prefix sums of each node's normalized drift
    /// `(2·pad + 3 − footprint)`; length `N + 1`, indexed by a
    /// `partition_point(end <= off)` for [`RegionIndex::norm_offset`].
    drift_prefix: Vec<i64>,
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
    /// Any cached diagnostic [`is_whole_document_scoped`] (region-find declines).
    has_whole_doc_scoped_diag: bool,
    /// Any cached [`Diagnostic::UnclosedBracket`]/[`Diagnostic::UnmatchedClose`]
    /// (globally-unbalanced delimiter — the prologue declines).
    has_unbalanced_delimiter: bool,
    /// Any cached node [`node_forbids_region_reuse`] (text-coupled / opaque — the
    /// prologue declines on the base side; the re-lexed side is still scanned).
    has_coupled_node: bool,
}

impl RegionIndex {
    /// Build the index in one pass over the store-free tables. Mirrors the
    /// linear recurrences of `structurally_safe`, `norm_offset`, and the
    /// prologue guards exactly.
    #[must_use]
    pub fn build(
        nodes: &[SourceNodeOwned],
        pairs: &[PairLink],
        diagnostics: &[Diagnostic],
    ) -> Self {
        // Node prefixes (nodes are sorted by `source_span.start`).
        let mut depth_prefix = Vec::with_capacity(nodes.len());
        let mut max_end_prefix = Vec::with_capacity(nodes.len());
        let mut drift_prefix = Vec::with_capacity(nodes.len() + 1);
        drift_prefix.push(0_i64);
        let mut depth: i32 = 0;
        let mut max_end: u32 = 0;
        let mut drift: i64 = 0;
        let mut has_coupled_node = false;
        for sn in nodes {
            match sn.node {
                NodeRefOwned::BlockOpen(_) => depth += 1,
                NodeRefOwned::BlockClose(_) => depth = (depth - 1).max(0),
                _ => {}
            }
            depth_prefix.push(depth);
            max_end = max_end.max(sn.source_span.end);
            max_end_prefix.push(max_end);
            let footprint = i64::from(sn.source_span.end - sn.source_span.start);
            let pad = i64::from(standalone_pad(sn.node));
            drift += 2 * pad + 3 - footprint;
            drift_prefix.push(drift);
            if node_forbids_region_reuse(sn.node) {
                has_coupled_node = true;
            }
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

        let has_whole_doc_scoped_diag = diagnostics.iter().any(is_whole_document_scoped);
        let has_unbalanced_delimiter = diagnostics.iter().any(|d| {
            matches!(
                d,
                Diagnostic::UnclosedBracket { .. } | Diagnostic::UnmatchedClose { .. }
            )
        });

        Self {
            depth_prefix,
            max_end_prefix,
            drift_prefix,
            pair_close_end,
            pair_suffix_min_open,
            diag_start_sorted,
            diag_max_end_prefix,
            has_whole_doc_scoped_diag,
            has_unbalanced_delimiter,
            has_coupled_node,
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

    /// `O(log n)` equivalent of [`norm_offset`]: the normalized offset for the
    /// structurally-safe interstitial boundary `san_off`. `cached` must be the
    /// output the index was built over.
    #[must_use]
    fn norm_offset(&self, cached: &OwnedLexOutput, san_off: u32) -> Option<u32> {
        let san_len = u32::try_from(cached.sanitized.len()).ok()?;
        if san_off > san_len {
            return None;
        }
        let k = cached
            .source_nodes
            .partition_point(|sn| sn.source_span.end <= san_off);
        let norm = shift_u32(san_off, self.drift_prefix[k])?;
        if !cached.normalized.is_char_boundary(norm as usize) {
            return None;
        }
        Some(norm)
    }

    /// Whether any diagnostic straddles boundary `b` (`span.start < b < span.end`)
    /// — one `O(log n)` probe of the start-sorted diagnostics.
    #[must_use]
    fn diag_straddles(&self, b: u32) -> bool {
        let k = self.diag_start_sorted.partition_point(|&s| s < b);
        k > 0 && self.diag_max_end_prefix[k - 1] > b
    }
}

/// The result of a successful **diagnostics-only** incremental splice.
///
/// **UNSTABLE — not subject to semver until v0.5.0** (the #237 incremental API).
///
/// Returned by [`crate::reparse_incremental_diagnostics_only`]: the spliced
/// diagnostics plus the store-free `source_nodes`/`pairs` the *next* edit's
/// region-find consumes, and the reuse accounting the LSP reports as cache
/// hits/misses. No [`OwnedLexOutput`] (no `normalized`, `registry`,
/// `container_pairs`, or [`crate::NodeStore`]) is built — that is the whole
/// point of the hot path.
///
/// `source_nodes` carries the region's nodes with the **re-lexed** sub-output's
/// `NodeRefOwned` handles verbatim (no store graft). This is sound only because
/// the next edit's region-find reads those handles store-independently — see
/// [`crate::reparse_incremental_diagnostics_only`] for the pinned invariant.
#[derive(Debug)]
pub struct DiagSplice {
    /// Spliced diagnostics (prefix unchanged ++ region re-lexed ++ shifted
    /// suffix), byte-identical to a full parse's diagnostics multiset.
    pub diagnostics: Vec<Diagnostic>,
    /// Spliced source-node side-table for the next edit's region-find. The
    /// region's nodes keep the re-lexed sub-output's `NodeRefOwned` handles
    /// verbatim (no store graft); only `source_span` + discriminant are read
    /// downstream.
    pub source_nodes: Vec<SourceNodeOwned>,
    /// Spliced resolved delimiter pairs (store-free offsets) for the next
    /// edit's region-find.
    pub pairs: Vec<PairLink>,
    /// Number of `cached` source nodes carried unchanged (prefix + suffix).
    pub reused_nodes: u64,
    /// Number of source nodes the isolated region re-lex produced.
    pub relexed_nodes: u64,
}

/// The store-independent prologue shared by [`reparse_incremental_owned`] and
/// [`crate::reparse_incremental_diagnostics_only`]: the validated region plus the
/// isolated region re-lex. Produced by [`splice_prologue`] (steps 1–5); both
/// engines build on it so they cannot drift.
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
/// boundary) or a directive opener `［` (could introduce a container or
/// forward reference, both whole-document-scoped concerns).
pub(crate) fn carries_structure(s: &str) -> bool {
    s.bytes().any(|b| b == b'\n' || b == b'\r') || s.contains('［')
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
/// the cached or re-lexed output makes [`reparse_incremental_owned`] fall back
/// to a full parse. The `corpus_incremental_merge` differential gate proves the
/// splice is byte-identical to a full parse over the reference corpus, so a
/// missing class here surfaces there as a divergence.
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
fn is_blank_line_boundary(bytes: &[u8], j: usize) -> bool {
    j >= 1
        && j < bytes.len()
        && bytes[j - 1] == b'\n'
        && (bytes[j] == b'\n'
            || (bytes[j] == b'\r' && j + 1 < bytes.len() && bytes[j + 1] == b'\n'))
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
    let bytes = source.as_bytes();
    (1..bytes.len())
        .filter(|&j| is_blank_line_boundary(bytes, j))
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
pub(crate) fn minimal_balanced_region(
    base: DiagBaseRef<'_>,
    edit: Range<usize>,
) -> Option<Range<u32>> {
    // O(1) flag read (was an O(#diags) scan): any whole-document-scoped
    // diagnostic perturbs beyond the region, so decline.
    if base.index.has_whole_doc_scoped_diag {
        return None;
    }
    let san = base.sanitized;
    let bytes = san.as_bytes();
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
            || (is_blank_line_boundary(bytes, j as usize)
                && base.index.structurally_safe(base.source_nodes, j))
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

/// A node's standalone-block padding (the `\n\n` inserted *before* its sentinel,
/// equal to the `\n\n` inserted *after*): `2` for a standalone block node, `0`
/// for an inline one. The normalizer pads only block-level nodes; an inline
/// region (傍点 / bare-range 太字 / 縦中横 / …) and an inline open/close get no
/// padding. Mirrors the normalize-stage rule that drives the `\n\n` + `<div>`
/// wrapping (see [`crate::RegionFormat::is_inline`] /
/// [`crate::RegionClose::is_inline`]).
fn standalone_pad(node: NodeRefOwned) -> u32 {
    match node {
        NodeRefOwned::BlockLeaf(_) => 2,
        NodeRefOwned::BlockOpen(rf) => u32::from(!rf.is_inline()) * 2,
        NodeRefOwned::BlockClose(rc) => u32::from(!rc.is_inline()) * 2,
        // [`NodeRefOwned::Inline`] gets no padding. The wildcard (mandatory —
        // `NodeRefOwned` is `#[non_exhaustive]`) also defaults any future
        // sentinel kind to no padding, the inline byte-1:1 assumption and the
        // conservative choice for the offset map.
        NodeRefOwned::Inline(_) | _ => 0,
    }
}

/// The normalized-text byte offset corresponding to sanitized-source offset
/// `san_off`. `san_off` must be a structurally-safe interstitial boundary (0,
/// sanitized_len, or a blank-line cut that no node's source_span straddles) —
/// exactly the boundaries [`minimal_balanced_region`] returns. At such a
/// position normalized == sanitized locally (plain text is 1:1); the only
/// divergence is the accumulated PUA sentinels (3 bytes each) plus standalone-
/// block "\n\n" padding (2 bytes lead + 2 trail) inserted before `san_off`.
///
/// The map is registry-free and closed-form. For every node fully before the
/// boundary (`source_span.end <= san_off`), the normalized stream replaced its
/// `footprint = end - start` sanitized bytes with `2·pad + 3` bytes (lead pad +
/// 3-byte sentinel + trail pad), a drift of `Δ = (2·pad + 3) − footprint`.
/// Summing `Δ` over those nodes and adding it to `san_off` lands the normalized
/// cursor, because plain runs between nodes are byte-identical. A node whose
/// `source_span.start == san_off` has `end > san_off`, so it sits *after* the
/// boundary and is correctly excluded.
///
/// Returns `None` if `san_off` exceeds the sanitized length, if the arithmetic
/// overflows, or (defensive tripwire) if the computed offset is not a char
/// boundary of `cached.normalized` — the boundary-never-in-padding proof means
/// this never fires for a valid interstitial boundary, but it converts any
/// surprise into a clean fallback rather than a bad splice.
///
/// Linear oracle retained for the property test that pins
/// [`RegionIndex::norm_offset`] byte-identical to it at every interstitial
/// boundary; the production owned splice uses the `O(log n)` indexed form.
/// (`dead_code` allowed outside `cfg(test)` because only the proptests call it.)
#[cfg_attr(
    not(test),
    allow(
        dead_code,
        reason = "linear oracle for the RegionIndex::norm_offset proptest"
    )
)]
pub(crate) fn norm_offset(cached: &OwnedLexOutput, san_off: u32) -> Option<u32> {
    let san_len = u32::try_from(cached.sanitized.len()).ok()?;
    if san_off > san_len {
        return None;
    }

    // Nodes fully before the boundary, in source order (`source_nodes` is sorted
    // by `source_span.start`, and an interstitial boundary is never straddled,
    // so `end <= san_off` partitions cleanly).
    let k = cached
        .source_nodes
        .partition_point(|sn| sn.source_span.end <= san_off);

    let mut drift: i64 = 0;
    for sn in &cached.source_nodes[..k] {
        let footprint = i64::from(sn.source_span.end - sn.source_span.start);
        let pad = i64::from(standalone_pad(sn.node));
        drift += 2 * pad + 3 - footprint;
    }

    let norm = shift_u32(san_off, drift)?;
    // Defensive tripwire: a valid interstitial boundary never lands inside a
    // sentinel or padding run, so the result is always a char boundary; if it
    // somehow is not, decline rather than splice at a bad offset.
    if !cached.normalized.is_char_boundary(norm as usize) {
        return None;
    }
    Some(norm)
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
///
/// Single-sources the classification through [`classify_node_ref`] (the #202
/// splice authority) so the two engines cannot drift.
fn node_forbids_region_reuse(node: NodeRefOwned) -> bool {
    matches!(
        classify_node_ref(node).1,
        SpliceSafety::Coupled(
            CoupledKind::ForwardReference | CoupledKind::HeadingHint | CoupledKind::MarginNote
        ) | SpliceSafety::Opaque
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

/// The store-independent prologue shared by both incremental engines (steps
/// 1–5 of the owned splice): find the minimal balanced region around the edit,
/// validate the edit, re-lex the region in isolation, and run the
/// self-containment + cross-region text-coupling guards. Returns the validated
/// [`Prologue`] (region, the region re-lex, and the sanitized suffix shift), or
/// `None` for any edit that cannot be proven local.
///
/// Reads only the store-free [`DiagBaseRef`] of the cached parse, so the
/// diagnostics-only hot path can run it without ever materialising the full
/// [`OwnedLexOutput`]. [`reparse_incremental_owned`] and
/// [`crate::reparse_incremental_diagnostics_only`] both build on this single helper, so
/// their region/relex/decline decisions cannot drift.
///
/// The decline conditions are exactly those documented on
/// [`reparse_incremental_owned`]'s region/validation/self-containment bullets.
fn splice_prologue(
    base: DiagBaseRef<'_>,
    new_sanitized: &str,
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
    if base.index.has_unbalanced_delimiter {
        return None;
    }

    // 2. Edit validation — the edit must actually transform `cached.sanitized`
    //    into `new_sanitized`, and touch no document structure. An incorrectly
    //    specified edit (bytes outside `edit_old` changed) falls back to a full
    //    parse.
    let old_source = base.sanitized;
    if edit_old.start > edit_old.end || edit_old.end > old_source.len() {
        return None;
    }
    let edit_delta =
        i64::try_from(new_sanitized.len()).ok()? - i64::try_from(old_source.len()).ok()?;
    let new_edit_end = usize::try_from(i64::try_from(edit_old.end).ok()? + edit_delta).ok()?;
    if new_edit_end > new_sanitized.len() {
        return None;
    }
    // Bytes outside `edit_old` are byte-identical (the suffix after applying the
    // edit delta). `.get(..)` keeps an out-of-range index from panicking.
    if old_source.as_bytes().get(..edit_old.start) != new_sanitized.as_bytes().get(..edit_old.start)
        || old_source.as_bytes().get(edit_old.end..) != new_sanitized.as_bytes().get(new_edit_end..)
    {
        return None;
    }
    let old_slice = old_source.get(edit_old.clone())?;
    let new_slice = new_sanitized.get(edit_old.start..new_edit_end)?;
    if carries_structure(old_slice) || carries_structure(new_slice) {
        return None;
    }

    // Decline if any cached diagnostic straddles a region boundary: it is
    // neither reproduced by the isolated region re-lex nor safely shiftable. A
    // span is bad iff it straddles `region.start` or `region.end` (proven:
    // `!(prefix || suffix || inside)` ⇔ `start < rs < end` ∨ `start < re < end`),
    // so two O(log n) probes replace the O(#diags) scan.
    if base.index.diag_straddles(region.start) || base.index.diag_straddles(region.end) {
        return None;
    }

    // 3. Re-lex the region in isolation. The region slice is already sanitized
    //    (a slice of the sanitized fixed point `new_sanitized`); if re-sanitize
    //    changes it, bail conservatively.
    let new_r_end = usize::try_from(i64::from(region.end) + edit_delta).ok()?;
    let new_region_src = new_sanitized.get(r_start..new_r_end)?;
    let relexed = Document::new(new_region_src).parse_owned();
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
    if base.index.has_coupled_node
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

/// Build the [`OwnedSplice`] (the spliced [`OwnedLexOutput`] plus reuse counts)
/// for the edited text `new_sanitized` (a sanitized fixed point) from `cached`
/// (the owned lex output of the pre-edit sanitized text) and the single
/// sanitized-coordinate edit `edit_old`, **without a full re-parse** — by
/// re-lexing only the minimal balanced region around the edit and splicing the
/// owned tables.
///
/// The result is byte-for-byte equal, on every resolved/rendered surface, to a
/// full re-parse of `new_sanitized`; the `corpus_incremental_merge` differential
/// gate enforces this over the reference corpus. Any edit that cannot be proven
/// local returns `None`, and the caller falls back to a full parse (trivially
/// correct). The fallbacks are:
///
/// - [`minimal_balanced_region`] declines (whole-document-scoped diagnostic in
///   `cached`, out-of-bounds edit, or no interior safe cut);
/// - `cached` carries a globally-unbalanced delimiter
///   ([`Diagnostic::UnclosedBracket`] / [`Diagnostic::UnmatchedClose`]) whose
///   open/close half swallows or strays across region boundaries;
/// - the edit does not transform `cached.sanitized` into `new_sanitized`
///   (bytes outside `edit_old` differ), or the edited bytes carry document
///   structure ([`carries_structure`]);
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
#[allow(
    clippy::too_many_lines,
    reason = "a single linear table-by-table splice; splitting the prefix/region/suffix walk per stream would scatter the one invariant (each table partitions at the same region boundary) across helpers and obscure it"
)]
pub(crate) fn reparse_incremental_owned(
    cached: &OwnedLexOutput,
    new_sanitized: &str,
    edit_old: Range<usize>,
) -> Option<OwnedSplice> {
    // Build the region-find acceleration index over the cached tables (this path
    // is already O(doc), so the O(N) build is free) and run steps 1–5
    // (store-independent): region find, edit validation, region re-lex,
    // self-containment, cross-region coupling guard — shared verbatim with the
    // diagnostics-only engine so the two cannot drift.
    let index = RegionIndex::build(&cached.source_nodes, &cached.pairs, &cached.diagnostics);
    let Prologue {
        region,
        relexed,
        d_san,
    } = splice_prologue(
        DiagBaseRef::with_index(cached, &index),
        new_sanitized,
        edit_old,
    )?;
    let r_start = region.start as usize;
    let r_end = region.end as usize;
    let old_source = cached.sanitized.as_str();

    // 6. Normalized boundaries & per-stream deltas (O(log n) indexed offset map).
    let norm_start = index.norm_offset(cached, region.start)?;
    let norm_end = index.norm_offset(cached, region.end)?;
    if norm_start > norm_end {
        return None;
    }
    let region_norm_len = i64::from(norm_end) - i64::from(norm_start);
    let d_norm = i64::try_from(relexed.normalized.len()).ok()? - region_norm_len;

    // 7. Graft the re-lexed nodes' handles into a clone of the cached store.
    //    The clone copies the whole pool, so every reused suffix node handle
    //    stays valid; graft only appends. `grafted[i]` is shared by both the
    //    source-node table and the registry (parallel arrays).
    let mut store = cached.store.clone();
    let grafted: Vec<NodeRefOwned> = relexed
        .source_nodes
        .iter()
        .map(|sn| store.graft_node_ref(&relexed.store, sn.node))
        .collect();
    // The registry is parallel to `source_nodes` (i-th entry ↔ i-th node); if a
    // build ever breaks that invariant the splice cannot pair them, so decline.
    if relexed.registry.len() != grafted.len() {
        return None;
    }
    // The differential gate only covers the inputs it samples; this `debug_assert`
    // pins the load-bearing parallel-array invariant for every build (incl. the
    // production LSP path, which has no gate), catching a registry refactor that
    // diverges `iter_sorted()` order from `source_nodes` order at equal length.
    debug_assert!(
        relexed
            .registry
            .iter_sorted()
            .map(|(_, node)| node)
            .eq(relexed.source_nodes.iter().map(|sn| sn.node)),
        "relexed registry must be parallel to source_nodes (same node sequence)",
    );

    // 8. Strings.
    let mut new_sanitized_out =
        String::with_capacity(old_source.len().saturating_add(relexed.sanitized.len()));
    new_sanitized_out.push_str(old_source.get(..r_start)?);
    new_sanitized_out.push_str(&relexed.sanitized);
    new_sanitized_out.push_str(old_source.get(r_end..)?);
    debug_assert_eq!(
        new_sanitized_out, new_sanitized,
        "reassembled sanitized buffer must equal the edited input",
    );

    let mut new_normalized = String::with_capacity(
        cached
            .normalized
            .len()
            .saturating_add(relexed.normalized.len()),
    );
    new_normalized.push_str(cached.normalized.get(..norm_start as usize)?);
    new_normalized.push_str(&relexed.normalized);
    new_normalized.push_str(cached.normalized.get(norm_end as usize..)?);

    // 9. source_nodes (sanitized, sorted by source_span.start): prefix
    //    (unchanged) ++ region (relexed, shifted by r_start) ++ suffix (shifted
    //    by d_san).
    let mut source_nodes: Vec<SourceNodeOwned> =
        Vec::with_capacity(cached.source_nodes.len() + relexed.source_nodes.len());
    for sn in &cached.source_nodes {
        if sn.source_span.end <= region.start {
            source_nodes.push(*sn);
        }
    }
    for (i, sn) in relexed.source_nodes.iter().enumerate() {
        source_nodes.push(SourceNodeOwned {
            source_span: sn.source_span.shifted(i64::from(region.start)),
            node: grafted[i],
        });
    }
    for sn in &cached.source_nodes {
        if sn.source_span.start >= region.end {
            source_nodes.push(SourceNodeOwned {
                source_span: sn.source_span.shifted(d_san),
                node: sn.node,
            });
        }
    }

    // 10. pairs (PairLink, sanitized, close order).
    let mut pairs: Vec<PairLink> = Vec::with_capacity(cached.pairs.len() + relexed.pairs.len());
    for p in &cached.pairs {
        if p.close.end <= region.start {
            pairs.push(*p);
        }
    }
    for p in &relexed.pairs {
        pairs.push(PairLink {
            kind: p.kind,
            open: p.open.shifted(i64::from(region.start)),
            close: p.close.shifted(i64::from(region.start)),
        });
    }
    for p in &cached.pairs {
        if p.open.start >= region.end {
            pairs.push(PairLink {
                kind: p.kind,
                open: p.open.shifted(d_san),
                close: p.close.shifted(d_san),
            });
        }
    }

    // 11. registry (normalized): prefix (pos < norm_start, unchanged) ++ region
    //     (relexed pos + norm_start, grafted[i]) ++ suffix (pos >= norm_end,
    //     shifted by d_norm). Ascending order is preserved by construction.
    let mut reg_entries: Vec<(u32, NodeRefOwned)> =
        Vec::with_capacity(cached.registry.len() + grafted.len());
    for (pos, node) in cached.registry.iter_sorted() {
        if pos < norm_start {
            reg_entries.push((pos, node));
        }
    }
    for (i, (pos, _node)) in relexed.registry.iter_sorted().enumerate() {
        reg_entries.push((shift_u32(pos, i64::from(norm_start))?, grafted[i]));
    }
    for (pos, node) in cached.registry.iter_sorted() {
        if pos >= norm_end {
            reg_entries.push((shift_u32(pos, d_norm)?, node));
        }
    }
    let registry = RegistryOwned::from_sorted_slice(&reg_entries);

    // 12. container_pairs (normalized, close order).
    let mut container_pairs: Vec<ContainerPair> =
        Vec::with_capacity(cached.container_pairs.len() + relexed.container_pairs.len());
    for cp in &cached.container_pairs {
        if cp.close.get() < norm_start {
            container_pairs.push(*cp);
        }
    }
    for cp in &relexed.container_pairs {
        container_pairs.push(ContainerPair {
            kind: cp.kind,
            open: NormalizedOffset::new(shift_u32(cp.open.get(), i64::from(norm_start))?),
            close: NormalizedOffset::new(shift_u32(cp.close.get(), i64::from(norm_start))?),
        });
    }
    for cp in &cached.container_pairs {
        if cp.open.get() >= norm_end {
            container_pairs.push(ContainerPair {
                kind: cp.kind,
                open: NormalizedOffset::new(shift_u32(cp.open.get(), d_norm)?),
                close: NormalizedOffset::new(shift_u32(cp.close.get(), d_norm)?),
            });
        }
    }

    // 13. diagnostics (sanitized): prefix (unchanged) ++ region (relexed,
    //     shifted by r_start) ++ suffix (shifted by d_san). Inside-region cached
    //     diagnostics are superseded by the relexed ones; straddlers were
    //     already declined above.
    let mut diagnostics: Vec<Diagnostic> =
        Vec::with_capacity(cached.diagnostics.len() + relexed.diagnostics.len());
    for d in &cached.diagnostics {
        if d.span().end <= region.start {
            diagnostics.push(d.clone());
        }
    }
    for d in &relexed.diagnostics {
        diagnostics.push(d.clone().shifted(i64::from(region.start)));
    }
    for d in &cached.diagnostics {
        if d.span().start >= region.end {
            diagnostics.push(d.clone().shifted(d_san));
        }
    }

    // 14. Reuse accounting (the LSP surface reports these as cache hits/misses):
    //     cached prefix nodes (end <= region.start) plus cached suffix nodes
    //     (start >= region.end) are carried unchanged; the region's nodes are
    //     the re-lexed ones.
    let reused_nodes = u64::try_from(
        cached
            .source_nodes
            .iter()
            .filter(|sn| sn.source_span.end <= region.start || sn.source_span.start >= region.end)
            .count(),
    )
    .ok()?;
    let relexed_nodes = u64::try_from(relexed.source_nodes.len()).ok()?;

    // 15. Assemble. `intern_stats` is carried verbatim (the gate ignores it).
    let sanitized_len = u32::try_from(new_sanitized_out.len()).ok()?;
    let output = OwnedLexOutput::new(
        new_normalized,
        new_sanitized_out,
        registry,
        diagnostics,
        sanitized_len,
        pairs,
        source_nodes,
        container_pairs,
        cached.intern_stats,
        store,
    );
    Some(OwnedSplice {
        output,
        reused_nodes,
        relexed_nodes,
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
/// region-find consumes — skipping the `O(doc)` normalized/sanitized string
/// rebuild, the store clone + graft, the registry Eytzinger rebuild, and the
/// container-pairs splice. The result is `O(region + #diagnostics)`.
///
/// Its `diagnostics` are byte-identical to [`reparse_incremental_owned`]'s (the
/// same step-13 concatenation over the same prologue), which the
/// `corpus_incremental_merge` differential gate pins to a full parse over the
/// reference corpus.
///
/// # Store-free-base soundness
///
/// The region's nodes carry the **re-lexed** sub-output's [`NodeRefOwned`]
/// handles verbatim — **no store graft, no store clone**. A `NodeRefOwned` that
/// embeds a `StrId`/`ContentRange` resolves only against the store it was built
/// in (here, the re-lex's transient store, which is dropped). This is sound
/// **only** because the resulting [`DiagSplice::source_nodes`]/`pairs` are
/// consumed exclusively by the next edit's region-find
/// ([`minimal_balanced_region`] → [`structurally_safe`], [`relexed_is_balanced`]
/// via the prologue, and the [`node_forbids_region_reuse`] coupling guard),
/// every one of which reads only a node's `source_span` (`Copy`,
/// store-independent) and the `NodeRefOwned` **discriminant** (`BlockOpen` /
/// `BlockClose` / `Inline` / …) — and never resolves a `StrId`/`ContentRange`
/// against a store. The `debug_assert` below pins this invariant: a future
/// change that resolves a region node's payload here must instead graft it into
/// a real store (i.e. take the [`reparse_incremental_owned`] path).
#[must_use]
#[allow(
    clippy::too_many_lines,
    reason = "a single linear table-by-table splice (source_nodes/pairs/diagnostics), each partitioning at the same region boundary; splitting per stream would scatter that one invariant across helpers and obscure it"
)]
pub(crate) fn reparse_incremental_diagnostics_only(
    base: DiagBaseRef<'_>,
    new_sanitized: &str,
    edit_old: Range<usize>,
) -> Option<DiagSplice> {
    // Steps 1–5 (store-independent), shared with the owned splice.
    let Prologue {
        region,
        relexed,
        d_san,
    } = splice_prologue(base, new_sanitized, edit_old)?;

    // Invariant pin (store-free base, see the doc above): the region nodes are
    // carried with the re-lex's own `NodeRefOwned` handles and never resolved
    // against a store on this path. If a future change needs a region node's
    // resolved payload here, it must graft into a real store (the owned path),
    // not read it off the transient re-lex store this `relexed` is about to
    // drop. This assert trips loudly if such a resolution is wired in by
    // accident: the region nodes must stay parallel to the re-lex registry.
    debug_assert_eq!(
        relexed.registry.len(),
        relexed.source_nodes.len(),
        "diagnostics-only path carries re-lex node handles verbatim (no store graft); \
         resolving a region node's payload here is unsound — graft into a store instead",
    );

    // 9. source_nodes (sanitized, sorted by source_span.start): prefix
    //    (unchanged) ++ region (relexed, shifted by region.start, handles AS-IS
    //    — NO graft) ++ suffix (shifted by d_san).
    let mut source_nodes: Vec<SourceNodeOwned> =
        Vec::with_capacity(base.source_nodes.len() + relexed.source_nodes.len());
    for sn in base.source_nodes {
        if sn.source_span.end <= region.start {
            source_nodes.push(*sn);
        }
    }
    for sn in &relexed.source_nodes {
        source_nodes.push(SourceNodeOwned {
            source_span: sn.source_span.shifted(i64::from(region.start)),
            node: sn.node,
        });
    }
    for sn in base.source_nodes {
        if sn.source_span.start >= region.end {
            source_nodes.push(SourceNodeOwned {
                source_span: sn.source_span.shifted(d_san),
                node: sn.node,
            });
        }
    }

    // 10. pairs (PairLink, sanitized, close order) — store-free offsets.
    let mut pairs: Vec<PairLink> = Vec::with_capacity(base.pairs.len() + relexed.pairs.len());
    for p in base.pairs {
        if p.close.end <= region.start {
            pairs.push(*p);
        }
    }
    for p in &relexed.pairs {
        pairs.push(PairLink {
            kind: p.kind,
            open: p.open.shifted(i64::from(region.start)),
            close: p.close.shifted(i64::from(region.start)),
        });
    }
    for p in base.pairs {
        if p.open.start >= region.end {
            pairs.push(PairLink {
                kind: p.kind,
                open: p.open.shifted(d_san),
                close: p.close.shifted(d_san),
            });
        }
    }

    // 13. diagnostics (sanitized): prefix (unchanged) ++ region (relexed,
    //     shifted by region.start) ++ suffix (shifted by d_san). Identical
    //     concatenation to the owned splice's step 13, over the same prologue,
    //     so the two engines' diagnostics are byte-identical.
    let mut diagnostics: Vec<Diagnostic> =
        Vec::with_capacity(base.diagnostics.len() + relexed.diagnostics.len());
    for d in base.diagnostics {
        if d.span().end <= region.start {
            diagnostics.push(d.clone());
        }
    }
    for d in &relexed.diagnostics {
        diagnostics.push(d.clone().shifted(i64::from(region.start)));
    }
    for d in base.diagnostics {
        if d.span().start >= region.end {
            diagnostics.push(d.clone().shifted(d_san));
        }
    }

    // 14. Reuse accounting (the LSP surface reports these as cache hits/misses).
    let reused_nodes = u64::try_from(
        base.source_nodes
            .iter()
            .filter(|sn| sn.source_span.end <= region.start || sn.source_span.start >= region.end)
            .count(),
    )
    .ok()?;
    let relexed_nodes = u64::try_from(relexed.source_nodes.len()).ok()?;

    Some(DiagSplice {
        diagnostics,
        source_nodes,
        pairs,
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

    /// A [`RegionIndex`] over a cached output's store-free tables.
    fn idx_of(cached: &OwnedLexOutput) -> RegionIndex {
        RegionIndex::build(&cached.source_nodes, &cached.pairs, &cached.diagnostics)
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
        let idx = idx_of(&cached);
        let len = u32::try_from(cached.sanitized.len()).unwrap();
        // Edit inside the middle paragraph "かきくけこ".
        let mid = src.find("かきくけこ").unwrap();
        let edit = mid..mid + "かき".len();
        let region = minimal_balanced_region(DiagBaseRef::with_index(&cached, &idx), edit.clone())
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
        let idx = idx_of(&cached);
        let edit = 3..6;
        assert_eq!(
            minimal_balanced_region(DiagBaseRef::with_index(&cached, &idx), edit),
            None
        );
    }

    #[test]
    fn whole_document_scoped_diagnostic_yields_none() {
        // An unresolved standalone gaiji reference is whole-document-scoped.
        let src = "前の段落\n\n※［＃存在しない外字、第1水準1-2-3］\n\n後の段落\n";
        let cached = owned(src);
        let idx = idx_of(&cached);
        assert!(
            cached.diagnostics.iter().any(is_whole_document_scoped),
            "fixture must carry a whole-document-scoped diagnostic, got {:?}",
            cached.diagnostics
        );
        // Region declines regardless of where the edit sits.
        assert_eq!(
            minimal_balanced_region(DiagBaseRef::with_index(&cached, &idx), 0..1),
            None
        );
        let mid = src.find("後の段落").unwrap();
        assert_eq!(
            minimal_balanced_region(DiagBaseRef::with_index(&cached, &idx), mid..mid + 3),
            None
        );
    }

    #[test]
    fn out_of_bounds_edit_yields_none() {
        let src = "あいうえお\n\nかきくけこ\n";
        let cached = owned(src);
        let idx = idx_of(&cached);
        let len = cached.sanitized.len();
        // end past sanitized length.
        assert_eq!(
            minimal_balanced_region(DiagBaseRef::with_index(&cached, &idx), 0..len + 10),
            None
        );
        // start > end (built without a literal reversed range to satisfy the
        // reversed_empty_ranges lint).
        let reversed = Range {
            start: 5usize,
            end: 2,
        };
        assert_eq!(
            minimal_balanced_region(DiagBaseRef::with_index(&cached, &idx), reversed),
            None
        );
    }

    #[test]
    fn edit_spanning_blank_line_widens_to_both_paragraphs() {
        let src = "あいうえお\n\nかきくけこ\n\nさしすせそ\n";
        let cached = owned(src);
        let idx = idx_of(&cached);
        // Edit straddles the blank line between paragraph 1 and 2.
        let p1 = src.find("うえお").unwrap();
        let p2_end = src.find("かきく").unwrap() + "かきく".len();
        let edit = p1..p2_end;
        let region = minimal_balanced_region(DiagBaseRef::with_index(&cached, &idx), edit.clone())
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
        let idx = idx_of(&cached);
        let san = &cached.sanitized;
        assert!(!san.contains('\r'), "sanitized buffer drops CR");
        let len = u32::try_from(san.len()).unwrap();
        // Edit the middle paragraph in SANITIZED coordinates.
        let mid = san.find("かきくけこ").unwrap();
        let edit = mid..mid + "かき".len();
        let region = minimal_balanced_region(DiagBaseRef::with_index(&cached, &idx), edit.clone())
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
        let idx = idx_of(&cached);
        assert_eq!(
            minimal_balanced_region(DiagBaseRef::with_index(&cached, &idx), 0..3),
            None
        );
    }

    #[test]
    fn empty_document_yields_none() {
        let cached = owned("");
        let idx = idx_of(&cached);
        assert_eq!(cached.sanitized.len(), 0, "empty source sanitizes to empty");
        // Zero-width edit on the empty document: the region is 0..0 == 0..len,
        // i.e. the whole (empty) document, so there is no sub-document benefit.
        let zero = 0usize;
        assert_eq!(
            minimal_balanced_region(DiagBaseRef::with_index(&cached, &idx), zero..zero),
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
        let idx = idx_of(&cached);
        let len = u32::try_from(cached.sanitized.len()).unwrap();
        let at_offset =
            |at: usize| minimal_balanced_region(DiagBaseRef::with_index(&cached, &idx), at..at);

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

    // ---- norm_offset (sanitized → normalized offset map) ----

    use crate::{BoutenKind, BoutenPosition, RegionClose, RegionFormat};

    /// The registry as a `(position, NodeRefOwned)` vec, parallel to
    /// `source_nodes` (same source order — the established invariant).
    fn reg_entries(cached: &OwnedLexOutput) -> Vec<(u32, NodeRefOwned)> {
        cached.registry.iter_sorted().collect()
    }

    /// `norm_offset(0) == 0` and `norm_offset(sanitized_len) == normalized_len`
    /// — the whole accumulated drift lands the document end exactly.
    fn assert_endpoints(src: &str) {
        let cached = owned(src);
        let san_len = u32::try_from(cached.sanitized.len()).unwrap();
        let norm_len = u32::try_from(cached.normalized.len()).unwrap();
        assert_eq!(norm_offset(&cached, 0), Some(0), "start of {src:?}");
        assert_eq!(
            norm_offset(&cached, san_len),
            Some(norm_len),
            "end of {src:?}",
        );
    }

    /// THE key cross-check: for every node, the sanitized boundary just after
    /// its span maps to the normalized cursor just after its sentinel (`+3`)
    /// plus trailing standalone padding — derived straight from the registry
    /// positions the pipeline actually produced. Skips any boundary another
    /// node straddles (it would not be a clean interstitial point).
    fn assert_registry_ground_truth(src: &str) {
        let cached = owned(src);
        let reg = reg_entries(&cached);
        let nodes = &cached.source_nodes;
        assert_eq!(
            reg.len(),
            nodes.len(),
            "registry parallel to source_nodes for {src:?}",
        );
        let mut checked = 0usize;
        for k in 1..=nodes.len() {
            let b = nodes[k - 1].source_span.end;
            // A clean interstitial boundary is straddled by no node.
            if nodes
                .iter()
                .any(|sn| sn.source_span.start < b && sn.source_span.end > b)
            {
                continue;
            }
            let expected = reg[k - 1].0 + 3 + standalone_pad(reg[k - 1].1);
            assert_eq!(
                norm_offset(&cached, b),
                Some(expected),
                "node {} boundary {b} in {src:?}",
                k - 1,
            );
            checked += 1;
        }
        assert!(
            nodes.is_empty() || checked > 0,
            "no clean boundary checked for {src:?}",
        );
    }

    /// Structurally diverse documents that exercise every padding case: plain
    /// (no node), inline ruby (pad 0, long base collapses), inline forward
    /// format, a standalone block leaf (改ページ, pad 2), an inline gaiji
    /// reference, and a block container open/close (字下げ, pad 2 each).
    const GROUND_TRUTH_DOCS: &[&str] = &[
        "あいうえお\n\nかきくけこ\n",
        "前｜漢字《かんじ》後\n",
        "前\n\n｜山《やま》\n\n後\n",
        "あ［＃「あ」は太字］い\n",
        "前\n\n［＃改ページ］\n\n後\n",
        "海※［＃感嘆符二つ、1-8-75］辺\n",
        "前\n\n［＃ここから２字下げ］\n本文\n［＃ここで字下げ終わり］\n\n後\n",
    ];

    #[test]
    fn norm_offset_endpoints_account_for_all_drift() {
        for src in GROUND_TRUTH_DOCS {
            assert_endpoints(src);
        }
        // Empty document: 0 maps to 0, both lengths zero.
        let empty = owned("");
        assert_eq!(norm_offset(&empty, 0), Some(0));
    }

    #[test]
    fn norm_offset_matches_registry_ground_truth() {
        for src in GROUND_TRUTH_DOCS {
            assert_registry_ground_truth(src);
        }
    }

    #[test]
    fn norm_offset_standalone_block_includes_lead_and_trail_padding() {
        // 改ページ is a standalone block leaf: pad 2 (lead) + 3 sentinel + 2
        // (trail). The boundary after it must skip the trailing pad too.
        let src = "前\n\n［＃改ページ］\n\n後\n";
        let cached = owned(src);
        let idx = cached
            .source_nodes
            .iter()
            .position(|sn| matches!(sn.node, NodeRefOwned::BlockLeaf(_)))
            .expect("改ページ is a block leaf");
        assert_eq!(
            standalone_pad(cached.source_nodes[idx].node),
            2,
            "standalone block leaf pads 2",
        );
        let reg = reg_entries(&cached);
        let b = cached.source_nodes[idx].source_span.end;
        assert_eq!(
            norm_offset(&cached, b),
            Some(reg[idx].0 + 3 + 2),
            "cursor sits after sentinel + trailing pad",
        );
    }

    #[test]
    fn norm_offset_block_container_pads_open_and_close() {
        // 字下げ container: BlockOpen + BlockClose, each block (pad 2).
        let src = "前\n\n［＃ここから２字下げ］\n本文\n［＃ここで字下げ終わり］\n\n後\n";
        let cached = owned(src);
        let opens: Vec<_> = cached
            .source_nodes
            .iter()
            .filter(|sn| matches!(sn.node, NodeRefOwned::BlockOpen(_)))
            .collect();
        let closes: Vec<_> = cached
            .source_nodes
            .iter()
            .filter(|sn| matches!(sn.node, NodeRefOwned::BlockClose(_)))
            .collect();
        assert_eq!(opens.len(), 1, "one container open");
        assert_eq!(closes.len(), 1, "one container close");
        assert_eq!(standalone_pad(opens[0].node), 2, "block open pads 2");
        assert_eq!(standalone_pad(closes[0].node), 2, "block close pads 2");
        assert_registry_ground_truth(src);
    }

    #[test]
    fn standalone_pad_table_inline_vs_block() {
        // Inline open/close (傍点 range, is_inline) get no padding; block
        // open/close (罫囲み) pad 2. Constructed directly to pin the table
        // independent of which directives the parser happens to emit inline.
        let inline_open = NodeRefOwned::BlockOpen(RegionFormat::Bouten {
            kind: BoutenKind::Goma,
            position: BoutenPosition::Right,
        });
        let inline_close = NodeRefOwned::BlockClose(RegionClose::Bouten {
            kind: BoutenKind::Goma,
            position: BoutenPosition::Right,
        });
        let block_open = NodeRefOwned::BlockOpen(RegionFormat::Framed);
        let block_close = NodeRefOwned::BlockClose(RegionClose::Framed);
        assert_eq!(standalone_pad(inline_open), 0, "inline open: no pad");
        assert_eq!(standalone_pad(inline_close), 0, "inline close: no pad");
        assert_eq!(standalone_pad(block_open), 2, "block open: pad 2");
        assert_eq!(standalone_pad(block_close), 2, "block close: pad 2");
    }

    #[test]
    fn norm_offset_crlf_source_is_in_sanitized_coordinates() {
        // Sanitize strips \r first, so source_nodes / normalized already live
        // in sanitized space; norm_offset operates entirely there.
        let src = "前\r\n\r\n［＃改ページ］\r\n\r\n後\r\n";
        let cached = owned(src);
        assert!(!cached.sanitized.contains('\r'), "sanitized drops CR");
        assert_endpoints(src);
        assert_registry_ground_truth(src);
    }

    #[test]
    fn norm_offset_interior_gap_matches_bracketing_form() {
        // An interior boundary in the MIDDLE of a plain gap (not exactly at a
        // node end): the cumulative form must equal the bracketing form
        // `reg[k-1].0 + 3 + pad + (b - source_span.end)` — plain text is 1:1.
        let src = "前\n\n［＃改ページ］\n\n後の段落です\n";
        let cached = owned(src);
        let reg = reg_entries(&cached);
        // The single node is the page break; pick a boundary a few bytes into
        // the trailing "後の段落です" plain run, well past its span end.
        let node_end = cached.source_nodes[0].source_span.end;
        let after = cached.sanitized.find("後の段落").unwrap();
        let b = u32::try_from(after + "後".len()).unwrap();
        assert!(b > node_end, "boundary sits in the gap after the node");
        let pad = standalone_pad(cached.source_nodes[0].node);
        let bracketing = reg[0].0 + 3 + pad + (b - node_end);
        assert_eq!(
            norm_offset(&cached, b),
            Some(bracketing),
            "cumulative form equals bracketing form in a plain gap",
        );
    }

    #[test]
    fn norm_offset_out_of_bounds_yields_none() {
        let cached = owned("あいうえお\n");
        let san_len = u32::try_from(cached.sanitized.len()).unwrap();
        assert_eq!(norm_offset(&cached, san_len + 1), None, "past the end");
    }

    #[test]
    fn norm_offset_mid_codepoint_yields_none() {
        // The defensive char-boundary tripwire: a san_off that lands inside a
        // multi-byte codepoint maps to a non-char-boundary normalized offset
        // and must decline (→ caller falls back) rather than produce a
        // mid-codepoint splice point. "あ" is 3 bytes, so byte 1 is interior.
        let cached = owned("あ\n");
        assert!(
            !cached.sanitized.is_char_boundary(1),
            "byte 1 is mid-codepoint in the sanitized buffer",
        );
        assert_eq!(
            norm_offset(&cached, 1),
            None,
            "mid-codepoint offset declines"
        );
    }

    #[test]
    fn norm_offset_no_node_interior_is_identity() {
        // A document with no classified nodes has zero drift, so norm_offset is
        // the identity at every char boundary (normalized == sanitized).
        let cached = owned("あいうえお\n");
        assert!(cached.source_nodes.is_empty(), "plain text has no nodes");
        assert_eq!(cached.normalized, cached.sanitized, "no sentinels inserted");
        for b in 0..=cached.sanitized.len() {
            if cached.sanitized.is_char_boundary(b) {
                let off = u32::try_from(b).unwrap();
                assert_eq!(norm_offset(&cached, off), Some(off), "identity at {b}");
            }
        }
    }

    // ---- reparse_incremental_owned (owned-table splice) ----

    use aozora_render::{render_html_owned, serialize_owned};

    /// Apply a single-region edit (`replacement` swapped for `edit`) to `san`.
    fn apply_edit(san: &str, edit: Range<usize>, replacement: &str) -> String {
        let mut out = String::with_capacity(san.len() + replacement.len());
        out.push_str(&san[..edit.start]);
        out.push_str(replacement);
        out.push_str(&san[edit.end..]);
        out
    }

    /// Assert the spliced output is byte-identical to a full re-parse on every
    /// resolved/rendered surface the differential gate also checks.
    fn assert_splice_matches_full(spliced: &OwnedLexOutput, full: &OwnedLexOutput) {
        assert_eq!(spliced.normalized, full.normalized, "normalized");
        assert_eq!(spliced.sanitized, full.sanitized, "sanitized");
        assert_eq!(spliced.sanitized_len, full.sanitized_len, "sanitized_len");
        assert_eq!(
            render_html_owned(spliced),
            render_html_owned(full),
            "rendered HTML",
        );
        assert_eq!(
            serialize_owned(spliced),
            serialize_owned(full),
            "serialized source",
        );
        assert_eq!(
            spliced.registry.len(),
            full.registry.len(),
            "registry length",
        );
        let spliced_positions: Vec<u32> = spliced.registry.iter_sorted().map(|(p, _)| p).collect();
        let full_positions: Vec<u32> = full.registry.iter_sorted().map(|(p, _)| p).collect();
        assert_eq!(spliced_positions, full_positions, "registry positions");
        assert_eq!(
            spliced.source_nodes.len(),
            full.source_nodes.len(),
            "source_nodes length",
        );
        assert_eq!(spliced.pairs.len(), full.pairs.len(), "pairs length");
        assert_eq!(
            spliced.container_pairs.len(),
            full.container_pairs.len(),
            "container_pairs length",
        );
    }

    #[test]
    fn plain_interior_edit_splices_byte_identical() {
        // Three blank-line-separated plain paragraphs; insert a plain kana
        // inside the middle one. The region is the middle paragraph alone.
        let cached = owned("あいうえお\n\nかきくけこ\n\nさしすせそ\n");
        let san = cached.sanitized.clone();
        let at = san.find("くけこ").expect("middle paragraph");
        let edit = at..at;
        let new_san = apply_edit(&san, edit.clone(), "も");
        let full = owned(&new_san);
        assert_eq!(full.sanitized, new_san, "edit is a sanitize fixed point");

        let spliced = reparse_incremental_owned(&cached, &new_san, edit)
            .expect("plain interior edit must take the fast path");
        assert!(
            spliced.reused_nodes > 0 || cached.source_nodes.is_empty(),
            "an interior splice of a multi-paragraph doc reuses flanking nodes",
        );
        assert_splice_matches_full(&spliced.output, &full);
    }

    #[test]
    fn standalone_block_adjacent_edit_splices_byte_identical() {
        // A standalone block leaf (改ページ) sits before the edited paragraph;
        // its normalized drift (lead + sentinel + trail padding) must be carried
        // through the prefix offset map. Edit the paragraph after the block.
        let cached = owned("前の段落\n\n［＃改ページ］\n\n後の段落です\n");
        let san = cached.sanitized.clone();
        let at = san.find("段落です").expect("trailing paragraph") + "段落".len();
        let edit = at..at;
        let new_san = apply_edit(&san, edit.clone(), "や");
        let full = owned(&new_san);
        assert_eq!(full.sanitized, new_san, "edit is a sanitize fixed point");

        let spliced = reparse_incremental_owned(&cached, &new_san, edit)
            .expect("block-adjacent interior edit must take the fast path");
        assert!(
            spliced.reused_nodes > 0,
            "the 改ページ block leaf sits in the reused prefix",
        );
        assert_eq!(
            spliced.relexed_nodes, 0,
            "the edited trailing paragraph re-lexes to plain text (no nodes)",
        );
        assert_splice_matches_full(&spliced.output, &full);
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

        assert!(
            reparse_incremental_owned(&cached, &new_san, edit).is_none(),
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

        assert!(
            reparse_incremental_owned(&cached, &new_san, edit).is_none(),
            "an unclosed delimiter half must decline",
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

        assert!(
            reparse_incremental_owned(&cached, &new_san, edit).is_none(),
            "a document with a forward reference must decline (text coupling)",
        );
    }
}

/// Property tests pinning every `RegionIndex` query byte-identical to the linear
/// oracle it replaces, over randomly-assembled aozora-shaped documents (#237
/// Tier 2). The oracles (`structurally_safe`, `norm_offset`,
/// `candidate_boundaries`) and a whole-buffer reference region finder are the
/// ground truth; the indexed/outward-scan production forms must match them
/// exactly, which (with the corpus differential gate) is the byte-identity
/// guarantee.
#[cfg(test)]
mod oracle_proptests {
    use core::mem::swap;
    use core::ops::Range;

    use proptest::collection::vec as prop_vec;
    use proptest::prelude::*;

    use super::{
        DiagBaseRef, OwnedLexOutput, RegionIndex, candidate_boundaries, is_whole_document_scoped,
        minimal_balanced_region, node_forbids_region_reuse, norm_offset, structurally_safe,
    };
    use crate::{Diagnostic, Document};

    /// Parse a generated document to its owned lex output.
    fn owned(src: &str) -> OwnedLexOutput {
        Document::new(src).parse_owned()
    }

    /// A [`RegionIndex`] over a cached output's store-free tables.
    fn idx_of(cached: &OwnedLexOutput) -> RegionIndex {
        RegionIndex::build(&cached.source_nodes, &cached.pairs, &cached.diagnostics)
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

        /// `RegionIndex::norm_offset` equals the linear `norm_offset` at every
        /// offset (interstitial or not — the closed form is offset-independent).
        #[test]
        fn norm_offset_indexed_matches_oracle(doc in doc_strategy()) {
            let cached = owned(&doc);
            let index = idx_of(&cached);
            let len = u32::try_from(cached.sanitized.len()).expect("len fits u32");
            for off in 0..=len + 2 {
                prop_assert_eq!(
                    index.norm_offset(&cached, off),
                    norm_offset(&cached, off),
                    "norm_offset mismatch at {} in {:?}", off, doc,
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
            let index = idx_of(&cached);
            let span = cached.sanitized.len() + 1;
            let es = a % span;
            let ee = b % span;
            let edit = Range { start: es, end: ee };
            prop_assert_eq!(
                minimal_balanced_region(
                    DiagBaseRef::with_index(&cached, &index),
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

        /// The `O(1)` flags equal the `any()` scans they replace.
        #[test]
        fn flags_match_any_scans(doc in doc_strategy()) {
            let cached = owned(&doc);
            let index = idx_of(&cached);
            prop_assert_eq!(
                index.has_whole_doc_scoped_diag,
                cached.diagnostics.iter().any(is_whole_document_scoped),
                "has_whole_doc_scoped_diag mismatch in {:?}", doc,
            );
            prop_assert_eq!(
                index.has_unbalanced_delimiter,
                cached.diagnostics.iter().any(|d| matches!(
                    d,
                    Diagnostic::UnclosedBracket { .. } | Diagnostic::UnmatchedClose { .. }
                )),
                "has_unbalanced_delimiter mismatch in {:?}", doc,
            );
            prop_assert_eq!(
                index.has_coupled_node,
                cached.source_nodes.iter().any(|sn| node_forbids_region_reuse(sn.node)),
                "has_coupled_node mismatch in {:?}", doc,
            );
        }
    }
}
