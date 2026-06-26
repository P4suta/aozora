//! `Document` — single owning handle to a parsed Aozora source
//! buffer, and `Tree<'a>` — a view a caller walks for output rendering.
//!
//! `Document` owns the source buffer; [`Document::parse`] returns a
//! [`Tree<'_>`] whose `'a` lifetime tracks only that `&self` source borrow —
//! the AST data itself is owned, lifetime-free, and `Send + Sync`
//! (an `OwnedLexOutput`). Owning source removes the self-referential-struct
//! problem that would otherwise plague driver wrappers (FFI/WASM/Py): callers
//! can hold a `Document` inside any wrapper without juggling source lifetimes.
//!
//! The owned AST stores interned strings and node payloads in a flat
//! `NodeStore` (the owned `StrInterner` deduplicates repeated string content);
//! dropping the tree frees them in one step, with no per-node `Drop`.

use core::fmt;

use aozora_pipeline::{NodeRefOwned, OwnedLexOutput, SourceNodeOwned, lex};
use aozora_render::{render_html_owned, serialize_owned};
use aozora_spec::{Diagnostic, NormalizedOffset, PairLink, SourceOffset};
use aozora_syntax::owned::ContainerPair;

/// Diagnostic policy applied at parse time.
///
/// Diagnostics are always collected best-effort — the lexer never
/// aborts mid-stream — but the policy controls whether the
/// returned [`Tree::diagnostics`] slice retains every entry,
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
    /// Library bugs (the four legacy internal sanity checks) are
    /// hidden from the result; CLI / batch consumers that prefer a
    /// terser stream can opt in.
    DropInternal,
}

/// Builder for the [`Document::parse`] entry point.
///
/// [`ParseOptions`] is the single tunable surface for the diagnostic policy.
/// [`Document::new`] is equivalent to `ParseOptions::new().build(source)`.
///
/// The builder methods consume `self` and return the next stage so
/// the chain reads top-to-bottom and so unused options never leave a
/// dangling builder around.
#[derive(Debug, Clone, Copy, Default)]
#[must_use]
pub struct ParseOptions {
    diagnostic_policy: DiagnosticPolicy,
}

impl ParseOptions {
    /// Default options: every diagnostic is collected.
    pub fn new() -> Self {
        Self::default()
    }

    /// Override the [`DiagnosticPolicy`].
    pub fn diagnostic_policy(mut self, policy: DiagnosticPolicy) -> Self {
        self.diagnostic_policy = policy;
        self
    }

    /// Build a [`Document`] from `source`, applying the configured diagnostic
    /// policy. The policy is recorded on the document and applied during
    /// [`Document::parse`].
    pub fn build(self, source: impl Into<Box<str>>) -> Document {
        Document {
            source: source.into(),
            diagnostic_policy: self.diagnostic_policy,
        }
    }
}

/// Single owning handle to a parsed Aozora source.
///
/// Owns the source buffer. [`Document::parse`] runs the owned, arena-free lex
/// pipeline and returns a [`Tree`] that owns all its AST data and borrows only
/// `&self`'s source.
pub struct Document {
    source: Box<str>,
    diagnostic_policy: DiagnosticPolicy,
}

impl Document {
    /// Wrap a source string in a `Document` with default options.
    /// Equivalent to `ParseOptions::new().build(source)`.
    #[must_use]
    pub fn new(source: impl Into<Box<str>>) -> Self {
        ParseOptions::new().build(source)
    }

    /// Construct a fresh [`ParseOptions`] for the builder chain.
    /// `Document::options().diagnostic_policy(P).build(s)` is the canonical
    /// configuration entry point.
    pub fn options() -> ParseOptions {
        ParseOptions::new()
    }

