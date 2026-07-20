//! `Document` owns an editable Aozora source buffer. `Snapshot` is an
//! immutable, cheaply cloned view of one parsed document version.
//!
//! [`Document::snapshot`] shares the immutable source and parsed state through
//! reference-counted storage. A snapshot remains valid after later edits and
//! is `Send + Sync`.
//!
//! The owned AST stores interned strings and node payloads in a flat
//! `NodeStore` (the owned `StrInterner` deduplicates repeated string content);
//! dropping the tree frees them in one step, with no per-node `Drop`.

use core::cmp::Ordering;
use core::fmt;
use core::ops::Range;
use core::slice;
use std::sync::{Arc, OnceLock};

#[cfg(not(target_arch = "wasm32"))]
use ropey::Rope;

use crate::encoding::gaiji::{GaijiResolution, gaiji_resolutions};
use crate::incremental::reparse;
use crate::pipeline::lexer::sanitize;
use crate::pipeline::{LexOutput, SanitizedText, SourceNode, lex_shared};
use crate::render::{
    DirectiveNormalization, RenderOptions, SerializeOptions, render_html, render_html_normalized,
    requires_verbatim_recovery, serialize, serialize_with,
};
use crate::spec::{Diagnostic, PairLink, SourceOffset};
use crate::splice::Coupling;
use crate::syntax::ast::{ContainerPair as AstContainerPair, Node, NodeRef, NodeStore};
use crate::syntax::{DirectiveKind, NodeKind, RegionFormat, RubySide};

/// Configurable parser for Aozora source.
#[derive(Debug, Clone, Copy)]
pub struct Parser {
    max_source_bytes: usize,
}

impl Default for Parser {
    fn default() -> Self {
        Self {
            max_source_bytes: usize::try_from(u32::MAX).unwrap_or(usize::MAX),
        }
    }
}

impl Parser {
    /// Create a parser.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Parse source into an editable document.
    ///
    /// # Errors
    ///
    /// Returns [`ParseError::SourceTooLarge`] when the UTF-8 source cannot be
    /// represented by the parser's `u32` spans.
    pub fn parse(self, source: impl Into<Arc<str>>) -> Result<Document, ParseError> {
        let source = source.into();
        if source.len() > self.max_source_bytes {
            return Err(ParseError::SourceTooLarge { len: source.len() });
        }
        Ok(Document::from_parts(source))
    }
}

/// Failure to accept parser input.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum ParseError {
    /// The UTF-8 source is too large for `u32` byte spans.
    #[error("source is {len} bytes; the parser limit is u32::MAX")]
    SourceTooLarge {
        /// Rejected UTF-8 byte length.
        len: usize,
    },
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
    /// The edited UTF-8 source no longer fits parser spans.
    #[error("edited source exceeds the parser span limit")]
    SourceTooLarge,
    /// The requested semantic edit could not be proven coherent.
    #[error("semantic edit could not be verified")]
    Unverifiable,
}

#[derive(Debug, Clone)]
struct CoordinateEdit {
    sanitized: Range<usize>,
    source: Range<usize>,
}

#[derive(Debug)]
struct CoordinateMap {
    edits: Arc<[CoordinateEdit]>,
    sanitized_len: usize,
    source_len: usize,
}

struct CoordinateMapBuilder<'a> {
    source: &'a str,
    sanitized: &'a str,
    edits: Vec<CoordinateEdit>,
    source_offset: usize,
    sanitized_offset: usize,
    next_accent: Option<usize>,
}

impl<'a> CoordinateMapBuilder<'a> {
    fn new(source: &'a str, sanitized: &'a str) -> Self {
        let mut source_offset = 0;
        while source[source_offset..].starts_with('\u{FEFF}') {
            source_offset += '\u{FEFF}'.len_utf8();
        }
        let edits = (source_offset != 0)
            .then_some(CoordinateEdit {
                sanitized: 0..0,
                source: 0..source_offset,
            })
            .into_iter()
            .collect();
        Self {
            source,
            sanitized,
            edits,
            source_offset,
            sanitized_offset: 0,
            next_accent: source.find('〔'),
        }
    }

    fn build(mut self) -> Vec<CoordinateEdit> {
        while self.source_offset < self.source.len() && self.sanitized_offset < self.sanitized.len()
        {
            let source_offset = self.source_offset;
            let sanitized_offset = self.sanitized_offset;
            if !self.consume_accent() {
                self.consume_common();
                if self.source_offset == source_offset {
                    self.consume_difference();
                }
            }
            assert_ne!(
                (self.source_offset, self.sanitized_offset),
                (source_offset, sanitized_offset),
                "coordinate mapping must advance"
            );
        }
        if self.source_offset != self.source.len() || self.sanitized_offset != self.sanitized.len()
        {
            self.edits.push(CoordinateEdit {
                sanitized: self.sanitized_offset..self.sanitized.len(),
                source: self.source_offset..self.source.len(),
            });
        }
        self.edits
    }

    #[inline]
    fn consume_accent(&mut self) -> bool {
        if self.next_accent != Some(self.source_offset) {
            return false;
        }
        if self.sanitized[self.sanitized_offset..].starts_with('〔') {
            let source_body = self.source_offset + '〔'.len_utf8();
            let sanitized_body = self.sanitized_offset + '〔'.len_utf8();
            let source_close = self.source[source_body..]
                .find('〕')
                .map(|at| source_body + at);
            let sanitized_close = self.sanitized[sanitized_body..]
                .find('〕')
                .map(|at| sanitized_body + at);
            if let (Some(source_close), Some(sanitized_close)) = (source_close, sanitized_close) {
                if self.source[source_body..source_close]
                    != self.sanitized[sanitized_body..sanitized_close]
                {
                    self.edits.push(CoordinateEdit {
                        sanitized: sanitized_body..sanitized_close,
                        source: source_body..source_close,
                    });
                }
                self.source_offset = source_close;
                self.sanitized_offset = sanitized_close;
                self.next_accent = self.find_next_accent(source_close);
                return true;
            }
        }
        self.next_accent = self.find_next_accent(self.source_offset);
        false
    }

    #[inline]
    fn find_next_accent(&self, after: usize) -> Option<usize> {
        let start = after + '〕'.len_utf8();
        self.source.get(start..)?.find('〔').map(|at| start + at)
    }

    #[inline]
    fn consume_common(&mut self) {
        let mut common = self.source.as_bytes()[self.source_offset..]
            .iter()
            .zip(&self.sanitized.as_bytes()[self.sanitized_offset..])
            .take_while(|(left, right)| left == right)
            .count();
        if let Some(accent) = self.next_accent {
            common = common.min(accent.saturating_sub(self.source_offset));
        }
        let mut boundary = common;
        while boundary != 0
            && !self.source.is_char_boundary(
                self.source_offset
                    .checked_add(boundary)
                    .expect("source boundary"),
            )
        {
            boundary = boundary.checked_sub(1).expect("positive boundary");
        }
        self.source_offset = self
            .source_offset
            .checked_add(boundary)
            .expect("source offset");
        self.sanitized_offset = self
            .sanitized_offset
            .checked_add(boundary)
            .expect("sanitized offset");
        debug_assert!(
            self.sanitized.is_char_boundary(self.sanitized_offset),
            "sanitized offset must remain on a UTF-8 boundary"
        );
    }

    #[inline]
    fn consume_difference(&mut self) {
        let source_char = self.source[self.source_offset..]
            .chars()
            .next()
            .expect("source offset is in bounds");
        let sanitized_char = self.sanitized[self.sanitized_offset..]
            .chars()
            .next()
            .expect("sanitized offset is in bounds");
        if source_char == sanitized_char {
            self.source_offset = self
                .source_offset
                .checked_add(source_char.len_utf8())
                .expect("source offset");
            self.sanitized_offset = self
                .sanitized_offset
                .checked_add(sanitized_char.len_utf8())
                .expect("sanitized offset");
        } else if source_char == '\r' && sanitized_char == '\n' {
            self.consume_carriage_return();
        } else if matches!(
            source_char,
            '\u{E001}' | '\u{E002}' | '\u{E003}' | '\u{E004}'
        ) && sanitized_char == '\u{FFFD}'
        {
            self.source_offset = self
                .source_offset
                .checked_add(source_char.len_utf8())
                .expect("source offset");
            self.sanitized_offset = self
                .sanitized_offset
                .checked_add(sanitized_char.len_utf8())
                .expect("sanitized offset");
        } else if sanitized_char == '\n' {
            let sanitized_end = self
                .sanitized_offset
                .checked_add(1)
                .expect("sanitized offset");
            self.edits.push(CoordinateEdit {
                sanitized: self.sanitized_offset..sanitized_end,
                source: self.source_offset..self.source_offset,
            });
            self.sanitized_offset = sanitized_end;
        } else {
            self.edits.push(CoordinateEdit {
                sanitized: self.sanitized_offset..self.sanitized.len(),
                source: self.source_offset..self.source.len(),
            });
            self.source_offset = self.source.len();
            self.sanitized_offset = self.sanitized.len();
        }
    }

    #[inline]
    fn consume_carriage_return(&mut self) {
        let source_end = self
            .source_offset
            .checked_add(if self.source[self.source_offset..].starts_with("\r\n") {
                2
            } else {
                1
            })
            .expect("source offset");
        let sanitized_end = self
            .sanitized_offset
            .checked_add(1)
            .expect("sanitized offset");
        self.edits.push(CoordinateEdit {
            sanitized: self.sanitized_offset..sanitized_end,
            source: self.source_offset..source_end,
        });
        self.source_offset = source_end;
        self.sanitized_offset = sanitized_end;
    }
}

impl CoordinateMap {
    fn new(source: &str, sanitized: &str, source_unchanged: bool) -> Self {
        if source_unchanged {
            return Self {
                edits: Arc::from([]),
                sanitized_len: sanitized.len(),
                source_len: source.len(),
            };
        }
        Self {
            edits: CoordinateMapBuilder::new(source, sanitized).build().into(),
            sanitized_len: sanitized.len(),
            source_len: source.len(),
        }
    }

    fn span_to_source(&self, span: crate::Span) -> crate::Span {
        crate::Span::new(
            self.sanitized_to_source(span.start as usize, false),
            self.sanitized_to_source(span.end as usize, true),
        )
    }

