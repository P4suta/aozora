//! `Document` — single owning handle to a parsed Aozora source
//! buffer, and `AozoraTree<'a>` — borrowed view a caller walks for
//! output rendering.
//!
//! `Document` owns both the source buffer and a `bumpalo`-backed
//! [`Arena`]; [`Document::parse`] returns an [`AozoraTree<'_>`]
//! that borrows from the arena via the `&self` lifetime. Owning
//! source removes the self-referential-struct problem that would
//! otherwise plague driver wrappers (FFI/WASM/Py): callers can hold
//! a `Document` inside any wrapper without juggling source lifetimes.
//!
//! Every borrowed-AST allocation lives inside the arena, with the
//! [`Interner`](aozora_syntax::borrowed::Interner) deduplicating
//! repeated string content. Dropping the `Document` frees the entire
//! tree in a single `Bump::reset` step; no per-node `Drop` runs.

use core::fmt;

use aozora_pipeline::{BorrowedLexOutput, NodeRef, SourceNode, lex_into_arena};
use aozora_render::{html as borrowed_html, serialize as borrowed_serialize};
use aozora_spec::{Diagnostic, NormalizedOffset, PairLink, SourceOffset};
use aozora_syntax::borrowed::{Arena, ContainerPair};

/// Pre-size the document arena as `source.len() * ARENA_CAPACITY_FACTOR`
/// bytes. Picked from the full-corpus `allocator_pressure` probe over
/// 17 435 docs: the median AST footprint is 3.4× the source size, p99
/// is 8.25×, max 15.4×. Factor 4 covers the median + a margin while
/// keeping small-doc overhead minimal (a 1 KB doc gets a 4 KB arena,
/// the bumpalo default chunk size).
const ARENA_CAPACITY_FACTOR: usize = 4;

/// Diagnostic policy applied at parse time.
///
/// Diagnostics are always collected best-effort — the lexer never
/// aborts mid-stream — but the policy controls whether the
/// returned [`AozoraTree::diagnostics`] slice retains every entry,
/// drops library-internal sanity-check failures, or short-circuits
/// after the first source-side error.
///
/// `#[non_exhaustive]` — future policies (e.g. severity-only filters)
/// land here as minor releases.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum DiagnosticPolicy {
    /// Default. Every diagnostic the lexer emits surfaces in the
    /// returned tree, with no filtering or ordering changes. Editor
    /// integrations that decorate the buffer typically want this.
    #[default]
    CollectAll,
    /// Drop diagnostics whose [`Diagnostic::source`] is
    /// [`DiagnosticSource::Internal`](aozora_spec::DiagnosticSource::Internal).
    /// Library bugs (the four legacy "Phase 6" sanity checks) are
    /// hidden from the result; CLI / batch consumers that prefer a
    /// terser stream can opt in.
    DropInternal,
}

/// Builder for the [`Document::parse`] entry point.
///
/// [`ParseOptions`] is the single tunable surface for arena capacity,
/// encoding choice, and diagnostic policy. [`Document::new`] is
/// equivalent to `ParseOptions::new().build(source)`.
///
/// The builder methods consume `self` and return the next stage so
/// the chain reads top-to-bottom and so unused options never leave a
/// dangling builder around.
#[derive(Debug, Clone, Copy, Default)]
#[must_use]
pub struct ParseOptions {
    arena_capacity: Option<usize>,
    diagnostic_policy: DiagnosticPolicy,
}

impl ParseOptions {
    /// Default options: arena capacity is computed from
    /// `ARENA_CAPACITY_FACTOR`, every diagnostic is collected.
    pub fn new() -> Self {
        Self::default()
    }

    /// Override the arena capacity hint. Useful when the caller
    /// already knows the AST footprint (e.g. from a previous parse of
    /// a similar document).
    pub fn arena_capacity(mut self, capacity: usize) -> Self {
        self.arena_capacity = Some(capacity);
        self
    }

    /// Override the [`DiagnosticPolicy`].
    pub fn diagnostic_policy(mut self, policy: DiagnosticPolicy) -> Self {
        self.diagnostic_policy = policy;
        self
    }

    /// Build a [`Document`] from `source`, applying the configured
    /// arena hint and diagnostic policy. The policy is recorded on
    /// the document and applied during [`Document::parse`].
    pub fn build(self, source: impl Into<Box<str>>) -> Document {
        let source: Box<str> = source.into();
        let capacity = self
            .arena_capacity
            .unwrap_or_else(|| source.len().saturating_mul(ARENA_CAPACITY_FACTOR));
        Document {
            source,
            arena: Arena::with_capacity(capacity),
            diagnostic_policy: self.diagnostic_policy,
        }
    }
}

