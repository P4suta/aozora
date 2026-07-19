//! `Document` — single owning handle to a parsed Aozora source
//! buffer, and `Snapshot` — a view a caller walks for output rendering.
//!
//! `Document` owns the source buffer; [`Document::snapshot`] returns a
//! [`Snapshot`] whose `'a` lifetime tracks only that `&self` source borrow —
//! the AST data itself is owned, lifetime-free, and `Send + Sync`
//! (an `LexOutput`). Owning source removes the self-referential-struct
//! problem that would otherwise plague driver wrappers (FFI/WASM/Py): callers
//! can hold a `Document` inside any wrapper without juggling source lifetimes.
//!
//! The owned AST stores interned strings and node payloads in a flat
//! `NodeStore` (the owned `StrInterner` deduplicates repeated string content);
//! dropping the tree frees them in one step, with no per-node `Drop`.

use core::fmt;
use core::ops::Range;
use std::sync::Arc;

use crate::pipeline::{LexOutput, SourceNode, lex};
use crate::render::{
    DirectiveNormalization, RenderOptions, SerializeOptions, render_html, render_html_normalized,
    serialize, serialize_with,
};
use crate::spec::{Diagnostic, DiagnosticSource, PairLink, SourceOffset, Span};
use crate::syntax::Resolved;
use crate::syntax::ast::{ContainerPair, ContentRange, Directive, Gaiji, NodeStore};

/// Configurable parser for Aozora source.
#[derive(Debug, Clone, Copy, Default)]
pub struct Parser {
    diagnostic_policy: DiagnosticPolicy,
}

impl Parser {
    /// Create a parser with the default diagnostic policy.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the diagnostic policy used by subsequent parses.
    #[must_use]
    pub fn diagnostic_policy(mut self, policy: DiagnosticPolicy) -> Self {
        self.diagnostic_policy = policy;
        self
    }

    /// Parse source into an editable document.
    #[must_use]
    pub fn parse(self, source: impl Into<Box<str>>) -> Document {
        Document::from_parts(source.into(), self.diagnostic_policy)
    }
}

/// A byte-range replacement against a document's current UTF-8 source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextEdit {
    range: Range<usize>,
    replacement: Box<str>,
}

impl TextEdit {
    /// Create an edit in old-source byte coordinates.
    #[must_use]
    pub fn new(range: Range<usize>, replacement: impl Into<Box<str>>) -> Self {
        Self {
            range,
            replacement: replacement.into(),
        }
    }

    /// The old-source byte range replaced by this edit.
    #[must_use]
    pub fn range(&self) -> Range<usize> {
        self.range.clone()
    }

    /// Replacement text.
    #[must_use]
    pub fn replacement(&self) -> &str {
        &self.replacement
    }
}

/// Failure to apply one or more text edits.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum EditError {
    /// The range starts after it ends.
    #[error("edit range starts after it ends")]
    InvertedRange,
    /// The range extends beyond the current source.
    #[error("edit range is outside the source")]
    OutOfBounds,
    /// A range endpoint is not a UTF-8 character boundary.
    #[error("edit range endpoint is not a UTF-8 character boundary")]
    NotCharBoundary,
    /// A batch is not sorted by start offset or contains overlapping ranges.
    #[error("edit batch is not sorted and disjoint")]
    UnsortedOrOverlapping,
}

/// Immutable, cheaply cloneable parsed view.
#[derive(Debug, Clone)]
pub struct Snapshot {
    source: Arc<str>,
    output: Arc<LexOutput>,
}

/// Diagnostic policy applied at parse time.
///
/// Diagnostics are always collected best-effort — the lexer never
/// aborts mid-stream — but the policy controls whether the
/// returned [`Snapshot::diagnostics`] slice retains every entry,
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
    /// [`DiagnosticSource::Internal`].
    /// Library bugs (the four legacy internal sanity checks) are
    /// hidden from the result; CLI / batch consumers that prefer a
    /// terser stream can opt in.
    DropInternal,
}

/// Builder for the [`Document::snapshot`] entry point.
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
    /// [`Document::snapshot`].
    pub fn build(self, source: impl Into<Box<str>>) -> Document {
        Document::from_parts(source.into(), self.diagnostic_policy)
    }
}