    fn sanitized_to_source(&self, offset: usize, end_bias: bool) -> u32 {
        let offset = offset.min(self.sanitized_len);
        let mut sanitized_cursor = 0usize;
        let mut source_cursor = 0usize;
        for edit in self.edits.iter() {
            if offset < edit.sanitized.start {
                let mapped = offset
                    .checked_sub(sanitized_cursor)
                    .and_then(|delta| source_cursor.checked_add(delta))
                    .expect("coordinate edit ordering");
                return u32::try_from(mapped).expect("source fits parser spans");
            }
            if edit.sanitized.is_empty() && offset == edit.sanitized.start {
                sanitized_cursor = edit.sanitized.end;
                source_cursor = edit.source.end;
                continue;
            }
            if offset == edit.sanitized.start {
                return u32::try_from(edit.source.start).expect("source fits parser spans");
            }
            if offset < edit.sanitized.end {
                let mapped = if end_bias {
                    edit.source.end
                } else {
                    edit.source.start
                };
                return u32::try_from(mapped).expect("source fits parser spans");
            }
            if offset == edit.sanitized.end {
                return u32::try_from(edit.source.end).expect("source fits parser spans");
            }
            sanitized_cursor = edit.sanitized.end;
            source_cursor = edit.source.end;
        }
        let mapped = offset
            .checked_sub(sanitized_cursor)
            .and_then(|delta| source_cursor.checked_add(delta))
            .expect("coordinate edit ordering");
        u32::try_from(mapped).expect("source fits parser spans")
    }

    fn source_to_sanitized(&self, offset: usize) -> Option<SourceOffset> {
        if offset > self.source_len {
            return None;
        }
        let mut sanitized_cursor = 0usize;
        let mut source_cursor = 0usize;
        for edit in self.edits.iter() {
            if offset.cmp(&edit.source.start).is_lt() {
                let mapped = offset
                    .checked_sub(source_cursor)?
                    .checked_add(sanitized_cursor)?;
                return u32::try_from(mapped).ok().map(SourceOffset::new);
            }
            match offset.cmp(&edit.source.end) {
                Ordering::Less => {
                    return u32::try_from(edit.sanitized.start)
                        .ok()
                        .map(SourceOffset::new);
                }
                Ordering::Equal => {
                    return u32::try_from(edit.sanitized.end)
                        .ok()
                        .map(SourceOffset::new);
                }
                Ordering::Greater => {}
            }
            sanitized_cursor = edit.sanitized.end;
            source_cursor = edit.source.end;
        }
        let mapped = offset
            .checked_sub(source_cursor)?
            .checked_add(sanitized_cursor)?;
        u32::try_from(mapped).ok().map(SourceOffset::new)
    }
}

/// Stable projection of a parsed node.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NodeView {
    kind: NodeKind,
    span: crate::Span,
}

impl NodeView {
    /// Semantic node kind.
    #[must_use]
    pub const fn kind(self) -> NodeKind {
        self.kind
    }

    /// Half-open UTF-8 byte span in source coordinates.
    #[must_use]
    pub const fn span(self) -> crate::Span {
        self.span
    }
}

/// Stable category for notation-shaped text with no parser semantics.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum LiteralMarkupKind {
    /// A non-empty `《…》` sequence that is not a ruby annotation.
    RubyDelimiters,
    /// A `｜` that does not introduce an explicit ruby base.
    RubyBaseMarker,
    /// A `［＃` opener that is not a parsed directive or gaiji.
    DirectiveMarker,
}

/// Stable projection of notation-shaped text preserved literally.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LiteralMarkupView {
    kind: LiteralMarkupKind,
    span: crate::Span,
}

impl LiteralMarkupView {
    /// Literal marker category.
    #[must_use]
    pub const fn kind(self) -> LiteralMarkupKind {
        self.kind
    }

    /// Half-open UTF-8 byte span of the marker in source coordinates.
    #[must_use]
    pub const fn span(self) -> crate::Span {
        self.span
    }
}

/// Stable classification for a literal or editorial directive.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum DirectiveClass {
    /// A known non-canonical spelling.
    NonCanonical,
    /// An editorial annotation with no parser semantics.
    Editorial,
    /// Sic marker.
    Sic,
    /// Base-text variant.
    BaseTextVariant,
    /// Split-annotation opener.
    WarichuOpen,
    /// Split-annotation closer.
    WarichuClose,
    /// Empty directive.
    Empty,
    /// Editor note.
    EditorNote,
    /// Ruby attached to a preceding base.
    RubyAttached,
    /// Ruby retargeting.
    RubyRetarget,
    /// Paired-ruby opener.
    RubyPairOpen,
    /// Paired-ruby closer.
    RubyPairClose,
    /// Paired margin-note opener.
    MarginNotePairOpen,
    /// Paired margin-note closer.
    MarginNotePairClose,
}

impl From<DirectiveKind> for DirectiveClass {
    fn from(value: DirectiveKind) -> Self {
        match value {
            DirectiveKind::NonCanonical => Self::NonCanonical,
            DirectiveKind::Editorial => Self::Editorial,
            DirectiveKind::Sic => Self::Sic,
            DirectiveKind::BaseTextVariant => Self::BaseTextVariant,
            DirectiveKind::WarichuOpen => Self::WarichuOpen,
            DirectiveKind::WarichuClose => Self::WarichuClose,
            DirectiveKind::Empty => Self::Empty,
            DirectiveKind::EditorNote => Self::EditorNote,
            DirectiveKind::RubyAttached => Self::RubyAttached,
            DirectiveKind::RubyRetarget => Self::RubyRetarget,
            DirectiveKind::RubyPairOpen => Self::RubyPairOpen,
            DirectiveKind::RubyPairClose => Self::RubyPairClose,
            DirectiveKind::MarginNotePairOpen => Self::MarginNotePairOpen,
            DirectiveKind::MarginNotePairClose => Self::MarginNotePairClose,
        }
    }
}

/// Borrowed view of a parsed literal or editorial directive.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DirectiveView<'a> {
    kind: DirectiveClass,
    span: crate::Span,
    source: &'a str,
}

impl<'a> DirectiveView<'a> {
    /// Directive classification.
    #[must_use]
    pub const fn kind(self) -> DirectiveClass {
        self.kind
    }

    /// Source byte span.
    #[must_use]
    pub const fn span(self) -> crate::Span {
        self.span
    }

    /// Original directive spelling.
    #[must_use]
    pub const fn source(self) -> &'a str {
        self.source
    }
}

/// Borrowed projection of a ruby annotation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RubyView<'a> {
    span: crate::Span,
    base: Option<&'a str>,
    reading: Option<&'a str>,
    side: RubySide,
}

impl<'a> RubyView<'a> {
    /// Complete annotation span.
    #[must_use]
    pub const fn span(self) -> crate::Span {
        self.span
    }

    /// Plain base text, or `None` when the base contains nested notation.
    #[must_use]
    pub const fn base(self) -> Option<&'a str> {
        self.base
    }

    /// Plain reading text, or `None` when the reading contains nested notation.
    #[must_use]
    pub const fn reading(self) -> Option<&'a str> {
        self.reading
    }

    /// Reading side.
    #[must_use]
    pub const fn side(self) -> RubySide {
        self.side
    }
}

/// Stable family tag for a paired block container.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ContainerKind {
    /// Indentation.
    Indent,
    /// Split annotation.
    Warichu,
    /// Framed content.
    Framed,
    /// Right-edge alignment.
    AlignEnd,
    /// Fixed line width.
    LineWidth,
    /// Emphasis-dot range.
    BoutenRange,
    /// Bold text.
    Bold,
    /// Gothic text.
    Gothic,
    /// Italic text.
    Italic,
    /// Heading.
    Heading,
    /// Columns.
    Columns,
    /// Table.
    Table,
    /// Horizontal writing.
    Horizontal,
    /// Font-size shift.
    FontSize,
    /// Small script.
    SmallScript,
    /// Caption.
    Caption,
}

impl ContainerKind {
    /// Every container family in wire order.
    pub const ALL: [Self; 16] = [
        Self::Indent,
        Self::Warichu,
        Self::Framed,
        Self::AlignEnd,
        Self::LineWidth,
        Self::BoutenRange,
        Self::Bold,
        Self::Gothic,
        Self::Italic,
        Self::Heading,
        Self::Columns,
        Self::Table,
        Self::Horizontal,
        Self::FontSize,
        Self::SmallScript,
        Self::Caption,
    ];

    /// Stable camel-case wire tag.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Indent => "indent",
            Self::Warichu => "warichu",
            Self::Framed => "framed",
            Self::AlignEnd => "alignEnd",
            Self::LineWidth => "lineWidth",
            Self::BoutenRange => "boutenRange",
            Self::Bold => "bold",
            Self::Gothic => "gothic",
            Self::Italic => "italic",
            Self::Heading => "heading",
            Self::Columns => "columns",
            Self::Table => "table",
            Self::Horizontal => "horizontal",
            Self::FontSize => "fontSize",
            Self::SmallScript => "smallScript",
            Self::Caption => "caption",
        }
    }
}

impl From<RegionFormat> for ContainerKind {
    fn from(value: RegionFormat) -> Self {
        match value {
            RegionFormat::Indent(_) => Self::Indent,
            RegionFormat::Warichu => Self::Warichu,
            RegionFormat::Framed(_) => Self::Framed,
            RegionFormat::AlignEnd { .. } => Self::AlignEnd,
            RegionFormat::LineWidth(_) => Self::LineWidth,
            RegionFormat::Bouten { .. } => Self::BoutenRange,
            RegionFormat::Bold { .. } => Self::Bold,
            RegionFormat::Gothic { .. } => Self::Gothic,
            RegionFormat::Italic { .. } => Self::Italic,
            RegionFormat::Heading { .. } => Self::Heading,
            RegionFormat::Columns(_) => Self::Columns,
            RegionFormat::Table => Self::Table,
            RegionFormat::Horizontal => Self::Horizontal,
            RegionFormat::FontSize(_) => Self::FontSize,
            RegionFormat::SmallScript(_) => Self::SmallScript,
            RegionFormat::Caption { .. } => Self::Caption,
        }
    }
}

/// Paired container markers in source coordinates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ContainerPair {
    kind: ContainerKind,
    open: crate::Span,
    close: crate::Span,
}

impl ContainerPair {
    /// Container family.
    #[must_use]
    pub const fn kind(self) -> ContainerKind {
        self.kind
    }

    /// Opening marker span.
    #[must_use]
    pub const fn open(self) -> crate::Span {
        self.open
    }

    /// Closing marker span.
    #[must_use]
    pub const fn close(self) -> crate::Span {
        self.close
    }
}

/// Immutable, cheaply cloneable parsed view.
#[derive(Debug, Clone)]
pub struct Snapshot {
    state: Arc<SnapshotState>,
}

#[derive(Debug)]
struct SnapshotState {
    source: Arc<str>,
    #[cfg(not(target_arch = "wasm32"))]
    rope: Option<Rope>,
    output: LexOutput,
    cache: SnapshotCache,
}