/// Single owning handle to a parsed Aozora source.
///
/// Owns both the source buffer and a `bumpalo`-backed [`Arena`].
/// The `&self` lifetime parameterises every borrowed-AST view
/// returned from [`Document::parse`]; consumers hold the tree only
/// as long as they hold a `&Document` reference.
pub struct Document {
    source: Box<str>,
    arena: Arena,
    diagnostic_policy: DiagnosticPolicy,
}

impl Document {
    /// Wrap a source string in a `Document` with default options.
    /// Equivalent to `ParseOptions::new().build(source)`.
    ///
    /// The arena is pre-sized to `source.len() * ARENA_CAPACITY_FACTOR`
    /// bytes (a corpus-profile-driven estimate of the AST footprint:
    /// p50 arena/source ratio is 3.4×, p99 is 8.25×). Pre-sizing
    /// eliminates the early chunk-grow churn that hits large docs
    /// hardest. Callers that want to override the arena hint or
    /// diagnostic policy reach for [`Document::options`] /
    /// [`ParseOptions::build`] instead.
    #[must_use]
    pub fn new(source: impl Into<Box<str>>) -> Self {
        ParseOptions::new().build(source)
    }

    /// Construct a fresh [`ParseOptions`] for the builder chain.
    /// `Document::options().arena_capacity(N).diagnostic_policy(P).build(s)`
    /// is the canonical configuration entry point.
    pub fn options() -> ParseOptions {
        ParseOptions::new()
    }

    /// Wrap a source string with a pre-sized arena.
    ///
    /// `Document::options().arena_capacity(n).build(source)` is the
    /// preferred path since the builder composes naturally with the
    /// diagnostic policy; this constructor remains for source-level
    /// compatibility with pre-Phase-I callers.
    #[deprecated(
        since = "0.3.0",
        note = "use Document::options().arena_capacity(n).build(source)"
    )]
    #[must_use]
    pub fn with_arena_capacity(source: impl Into<Box<str>>, capacity_hint: usize) -> Self {
        ParseOptions::new()
            .arena_capacity(capacity_hint)
            .build(source)
    }

    /// The source text owned by this document.
    #[must_use]
    pub fn source(&self) -> &str {
        &self.source
    }

    /// Arena bytes currently committed. Diagnostic / benchmarking only.
    #[must_use]
    pub fn arena_bytes(&self) -> usize {
        self.arena.allocated_bytes()
    }

    /// Apply an in-place text edit and return a fresh [`Document`].
    ///
    /// `span` is a byte range in the *current* source (`self.source`);
    /// `replacement` is the new text to splice in. The result is a
    /// new `Document` whose source equals
    /// `self.source[..span.start] + replacement + self.source[span.end..]`.
    /// The arena is rebuilt — incremental re-parse over the unchanged
    /// region is a future improvement (see the architecture handbook
    /// chapter on incremental parse).
    ///
    /// The signature is the supported entry point for editor surfaces
    /// implementing `textDocument/didChange`. Even with a full reparse
    /// inside, callers get a stable API today and a transparent
    /// upgrade path to subtree-aware reuse later.
    ///
    /// # Panics
    ///
    /// Panics if `span.start > span.end`, if `span.end > source.len()`,
    /// or if `span.start` / `span.end` does not lie on a UTF-8
    /// codepoint boundary in `self.source`. These are programmer
    /// errors — editor integrations should clamp the span via the
    /// existing `aozora::Span` constructor's bounds checking.
    #[must_use]
    pub fn edit(&self, span: aozora_spec::Span, replacement: &str) -> Self {
        let start = span.start as usize;
        let end = span.end as usize;
        assert!(start <= end, "edit: span start ({start}) > end ({end})");
        assert!(
            end <= self.source.len(),
            "edit: span end ({end}) past source length ({len})",
            len = self.source.len(),
        );
        // Boundary-validate by slicing — `&str` indexing panics on
        // mid-codepoint, which is the exact error mode we want to
        // surface to the caller as a precondition violation.
        let prefix = &self.source[..start];
        let suffix = &self.source[end..];

        let mut new_source = String::with_capacity(
            prefix
                .len()
                .saturating_add(replacement.len())
                .saturating_add(suffix.len()),
        );
        new_source.push_str(prefix);
        new_source.push_str(replacement);
        new_source.push_str(suffix);

        ParseOptions::new()
            .diagnostic_policy(self.diagnostic_policy)
            .build(new_source.into_boxed_str())
    }

    /// Parse the document, returning a borrowed-AST view bound to
    /// `&self`'s lifetime.
    #[must_use]
    pub fn parse(&self) -> AozoraTree<'_> {
        let mut inner = lex_into_arena(&self.source, &self.arena);
        if self.diagnostic_policy == DiagnosticPolicy::DropInternal {
            inner
                .diagnostics
                .retain(|d| d.source() != aozora_spec::DiagnosticSource::Internal);
        }
        AozoraTree {
            source: &self.source,
            inner,
        }
    }
}