    /// The source text owned by this document.
    #[must_use]
    pub fn source(&self) -> &str {
        &self.source
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

    /// Apply a node-aware minimal-diff edit and return a fresh [`Document`].
    ///
    /// `region` must come from this document's
    /// [`Tree::owned_regions`](crate::Tree::owned_regions) /
    /// [`Tree::owned_region_at`](crate::Tree::owned_region_at); `replacement`
    /// is the new source for the region's own bytes. Unlike [`Self::edit`] —
    /// which takes a raw byte span and trusts the caller — this routes through
    /// [`Tree::splice`](crate::Tree::splice): a
    /// [`Coupled`](crate::SpliceSafety::Coupled) region's partner (a forward
    /// reference's upstream literal, a container's matching close) is derived
    /// and the edit is verified by re-parse, so the result cannot silently
    /// desync.
    ///
    /// The returned document's source is the **sanitized**-then-spliced text:
    /// byte-identical to [`Self::edit`] on inputs that triggered no sanitize
    /// rewrite, and equal to `splice + Document::new` otherwise (a
    /// sanitized-coordinate region cannot be applied to un-sanitized bytes).
    ///
    /// # Errors
    ///
    /// Propagates [`SpliceError`](crate::SpliceError) from
    /// [`Tree::splice`](crate::Tree::splice): an unverifiable coupled edit or
    /// an opaque node kind. On error the original document is unchanged (the
    /// caller still owns `self`).
    pub fn edit_region(
        &self,
        region: crate::OwnedRegion,
        replacement: &str,
    ) -> Result<Self, crate::SpliceError> {
        let spliced = self.parse().splice(region, replacement)?;
        Ok(ParseOptions::new()
            .diagnostic_policy(self.diagnostic_policy)
            .build(spliced))
    }

    /// Parse the document, returning a [`Tree<'_>`] view bound to `&self`'s
    /// lifetime (only its `source` borrow; the AST data is owned).
    ///
    /// Delegates to [`Self::parse_owned`] — the default parse now produces the
    /// owned, lifetime-free representation — and wraps it with the source
    /// borrow so the editor-facing [`Tree`] surface keeps the same shape.
    #[must_use]
    pub fn parse(&self) -> Tree<'_> {
        Tree {
            source: &self.source,
            inner: TreeInner::Owned(self.parse_owned()),
        }
    }

    /// Parse the document into the owned, lifetime-free [`OwnedLexOutput`].
    ///
    /// The owned twin of [`Self::parse`]: it runs the same lex pipeline through
    /// the native owned fold ([`lex`]), so the result owns all its payloads and
    /// is `Send + Sync`. Applies the same [`DiagnosticPolicy`] filtering as
    /// [`Self::parse`].
    ///
    /// This is the entry point the #237 incremental-reparse LSP consumer holds
    /// across edits; renderers reach it through `aozora_render`'s owned paths
    /// (`serialize_owned` / `render_html_owned`).
    #[must_use]
    pub fn parse_owned(&self) -> OwnedLexOutput {
        let mut out = lex(&self.source);
        if self.diagnostic_policy == DiagnosticPolicy::DropInternal {
            out.diagnostics
                .retain(|d| d.source() != aozora_spec::DiagnosticSource::Internal);
        }
        out
    }
}

impl fmt::Debug for Document {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Document")
            .field("source_len", &self.source.len())
            .field("diagnostic_policy", &self.diagnostic_policy)
            .finish()
    }
}

/// View into a parsed Aozora document.
///
/// Wraps an owned, lifetime-free [`OwnedLexOutput`] (the `'a` lifetime now
/// tracks only the `source` borrow). The output may be **owned** by this tree
/// (the usual [`Document::parse`] case) or **borrowed** from a longer-lived
/// holder (via [`Tree::view`], used by caches such as the LSP `ParseCache`
/// that retain the owned output and want tree access without re-parsing).
/// Renderer methods dispatch to `aozora_render`'s owned-AST implementations;
/// the side-table accessors return owned types (`SourceNodeOwned` /
/// `NodeRefOwned`) whose payload text resolves through the output's
/// `NodeStore`.
#[derive(Debug)]
pub struct Tree<'a> {
    source: &'a str,
    inner: TreeInner<'a>,
}

/// Storage for a [`Tree`]'s parsed output: either owned outright or borrowed
/// from a longer-lived holder. Both forms expose the same `&OwnedLexOutput`
/// through [`Tree::inner`].
#[derive(Debug)]
enum TreeInner<'a> {
    /// The tree owns its output (the [`Document::parse`] case).
    Owned(OwnedLexOutput),
    /// The tree borrows an output owned elsewhere (the [`Tree::view`] case).
    Borrowed(&'a OwnedLexOutput),
}