impl SnapshotState {
    fn source(&self) -> &str {
        &self.source
    }
}

#[derive(Debug, Default)]
struct ProjectionCache {
    diagnostics: OnceLock<Arc<[Diagnostic]>>,
    gaiji_resolutions: OnceLock<Arc<[GaijiResolution]>>,
    pairs: OnceLock<Arc<[PairLink]>>,
    nodes: OnceLock<Arc<[NodeView]>>,
    literal_markup: OnceLock<Arc<[LiteralMarkupView]>>,
    container_pairs: OnceLock<Arc<[ContainerPair]>>,
}

#[derive(Debug, Default)]
struct SnapshotCache {
    coordinate_map: OnceLock<CoordinateMap>,
    projections: ProjectionCache,
    paragraphs: OnceLock<Arc<[Arc<ParagraphSnapshot>]>>,
}

#[derive(Debug)]
struct ParagraphSnapshot {
    source: Arc<str>,
}

/// Single owning handle to a parsed Aozora source.
///
/// Owns the source buffer and parsed state. [`Document::snapshot`] shares the
/// immutable state with a [`Snapshot`] through reference-counted storage.
pub struct Document {
    state: Arc<SnapshotState>,
}

impl Document {
    fn from_parts(source: Arc<str>) -> Self {
        let output = lex_shared(Arc::clone(&source));
        #[cfg(not(target_arch = "wasm32"))]
        {
            Self::from_output(source, None, output, SnapshotCache::default())
        }
        #[cfg(target_arch = "wasm32")]
        {
            Self::from_output(source, output, SnapshotCache::default())
        }
    }

    fn from_output(
        source: Arc<str>,
        #[cfg(not(target_arch = "wasm32"))] rope: Option<Rope>,
        output: LexOutput,
        cache: SnapshotCache,
    ) -> Self {
        Self {
            state: Arc::new(SnapshotState {
                source,
                #[cfg(not(target_arch = "wasm32"))]
                rope,
                output,
                cache,
            }),
        }
    }

    pub(crate) fn new(source: impl Into<Arc<str>>) -> Self {
        Parser::new()
            .parse(source)
            .expect("internal source must fit parser spans")
    }

    /// The source text owned by this document.
    #[must_use]
    pub fn source(&self) -> &str {
        self.state.source()
    }

    /// Return an immutable parsed view of the current document state.
    #[must_use]
    pub fn snapshot(&self) -> Snapshot {
        Snapshot {
            state: Arc::clone(&self.state),
        }
    }

    /// Apply a sorted, disjoint batch atomically in old-source coordinates.
    ///
    /// # Errors
    ///
    /// Returns an [`EditError`] when any range is invalid or the batch is not
    /// sorted and disjoint. The document remains unchanged.
    pub fn edit(&mut self, edits: impl IntoIterator<Item = TextEdit>) -> Result<(), EditError> {
        let edits: Vec<TextEdit> = edits.into_iter().collect();
        self.validate_edits(&edits)?;
        if edits.is_empty() {
            return Ok(());
        }
        let mut next = Self {
            state: Arc::clone(&self.state),
        };
        for edit in edits.iter().rev() {
            next.edit_one(edit);
        }
        debug_assert_eq!(
            next.source(),
            apply_edits_to_source(self.state.source(), &edits),
            "incremental batch and source splice must agree"
        );
        *self = next;
        Ok(())
    }

    fn edit_one(&mut self, edit: &TextEdit) {
        #[cfg(not(target_arch = "wasm32"))]
        let rope = self.apply_edits(slice::from_ref(edit));
        #[cfg(not(target_arch = "wasm32"))]
        debug_assert_eq!(
            rope.to_string(),
            apply_edits_to_source(self.state.source(), slice::from_ref(edit)),
            "rope and source edit application must agree"
        );
        let ParsedEdit { mut source, output } = self.parse_after_edit(edit);
        let cache = if self.state.cache.paragraphs.get().is_some() {
            let contiguous = source.get_or_insert_with(|| {
                #[cfg(not(target_arch = "wasm32"))]
                {
                    Arc::from(rope.to_string())
                }
                #[cfg(target_arch = "wasm32")]
                unreachable!("wasm edits retain contiguous source")
            });
            self.cache_after_edit(contiguous)
        } else {
            SnapshotCache::default()
        };
        #[cfg(not(target_arch = "wasm32"))]
        let source = source.unwrap_or_else(|| Arc::from(rope.to_string()));
        #[cfg(not(target_arch = "wasm32"))]
        let next = Self::from_output(source, Some(rope), output, cache);
        #[cfg(target_arch = "wasm32")]
        let next = Self::from_output(
            source.expect("wasm edits retain contiguous source"),
            output,
            cache,
        );
        *self = next;
    }

    fn validate_edits(&self, edits: &[TextEdit]) -> Result<(), EditError> {
        let mut previous_end = 0;
        for (index, edit) in edits.iter().enumerate() {
            if edit.range.start > edit.range.end {
                return Err(EditError::InvertedRange);
            }
            if edit.range.end > self.state.source().len() {
                return Err(EditError::OutOfBounds);
            }
            if !self.state.source().is_char_boundary(edit.range.start)
                || !self.state.source().is_char_boundary(edit.range.end)
            {
                return Err(EditError::NotCharBoundary);
            }
            if index != 0 && edit.range.start < previous_end {
                return Err(EditError::UnsortedOrOverlapping);
            }
            previous_end = edit.range.end;
        }
        let capacity = replacement_capacity(self.state.source().len(), edits);
        if u32::try_from(capacity).is_err() {
            return Err(EditError::SourceTooLarge);
        }
        Ok(())
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn apply_edits(&self, edits: &[TextEdit]) -> Rope {
        let mut rope = self
            .state
            .rope
            .clone()
            .unwrap_or_else(|| Rope::from(self.state.source()));
        for edit in edits.iter().rev() {
            let start = rope.byte_to_char(edit.range.start);
            let end = rope.byte_to_char(edit.range.end);
            if start != end {
                rope.remove(start..end);
            }
            if !edit.replacement.is_empty() {
                rope.insert(start, &edit.replacement);
            }
        }
        rope
    }

    fn parse_after_edit(&self, edit: &TextEdit) -> ParsedEdit {
        let map = self.state.cache.coordinate_map.get_or_init(|| {
            CoordinateMap::new(
                self.state.source(),
                &self.state.output.sanitized,
                self.state.output.source_unchanged,
            )
        });
        if let Some(sanitized) = self.sanitize_after_edit(edit, map)
            && diagnostics_allow_incremental(&self.state.output.diagnostics)
            && let Some(incremental) = reparse(
                &self.state.output,
                sanitized.text,
                sanitized.edit_range,
                sanitized.source_unchanged,
            )
        {
            return ParsedEdit {
                source: sanitized.source,
                output: incremental.output,
            };
        }
        let source = self.edited_source(edit);
        ParsedEdit {
            output: lex_shared(Arc::clone(&source)),
            source: Some(source),
        }
    }

    fn sanitize_after_edit(
        &self,
        edit: &TextEdit,
        map: &CoordinateMap,
    ) -> Option<IncrementalSanitized> {
        if edit.replacement.contains('〔') || edit.replacement.contains('〕') {
            return None;
        }
        let source = self.state.source();
        let source_region = source_sanitization_region(source, &edit.range);
        let region = map
            .source_to_sanitized(source_region.start)
            .zip(map.source_to_sanitized(source_region.end))
            .map(|(start, end)| start.get() as usize..end.get() as usize)?;
        let fragment = edited_fragment(source, edit, source_region.clone())?;
        let source_start = source_region.start;
        if source_start != 0 && fragment.starts_with('\u{FEFF}') {
            return None;
        }
        let sanitized = sanitize(&fragment);
        if !sanitized.diagnostics.is_empty() {
            return None;
        }
        let source_unchanged = self.state.output.source_unchanged && sanitized.source_unchanged;
        if source_unchanged {
            let source = self.edited_source(edit);
            return Some(IncrementalSanitized {
                text: SanitizedText::shared(Arc::clone(&source)),
                source: Some(source),
                source_unchanged,
                edit_range: region,
            });
        }
        let text = splice_sanitized(
            &self.state.output.sanitized,
            region.clone(),
            &sanitized.text,
        )?;
        Some(IncrementalSanitized {
            text: SanitizedText::owned(text),
            source: {
                #[cfg(not(target_arch = "wasm32"))]
                {
                    None
                }
                #[cfg(target_arch = "wasm32")]
                {
                    Some(self.edited_source(edit))
                }
            },
            source_unchanged,
            edit_range: region,
        })
    }

    fn edited_source(&self, edit: &TextEdit) -> Arc<str> {
        Arc::from(apply_edits_to_source(
            self.state.source(),
            slice::from_ref(edit),
        ))
    }

    fn cache_after_edit(&self, source: &str) -> SnapshotCache {
        let paragraphs = self
            .state
            .cache
            .paragraphs
            .get()
            .map(|prior| OnceLock::from(project_paragraphs(source, Some(prior))))
            .unwrap_or_default();
        SnapshotCache {
            paragraphs,
            ..SnapshotCache::default()
        }
    }
}

struct ParsedEdit {
    source: Option<Arc<str>>,
    output: LexOutput,
}

struct IncrementalSanitized {
    text: SanitizedText,
    source: Option<Arc<str>>,
    source_unchanged: bool,
    edit_range: Range<usize>,
}

fn diagnostics_allow_incremental(diagnostics: &[Diagnostic]) -> bool {
    diagnostics.iter().all(|diagnostic| {
        matches!(
            diagnostic,
            Diagnostic::UnresolvedGaiji { .. } | Diagnostic::NonCanonicalDirective { .. }
        )
    })
}

fn source_sanitization_region(source: &str, edit: &Range<usize>) -> Range<usize> {
    let start = ["\r\n\r\n", "\n\n", "\r\r"]
        .into_iter()
        .filter_map(|boundary| {
            source
                .get(..edit.start)?
                .rfind(boundary)
                .map(|offset| offset + boundary.len())
        })
        .max()
        .unwrap_or(0);
    let end = ["\r\n\r\n", "\n\n", "\r\r"]
        .into_iter()
        .filter_map(|boundary| {
            source
                .get(edit.end..)?
                .find(boundary)
                .map(|offset| edit.end + offset + boundary.len())
        })
        .min()
        .unwrap_or(source.len());
    start..end
}

fn edited_fragment(source: &str, edit: &TextEdit, region: Range<usize>) -> Option<String> {
    let prefix = source.get(region.start..edit.range.start)?;
    let suffix = source.get(edit.range.end..region.end)?;
    let mut fragment = String::with_capacity(prefix.len() + edit.replacement.len() + suffix.len());
    fragment.push_str(prefix);
    fragment.push_str(&edit.replacement);
    fragment.push_str(suffix);
    Some(fragment)
}

fn splice_sanitized(prior: &str, region: Range<usize>, replacement: &str) -> Option<String> {
    let prefix = prior.get(..region.start)?;
    let suffix = prior.get(region.end..)?;
    let mut text = String::with_capacity(prefix.len() + replacement.len() + suffix.len());
    text.push_str(prefix);
    text.push_str(replacement);
    text.push_str(suffix);
    Some(text)
}

#[expect(
    clippy::fn_params_excessive_bools,
    reason = "the independent recovery predicates form a complete decision table"
)]
const fn should_recover_verbatim(
    directive_normalization_off: bool,
    recovery_required: bool,
) -> bool {
    directive_normalization_off && recovery_required
}