/// Single owning handle to a parsed Aozora source.
///
/// Owns the source buffer. [`Document::snapshot`] runs the owned, arena-free lex
/// pipeline and returns a [`Snapshot`] that owns all its AST data and borrows only
/// `&self`'s source.
pub struct Document {
    source: Arc<str>,
    diagnostic_policy: DiagnosticPolicy,
    output: Arc<LexOutput>,
}

impl Document {
    fn from_parts(source: Box<str>, diagnostic_policy: DiagnosticPolicy) -> Self {
        let mut output = lex(&source);
        if diagnostic_policy == DiagnosticPolicy::DropInternal {
            output
                .diagnostics
                .retain(|diagnostic| diagnostic.source() != DiagnosticSource::Internal);
        }
        Self {
            source: Arc::from(source),
            diagnostic_policy,
            output: Arc::new(output),
        }
    }

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

    /// Return an immutable parsed view of the current document state.
    #[must_use]
    pub fn snapshot(&self) -> Snapshot {
        Snapshot {
            source: Arc::clone(&self.source),
            output: Arc::clone(&self.output),
        }
    }

    /// Apply one edit atomically.
    ///
    /// # Errors
    ///
    /// Returns an [`EditError`] when the range is inverted, outside the source,
    /// or not aligned to UTF-8 character boundaries.
    pub fn apply_edit(&mut self, edit: TextEdit) -> Result<(), EditError> {
        self.apply_edits([edit])
    }

    /// Apply a sorted, disjoint batch atomically in old-source coordinates.
    ///
    /// # Errors
    ///
    /// Returns an [`EditError`] when any range is invalid or the batch is not
    /// sorted and disjoint. The document remains unchanged.
    pub fn apply_edits(
        &mut self,
        edits: impl IntoIterator<Item = TextEdit>,
    ) -> Result<(), EditError> {
        let edits: Vec<TextEdit> = edits.into_iter().collect();
        let mut previous_end = 0;
        for (index, edit) in edits.iter().enumerate() {
            if edit.range.start > edit.range.end {
                return Err(EditError::InvertedRange);
            }
            if edit.range.end > self.source.len() {
                return Err(EditError::OutOfBounds);
            }
            if !self.source.is_char_boundary(edit.range.start)
                || !self.source.is_char_boundary(edit.range.end)
            {
                return Err(EditError::NotCharBoundary);
            }
            if index != 0 && edit.range.start < previous_end {
                return Err(EditError::UnsortedOrOverlapping);
            }
            previous_end = edit.range.end;
        }

        let added = edits.iter().fold(0usize, |total, edit| {
            total.saturating_add(edit.replacement.len())
        });
        let removed = edits.iter().fold(0usize, |total, edit| {
            total.saturating_add(edit.range.end - edit.range.start)
        });
        let mut source = String::with_capacity(
            self.source
                .len()
                .saturating_sub(removed)
                .saturating_add(added),
        );
        let mut cursor = 0;
        for edit in &edits {
            source.push_str(&self.source[cursor..edit.range.start]);
            source.push_str(&edit.replacement);
            cursor = edit.range.end;
        }
        source.push_str(&self.source[cursor..]);
        *self = Self::from_parts(source.into_boxed_str(), self.diagnostic_policy);
        Ok(())
    }