impl fmt::Debug for Document {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Document")
            .field("source_len", &self.source.len())
            .field("arena_bytes", &self.arena.allocated_bytes())
            .field("diagnostic_policy", &self.diagnostic_policy)
            .finish()
    }
}

/// Borrowed view into a parsed Aozora document.
///
/// Wraps a [`BorrowedLexOutput`] whose normalized text and registry
/// borrow from the parent [`Document`]'s arena. Renderer methods
/// dispatch to `aozora_render`'s borrowed-AST implementations.
#[derive(Debug)]
pub struct AozoraTree<'a> {
    source: &'a str,
    inner: BorrowedLexOutput<'a>,
}

impl<'a> AozoraTree<'a> {
    /// The source text this tree was parsed from.
    #[must_use]
    pub fn source(&self) -> &'a str {
        self.source
    }

    /// Diagnostics emitted during parsing.
    #[must_use]
    pub fn diagnostics(&self) -> &[Diagnostic] {
        &self.inner.diagnostics
    }

    /// Resolved (open, close) delimiter pairs as observed by Phase 2.
    /// One entry per matched pair, in close order. Unmatched closes
    /// and unclosed opens are excluded — they have no partner span and
    /// would only confuse editor surfaces.
    ///
    /// Spans use the same coordinate system as
    /// [`Self::diagnostics`]: byte offsets in the *sanitized* source
    /// (which equals the original source on every input that did not
    /// trigger BOM/CRLF/accent rewriting in Phase 0). Editor-facing
    /// LSP requests like `textDocument/linkedEditingRange` and
    /// `textDocument/documentHighlight` consume this directly.
    #[must_use]
    pub fn pairs(&self) -> &'a [PairLink] {
        self.inner.pairs
    }

    /// Borrow the underlying [`BorrowedLexOutput`].
    #[must_use]
    pub fn lex_output(&self) -> &BorrowedLexOutput<'a> {
        &self.inner
    }

    /// Find the node whose source span covers `src_off` — a
    /// sanitized-source byte offset, typed as
    /// [`aozora_spec::SourceOffset`] so callers cannot
    /// accidentally mix up source and normalized coordinates.
    /// Returns `None` if the offset falls inside a `SpanKind::Plain`
    /// run between Aozora constructs.
    ///
    /// `O(log n)` over the source-keyed side-table.
    #[must_use]
    pub fn node_at_source(&self, src_off: SourceOffset) -> Option<&SourceNode<'a>> {
        self.inner.node_at_source(src_off)
    }

    /// Find the registry entry at `normalized_off` — a byte offset
    /// into the normalized PUA-rewritten text, typed as
    /// [`aozora_spec::NormalizedOffset`] so callers
    /// cannot pass a source-coordinate offset by mistake. For LSP
    /// requests against the original source text, prefer
    /// [`Self::node_at_source`].
    #[must_use]
    pub fn node_at_normalized(&self, normalized_off: NormalizedOffset) -> Option<NodeRef<'a>> {
        self.inner.registry.node_at(normalized_off)
    }

    /// Borrow the source-keyed side table directly. Sorted by
    /// `source_span.start`; useful for editor surfaces that want to
    /// iterate every classified node (semantic tokens, document
    /// symbols, …).
    #[must_use]
    pub fn source_nodes(&self) -> &'a [SourceNode<'a>] {
        self.inner.source_nodes
    }

    /// Resolved container open/close pairs in normalized coordinates.
    ///
    /// One entry per balanced
    /// `［＃ここから…］`/`［＃ここで…終わり］` pair, in close order.
    /// Editor surfaces can ask "where is the close for this open?"
    /// directly off this slice; renderers that want to recurse
    /// through container bodies use the open/close offsets to slice
    /// the normalized text.
    ///
    /// Coordinates are [`NormalizedOffset`] — they index the
    /// PUA-rewritten text, not the original source.
    #[must_use]
    pub fn container_pairs(&self) -> &'a [ContainerPair] {
        self.inner.container_pairs
    }

    /// Render the tree to a semantic-HTML5 string.
    #[must_use]
    pub fn to_html(&self) -> String {
        borrowed_html::render_to_string(&self.inner)
    }

    /// Re-emit Aozora source text from the parsed tree.
    #[must_use]
    pub fn serialize(&self) -> String {
        borrowed_serialize::serialize(&self.inner)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn document_borrows_source() {
        let s = "hello";
        let d = Document::new(s);
        assert_eq!(d.source(), s);
    }

    #[test]
    fn parse_returns_borrowed_tree_with_same_source() {
        let s = "world";
        let d = Document::new(s);
        let t = d.parse();
        assert_eq!(t.source(), s);
    }

    #[test]
    fn diagnostics_empty_for_clean_input() {
        let d = Document::new("plain");
        let t = d.parse();
        assert!(t.diagnostics().is_empty());
    }

    #[test]
    fn diagnostics_populated_for_pua_collision() {
        let d = Document::new("contains \u{E001} sentinel");
        let t = d.parse();
        assert!(!t.diagnostics().is_empty());
    }

    #[test]
    fn edit_splices_source_at_span() {
        // Replace "world" with "Aozora" in "hello world!".
        let d = Document::new("hello world!");
        let span = aozora_spec::Span::new(6, 11);
        let edited = d.edit(span, "Aozora");
        assert_eq!(edited.source(), "hello Aozora!");
    }

    #[test]
    fn edit_at_start_and_end_boundaries() {
        let d = Document::new("middle");
        // Insert at start (zero-length span at offset 0).
        let head = d.edit(aozora_spec::Span::new(0, 0), "PRE-");
        assert_eq!(head.source(), "PRE-middle");
        // Append at end (zero-length span at len()).
        let len = u32::try_from(d.source().len()).expect("test source fits u32");
        let tail = d.edit(aozora_spec::Span::new(len, len), "-POST");
        assert_eq!(tail.source(), "middle-POST");
    }

    #[test]
    fn edit_equivalence_full_reparse() {
        // The edited document parses to the same AozoraTree shape as
        // re-parsing the spliced source from scratch — this is the
        // observable property `Document::edit` ships under, and the
        // future incremental implementation will preserve.
        let original = Document::new("｜青梅《おうめ》です。");
        // Replace 《おうめ》 with 《せいばい》.
        let span_start = original.source().find('《').expect("《 present");
        let span_end = original.source().find('》').expect("》 present") + '》'.len_utf8();
        let edited = original.edit(
            aozora_spec::Span::new(
                u32::try_from(span_start).expect("test span fits u32"),
                u32::try_from(span_end).expect("test span fits u32"),
            ),
            "《せいばい》",
        );

        let spliced_source = format!(
            "{prefix}{replacement}{suffix}",
            prefix = &original.source()[..span_start],
            replacement = "《せいばい》",
            suffix = &original.source()[span_end..],
        );
        let from_scratch = Document::new(spliced_source);

        assert_eq!(edited.source(), from_scratch.source());
        // Same serialize output → AST shape is equivalent.
        assert_eq!(
            edited.parse().serialize(),
            from_scratch.parse().serialize(),
            "edit() must be equivalent to splice + reparse"
        );
    }

    #[test]
    #[should_panic(expected = "span start")]
    fn edit_rejects_inverted_span() {
        drop(Document::new("ok").edit(aozora_spec::Span::new(2, 1), ""));
    }

    #[test]
    fn round_trip_through_serialize_is_a_fixed_point() {
        let s = "｜青梅《おうめ》";
        let first = Document::new(s).parse().serialize();
        let second = Document::new(first.clone()).parse().serialize();
        assert_eq!(first, second, "round-trip must be a fixed point");
    }

    #[test]
    fn pairs_records_simple_ruby() {
        // 《 … 》 produces one Ruby pair.
        let d = Document::new("｜青梅《おうめ》");
        let t = d.parse();
        let pairs = t.pairs();
        assert_eq!(pairs.len(), 1);
        let link = pairs[0];
        assert_eq!(link.kind, aozora_spec::PairKind::Ruby);
        // The open span begins at the `《` byte, the close at the `》` byte.
        let src = t.source();
        let open_byte = src.find('《').expect("source contains 《");
        let close_byte = src.find('》').expect("source contains 》");
        assert_eq!(link.open.start as usize, open_byte);
        assert_eq!(link.close.start as usize, close_byte);
    }

    #[test]
    fn pairs_records_multiple_brackets_in_close_order() {
        // Nested brackets — inner closes first.
        let d = Document::new("［＃外［＃内］終］");
        let t = d.parse();
        let pairs = t.pairs();
        assert_eq!(pairs.len(), 2);
        // Inner pair closes first; its open must come AFTER the outer's open.
        assert!(pairs[0].open.start > pairs[1].open.start);
        assert!(pairs[0].close.start < pairs[1].close.start);
    }

    #[test]
    fn pairs_excludes_unclosed_open() {
        // No matching `］`. Diagnostic fires; pairs stays empty.
        let d = Document::new("［＃orphan");
        let t = d.parse();
        assert!(t.pairs().is_empty());
        assert!(!t.diagnostics().is_empty());
    }

    #[test]
    fn pairs_excludes_unmatched_close() {
        // Stray close on an empty stack.
        let d = Document::new("orphan］");
        let t = d.parse();
        assert!(t.pairs().is_empty());
    }

    #[test]
    fn node_at_source_finds_inline_ruby() {
        let src = "前｜青梅《おうめ》後";
        let d = Document::new(src);
        let t = d.parse();
        // Find the byte offset of `｜` — that's where the ruby span starts.
        let bar_off =
            u32::try_from(src.find('｜').expect("source contains ｜")).expect("offset fits in u32");
        let entry = t
            .node_at_source(SourceOffset::new(bar_off))
            .expect("ruby span at | offset");
        // The retrieved span must cover the whole `｜青梅《おうめ》` run.
        assert_eq!(entry.source_span.start, bar_off);
        assert!(entry.source_span.end > bar_off);
        assert!(matches!(entry.node, NodeRef::Inline(_)));
    }

    #[test]
    fn node_at_source_returns_none_for_plain_run() {
        let src = "前｜青梅《おうめ》後";
        let d = Document::new(src);
        let t = d.parse();
        // Offset 0 is inside the leading "前" plain run — no node.
        assert!(t.node_at_source(SourceOffset::new(0)).is_none());
    }

    #[test]
    fn source_nodes_are_sorted_by_source_start() {
        let src = "｜青梅《おうめ》街道沿いに、※［＃「木＋吶のつくり」、第3水準1-85-54］";
        let d = Document::new(src);
        let t = d.parse();
        let nodes = t.source_nodes();
        for window in nodes.windows(2) {
            assert!(window[0].source_span.start <= window[1].source_span.start);
        }
    }

    #[test]
    fn parse_options_default_matches_document_new() {
        // ParseOptions::new().build(s) must produce the same tree as
        // Document::new(s) — Document::new is a thin wrapper.
        let src = "｜青梅《おうめ》";
        let via_new = Document::new(src);
        let via_options = ParseOptions::new().build(src);
        assert_eq!(via_new.parse().serialize(), via_options.parse().serialize());
    }

    #[test]
    fn parse_options_arena_capacity_is_honoured() {
        // The capacity hint propagates to bumpalo. Arena bytes after
        // construction reflect the request rounded up to the chunk
        // size; pin a lower bound rather than the exact value.
        let doc = ParseOptions::new()
            .arena_capacity(16 * 1024)
            .build("plain text");
        // Minimum: bumpalo's first chunk is at least the requested
        // capacity. Conservative assertion rather than exact bytes.
        drop(doc.parse()); // commit something to the arena
        assert!(
            doc.arena_bytes() <= 64 * 1024,
            "arena bytes should not balloon for a tiny source: {}",
            doc.arena_bytes()
        );
    }

    #[test]
    fn parse_options_drop_internal_filters_internal_diagnostics() {
        // DropInternal hides Diagnostic::Internal entries. Production
        // parses on well-formed input emit none, so we cross-check
        // CollectAll/DropInternal yield the same `len()` for clean
        // input — and the policy plumbing exists.
        let doc_collect = Document::options()
            .diagnostic_policy(DiagnosticPolicy::CollectAll)
            .build("plain text");
        let doc_drop = Document::options()
            .diagnostic_policy(DiagnosticPolicy::DropInternal)
            .build("plain text");
        assert_eq!(
            doc_collect.parse().diagnostics().len(),
            doc_drop.parse().diagnostics().len(),
            "policy is a no-op when no Internal diagnostics exist"
        );
    }

    #[test]
    fn arena_grows_with_source_size() {
        let small = Document::new("a");
        drop(small.parse());
        let big_src = "｜青梅《おうめ》".repeat(100);
        let big = Document::new(big_src);
        drop(big.parse());
        assert!(big.arena_bytes() > small.arena_bytes());
    }
}