impl fmt::Debug for Document {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Document")
            .field("source_len", &self.state.source().len())
            .finish_non_exhaustive()
    }
}

impl Snapshot {
    fn coordinate_map(&self) -> &CoordinateMap {
        self.state.cache.coordinate_map.get_or_init(|| {
            CoordinateMap::new(
                self.state.source(),
                &self.state.output.sanitized,
                self.state.output.source_unchanged,
            )
        })
    }

    pub(crate) fn node_store(&self) -> &NodeStore {
        &self.state.output.store
    }

    #[cfg(feature = "pandoc")]
    // mutants::skip — pandoc owns the only consumer and is excluded from the
    // diff mutation command.
    #[cfg_attr(test, mutants::skip)]
    pub(crate) fn output(&self) -> &LexOutput {
        &self.state.output
    }

    /// Source text for this immutable view.
    #[must_use]
    pub fn source(&self) -> &str {
        self.state.source()
    }

    /// Diagnostics emitted while parsing.
    #[must_use]
    pub fn diagnostics(&self) -> &[Diagnostic] {
        if self.state.output.source_unchanged {
            return &self.state.output.diagnostics;
        }
        self.state
            .cache
            .projections
            .diagnostics
            .get_or_init(|| {
                self.state
                    .output
                    .diagnostics
                    .iter()
                    .cloned()
                    .map(|diagnostic| {
                        let span = self.coordinate_map().span_to_source(diagnostic.span());
                        diagnostic.with_span(span)
                    })
                    .collect()
            })
            .as_ref()
    }

    /// Resolved gaiji references ordered by source span.
    #[must_use]
    pub fn gaiji_resolutions(&self) -> &[GaijiResolution] {
        self.state
            .cache
            .projections
            .gaiji_resolutions
            .get_or_init(|| gaiji_resolutions(self.source()).into())
            .as_ref()
    }

    /// Resolved gaiji reference containing a source byte offset.
    #[must_use]
    pub fn gaiji_resolution_at(&self, byte_offset: usize) -> Option<&GaijiResolution> {
        let byte_offset = u32::try_from(byte_offset).ok()?;
        let entries = self.gaiji_resolutions();
        let index = entries.partition_point(|entry| entry.span().start <= byte_offset);
        index
            .checked_sub(1)
            .and_then(|index| entries.get(index))
            .filter(|entry| byte_offset < entry.span().end)
    }

    /// Resolved delimiter pairs in source byte coordinates.
    #[must_use]
    pub fn pairs(&self) -> &[PairLink] {
        self.state
            .cache
            .projections
            .pairs
            .get_or_init(|| {
                self.state
                    .output
                    .pairs
                    .iter()
                    .map(|pair| {
                        PairLink::new(
                            pair.kind,
                            self.coordinate_map().span_to_source(pair.open),
                            self.coordinate_map().span_to_source(pair.close),
                        )
                    })
                    .collect()
            })
            .as_ref()
    }

    /// Classified nodes ordered by source span.
    #[must_use]
    pub(crate) fn source_nodes(&self) -> &[SourceNode] {
        &self.state.output.source_nodes
    }

    /// Classified nodes ordered by source span.
    #[must_use]
    pub fn nodes(&self) -> &[NodeView] {
        self.state
            .cache
            .projections
            .nodes
            .get_or_init(|| {
                self.state
                    .output
                    .source_nodes
                    .iter()
                    .map(|node| NodeView {
                        kind: node.node.kind(),
                        span: self.coordinate_map().span_to_source(node.source_span),
                    })
                    .collect()
            })
            .as_ref()
    }

    /// Notation-shaped source text intentionally preserved as literal text.
    #[must_use]
    pub fn literal_markup(&self) -> &[LiteralMarkupView] {
        self.state
            .cache
            .projections
            .literal_markup
            .get_or_init(|| project_literal_markup(self.state.source(), self.nodes()))
            .as_ref()
    }

    /// Literal and editorial directives ordered by source span.
    pub fn directives(&self) -> impl Iterator<Item = DirectiveView<'_>> {
        self.state.output.source_nodes.iter().filter_map(|entry| {
            let (NodeRef::Inline(Node::Directive(directive))
            | NodeRef::BlockLeaf(Node::Directive(directive))) = entry.node
            else {
                return None;
            };
            Some(DirectiveView {
                kind: directive.kind.into(),
                span: self.coordinate_map().span_to_source(entry.source_span),
                source: self
                    .slice(self.coordinate_map().span_to_source(entry.source_span))
                    .unwrap_or_else(|| self.state.output.store.resolve_str(directive.raw)),
            })
        })
    }

    /// Ruby annotations ordered by source span.
    pub fn rubies(&self) -> impl Iterator<Item = RubyView<'_>> {
        self.state.output.source_nodes.iter().filter_map(|entry| {
            let (NodeRef::Inline(Node::Ruby(ruby)) | NodeRef::BlockLeaf(Node::Ruby(ruby))) =
                entry.node
            else {
                return None;
            };
            Some(RubyView {
                span: self.coordinate_map().span_to_source(entry.source_span),
                base: self.state.output.store.content_range_as_plain(ruby.base),
                reading: self.state.output.store.content_range_as_plain(ruby.reading),
                side: ruby.side,
            })
        })
    }

    /// Source text covered by a span.
    #[must_use]
    pub fn slice(&self, span: crate::Span) -> Option<&str> {
        self.state
            .source()
            .get(span.start as usize..span.end as usize)
    }

    /// Find the classified node covering a source byte offset.
    #[must_use]
    pub(crate) fn node_at_source(&self, offset: SourceOffset) -> Option<&SourceNode> {
        self.state.output.node_at_source(offset)
    }

    /// Find the classified node covering a source byte offset.
    #[must_use]
    pub fn node_at(&self, offset: usize) -> Option<NodeView> {
        let offset = u32::try_from(offset).ok()?;
        let nodes = self.nodes();
        let index = nodes.partition_point(|entry| entry.span.start <= offset);
        let entry = index.checked_sub(1).and_then(|i| nodes.get(i))?;
        (offset < entry.span.end).then_some(*entry)
    }

    /// Resolved container pairs in source coordinates.
    #[must_use]
    pub fn container_pairs(&self) -> &[ContainerPair] {
        self.state
            .cache
            .projections
            .container_pairs
            .get_or_init(|| project_container_pairs(&self.state.output, self.coordinate_map()))
            .as_ref()
    }

    /// Parser-normalized source.
    #[must_use]
    pub fn normalized_source(&self) -> &str {
        &self.state.output.sanitized
    }

    /// Source span of a coupled semantic edit at a byte offset.
    #[must_use]
    pub fn coupled_span(&self, offset: usize) -> Option<crate::Span> {
        use crate::splice::SpliceSafety;

        let offset = self.coordinate_map().source_to_sanitized(offset)?;
        let region = self.region_at(offset)?;
        matches!(region.safety, SpliceSafety::Coupled(_))
            .then(|| self.coordinate_map().span_to_source(region.span))
    }

    /// Build verified source-coordinate edits for a coupled semantic change.
    ///
    /// # Errors
    ///
    /// Returns [`EditError::Unverifiable`] when the replacement does not form
    /// the same coupled construct after parsing.
    pub fn replacement_edits(
        &self,
        offset: usize,
        replacement: &str,
    ) -> Result<Vec<TextEdit>, EditError> {
        use crate::splice::SpliceSafety;

        let offset = self
            .coordinate_map()
            .source_to_sanitized(offset)
            .ok_or(EditError::OutOfBounds)?;
        let Some(region) = self.region_at(offset) else {
            return Ok(Vec::new());
        };
        if !matches!(region.safety, SpliceSafety::Coupled(_)) {
            return Ok(Vec::new());
        }
        let coupling = self.coupling(region).ok_or(EditError::Unverifiable)?;
        let source = self
            .splice(region, replacement)
            .map_err(|_| EditError::Unverifiable)?;
        source_edits_from_splice(
            &SourceEditContext {
                original: self.source(),
                sanitized: self.normalized_source(),
                coordinate_map: self.coordinate_map(),
            },
            &source,
            coupling,
        )
    }

    /// Render semantic HTML.
    #[must_use]
    pub fn to_html(&self) -> String {
        render_html(&self.state.output)
    }

    /// Render semantic HTML with explicit options.
    #[must_use]
    pub fn to_html_with(&self, options: RenderOptions) -> String {
        match options.directives {
            DirectiveNormalization::Off => self.to_html(),
            level => render_html_normalized(&self.state.output, level),
        }
    }

    /// Serialize the parsed document to Aozora source.
    #[must_use]
    pub fn to_source(&self) -> String {
        if requires_verbatim_recovery(&self.state.output) {
            self.source().to_owned()
        } else {
            serialize(&self.state.output)
        }
    }

    /// Serialize with explicit options.
    #[must_use]
    pub fn to_source_with(&self, options: SerializeOptions) -> String {
        if should_recover_verbatim(
            options.directives == DirectiveNormalization::Off,
            requires_verbatim_recovery(&self.state.output),
        ) {
            self.source().to_owned()
        } else {
            serialize_with(&self.state.output, options)
        }
    }

    /// Recover the original source without canonical reserialization.
    #[must_use]
    pub fn to_source_verbatim(&self) -> String {
        self.state.source().to_owned()
    }
}