    /// Apply an in-place text edit and return a fresh [`Document`].
    ///
    /// `span` is a byte range in the *current* source (`self.source`);
    /// `replacement` is the new text to splice in. On success the result is a
    /// new `Document` whose source equals
    /// `self.source[..span.start] + replacement + self.source[span.end..]`.
    /// `try_edit` intentionally does a full rebuild: `span` is a raw,
    /// untrusted byte range, so the arena is reparsed from the spliced
    /// source. Callers that want subtree reuse over the unchanged region
    /// take a [`Region`](crate::Region) through
    /// [`Self::edit_region`] / [`Snapshot::splice`](crate::Snapshot::splice).
    ///
    /// The signature is the supported entry point for editor surfaces
    /// implementing `textDocument/didChange`. Even with a full reparse
    /// inside, callers get a stable API today and a transparent
    /// upgrade path to subtree-aware reuse later. It is fallible (rather
    /// than panicking) so a bad span cannot tear down a host running under
    /// `panic = "abort"` — symmetric with [`Self::edit_region`].
    ///
    /// # Errors
    ///
    /// Returns [`SpliceError::InvalidEditSpan`](crate::SpliceError::InvalidEditSpan)
    /// if `span.start > span.end`, if `span.end > source.len()`, or if
    /// `span.start` / `span.end` does not lie on a UTF-8 codepoint boundary in
    /// `self.source`. On error the original document is unchanged.
    pub fn try_edit(&self, span: Span, replacement: &str) -> Result<Self, crate::SpliceError> {
        let start = span.start as usize;
        let end = span.end as usize;
        if start > end
            || end > self.source.len()
            || !self.source.is_char_boundary(start)
            || !self.source.is_char_boundary(end)
        {
            return Err(crate::SpliceError::InvalidEditSpan {
                span,
                source_len: self.source.len(),
            });
        }
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

        Ok(ParseOptions::new()
            .diagnostic_policy(self.diagnostic_policy)
            .build(new_source.into_boxed_str()))
    }

    /// Apply a node-aware minimal-diff edit and return a fresh [`Document`].
    ///
    /// `region` must come from this document's
    /// [`Snapshot::regions`](crate::Snapshot::regions) /
    /// [`Snapshot::region_at`](crate::Snapshot::region_at); `replacement`
    /// is the new source for the region's own bytes. Unlike [`Self::try_edit`] —
    /// which takes a raw byte span and trusts the caller — this routes through
    /// [`Snapshot::splice`](crate::Snapshot::splice): a
    /// [`Coupled`](crate::SpliceSafety::Coupled) region's partner (a forward
    /// reference's upstream literal, a container's matching close) is derived
    /// and the edit is verified by re-parse, so the result cannot silently
    /// desync.
    ///
    /// The returned document's source is the **sanitized**-then-spliced text:
    /// byte-identical to [`Self::try_edit`] on inputs that triggered no sanitize
    /// rewrite, and equal to `splice + Document::new` otherwise (a
    /// sanitized-coordinate region cannot be applied to un-sanitized bytes).
    ///
    /// # Errors
    ///
    /// Propagates [`SpliceError`](crate::SpliceError) from
    /// [`Snapshot::splice`](crate::Snapshot::splice): an unverifiable coupled edit or
    /// an opaque node kind. On error the original document is unchanged (the
    /// caller still owns `self`).
    pub fn edit_region(
        &self,
        region: crate::Region,
        replacement: &str,
    ) -> Result<Self, crate::SpliceError> {
        let spliced = self.snapshot().splice(region, replacement)?;
        Ok(ParseOptions::new()
            .diagnostic_policy(self.diagnostic_policy)
            .build(spliced))
    }
}

impl fmt::Debug for Document {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Document")
            .field("source_len", &self.source.len())
            .field("diagnostic_policy", &self.diagnostic_policy)
            .finish_non_exhaustive()
    }
}

impl Snapshot {
    pub(crate) fn node_store(&self) -> &NodeStore {
        &self.output.store
    }

    #[cfg(feature = "pandoc")]
    pub(crate) fn output(&self) -> &LexOutput {
        &self.output
    }

    /// Source text for this immutable view.
    #[must_use]
    pub fn source(&self) -> &str {
        &self.source
    }

    /// Diagnostics emitted while parsing.
    #[must_use]
    pub fn diagnostics(&self) -> &[Diagnostic] {
        &self.output.diagnostics
    }

    /// Resolved delimiter pairs in source byte coordinates.
    #[must_use]
    pub fn pairs(&self) -> &[PairLink] {
        &self.output.pairs
    }

    /// Classified nodes ordered by source span.
    #[must_use]
    pub fn source_nodes(&self) -> &[SourceNode] {
        &self.output.source_nodes
    }

    /// Resolve a directive's original source spelling.
    #[must_use]
    pub fn directive_source(&self, directive: Directive) -> &str {
        self.output.store.resolve_str(directive.raw)
    }

    /// Resolve a plain content range, returning `None` for nested content.
    #[must_use]
    pub fn plain_content(&self, range: ContentRange) -> Option<&str> {
        self.output.store.content_range_as_plain(range)
    }