impl<'a> Tree<'a> {
    /// Build a [`Tree`] view that borrows an already-parsed [`OwnedLexOutput`].
    /// Used by long-lived caches (e.g. the LSP `ParseCache`) that retain the
    /// owned output and want tree access without re-parsing.
    #[must_use]
    pub fn view(source: &'a str, output: &'a OwnedLexOutput) -> Self {
        Self {
            source,
            inner: TreeInner::Borrowed(output),
        }
    }

    /// Borrow the underlying output regardless of owned/borrowed storage.
    fn inner(&self) -> &OwnedLexOutput {
        match &self.inner {
            TreeInner::Owned(o) => o,
            TreeInner::Borrowed(o) => o,
        }
    }

    /// The source text this tree was parsed from.
    #[must_use]
    pub fn source(&self) -> &'a str {
        self.source
    }

    /// Diagnostics emitted during parsing.
    #[must_use]
    pub fn diagnostics(&self) -> &[Diagnostic] {
        &self.inner().diagnostics
    }

    /// Resolved (open, close) delimiter pairs as observed by the pair stage.
    /// One entry per matched pair, in close order. Unmatched closes
    /// and unclosed opens are excluded — they have no partner span and
    /// would only confuse editor surfaces.
    ///
    /// Spans use the same coordinate system as
    /// [`Self::diagnostics`]: byte offsets in the *sanitized* source
    /// (which equals the original source on every input that did not
    /// trigger BOM/CRLF/accent rewriting in the sanitize stage). Editor-facing
    /// LSP requests like `textDocument/linkedEditingRange` and
    /// `textDocument/documentHighlight` consume this directly.
    #[must_use]
    pub fn pairs(&self) -> &[PairLink] {
        &self.inner().pairs
    }

    /// Borrow the underlying [`OwnedLexOutput`].
    #[must_use]
    pub fn lex_output(&self) -> &OwnedLexOutput {
        self.inner()
    }

    /// Find the node whose source span covers `src_off` — a
    /// sanitized-source byte offset, typed as
    /// [`aozora_spec::SourceOffset`] so callers cannot
    /// accidentally mix up source and normalized coordinates.
    /// Returns `None` if the offset falls inside a `SpanKind::Plain`
    /// run between Aozora constructs.
    ///
    /// This is the canonical offset→node lookup: editor and LSP surfaces
    /// work in source coordinates, so this is the position query to reach
    /// for (the normalized-coordinate registry is a low-level internal).
    ///
    /// `O(log n)` over the source-keyed side-table.
    #[must_use]
    pub fn node_at_source(&self, src_off: SourceOffset) -> Option<&SourceNodeOwned> {
        self.inner().node_at_source(src_off)
    }

    /// Find the registry entry at `normalized_off` — a byte offset into
    /// the normalized PUA-rewritten text.
    ///
    /// The normalized coordinate space is a low-level implementation
    /// detail with no external consumer. Source-coordinate lookups should
    /// use [`Self::node_at_source`]; a consumer that genuinely needs a
    /// normalized-offset lookup (e.g. the afm integration) should reach
    /// the `registry` through [`Self::lex_output`] directly.
    #[doc(hidden)]
    #[deprecated(note = "use lex_output().registry.node_at() for normalized-offset lookups")]
    #[must_use]
    pub fn node_at_normalized(&self, normalized_off: NormalizedOffset) -> Option<NodeRefOwned> {
        self.inner().registry.node_at(normalized_off)
    }

    /// Borrow the source-keyed side table directly. Sorted by
    /// `source_span.start`; useful for editor surfaces that want to
    /// iterate every classified node (semantic tokens, document
    /// symbols, …).
    ///
    /// **Host literal contexts.** A host that embeds aozora into a larger
    /// grammar (e.g. CommonMark via comrak) collapses each notation into a
    /// PUA sentinel before its own parse. When the host routes a sentinel
    /// into a *literal* field — a code span `` `…` `` or a link/image
    /// destination — the notation must appear as its **original source**, not
    /// be interpreted. Such a host must resolve *every* sentinel it emits
    /// (including ones in literal regions) and recover the original text from
    /// `SourceNode::source_span` + [`Span::slice`](crate::Span::slice);
    /// resolving only "normal"-text sentinels leaks the raw sentinel and
    /// desyncs the registry cursor. See the *Notations in host literal
    /// contexts* recipe in the handbook.
    #[must_use]
    pub fn source_nodes(&self) -> &[SourceNodeOwned] {
        &self.inner().source_nodes
    }

    /// The sanitized source buffer — the exact bytes the lexer
    /// classified, after the sanitize stage (BOM-strip, CRLF→LF,
    /// `〔...〕` accent decomposition, decorative-rule isolation, PUA
    /// neutralization).
    ///
    /// This is the coordinate space every `source_span` on
    /// [`Self::source_nodes`] / [`Self::pairs`] / [`Self::diagnostics`]
    /// indexes — equal to [`Self::source`] byte-for-byte on inputs that
    /// triggered no sanitize rewrite. It carries no PUA sentinels and no
    /// synthesized block padding, so it is the verbatim round-trip basis
    /// returned by [`Self::to_source_verbatim`].
    #[must_use]
    pub fn sanitized(&self) -> &str {
        &self.inner().sanitized
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
    pub fn container_pairs(&self) -> &[ContainerPair] {
        &self.inner().container_pairs
    }

    /// Render the tree to a semantic-HTML5 string.
    #[must_use]
    pub fn to_html(&self) -> String {
        render_html_owned(self.inner())
    }

    /// Re-emit Aozora source text from the parsed tree.
    #[must_use]
    pub fn to_source(&self) -> String {
        serialize_owned(self.inner())
    }

    /// Recover the source text **verbatim** — byte-for-byte equal to
    /// `sanitize(source)`, the input the lexer actually classified.
    ///
    /// Distinct from [`Self::to_source`], which *re-serializes* the
    /// parsed tree (walking the normalized stream and synthesizing block
    /// padding around sentinels — a canonical, not byte-preserving,
    /// form). `to_source_verbatim` instead returns the retained
    /// sanitized buffer unchanged, so the contract is the strongest
    /// fixed point the pipeline can offer:
    ///
    /// ```text
    /// to_source_verbatim(parse(doc)) == sanitize(doc)
    /// ```
    ///
    /// It is **not** equal to the original `doc` whenever sanitation
    /// fired (BOM-strip and PUA neutralization are lossy/irreversible),
    /// which is why the basis is `sanitize(doc)` rather than `doc`.
    ///
    /// The buffer is returned directly from the side table populated at
    /// parse time; no tree walk and no re-sanitation runs. Allocates one
    /// owned `String` (matching [`Self::to_source`]'s signature); callers
    /// that want a borrow can use [`Self::sanitized`] instead.
    #[must_use]
    pub fn to_source_verbatim(&self) -> String {
        self.inner().sanitized.clone()
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
        // The edited document parses to the same Tree shape as
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
        // Same to_source output → AST shape is equivalent.
        assert_eq!(
            edited.parse().to_source(),
            from_scratch.parse().to_source(),
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
        let first = Document::new(s).parse().to_source();
        let second = Document::new(first.clone()).parse().to_source();
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
        assert!(matches!(entry.node, NodeRefOwned::Inline(_)));
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
        assert_eq!(via_new.parse().to_source(), via_options.parse().to_source());
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
    // ---- to_source_verbatim / sanitized contract ----
    //
    // Contract: `to_source_verbatim(parse(doc)) == sanitize(doc).text`,
    // byte-for-byte (NOT `== doc` — sanitation is lossy). The accessor
    // `sanitized()` returns the same held buffer, so the two must agree
    // for every input regardless of node shape or sanitize rewrite.

    /// The independent oracle: run the real sanitize stage on `doc` and
    /// assert the parsed tree reproduces it verbatim through both
    /// surfaces. `sanitize` is reached via the unconditional
    /// `crate::pipeline` re-export (same path `cst::from_tree` uses).
    fn assert_verbatim_equals_sanitize(doc: &str) {
        use crate::pipeline::lexer::sanitize::sanitize;
        let expected = sanitize(doc).text;
        let d = Document::new(doc);
        let t = d.parse();
        assert_eq!(
            t.to_source_verbatim(),
            *expected,
            "to_source_verbatim must equal sanitize(doc).text"
        );
        // The borrowing accessor exposes the identical buffer.
        assert_eq!(
            t.sanitized(),
            &*expected,
            "sanitized() must equal sanitize(doc).text"
        );
        // Internal self-consistency: the owned and borrowed surfaces agree.
        assert_eq!(t.to_source_verbatim(), t.sanitized());
    }

    #[test]
    fn verbatim_plain_only() {
        // A plain run with no Aozora construct: source_nodes is empty,
        // so the buffer must still be recovered intact.
        assert_verbatim_equals_sanitize("ただの平文です。\n二行目。\n");
    }

    #[test]
    fn verbatim_plain_construct_ruby() {
        // ｜青梅《おうめ》 — one inline ruby node surrounded by plain runs
        // (leading-gap + trailing-gap + the node itself).
        assert_verbatim_equals_sanitize("前置き｜青梅《おうめ》後置き");
    }

    #[test]
    fn verbatim_block_container() {
        // ［＃ここから２字下げ］ … ［＃ここで字下げ終わり］ — a block
        // container open/close pair plus body text.
        assert_verbatim_equals_sanitize(
            "序\n［＃ここから２字下げ］\n本文の段落。\n［＃ここで字下げ終わり］\n了\n",
        );
    }

    #[test]
    fn verbatim_consecutive_nodes_no_gap() {
        // Two ruby runs back-to-back: adjacent source spans with no
        // intervening plain text. Locks that touching nodes round-trip.
        assert_verbatim_equals_sanitize("｜青梅《おうめ》｜街道《かいどう》");
    }

    #[test]
    fn verbatim_leading_node_no_head_gap() {
        // Construct at byte 0 (no leading plain run) followed by a tail
        // gap. Exercises the head-of-buffer boundary.
        assert_verbatim_equals_sanitize("｜青梅《おうめ》のち平文");
    }

    #[test]
    fn verbatim_trailing_node_no_tail_gap() {
        // Construct flush against the end of the buffer (no trailing
        // plain run). Exercises the tail-of-buffer boundary.
        assert_verbatim_equals_sanitize("平文のち｜青梅《おうめ》");
    }

    #[test]
    fn verbatim_basis_is_sanitize_not_raw_doc_bom() {
        // BOM strip is lossy: verbatim must equal the POST-sanitize text
        // (BOM gone), not the raw doc. Stacked BOMs collapse to nothing.
        let doc = "\u{FEFF}\u{FEFF}｜青梅《おうめ》";
        assert_verbatim_equals_sanitize(doc);
        let t_doc = Document::new(doc);
        assert_ne!(
            t_doc.parse().to_source_verbatim(),
            doc,
            "verbatim must NOT equal the raw doc once a BOM was stripped"
        );
        assert!(
            !t_doc.parse().to_source_verbatim().starts_with('\u{FEFF}'),
            "stacked BOMs must be gone from the verbatim text"
        );
    }

    #[test]
    fn verbatim_basis_is_sanitize_crlf() {
        // CRLF → LF is applied by sanitize; verbatim reflects the LF form.
        let doc = "一行目\r\n二行目\r\n｜青梅《おうめ》\r\n";
        assert_verbatim_equals_sanitize(doc);
        assert!(
            !Document::new(doc)
                .parse()
                .to_source_verbatim()
                .contains('\r'),
            "CR must be normalized out of the verbatim text"
        );
    }

    #[test]
    fn verbatim_basis_is_sanitize_accent_span() {
        // 〔...〕 accent decomposition rewrites the bytes inside the
        // tortoiseshell brackets; verbatim must carry the rewritten form.
        assert_verbatim_equals_sanitize("カフェ〔cafe'〕で待つ");
    }

    #[test]
    fn verbatim_basis_is_sanitize_decorative_rule() {
        // A ≥10-char decorative rule line gets a blank line inserted
        // before it by sanitize; verbatim reflects that insertion.
        assert_verbatim_equals_sanitize("段落の文\n----------\nつづき\n");
    }

    #[test]
    fn verbatim_basis_is_sanitize_pua_neutralized() {
        // Raw U+E001..U+E004 are irreversibly rewritten to U+FFFD by the
        // PUA-neutralize step. Verbatim must show the U+FFFD, and must
        // NOT equal the raw doc (the rewrite is lossy).
        let doc = "before\u{E001}mid\u{E004}after";
        assert_verbatim_equals_sanitize(doc);
        let recovered = Document::new(doc).parse().to_source_verbatim();
        assert!(
            recovered.contains('\u{FFFD}') && !recovered.contains('\u{E001}'),
            "raw PUA sentinels must come back as U+FFFD"
        );
        assert_ne!(recovered, doc, "PUA neutralization is irreversible");
    }
}