fn project_literal_markup(source: &str, nodes: &[NodeView]) -> Arc<[LiteralMarkupView]> {
    let mut ruby_delimiters = Vec::new();
    let mut ruby_base_markers = Vec::new();
    let mut directive_markers = Vec::new();
    for node in nodes {
        let start = node.span.start as usize;
        let end = node.span.end as usize;
        let Some(text) = source.get(start..end) else {
            continue;
        };
        if node.kind == NodeKind::Ruby {
            if let Some(relative) = text.find('《') {
                ruby_delimiters.push(start.checked_add(relative).expect("source offset"));
            }
            if text.starts_with('｜') {
                ruby_base_markers.push(start);
            }
        } else if let Some(relative) = text.rfind("［＃") {
            directive_markers.push(start.checked_add(relative).expect("source offset"));
        }
    }
    ruby_delimiters.sort_unstable();
    ruby_delimiters.dedup();
    ruby_base_markers.sort_unstable();
    ruby_base_markers.dedup();
    directive_markers.sort_unstable();
    directive_markers.dedup();

    source
        .char_indices()
        .filter_map(|(offset, ch)| {
            let rest = &source[offset..];
            let (kind, semantic) = match ch {
                '《' if !rest.starts_with("《》") => (
                    LiteralMarkupKind::RubyDelimiters,
                    ruby_delimiters.binary_search(&offset).is_ok(),
                ),
                '｜' => (
                    LiteralMarkupKind::RubyBaseMarker,
                    ruby_base_markers.binary_search(&offset).is_ok(),
                ),
                '［' if rest.starts_with("［＃") => (
                    LiteralMarkupKind::DirectiveMarker,
                    directive_markers.binary_search(&offset).is_ok(),
                ),
                _ => return None,
            };
            (!semantic).then(|| {
                let start = u32::try_from(offset).expect("source fits parser spans");
                let end = u32::try_from(
                    offset
                        .checked_add(ch.len_utf8())
                        .expect("source character end"),
                )
                .expect("source fits parser spans");
                LiteralMarkupView {
                    kind,
                    span: crate::Span::new(start, end),
                }
            })
        })
        .collect()
}

fn project_paragraphs(
    source: &str,
    prior: Option<&[Arc<ParagraphSnapshot>]>,
) -> Arc<[Arc<ParagraphSnapshot>]> {
    let mut ranges = Vec::new();
    let mut start = 0;
    while let Some(relative) = source[start..].find("\n\n") {
        let end = start
            .checked_add(relative)
            .and_then(|offset| offset.checked_add("\n\n".len()))
            .expect("paragraph boundary");
        assert!(end > start, "paragraph scan must advance");
        ranges.push(start..end);
        start = end;
    }
    ranges.push(start..source.len());

    let mut paragraphs = Vec::with_capacity(ranges.len());
    for (index, range) in ranges.into_iter().enumerate() {
        let text = &source[range];
        let shared = prior
            .and_then(|items| items.get(index))
            .filter(|item| item.source.as_ref() == text)
            .cloned()
            .or_else(|| {
                prior
                    .and_then(|items| items.iter().rev().find(|item| item.source.as_ref() == text))
                    .cloned()
            });
        paragraphs.push(shared.unwrap_or_else(|| {
            Arc::new(ParagraphSnapshot {
                source: Arc::from(text),
            })
        }));
    }
    paragraphs.into()
}

fn project_container_pairs(
    output: &LexOutput,
    coordinate_map: &CoordinateMap,
) -> Arc<[ContainerPair]> {
    output
        .container_pairs
        .iter()
        .filter_map(|pair: &AstContainerPair| {
            let open = output
                .source_nodes
                .iter()
                .find(|node| node.normalized_offset == pair.open)?
                .source_span;
            let close = output
                .source_nodes
                .iter()
                .find(|node| node.normalized_offset == pair.close)?
                .source_span;
            Some(ContainerPair {
                kind: pair.kind.into(),
                open: coordinate_map.span_to_source(open),
                close: coordinate_map.span_to_source(close),
            })
        })
        .collect()
}

struct SourceEditContext<'a> {
    original: &'a str,
    sanitized: &'a str,
    coordinate_map: &'a CoordinateMap,
}

fn source_edits_from_splice(
    context: &SourceEditContext<'_>,
    after: &str,
    coupling: Coupling,
) -> Result<Vec<TextEdit>, EditError> {
    let before = context.sanitized;
    let (first, second) = if coupling.primary.start <= coupling.partner.start {
        (coupling.primary, coupling.partner)
    } else {
        (coupling.partner, coupling.primary)
    };
    if !ordered_non_overlapping(first.end, second.start) {
        return Err(EditError::Unverifiable);
    }

    let prefix = before
        .get(..first.start as usize)
        .ok_or(EditError::Unverifiable)?;
    let suffix = before
        .get(second.end as usize..)
        .ok_or(EditError::Unverifiable)?;
    if !after.starts_with(prefix) {
        return Err(EditError::Unverifiable);
    }
    if !after.ends_with(suffix) {
        return Err(EditError::Unverifiable);
    }

    let core_end = after
        .len()
        .checked_sub(suffix.len())
        .ok_or(EditError::Unverifiable)?;
    let core = after
        .get(prefix.len()..core_end)
        .ok_or(EditError::Unverifiable)?;
    let middle = before
        .get(first.end as usize..second.start as usize)
        .ok_or(EditError::Unverifiable)?;

    if middle.is_empty() {
        let span = context
            .coordinate_map
            .span_to_source(crate::Span::new(first.start, second.end));
        let range = span.start as usize..span.end as usize;
        context
            .original
            .get(range.clone())
            .ok_or(EditError::Unverifiable)?;
        return Ok(vec![TextEdit::new(range, core)]);
    }

    let split = core.find(middle).ok_or(EditError::Unverifiable)?;
    let first_replacement = &core[..split];
    let second_replacement = &core[split + middle.len()..];
    let mut edits = Vec::with_capacity(2);
    push_source_edit(&mut edits, context, first, first_replacement)?;
    push_source_edit(&mut edits, context, second, second_replacement)?;
    Ok(edits)
}

const fn ordered_non_overlapping(first_end: u32, second_start: u32) -> bool {
    first_end <= second_start
}

fn push_source_edit(
    edits: &mut Vec<TextEdit>,
    context: &SourceEditContext<'_>,
    sanitized_span: crate::Span,
    replacement: &str,
) -> Result<(), EditError> {
    let old = context
        .sanitized
        .get(sanitized_span.start as usize..sanitized_span.end as usize)
        .ok_or(EditError::Unverifiable)?;
    if old == replacement {
        return Ok(());
    }
    let source_span = context.coordinate_map.span_to_source(sanitized_span);
    let range = source_span.start as usize..source_span.end as usize;
    context
        .original
        .get(range.clone())
        .ok_or(EditError::Unverifiable)?;
    edits.push(TextEdit::new(range, replacement));
    Ok(())
}

fn apply_edits_to_source(source: &str, edits: &[TextEdit]) -> String {
    let capacity = replacement_capacity(source.len(), edits);
    let mut edited = String::with_capacity(capacity);
    let mut cursor = 0;
    for edit in edits {
        edited.push_str(&source[cursor..edit.range.start]);
        edited.push_str(&edit.replacement);
        cursor = edit.range.end;
    }
    edited.push_str(&source[cursor..]);
    edited
}