    /// Resolve a gaiji node to its concrete glyph.
    #[must_use]
    pub fn resolve_gaiji(&self, gaiji: &Gaiji) -> Option<Resolved> {
        gaiji.resolve(&self.output.store)
    }

    /// Return the canonical mencode tail carried by a gaiji.
    ///
    /// # Panics
    ///
    /// Panics only if formatting into an owned `String` fails.
    #[must_use]
    pub fn gaiji_mencode(&self, gaiji: &Gaiji) -> Option<String> {
        gaiji.canonical.has_mencode().then(|| {
            let mut mencode = String::new();
            gaiji
                .canonical
                .write_mencode(&self.output.store, &mut mencode)
                .expect("writing to String cannot fail");
            mencode
        })
    }

    /// Find the classified node covering a source byte offset.
    #[must_use]
    pub fn node_at_source(&self, offset: SourceOffset) -> Option<&SourceNode> {
        self.output.node_at_source(offset)
    }

    /// Resolved container pairs.
    #[must_use]
    pub fn container_pairs(&self) -> &[ContainerPair] {
        &self.output.container_pairs
    }

    /// Sanitized text retained by the parser.
    #[must_use]
    pub fn sanitized(&self) -> &str {
        &self.output.sanitized
    }

    /// Render semantic HTML.
    #[must_use]
    pub fn to_html(&self) -> String {
        render_html(&self.output)
    }

    /// Render semantic HTML with explicit options.
    #[must_use]
    pub fn to_html_with(&self, options: RenderOptions) -> String {
        match options.directives {
            DirectiveNormalization::Off => self.to_html(),
            level => render_html_normalized(&self.output, level),
        }
    }

    /// Serialize the parsed document to Aozora source.
    #[must_use]
    pub fn to_source(&self) -> String {
        serialize(&self.output)
    }

    /// Serialize with explicit options.
    #[must_use]
    pub fn to_source_with(&self, options: SerializeOptions) -> String {
        serialize_with(&self.output, options)
    }

    /// Recover the sanitized parser input without canonical reserialization.
    #[must_use]
    pub fn to_source_verbatim(&self) -> String {
        self.output.sanitized.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::spec::PairKind;
    use crate::syntax::ast::NodeRef;

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
        let t = d.snapshot();
        assert_eq!(t.source(), s);
    }

    #[test]
    fn diagnostics_empty_for_clean_input() {
        let d = Document::new("plain");
        let t = d.snapshot();
        assert!(t.diagnostics().is_empty());
    }

    #[test]
    fn edit_batch_is_atomic() {
        let mut document = Parser::new().parse("aあz");
        let before = document.source().to_owned();
        let result =
            document.apply_edits([TextEdit::new(0..1, "A"), TextEdit::new(2..3, "invalid")]);
        assert_eq!(result, Err(EditError::NotCharBoundary));
        assert_eq!(document.source(), before);
    }

    #[test]
    fn edit_batch_uses_old_source_coordinates() {
        let mut document = crate::parse("alpha beta gamma");
        document
            .apply_edits([TextEdit::new(0..5, "A"), TextEdit::new(11..16, "G")])
            .expect("sorted disjoint edits");
        assert_eq!(document.source(), "A beta G");
    }

    #[test]
    fn snapshot_is_immutable_across_edits() {
        let mut document = crate::parse("plain");
        let snapshot = document.snapshot();
        document
            .apply_edit(TextEdit::new(0..5, "changed"))
            .expect("valid edit");
        assert_eq!(snapshot.source(), "plain");
        assert_eq!(document.snapshot().source(), "changed");
    }