// mutants::skip — capacity arithmetic changes allocation pressure but not
// observable output.
#[cfg_attr(test, mutants::skip)]
fn replacement_capacity(source_len: usize, edits: &[TextEdit]) -> usize {
    let added = edits.iter().fold(0usize, |total, edit| {
        total.saturating_add(edit.replacement.len())
    });
    let removed = edits.iter().fold(0usize, |total, edit| {
        total.saturating_add(edit.range.end - edit.range.start)
    });
    source_len.saturating_sub(removed).saturating_add(added)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse;
    use crate::spec::PairKind;
    use crate::syntax::ast::NodeRef;

    fn assert_snapshots_match(actual: &Snapshot, expected: &Snapshot) {
        assert_eq!(actual.source(), expected.source());
        assert_eq!(actual.normalized_source(), expected.normalized_source());
        assert_eq!(
            actual
                .diagnostics()
                .iter()
                .map(|diagnostic| (
                    diagnostic.code(),
                    diagnostic.severity(),
                    diagnostic.source(),
                    diagnostic.span(),
                    diagnostic.to_string(),
                ))
                .collect::<Vec<_>>(),
            expected
                .diagnostics()
                .iter()
                .map(|diagnostic| (
                    diagnostic.code(),
                    diagnostic.severity(),
                    diagnostic.source(),
                    diagnostic.span(),
                    diagnostic.to_string(),
                ))
                .collect::<Vec<_>>()
        );
        assert_eq!(actual.nodes(), expected.nodes());
        assert_eq!(actual.pairs(), expected.pairs());
        assert_eq!(actual.container_pairs(), expected.container_pairs());
        assert_eq!(actual.literal_markup(), expected.literal_markup());
        assert_eq!(
            actual.directives().collect::<Vec<_>>(),
            expected.directives().collect::<Vec<_>>()
        );
        assert_eq!(
            actual.rubies().collect::<Vec<_>>(),
            expected.rubies().collect::<Vec<_>>()
        );
        assert_eq!(actual.to_html(), expected.to_html());
        assert_eq!(actual.to_source(), expected.to_source());
        assert_eq!(actual.to_source_verbatim(), expected.to_source_verbatim());
    }

    #[test]
    fn incremental_diagnostics_are_limited_to_stateless_kinds() {
        let span = crate::Span::new(0, 1);
        assert!(diagnostics_allow_incremental(&[]));
        assert!(diagnostics_allow_incremental(&[
            Diagnostic::unresolved_gaiji(span),
            Diagnostic::non_canonical_directive(span, "canonical"),
        ]));
        assert!(!diagnostics_allow_incremental(&[
            Diagnostic::accent_decomposition_applied(span),
        ]));
    }

    #[test]
    fn document_owns_source() {
        let document = {
            let source = String::from("hello");
            Document::new(source)
        };
        assert_eq!(document.source(), "hello");
    }

    #[test]
    fn parser_reuses_an_arc_source() {
        let source: Arc<str> = Arc::from("shared");
        let pointer = source.as_ptr();
        let document = Parser::new()
            .parse(Arc::clone(&source))
            .expect("short source fits parser spans");
        assert_eq!(document.source().as_ptr(), pointer);
    }

    #[test]
    fn snapshot_owns_the_same_source_after_document_drops() {
        let snapshot = {
            let document = Document::new(String::from("world"));
            document.snapshot()
        };
        assert_eq!(snapshot.source(), "world");
    }

    #[test]
    fn diagnostics_empty_for_clean_input() {
        let d = Document::new("plain");
        let t = d.snapshot();
        assert!(t.diagnostics().is_empty());
    }

    #[test]
    fn parser_source_limit_is_inclusive() {
        let parser = Parser {
            max_source_bytes: 3,
        };
        parser.parse("ab").expect("source within parser limit");
        parser.parse("abc").expect("source at parser limit");
        assert_eq!(
            parser.parse("abcd").expect_err("over-limit source"),
            ParseError::SourceTooLarge { len: 4 }
        );
    }

    #[test]
    fn coordinate_builder_records_one_sided_tails() {
        let source_tail = CoordinateMapBuilder::new("tail", "").build();
        assert_eq!(source_tail.len(), 1);
        assert_eq!(source_tail[0].source, 0.."tail".len());
        assert_eq!(source_tail[0].sanitized, 0..0);

        let sanitized_tail = CoordinateMapBuilder::new("", "tail").build();
        assert_eq!(sanitized_tail.len(), 1);
        assert_eq!(sanitized_tail[0].source, 0..0);
        assert_eq!(sanitized_tail[0].sanitized, 0.."tail".len());
    }

    #[test]
    fn coordinate_builder_tracks_each_changed_accent_body() {
        assert!(
            CoordinateMapBuilder::new("〔plain〕", "〔plain〕")
                .build()
                .is_empty()
        );

        let source = "〔e'〕x〔a^〕";
        let sanitized = sanitize(source).text;
        assert_eq!(sanitized.as_ref(), "〔é〕x〔â〕");
        let edits = CoordinateMapBuilder::new(source, &sanitized).build();
        assert_eq!(
            edits
                .iter()
                .map(|edit| (
                    &source[edit.source.clone()],
                    &sanitized[edit.sanitized.clone()]
                ))
                .collect::<Vec<_>>(),
            vec![("e'", "é"), ("a^", "â")]
        );
    }

    #[test]
    fn coordinate_builder_consumes_maximal_shared_char_boundaries() {
        let mut ascii = CoordinateMapBuilder::new("abcX", "abcY");
        ascii.consume_common();
        assert_eq!((ascii.source_offset, ascii.sanitized_offset), (3, 3));

        let mut partial_utf8 = CoordinateMapBuilder::new("あ", "い");
        partial_utf8.consume_common();
        assert_eq!(
            (partial_utf8.source_offset, partial_utf8.sanitized_offset),
            (0, 0)
        );

        let mut before_accent = CoordinateMapBuilder::new("pre〔e'〕", "pre〔é〕");
        before_accent.consume_common();
        assert_eq!(
            (before_accent.source_offset, before_accent.sanitized_offset),
            (3, 3)
        );
    }

    #[test]
    fn coordinate_builder_requires_both_crlf_conditions() {
        let crlf = CoordinateMapBuilder::new("\r\n", "\n").build();
        assert_eq!(crlf.len(), 1);
        assert_eq!(crlf[0].source, 0..2);
        assert_eq!(crlf[0].sanitized, 0..1);

        let source_only = CoordinateMapBuilder::new("\rX", "Y").build();
        assert_eq!(source_only.len(), 1);
        assert_eq!(source_only[0].source, 0..2);
        assert_eq!(source_only[0].sanitized, 0..1);

        let sanitized_only = CoordinateMapBuilder::new("X", "\nX").build();
        assert_eq!(sanitized_only.len(), 1);
        assert_eq!(sanitized_only[0].source, 0..0);
        assert_eq!(sanitized_only[0].sanitized, 0..1);
    }

    #[test]
    fn coordinate_builder_requires_both_pua_replacement_conditions() {
        assert!(
            CoordinateMapBuilder::new("\u{E001}", "\u{FFFD}")
                .build()
                .is_empty()
        );

        let source_only = CoordinateMapBuilder::new("\u{E001}", "X").build();
        assert_eq!(source_only.len(), 1);
        assert_eq!(source_only[0].source, 0..'\u{E001}'.len_utf8());
        assert_eq!(source_only[0].sanitized, 0..1);

        let sanitized_only = CoordinateMapBuilder::new("X", "\u{FFFD}").build();
        assert_eq!(sanitized_only.len(), 1);
        assert_eq!(sanitized_only[0].source, 0..1);
        assert_eq!(sanitized_only[0].sanitized, 0..'\u{FFFD}'.len_utf8());
    }

    #[test]
    fn coordinate_map_preserves_empty_and_nonempty_edit_boundaries() {
        let bom = CoordinateMap::new("\u{FEFF}x", "x", false);
        assert_eq!(
            bom.sanitized_to_source(0, false),
            u32::try_from('\u{FEFF}'.len_utf8()).expect("BOM length fits u32")
        );
        assert_eq!(
            bom.sanitized_to_source(1, true),
            u32::try_from(("\u{FEFF}x").len()).expect("test source length fits u32")
        );

        let crlf = CoordinateMap::new("a\r\nb", "a\nb", false);
        assert_eq!(crlf.sanitized_to_source(0, false), 0);
        assert_eq!(crlf.sanitized_to_source(1, false), 1);
        assert_eq!(crlf.sanitized_to_source(2, true), 3);
        assert_eq!(crlf.sanitized_to_source(3, true), 4);

        let accent = CoordinateMap::new("〔e'〕", "〔é〕", false);
        assert_eq!(accent.sanitized_to_source(3, false), 3);
        assert_eq!(accent.sanitized_to_source(4, false), 3);
        assert_eq!(accent.sanitized_to_source(4, true), 5);
        assert_eq!(accent.sanitized_to_source(5, true), 5);

        let identity = CoordinateMap::new("abc", "abc", true);
        assert_eq!(
            identity.source_to_sanitized(3).map(SourceOffset::get),
            Some(3)
        );
        assert_eq!(identity.source_to_sanitized(4), None);

        assert_eq!(crlf.source_to_sanitized(0).map(SourceOffset::get), Some(0));
        assert_eq!(crlf.source_to_sanitized(1).map(SourceOffset::get), Some(1));
        assert_eq!(crlf.source_to_sanitized(2).map(SourceOffset::get), Some(1));
        assert_eq!(crlf.source_to_sanitized(3).map(SourceOffset::get), Some(2));
        assert_eq!(crlf.source_to_sanitized(4).map(SourceOffset::get), Some(3));

        assert_eq!(bom.source_to_sanitized(0).map(SourceOffset::get), Some(0));
        assert_eq!(bom.source_to_sanitized(2).map(SourceOffset::get), Some(0));
        assert_eq!(bom.source_to_sanitized(3).map(SourceOffset::get), Some(0));
        assert_eq!(bom.source_to_sanitized(4).map(SourceOffset::get), Some(1));
    }

    #[test]
    fn edit_batch_is_atomic() {
        let mut document = Parser::new().parse("aあz").expect("small source");
        let before = document.source().to_owned();
        let result = document.edit([TextEdit::new(0..1, "A"), TextEdit::new(2..3, "invalid")]);
        assert_eq!(result, Err(EditError::NotCharBoundary));
        assert_eq!(document.source(), before);
    }

    #[test]
    fn edit_batch_uses_old_source_coordinates() {
        let mut document = Document::new("alpha beta gamma");
        document
            .edit([TextEdit::new(0..5, "A"), TextEdit::new(11..16, "G")])
            .expect("sorted disjoint edits");
        assert_eq!(document.source(), "A beta G");
    }

    #[test]
    fn edit_batch_incremental_result_matches_full_parse() {
        let source = "｜前《まえ》\n\n中央［＃「中央」は太字］\n\n｜後《あと》";
        let first = source.find("まえ").expect("first reading");
        let last = source.rfind('後').expect("last base");
        let edits = [
            TextEdit::new(first..first + "まえ".len(), "ぜん"),
            TextEdit::new(last..last + '後'.len_utf8(), "末尾"),
        ];
        let expected_source = apply_edits_to_source(source, &edits);
        let mut document = Document::new(source);
        document.edit(edits).expect("valid edit batch");
        let expected = Document::new(expected_source);
        assert_snapshots_match(&document.snapshot(), &expected.snapshot());
    }

    #[test]
    fn edit_batch_preserves_same_offset_insertion_order() {
        let mut document = Document::new("tail");
        document
            .edit([
                TextEdit::new(0..0, "first-"),
                TextEdit::new(0..0, "second-"),
            ])
            .expect("same-offset insertions are disjoint");
        assert_eq!(document.source(), "first-second-tail");
        assert_snapshots_match(
            &document.snapshot(),
            &Document::new("first-second-tail").snapshot(),
        );
    }

    #[test]
    fn source_splice_applies_sorted_edits_in_old_coordinates() {
        let source = "alpha beta gamma";
        let edits = [
            TextEdit::new(0..5, "A"),
            TextEdit::new(6..6, "new "),
            TextEdit::new(11..16, "G"),
        ];
        assert_eq!(apply_edits_to_source(source, &edits), "A new beta G");
    }

    #[test]
    fn source_edits_validate_and_order_coupled_regions() {
        use crate::splice::CoupledKind;

        assert!(ordered_non_overlapping(3, 3));
        assert!(ordered_non_overlapping(3, 5));
        assert!(!ordered_non_overlapping(5, 3));

        let before = "pAA--BBs";
        let map = CoordinateMap::new(before, before, true);
        let context = SourceEditContext {
            original: before,
            sanitized: before,
            coordinate_map: &map,
        };
        let coupling = Coupling {
            kind: CoupledKind::Container,
            primary: crate::Span::new(5, 7),
            partner: crate::Span::new(1, 3),
        };
        let edits = source_edits_from_splice(&context, "pXX--YYs", coupling).expect("valid splice");
        assert_eq!(edits.len(), 2);
        assert_eq!(edits[0].range(), 1..3);
        assert_eq!(edits[0].replacement(), "XX");
        assert_eq!(edits[1].range(), 5..7);
        assert_eq!(edits[1].replacement(), "YY");
        assert_eq!(
            source_edits_from_splice(&context, "qXX--YYs", coupling),
            Err(EditError::Unverifiable)
        );
        assert_eq!(
            source_edits_from_splice(&context, "pXX--YYq", coupling),
            Err(EditError::Unverifiable)
        );

        let touching = "pAABBs";
        let touching_map = CoordinateMap::new(touching, touching, true);
        let touching_context = SourceEditContext {
            original: touching,
            sanitized: touching,
            coordinate_map: &touching_map,
        };
        let touching_coupling = Coupling {
            kind: CoupledKind::Container,
            primary: crate::Span::new(1, 3),
            partner: crate::Span::new(3, 5),
        };
        let edits = source_edits_from_splice(&touching_context, "pXXYYs", touching_coupling)
            .expect("touching splice");
        assert_eq!(edits.len(), 1);
        assert_eq!(edits[0].range(), 1..5);
        assert_eq!(edits[0].replacement(), "XXYY");
    }

    #[test]
    fn source_sanitization_region_uses_blank_line_boundaries() {
        assert_eq!(
            source_sanitization_region("first\n\nmiddle\n\nlast", &(9..9)),
            7..15,
        );
        assert_eq!(source_sanitization_region("single", &(3..3)), 0..6);
        assert_eq!(
            source_sanitization_region("first\r\n\r\nmiddle\r\n\r\nlast", &(11..11)),
            9..19,
        );
    }

    #[test]
    fn incremental_sanitization_matches_full_crlf_parse() {
        let source = "first\r\n\r\nsecond\r\n\r\nthird";
        let at = source.find("second").expect("second paragraph");
        let mut document = parse(source).expect("document");
        document
            .edit([TextEdit::new(at..at, "x")])
            .expect("valid edit");

        let mut edited = source.to_owned();
        edited.insert(at, 'x');
        let full = parse(edited).expect("full parse");
        assert_snapshots_match(&document.snapshot(), &full.snapshot());
    }

    #[test]
    fn incremental_sanitization_matches_new_rule_isolation() {
        let source = "title\r\n---------\r\n\r\nend";
        let at = source.find('-').expect("rule");
        let mut document = parse(source).expect("document");
        document
            .edit([TextEdit::new(at..at, "-")])
            .expect("valid edit");

        let mut edited = source.to_owned();
        edited.insert(at, '-');
        let full = parse(edited).expect("full parse");
        assert_snapshots_match(&document.snapshot(), &full.snapshot());
    }

    #[test]
    fn edits_preserve_sanitize_diagnostics_outside_the_changed_paragraph() {
        let source = "前〔cafe'〕\n\nmiddle\n\n後〔cafe'〕";
        let at = source.find("middle").expect("middle paragraph");
        let mut document = parse(source).expect("document");
        document
            .edit([TextEdit::new(at..at, "x")])
            .expect("valid edit");

        let mut edited = source.to_owned();
        edited.insert(at, 'x');
        let full = parse(edited).expect("full parse");
        assert_snapshots_match(&document.snapshot(), &full.snapshot());
    }

    #[test]
    fn verbatim_recovery_policy_requires_off_and_recovery() {
        assert!(!should_recover_verbatim(false, false));
        assert!(!should_recover_verbatim(false, true));
        assert!(!should_recover_verbatim(true, false));
        assert!(should_recover_verbatim(true, true));
    }

    #[test]
    fn text_edit_accessors_preserve_constructor_values() {
        let edit = TextEdit::new(1..3, "replacement");
        assert_eq!(edit.range(), 1..3);
        assert_eq!(edit.replacement(), "replacement");
    }

    #[test]
    fn edit_validation_covers_every_batch_boundary() {
        let mut document = Document::new("abcd");
        let inverted_start = 2;
        let inverted_end = 1;
        assert_eq!(
            document.edit([TextEdit::new(inverted_start..inverted_end, "")]),
            Err(EditError::InvertedRange)
        );
        assert_eq!(
            document.edit([TextEdit::new(0..5, "")]),
            Err(EditError::OutOfBounds)
        );
        document
            .edit([TextEdit::new(1..1, "X")])
            .expect("empty ranges are valid insertions");
        assert_eq!(document.source(), "aXbcd");

        let mut document = Document::new("abcd");
        document
            .edit([TextEdit::new(0..1, "A"), TextEdit::new(1..2, "B")])
            .expect("adjacent edits are disjoint");
        assert_eq!(document.source(), "ABcd");

        let mut document = Document::new("abcd");
        assert_eq!(
            document.edit([TextEdit::new(1..3, ""), TextEdit::new(2..4, "")]),
            Err(EditError::UnsortedOrOverlapping)
        );
    }

    #[test]
    fn edit_rejects_each_non_character_boundary() {
        let mut document = Document::new("あ");
        assert_eq!(
            document.edit([TextEdit::new(1..3, "")]),
            Err(EditError::NotCharBoundary)
        );
        assert_eq!(
            document.edit([TextEdit::new(0..1, "")]),
            Err(EditError::NotCharBoundary)
        );
    }

    #[test]
    fn snapshot_resolves_public_payloads() {
        let snapshot =
            Document::new("｜青梅《おうめ》※［＃「木＋吶のつくり」、第3水準1-85-54］［＃未知］")
                .snapshot();
        let ruby = snapshot.rubies().next().expect("ruby projection");
        assert_eq!(ruby.base(), Some("青梅"));
        assert_eq!(ruby.reading(), Some("おうめ"));
        assert!(
            snapshot
                .gaiji_resolutions()
                .iter()
                .any(|gaiji| gaiji.resolved().is_some())
        );
        assert!(snapshot.directives().any(|directive| {
            directive.kind() == DirectiveClass::Editorial && directive.source() == "［＃未知］"
        }));
    }

    #[test]
    fn gaiji_lookup_uses_half_open_source_spans() {
        let snapshot = Document::new("x※［＃「木＋吶のつくり」、第3水準1-85-54］y").snapshot();
        let resolution = snapshot
            .gaiji_resolutions()
            .first()
            .expect("gaiji resolution");
        let span = resolution.span();
        assert!(
            snapshot
                .gaiji_resolution_at(span.start as usize - 1)
                .is_none()
        );
        assert!(snapshot.gaiji_resolution_at(span.start as usize).is_some());
        assert!(
            snapshot
                .gaiji_resolution_at(span.end as usize - 1)
                .is_some()
        );
        assert!(snapshot.gaiji_resolution_at(span.end as usize).is_none());
        assert!(
            snapshot
                .gaiji_resolution_at(snapshot.source().len())
                .is_none()
        );
    }

    #[test]
    fn node_lookup_covers_nodes_but_not_gaps_or_ends() {
        let snapshot = Document::new("x｜青《あお》gap｜赤《あか》y").snapshot();
        let nodes = snapshot.nodes();
        assert_eq!(nodes.len(), 2);
        let first = nodes[0];
        let second = nodes[1];

        assert_eq!(snapshot.node_at(first.span.start as usize), Some(first));
        assert_eq!(snapshot.node_at(first.span.end as usize - 1), Some(first));
        assert_eq!(snapshot.node_at(first.span.end as usize), None);
        assert_eq!(snapshot.node_at(second.span.start as usize - 1), None);
        assert_eq!(snapshot.node_at(second.span.start as usize), Some(second));
        assert_eq!(snapshot.node_at(second.span.end as usize - 1), Some(second));
        assert_eq!(snapshot.node_at(second.span.end as usize), None);
        assert_eq!(snapshot.node_at(snapshot.source().len()), None);
    }

    #[test]
    fn coupled_span_follows_half_open_container_markers() {
        let open = "［＃ここから2字下げ］";
        let close = "［＃ここで字下げ終わり］";
        let source = format!("{open}\nbody\n{close}");
        let snapshot = Document::new(source.as_str()).snapshot();
        let open_span = crate::Span::new(
            0,
            u32::try_from(open.len()).expect("open marker length fits u32"),
        );
        let close_start = source.find(close).expect("container close");
        let close_span = crate::Span::new(
            u32::try_from(close_start).expect("close marker offset fits u32"),
            u32::try_from(close_start + close.len()).expect("close marker end fits u32"),
        );

        assert_eq!(snapshot.coupled_span(0), Some(open_span));
        assert_eq!(snapshot.coupled_span(open.len() - 1), Some(open_span));
        assert_eq!(snapshot.coupled_span(open.len()), None);
        assert_eq!(snapshot.coupled_span(close_start - 1), None);
        assert_eq!(snapshot.coupled_span(close_start), Some(close_span));
        assert_eq!(
            snapshot.coupled_span(close_start + close.len() - 1),
            Some(close_span)
        );
        assert_eq!(snapshot.coupled_span(close_start + close.len()), None);
        assert_eq!(snapshot.coupled_span(source.len()), None);
    }

    #[test]
    fn margin_note_replacement_updates_base_and_annotation() {
        let source = "青空［＃「青空」の左に「あお」の注記］";
        let marker = source.find('［').expect("margin note marker");
        let snapshot = Document::new(source).snapshot();
        assert_eq!(
            snapshot.coupled_span(marker),
            Some(crate::Span::new(
                0,
                u32::try_from(source.len()).expect("test source length fits u32"),
            ))
        );
        let edits = snapshot
            .replacement_edits(marker, "蒼天［＃「蒼天」の左に「あお」の注記］")
            .expect("verified margin note replacement");
        assert_eq!(edits.len(), 1);
        assert_eq!(
            apply_edits_to_source(source, &edits),
            "蒼天［＃「蒼天」の左に「あお」の注記］"
        );
        assert_eq!(
            snapshot.replacement_edits(marker, "青空［＃「蒼天」の左に「あお」の注記］"),
            Err(EditError::Unverifiable)
        );
        assert_eq!(
            snapshot.replacement_edits(marker, "蒼天［＃「蒼天」に傍点］"),
            Err(EditError::Unverifiable)
        );
    }

    #[test]
    fn malformed_recovery_preserves_original_source_coordinates() {
        let source = "\u{feff}〔e'〕［＃閉じない\r\n次";
        let snapshot = Document::new(source).snapshot();
        assert_eq!(snapshot.to_source(), source);
        assert_eq!(snapshot.to_source_with(SerializeOptions::default()), source);
    }

    #[test]
    fn snapshot_separates_literal_markup_from_semantic_nodes() {
        let snapshot = Document::new("｜青梅《おうめ》 六ヶ《むつか》 ｜ ［＃閉じない").snapshot();
        let literal = snapshot.literal_markup();
        assert_eq!(literal.len(), 3);
        assert_eq!(literal[0].kind(), LiteralMarkupKind::RubyDelimiters);
        assert_eq!(snapshot.slice(literal[0].span()), Some("《"));
        assert_eq!(literal[1].kind(), LiteralMarkupKind::RubyBaseMarker);
        assert_eq!(snapshot.slice(literal[1].span()), Some("｜"));
        assert_eq!(literal[2].kind(), LiteralMarkupKind::DirectiveMarker);
        assert_eq!(snapshot.slice(literal[2].span()), Some("［"));

        let semantic = Document::new("語句［＃「語句」は太字］").snapshot();
        assert!(semantic.literal_markup().is_empty());

        let empty_pair = Document::new("《》").snapshot();
        assert!(empty_pair.literal_markup().is_empty());

        let ordinary_brackets = Document::new("［plain］").snapshot();
        assert!(ordinary_brackets.literal_markup().is_empty());
    }

    #[test]
    fn edit_reuses_unchanged_paragraph_snapshots() {
        let mut document = Document::new("first\n\nmiddle\n\nlast");
        let before = document.snapshot();
        let before_paragraphs = before
            .state
            .cache
            .paragraphs
            .get_or_init(|| project_paragraphs(before.state.source(), None));
        let first = Arc::clone(&before_paragraphs[0]);
        let last = Arc::clone(&before_paragraphs[2]);
        document
            .edit([TextEdit::new(7..13, "changed")])
            .expect("middle paragraph edit");
        let after = document.snapshot();
        let after_paragraphs = after
            .state
            .cache
            .paragraphs
            .get()
            .expect("paragraphs initialized");
        assert_eq!(after_paragraphs.len(), 3);
        assert!(Arc::ptr_eq(&after_paragraphs[0], &first));
        assert!(Arc::ptr_eq(&after_paragraphs[2], &last));
        assert!(!Arc::ptr_eq(&after_paragraphs[1], &before_paragraphs[1]));
    }

    #[test]
    fn editing_leaves_unused_paragraph_projections_lazy() {
        let mut document = Document::new("first\n\nmiddle\n\nlast");
        let before = document.snapshot();
        assert!(before.state.cache.paragraphs.get().is_none());
        document
            .edit([TextEdit::new(7..13, "changed")])
            .expect("middle paragraph edit");
        assert!(before.state.cache.paragraphs.get().is_none());
        let after = document.snapshot();
        assert!(after.state.cache.paragraphs.get().is_none());
        let paragraphs = after
            .state
            .cache
            .paragraphs
            .get_or_init(|| project_paragraphs(after.state.source(), None));
        assert_eq!(paragraphs[1].source.as_ref(), "changed\n\n");
    }

    #[test]
    fn edit_batch_reuses_paragraphs_untouched_between_changes() {
        let mut document = Document::new("first\n\nmiddle\n\nlast");
        let before = document.snapshot();
        let before_paragraphs = before
            .state
            .cache
            .paragraphs
            .get_or_init(|| project_paragraphs(before.state.source(), None));
        let middle = Arc::clone(&before_paragraphs[1]);
        document
            .edit([TextEdit::new(0..5, "head"), TextEdit::new(15..19, "tail")])
            .expect("valid batch");
        let after = document.snapshot();
        let after_paragraphs = after
            .state
            .cache
            .paragraphs
            .get()
            .expect("paragraphs initialized");
        assert!(Arc::ptr_eq(&after_paragraphs[1], &middle));
        assert!(!Arc::ptr_eq(&after_paragraphs[0], &before_paragraphs[0]));
        assert!(!Arc::ptr_eq(&after_paragraphs[2], &before_paragraphs[2]));
    }

    #[test]
    fn paragraph_projection_preserves_delimiters_and_trailing_empty_range() {
        let paragraphs = project_paragraphs("first\n\nmiddle\n\nlast\n\n", None);
        assert_eq!(
            paragraphs
                .iter()
                .map(|paragraph| paragraph.source.as_ref())
                .collect::<Vec<_>>(),
            vec!["first\n\n", "middle\n\n", "last\n\n", ""]
        );
        let empty = project_paragraphs("", None);
        assert_eq!(empty.len(), 1);
        assert!(empty[0].source.is_empty());
    }

    #[test]
    fn paragraph_projection_reuses_snapshots_after_index_shift() {
        let prior = project_paragraphs("first\n\nmiddle\n\nlast", None);
        let shifted = project_paragraphs("new\n\nfirst\n\nmiddle\n\nlast", Some(&prior));
        assert_eq!(shifted.len(), 4);
        assert_eq!(shifted[0].source.as_ref(), "new\n\n");
        assert!(Arc::ptr_eq(&shifted[1], &prior[0]));
        assert!(Arc::ptr_eq(&shifted[2], &prior[1]));
        assert!(Arc::ptr_eq(&shifted[3], &prior[2]));
    }

    #[test]
    fn snapshot_is_immutable_across_edits() {
        let mut document = Document::new("plain");
        let snapshot = document.snapshot();
        document
            .edit([TextEdit::new(0..5, "changed")])
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
        let mut document = Document::new("hello world!");
        document
            .edit([TextEdit::new(6..11, "Aozora")])
            .expect("in-bounds span");
        assert_eq!(document.source(), "hello Aozora!");
    }

    #[test]
    fn edit_at_start_and_end_boundaries() {
        let mut head = Document::new("middle");
        head.edit([TextEdit::new(0..0, "PRE-")])
            .expect("zero-length span at 0");
        assert_eq!(head.source(), "PRE-middle");

        let mut tail = Document::new("middle");
        let len = tail.source().len();
        tail.edit([TextEdit::new(len..len, "-POST")])
            .expect("zero-length span at len");
        assert_eq!(tail.source(), "middle-POST");
    }

    #[test]
    fn edit_equivalence_full_reparse() {
        // The edited document parses to the same Tree shape as
        // re-parsing the spliced source from scratch — this is the
        // observable property `Document::edit` ships under, which the
        // incremental engine preserves.
        let mut edited = Document::new("｜青梅《おうめ》です。");
        let original = edited.source().to_owned();
        let span_start = original.find('《').expect("《 present");
        let span_end = original.find('》').expect("》 present") + '》'.len_utf8();
        edited
            .edit([TextEdit::new(span_start..span_end, "《せいばい》")])
            .expect("in-bounds span");

        let spliced_source = format!(
            "{prefix}{replacement}{suffix}",
            prefix = &original[..span_start],
            replacement = "《せいばい》",
            suffix = &original[span_end..],
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

    fn assert_original_and_normalized(doc: &str) {
        use crate::pipeline::lexer::sanitize::sanitize;
        let expected = sanitize(doc).text;
        let d = Document::new(doc);
        let t = d.snapshot();
        assert_eq!(t.to_source_verbatim(), doc);
        assert_eq!(t.normalized_source(), &*expected);
    }

    #[test]
    fn verbatim_plain_only() {
        assert_original_and_normalized("ただの平文です。\n二行目。\n");
    }

    #[test]
    fn verbatim_plain_construct_ruby() {
        assert_original_and_normalized("前置き｜青梅《おうめ》後置き");
    }

    #[test]
    fn verbatim_block_container() {
        assert_original_and_normalized(
            "序\n［＃ここから２字下げ］\n本文の段落。\n［＃ここで字下げ終わり］\n了\n",
        );
    }

    #[test]
    fn verbatim_consecutive_nodes_no_gap() {
        assert_original_and_normalized("｜青梅《おうめ》｜街道《かいどう》");
    }

    #[test]
    fn verbatim_leading_node_no_head_gap() {
        assert_original_and_normalized("｜青梅《おうめ》のち平文");
    }

    #[test]
    fn verbatim_trailing_node_no_tail_gap() {
        assert_original_and_normalized("平文のち｜青梅《おうめ》");
    }

    #[test]
    fn original_and_normalized_sources_diverge_after_bom_removal() {
        let doc = "\u{FEFF}\u{FEFF}｜青梅《おうめ》";
        assert_original_and_normalized(doc);
        let snapshot = Document::new(doc).snapshot();
        assert!(snapshot.normalized_source().starts_with('｜'));
    }

    #[test]
    fn original_and_normalized_sources_diverge_after_crlf_folding() {
        let doc = "一行目\r\n二行目\r\n｜青梅《おうめ》\r\n";
        assert_original_and_normalized(doc);
        let snapshot = Document::new(doc).snapshot();
        assert!(!snapshot.normalized_source().contains('\r'));
    }

    #[test]
    fn original_and_normalized_sources_diverge_after_accent_decomposition() {
        assert_original_and_normalized("カフェ〔cafe'〕で待つ");
    }

    #[test]
    fn original_and_normalized_sources_diverge_after_rule_separation() {
        assert_original_and_normalized("段落の文\n----------\nつづき\n");
    }

    #[test]
    fn original_and_normalized_sources_diverge_after_pua_neutralization() {
        let doc = "before\u{E001}mid\u{E004}after";
        assert_original_and_normalized(doc);
        let snapshot = Document::new(doc).snapshot();
        assert!(snapshot.normalized_source().contains('\u{FFFD}'));
        assert!(!snapshot.normalized_source().contains('\u{E001}'));
    }

    #[test]
    fn projections_use_original_offsets_after_bom_and_crlf() {
        let source = "\u{FEFF}前\r\n｜青梅《おうめ》\r\n後";
        let snapshot = Document::new(source).snapshot();
        let ruby_start = source.find('｜').expect("ruby start");
        let ruby_end = source.find('》').expect("ruby end") + '》'.len_utf8();
        let ruby = snapshot.rubies().next().expect("ruby");
        assert_eq!(
            ruby.span(),
            crate::Span::new(
                u32::try_from(ruby_start).expect("test source fits"),
                u32::try_from(ruby_end).expect("test source fits")
            )
        );
        assert_eq!(snapshot.slice(ruby.span()), Some("｜青梅《おうめ》"));

        let pair = snapshot
            .pairs()
            .iter()
            .find(|pair| pair.kind == PairKind::Ruby)
            .expect("ruby pair");
        assert_eq!(pair.open.slice(source), "《");
        assert_eq!(pair.close.slice(source), "》");
    }

    #[test]
    fn diagnostics_use_original_offsets_after_sanitize_rewrites() {
        let source = "\u{FEFF}前\r\nカフェ〔cafe'〕で待つ";
        let snapshot = Document::new(source).snapshot();
        let diagnostic = snapshot
            .diagnostics()
            .iter()
            .find(|diagnostic| matches!(diagnostic, Diagnostic::AccentDecompositionApplied { .. }))
            .expect("accent diagnostic");
        assert_eq!(diagnostic.span().slice(source), "〔cafe'〕");
    }

    #[test]
    fn coupled_edits_preserve_bom_and_crlf() {
        let source =
            "\u{FEFF}前\r\n［＃ここから2字下げ］\r\n本文\r\n［＃ここで字下げ終わり］\r\n後";
        let snapshot = Document::new(source).snapshot();
        let offset = source.find('［').expect("container open");
        let edits = snapshot
            .replacement_edits(offset, "［＃ここから罫囲み］")
            .expect("verified replacement");
        let mut edited = source.to_owned();
        for edit in edits.iter().rev() {
            edited.replace_range(edit.range(), edit.replacement());
        }
        assert_eq!(
            edited,
            "\u{FEFF}前\r\n［＃ここから罫囲み］\r\n本文\r\n［＃罫囲み終わり］\r\n後"
        );
    }

    #[test]
    fn document_debug_pins_source_len() {
        let document = Document::new("hello");
        assert_eq!(format!("{document:?}"), "Document { source_len: 5, .. }");
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
            pairs[0].open().start < pairs[0].close().start,
            "container open offset precedes its close offset",
        );
    }
}