    #[test]
    fn snapshot_is_send_and_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<Snapshot>();
    }

    #[test]
    fn diagnostics_populated_for_pua_collision() {
        let d = Document::new("contains \u{E001} sentinel");
        let t = d.snapshot();
        assert!(!t.diagnostics().is_empty());
    }

    #[test]
    fn edit_splices_source_at_span() {
        // Replace "world" with "Aozora" in "hello world!".
        let d = Document::new("hello world!");
        let span = Span::new(6, 11);
        let edited = d.try_edit(span, "Aozora").expect("in-bounds span");
        assert_eq!(edited.source(), "hello Aozora!");
    }

    #[test]
    fn edit_at_start_and_end_boundaries() {
        let d = Document::new("middle");
        // Insert at start (zero-length span at offset 0).
        let head = d
            .try_edit(Span::new(0, 0), "PRE-")
            .expect("zero-length span at 0");
        assert_eq!(head.source(), "PRE-middle");
        // Append at end (zero-length span at len()).
        let len = u32::try_from(d.source().len()).expect("test source fits u32");
        let tail = d
            .try_edit(Span::new(len, len), "-POST")
            .expect("zero-length span at len");
        assert_eq!(tail.source(), "middle-POST");
    }

    #[test]
    fn edit_equivalence_full_reparse() {
        // The edited document parses to the same Tree shape as
        // re-parsing the spliced source from scratch — this is the
        // observable property `Document::edit` ships under, which the
        // incremental engine preserves.
        let original = Document::new("｜青梅《おうめ》です。");
        // Replace 《おうめ》 with 《せいばい》.
        let span_start = original.source().find('《').expect("《 present");
        let span_end = original.source().find('》').expect("》 present") + '》'.len_utf8();
        let edited = original
            .try_edit(
                Span::new(
                    u32::try_from(span_start).expect("test span fits u32"),
                    u32::try_from(span_end).expect("test span fits u32"),
                ),
                "《せいばい》",
            )
            .expect("in-bounds span");

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
            edited.snapshot().to_source(),
            from_scratch.snapshot().to_source(),
            "edit() must be equivalent to splice + reparse"
        );
    }

    #[test]
    fn edit_rejects_inverted_span() {
        let err = Document::new("ok")
            .try_edit(Span::new(2, 1), "")
            .expect_err("inverted span must be rejected");
        assert!(matches!(err, crate::SpliceError::InvalidEditSpan { .. }));
    }

    #[test]
    fn edit_rejects_off_boundary_endpoints_independently() {
        // "あ" is a single 3-byte codepoint, so byte offsets 1 and 2 land
        // mid-sequence. Each endpoint's codepoint-boundary guard must reject
        // on its own: a start that lands off a boundary while the end is
        // valid, and an end that lands off a boundary while the start is
        // valid. Neither case triggers the ordering (`start > end`) or the
        // length (`end > len`) guard, so only the endpoint under test is at
        // fault — exercising each side of the boundary check separately.
        let d = Document::new("あ");

        // Start alone is off a boundary (end = len, itself a boundary).
        let bad_start = d
            .try_edit(Span::new(1, 3), "")
            .expect_err("off-boundary start must be rejected");
        assert!(matches!(
            bad_start,
            crate::SpliceError::InvalidEditSpan { .. }
        ));

        // End alone is off a boundary (start = 0, itself a boundary).
        let bad_end = d
            .try_edit(Span::new(0, 1), "")
            .expect_err("off-boundary end must be rejected");
        assert!(matches!(
            bad_end,
            crate::SpliceError::InvalidEditSpan { .. }
        ));
    }

    #[test]
    fn round_trip_through_serialize_is_a_fixed_point() {
        let s = "｜青梅《おうめ》";
        let first = Document::new(s).snapshot().to_source();
        let second = Document::new(first.clone()).snapshot().to_source();
        assert_eq!(first, second, "round-trip must be a fixed point");
    }

    #[test]
    fn pairs_records_simple_ruby() {
        // 《 … 》 produces one Ruby pair.
        let d = Document::new("｜青梅《おうめ》");
        let t = d.snapshot();
        let pairs = t.pairs();
        assert_eq!(pairs.len(), 1);
        let link = pairs[0];
        assert_eq!(link.kind, PairKind::Ruby);
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
        let t = d.snapshot();
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
        let t = d.snapshot();
        assert!(t.pairs().is_empty());
        assert!(!t.diagnostics().is_empty());
    }

    #[test]
    fn pairs_excludes_unmatched_close() {
        // Stray close on an empty stack.
        let d = Document::new("orphan］");
        let t = d.snapshot();
        assert!(t.pairs().is_empty());
    }

    #[test]
    fn node_at_source_finds_inline_ruby() {
        let src = "前｜青梅《おうめ》後";
        let d = Document::new(src);
        let t = d.snapshot();
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
        let t = d.snapshot();
        // Offset 0 is inside the leading "前" plain run — no node.
        assert!(t.node_at_source(SourceOffset::new(0)).is_none());
    }

    #[test]
    fn source_nodes_are_sorted_by_source_start() {
        let src = "｜青梅《おうめ》街道沿いに、※［＃「木＋吶のつくり」、第3水準1-85-54］";
        let d = Document::new(src);
        let t = d.snapshot();
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
        assert_eq!(
            via_new.snapshot().to_source(),
            via_options.snapshot().to_source()
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
            doc_collect.snapshot().diagnostics().len(),
            doc_drop.snapshot().diagnostics().len(),
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
    /// surfaces. `sanitize` is reached through the internal lex
    /// pipeline directly (same path `cst::from_tree` uses).
    fn assert_verbatim_equals_sanitize(doc: &str) {
        use crate::pipeline::lexer::sanitize::sanitize;
        let expected = sanitize(doc).text;
        let d = Document::new(doc);
        let t = d.snapshot();
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
            t_doc.snapshot().to_source_verbatim(),
            doc,
            "verbatim must NOT equal the raw doc once a BOM was stripped"
        );
        assert!(
            !t_doc
                .snapshot()
                .to_source_verbatim()
                .starts_with('\u{FEFF}'),
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
                .snapshot()
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
        let recovered = Document::new(doc).snapshot().to_source_verbatim();
        assert!(
            recovered.contains('\u{FFFD}') && !recovered.contains('\u{E001}'),
            "raw PUA sentinels must come back as U+FFFD"
        );
        assert_ne!(recovered, doc, "PUA neutralization is irreversible");
    }

    #[test]
    fn document_debug_pins_source_len_and_policy() {
        // The `Debug` impl writes `source_len` (byte length) and the
        // policy verbatim. `Ok(Default::default())` for the whole impl
        // would emit the empty string; the `diagnostic_policy` builder
        // returning `Default::default()` would flip `DropInternal` back
        // to the default `CollectAll`. Pin both strings exactly.
        let collect = Document::new("hello");
        assert_eq!(
            format!("{collect:?}"),
            "Document { source_len: 5, diagnostic_policy: CollectAll, .. }",
        );
        let dropped = Document::options()
            .diagnostic_policy(DiagnosticPolicy::DropInternal)
            .build("hi");
        assert_eq!(
            format!("{dropped:?}"),
            "Document { source_len: 2, diagnostic_policy: DropInternal, .. }",
        );
    }

    #[test]
    fn drop_internal_retains_source_origin_diagnostics() {
        // A raw `U+E001` sentinel makes sanitize emit exactly one
        // `SourceContainsPua` diagnostic whose origin is
        // `DiagnosticSource::Source`. `DropInternal` filters *only*
        // `Internal`-origin entries, so this one must survive the
        // `retain`. Flipping the predicate `!= Internal` → `== Internal`
        // would drop it (empty diagnostics), which this pins against.
        let doc = Document::options()
            .diagnostic_policy(DiagnosticPolicy::DropInternal)
            .build("contains \u{E001} sentinel");
        let t = doc.snapshot();
        let diags = t.diagnostics();
        assert_eq!(
            diags.len(),
            1,
            "the Source-origin PUA diagnostic survives DropInternal",
        );
        assert_eq!(
            diags[0].source(),
            DiagnosticSource::Source,
            "retained diagnostic is Source-origin, not Internal",
        );
    }

    #[test]
    fn container_pairs_records_block_container() {
        // A balanced ［＃ここから…］ / ［＃ここで…終わり］ pair yields one
        // container pair entry. `Vec::leak(Vec::new())` would return an
        // empty slice, so pin the non-empty length and the open→close
        // ordering.
        let d = Document::new(
            "序\n［＃ここから２字下げ］\n本文の段落。\n［＃ここで字下げ終わり］\n了\n",
        );
        let t = d.snapshot();
        let pairs = t.container_pairs();
        assert_eq!(pairs.len(), 1, "one balanced container pair");
        assert!(
            pairs[0].open.0 < pairs[0].close.0,
            "container open offset precedes its close offset",
        );
    }
}
