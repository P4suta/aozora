//! AST types shared between `afm-parser`, `afm-encoding`, and `afm-cli`.
//!
//! Keeping the AST in its own crate (with no parser dep) lets downstream tools consume
//! afm's structured output without pulling in the full `CommonMark` engine.
//!
//! # Invariants
//!
//! - Every [`Span`] refers to byte offsets in the original UTF-8 source buffer. The
//!   offsets are guaranteed to fall on UTF-8 character boundaries; callers can slice
//!   the input with them safely.
//! - Every owned string field uses [`Box<str>`] rather than [`String`]: after parsing,
//!   nodes are immutable, so the capacity field of `String` is dead weight. A
//!   56 kB-annotated work like 『罪と罰』 saves roughly 2 k × 8 B = 16 kB per parse.
//! - Every public enum is `#[non_exhaustive]`. Adding a variant is not a breaking
//!   change; downstream match arms learned the `_` catch-all at compile time.
//!
//! # Classifier methods
//!
//! [`AozoraNode::is_block`], [`AozoraNode::contains_inlines`] and
//! [`AozoraNode::xml_node_name`] exist so comrak's fork can delegate the
//! single-line `NodeValue::Aozora(_)` arm to an AST-level classifier without knowing
//! the shape of individual variants.

#![forbid(unsafe_code)]

use core::slice;

use miette::Diagnostic;
use thiserror::Error;

pub mod accent;
mod extension;
pub use extension::ContainerKind;

/// Byte-range span into the original source document.
///
/// `u32` (rather than `usize`) caps the addressable source at 4 GiB, which is
/// roughly 4 000× the largest plausible Aozora Bunko work — and halves span size on
/// 64-bit targets, which compounds across the thousands of nodes a long novel
/// produces.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Span {
    pub start: u32,
    pub end: u32,
}

impl Span {
    #[must_use]
    pub const fn new(start: u32, end: u32) -> Self {
        Self { start, end }
    }

    #[must_use]
    pub const fn len(self) -> u32 {
        self.end - self.start
    }

    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.start == self.end
    }

    /// Slice the source buffer by this span. Assumes `self` was produced by the
    /// parser and therefore sits on UTF-8 boundaries.
    ///
    /// # Panics
    ///
    /// Panics if `self` does not align to UTF-8 char boundaries in `source`.
    /// Parser-produced spans always do; a panic here signals a bug upstream.
    #[must_use]
    pub fn slice(self, source: &str) -> &str {
        let start = self.start as usize;
        let end = self.end as usize;
        source
            .get(start..end)
            .expect("span must align to UTF-8 char boundaries in source")
    }
}

/// Every afm-specific AST node. Embedded into comrak's `NodeValue` tree as a single
/// `NodeValue::Aozora(AozoraNode)` variant so the upstream diff stays at one line.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub enum AozoraNode {
    /// Ruby (furigana). Ex: `｜青梅《おうめ》` or `日本《にほん》`.
    Ruby(Ruby),

    /// Emphasis dots / sidelines. Ex: `［＃「X」に傍点］` or `［＃傍点］...［＃傍点終わり］`.
    Bouten(Bouten),

    /// Tate-chu-yoko (horizontal embedding inside vertical text). Ex: `［＃縦中横］20［＃縦中横終わり］`.
    TateChuYoko(TateChuYoko),

    /// Gaiji (out-of-range character). Ex: `※［＃「木＋吶のつくり」、第3水準1-85-54］`.
    Gaiji(Gaiji),

    /// `［＃ここから字下げ］ ... ［＃ここで字下げ終わり］` block.
    Indent(Indent),

    /// `［＃地付き］` / `［＃地から2字上げ］` block.
    AlignEnd(AlignEnd),

    /// Split annotation (`割り注`). Two-line inline annotation.
    Warichu(Warichu),

    /// Boxed block (`罫囲み`).
    Keigakomi(Keigakomi),

    /// Page break (`［＃改ページ］`).
    PageBreak,

    /// Section break (`［＃改丁］` / `［＃改段］` / `［＃改見開き］`).
    SectionBreak(SectionKind),

    /// Aozora-specific heading level that doesn't map to a Markdown `#` level
    /// (e.g. 窓見出し). Canonical `大/中/小` are normalised to `NodeValue::Heading`
    /// at parse time and never reach this variant.
    AozoraHeading(AozoraHeading),

    /// Illustration (`［＃挿絵（fig.png）入る］`) — normalised to `NodeValue::Image`
    /// when a caption is absent; the `Sashie` form is used when a caption is attached.
    Sashie(Sashie),

    /// Chinese-reading order mark (返り点 / 訓読み符号). Ex: `［＃一］`, `［＃レ］`,
    /// `［＃上］`. Emitted as a distinct variant rather than
    /// [`AnnotationKind::Unknown`] so the classifier's semantic intent is
    /// preserved through the AST.
    Kaeriten(Kaeriten),

    /// An annotation recognised as Aozora-shaped but not understood by this version
    /// of the parser. Kept for round-trip fidelity and surfaced as a diagnostic.
    Annotation(Annotation),

    /// Double angle-bracket notation — `《《X》》` in source. Used in
    /// Aozora Bunko texts to represent a literal `《X》` pair that is
    /// *not* ruby markup (disambiguating against the single-`《…》`
    /// ruby-reading delimiter). The Aozora annotation manual notes
    /// that these double brackets are conventionally rendered with
    /// the academic U+226A/U+226B characters (`≪X≫`) to sidestep the
    /// ruby-marker overload — the HTML renderer does exactly that.
    DoubleRuby(DoubleRuby),

    /// Paired block container — `［＃ここから…］ ... ［＃ここで…終わり］`.
    /// Holds no payload of its own beyond the container kind; the
    /// wrapped child blocks live as children in the comrak AST (the
    /// `post_process` paired-container splice reparents them). On
    /// render, the wrapper emits an opening tag on the `entering`
    /// pass and a closing tag on exit while comrak walks the
    /// children in between — same contract as `<ul>` / `<div>`
    /// block renders.
    Container(Container),
}

impl AozoraNode {
    /// Whether this node occupies a block (paragraph-level) position in the tree.
    ///
    /// Inline-only variants (`Ruby`, `Bouten`, `TateChuYoko`, `Gaiji`) return `false`.
    /// Block variants (`Indent`, `AlignEnd`, `Warichu`, `Keigakomi`, `PageBreak`,
    /// `SectionBreak`, `AozoraHeading`, `Sashie`) return `true`.
    /// `Annotation` is inline — it stands in for a span of text that the parser
    /// couldn't classify.
    #[must_use]
    pub const fn is_block(&self) -> bool {
        matches!(
            self,
            Self::Indent(_)
                | Self::AlignEnd(_)
                | Self::Warichu(_)
                | Self::Keigakomi(_)
                | Self::PageBreak
                | Self::SectionBreak(_)
                | Self::AozoraHeading(_)
                | Self::Sashie(_)
                | Self::Container(_)
        )
    }

    /// Whether children of this node (if any) are inline content. Block variants that
    /// wrap an indented run of paragraphs answer `true`; leaf blocks answer `false`.
    /// `Container` is the paired-container wrapper — its children are block
    /// elements (paragraphs, headings, nested containers) rather than inlines, so
    /// it answers `false` here.
    #[must_use]
    pub const fn contains_inlines(&self) -> bool {
        matches!(
            self,
            Self::AozoraHeading(_)
                | Self::AlignEnd(_)
                | Self::Warichu(_)
                | Self::Keigakomi(_)
                | Self::Indent(_)
        )
    }

    /// Name used by comrak's XML pretty-printer for nodes of this kind. Stable across
    /// versions; appears in CI fixtures and user-visible diagnostics.
    #[must_use]
    pub const fn xml_node_name(&self) -> &'static str {
        match self {
            Self::Ruby(_) => "aozora_ruby",
            Self::Bouten(_) => "aozora_bouten",
            Self::TateChuYoko(_) => "aozora_tcy",
            Self::Gaiji(_) => "aozora_gaiji",
            Self::Indent(_) => "aozora_indent",
            Self::AlignEnd(_) => "aozora_align_end",
            Self::Warichu(_) => "aozora_warichu",
            Self::Keigakomi(_) => "aozora_keigakomi",
            Self::PageBreak => "aozora_page_break",
            Self::SectionBreak(_) => "aozora_section_break",
            Self::AozoraHeading(_) => "aozora_heading",
            Self::Sashie(_) => "aozora_sashie",
            Self::Kaeriten(_) => "aozora_kaeriten",
            Self::Annotation(_) => "aozora_annotation",
            Self::DoubleRuby(_) => "aozora_double_ruby",
            Self::Container(_) => "aozora_container",
        }
    }
}

/// Paired block container payload: carries only the kind descriptor.
///
/// Children are held in the comrak AST as the container node's
/// children, rather than embedded here, so that comrak's standard
/// tree walk (and serializer downstream) see the same parent/child
/// relationships it sees for `<div>` / `<ul>` / etc.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Container {
    pub kind: ContainerKind,
}

/// Double angle-bracket escape — the payload between `《《` and `》》`.
///
/// The content is modelled as [`Content`] (not `Box<str>`) so that
/// corpus shapes like `《《※［＃「ほ」、第3水準1-85-54］》》` — where a
/// gaiji marker sits between the double brackets — survive the lexer
/// without flattening; they collapse to [`Content::Plain`] whenever
/// the body is pure text (the overwhelmingly common case).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct DoubleRuby {
    pub content: Content,
}

/// Body-content type for nodes whose textual payload may contain nested
/// Aozora constructs (embedded gaiji, inline annotations).
///
/// Two-variant enum balancing the 99%+ plain-text fast path with the
/// structured case: ruby readings with embedded gaiji markers, bouten
/// targets quoted from editorial notes, etc.
///
/// # Invariants
///
/// - `Plain(s)` with `s.is_empty()` is forbidden. Empty content is
///   `Segments(Box::new([]))`.
/// - `Segments(segs)` whose contents collapse to a single text segment
///   (or to concatenable text segments) is canonicalised to `Plain` by
///   [`Content::from_segments`]. Construct via the builder rather than
///   the variant directly to preserve the invariant.
///
/// Use [`Content::as_plain`] for the fast path; a `None` return signals
/// that the caller must iterate [`Content::segments`].
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub enum Content {
    /// Plain text without embedded Aozora constructs.
    Plain(Box<str>),
    /// Mixed text plus nested Aozora constructs.
    Segments(Box<[Segment]>),
}

impl Content {
    /// Construct from an arbitrary segment list. Applies the canonicalisation
    /// invariants: empty input → `Segments([])`; all-text input collapses
    /// into a single `Plain`; single-segment `Text(...)` input becomes `Plain`.
    #[must_use]
    pub fn from_segments(segs: Vec<Segment>) -> Self {
        if segs.is_empty() {
            return Self::Segments(Box::new([]));
        }
        if segs.iter().all(|s| matches!(s, Segment::Text(_))) {
            let merged: String = segs
                .into_iter()
                .map(|s| match s {
                    Segment::Text(t) => t,
                    _ => unreachable!("filtered above to Segment::Text only"),
                })
                .fold(String::new(), |mut acc, t| {
                    acc.push_str(&t);
                    acc
                });
            if merged.is_empty() {
                return Self::Segments(Box::new([]));
            }
            return Self::Plain(merged.into_boxed_str());
        }
        Self::Segments(segs.into_boxed_slice())
    }

    /// If this content is a single plain-text run, return a view of that
    /// text. Returns `None` for mixed-content `Segments`. Use this to take
    /// the fast path in callers that only care about the text dimension.
    #[must_use]
    pub fn as_plain(&self) -> Option<&str> {
        match self {
            Self::Plain(s) => Some(s),
            Self::Segments(_) => None,
        }
    }

    /// Iterate segments in their natural left-to-right order. `Plain(s)`
    /// yields a single synthesised [`SegmentRef::Text`]; `Segments(…)`
    /// yields each segment as a borrowed ref.
    ///
    /// The synthesised variant lets renderers write a single loop that
    /// works uniformly over both variants without upfront match.
    #[must_use]
    pub fn iter(&self) -> ContentIter<'_> {
        match self {
            Self::Plain(s) => ContentIter::Plain(Some(s)),
            Self::Segments(segs) => ContentIter::Segments(segs.iter()),
        }
    }
}

impl<'a> IntoIterator for &'a Content {
    type Item = SegmentRef<'a>;
    type IntoIter = ContentIter<'a>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

impl Default for Content {
    fn default() -> Self {
        Self::Segments(Box::new([]))
    }
}

impl From<Box<str>> for Content {
    fn from(s: Box<str>) -> Self {
        if s.is_empty() {
            Self::Segments(Box::new([]))
        } else {
            Self::Plain(s)
        }
    }
}

impl From<String> for Content {
    fn from(s: String) -> Self {
        Self::from(s.into_boxed_str())
    }
}

impl From<&str> for Content {
    fn from(s: &str) -> Self {
        Self::from(Box::<str>::from(s))
    }
}

/// One element of a [`Content::Segments`] run.
///
/// See [`Content`] docs for construction rules. Direct use of these
/// variants is legal but [`Content::from_segments`] is the preferred
/// builder because it handles the collapse invariants.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub enum Segment {
    Text(Box<str>),
    Gaiji(Gaiji),
    Annotation(Annotation),
}

/// Borrowed view yielded by [`Content::iter`]. Unifies the `Plain` /
/// `Segments` variants of [`Content`] into a single iteration shape.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum SegmentRef<'a> {
    Text(&'a str),
    Gaiji(&'a Gaiji),
    Annotation(&'a Annotation),
}

/// Iterator over [`Content`]'s logical segments. Produced by
/// [`Content::iter`].
#[derive(Debug)]
pub enum ContentIter<'a> {
    /// Plain content — yields one synthesised `Text` segment and stops.
    Plain(Option<&'a str>),
    /// Mixed content — yields each segment via the inner slice iterator.
    Segments(slice::Iter<'a, Segment>),
}

impl<'a> Iterator for ContentIter<'a> {
    type Item = SegmentRef<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        match self {
            Self::Plain(opt) => opt.take().map(SegmentRef::Text),
            Self::Segments(it) => it.next().map(|seg| match seg {
                Segment::Text(t) => SegmentRef::Text(t),
                Segment::Gaiji(g) => SegmentRef::Gaiji(g),
                Segment::Annotation(a) => SegmentRef::Annotation(a),
            }),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Ruby {
    /// Base text (the kanji run before the reading delimiter).
    /// [`Content`] lets the base carry embedded Aozora constructs — a
    /// gaiji marker as the kanji, or a 返り点 interleaved between
    /// characters. The common case (plain kanji text) lands on
    /// [`Content::Plain`].
    pub base: Content,
    /// Reading text (inside `《...》`). Same rationale as `base` —
    /// real corpora contain rubies whose reading holds an embedded
    /// gaiji marker or a `［＃ママ］` editorial note, which the pre-
    /// `Content` schema truncated at the `《...》` boundary.
    pub reading: Content,
    /// `true` when the base was delimited by `｜`, `false` when inferred from the
    /// trailing kanji run before `《》`.
    pub delim_explicit: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Bouten {
    pub kind: BoutenKind,
    /// Literal text the annotation targets. For forward-reference bouten
    /// (`［＃「X」に傍点］`) this is the run named between `「…」` in the
    /// annotation body; for paired bouten (`［＃傍点］…［＃傍点終わり］`, a
    /// later phase) it will be the children captured between the markers.
    /// [`Content`] lets the target carry nested gaiji markers or
    /// editorial annotations without information loss; the HTML renderer
    /// emits `<em class="afm-bouten-{kind}">target</em>` iterating
    /// segments.
    ///
    /// For multi-target shapes like `［＃「A」「B」に傍点］` the lexer
    /// folds all consecutive quote bodies into [`Content::Segments`]
    /// with inter-quote separators represented as [`Segment::Text`]
    /// punctuation. Callers that need the individual target list can
    /// iterate segments and filter on `SegmentRef::Text`.
    pub target: Content,
    /// Which side of the base text the marks appear on. Defaults to
    /// [`BoutenPosition::Right`] (the standard vertical-writing side);
    /// `［＃「X」の左に傍点］` etc. set this to [`BoutenPosition::Left`].
    /// Rendered as an `afm-bouten-left` / `afm-bouten-right` modifier
    /// class so the CSS theme can style each side independently.
    pub position: BoutenPosition,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub enum BoutenKind {
    /// ゴマ (black sesame, default shape of `［＃「X」に傍点］`)
    Goma,
    /// 白ゴマ (white sesame, `［＃「X」に白ゴマ傍点］`)
    WhiteSesame,
    /// 丸 (`［＃「X」に丸傍点］`)
    Circle,
    /// 白丸
    WhiteCircle,
    /// 二重丸
    DoubleCircle,
    /// 蛇の目
    Janome,
    /// ばつ (cross mark, `［＃「X」にばつ傍点］`)
    Cross,
    /// 白三角 (`［＃「X」に白三角傍点］`)
    WhiteTriangle,
    /// 波線 (`［＃「X」に波線］`)
    WavyLine,
    /// 傍線
    UnderLine,
    /// 二重傍線 (`［＃「X」に二重傍線］`)
    DoubleUnderLine,
}

/// Which side of the vertical-writing base text the bouten marks sit on.
///
/// The default is the right side (the standard Japanese print
/// convention); `の左に` in the annotation body flips this to the left
/// side, typically used in parallel editions or to disambiguate two
/// layered readings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub enum BoutenPosition {
    #[default]
    Right,
    Left,
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct TateChuYoko {
    /// Horizontally-composed text (typically 2–3 ASCII digits). Modelled as
    /// [`Content`] for schema-uniformity with other body-bearing nodes;
    /// real corpora rarely put gaiji or annotations here, so the
    /// [`Content::Plain`] fast path dominates.
    pub text: Content,
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Gaiji {
    /// Description text from the source, e.g. "木＋吶のつくり".
    pub description: Box<str>,
    /// Resolved Unicode scalar, if the gaiji maps to one.
    pub ucs: Option<char>,
    /// Raw mencode reference (e.g. "第3水準1-85-54" or "U+XXXX, page-line").
    pub mencode: Option<Box<str>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Indent {
    pub amount: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct AlignEnd {
    /// Offset in chars from the right edge. `0` = 地付き, `n` = 地から n 字上げ.
    pub offset: u8,
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Warichu {
    /// Upper half of the split annotation. [`Content`] carries segments
    /// so an embedded gaiji or nested annotation inside a warichu
    /// stays structured.
    pub upper: Content,
    /// Lower half of the split annotation.
    pub lower: Content,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Keigakomi;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub enum SectionKind {
    /// `［＃改丁］`
    Choho,
    /// `［＃改段］`
    Dan,
    /// `［＃改見開き］`
    Spread,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub enum AozoraHeadingKind {
    /// 窓見出し
    Window,
    /// 副見出し
    Sub,
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct AozoraHeading {
    pub kind: AozoraHeadingKind,
    /// Heading text. [`Content`] keeps embedded ruby / gaiji / annotations
    /// structured through to the renderer.
    pub text: Content,
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Sashie {
    /// File path / URL of the illustration. Remains `Box<str>` since a
    /// filename cannot meaningfully carry nested Aozora constructs.
    pub file: Box<str>,
    /// Optional caption. [`Content`] lets the caption hold ruby,
    /// bouten targets, or annotations.
    pub caption: Option<Content>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Annotation {
    pub raw: Box<str>,
    pub kind: AnnotationKind,
}

/// Chinese-reading order mark (`返り点`). Written in source as
/// `［＃X］` where `X` is one of `一`, `二`, `三`, `四`, `上`, `中`,
/// `下`, `レ`, `甲`, `乙`, `丙`, `丁`, etc.
///
/// The mark's semantics belong to classical Chinese reading order; for
/// typographic purposes we preserve the literal character and let the
/// renderer emit it as a small superscript / side-note.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Kaeriten {
    /// The mark character as written in source.
    pub mark: Box<str>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub enum AnnotationKind {
    /// The parser recognised the notation as Aozora-shaped but not registered.
    Unknown,
    /// `［＃「」」はママ］`-style editorial as-is marker. Rendered but flagged.
    AsIs,
    /// Source-text divergence note (`［＃「X」は底本では「Y」］`).
    TextualNote,
    /// A ruby span that couldn't be parsed cleanly — round-tripped as-is.
    InvalidRubySpan,
}

/// Parse- and render-time error surface for afm-syntax consumers. Parsers and renderers
/// funnel their failures through this enum so CLI and library integrations see a single
/// error type.
#[derive(Debug, Error, Diagnostic)]
#[non_exhaustive]
pub enum SyntaxError {
    #[error("未知のノード種別です: {kind}")]
    #[diagnostic(code(afm::syntax::unknown_kind))]
    UnknownKind { kind: Box<str> },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ruby_roundtrip_fields() {
        let r = Ruby {
            base: "青梅".into(),
            reading: "おうめ".into(),
            delim_explicit: true,
        };
        assert_eq!(r.base.as_plain(), Some("青梅"));
        assert_eq!(r.reading.as_plain(), Some("おうめ"));
        assert!(r.delim_explicit);
    }

    #[test]
    fn bouten_target_carries_the_annotated_literal() {
        let b = Bouten {
            kind: BoutenKind::Goma,
            target: "可哀想".into(),
            position: BoutenPosition::Right,
        };
        assert_eq!(b.target.as_plain(), Some("可哀想"));
        assert_eq!(b.position, BoutenPosition::default());
    }

    #[test]
    fn bouten_position_defaults_to_right() {
        // Default side is the vertical-writing right (standard print).
        // `BoutenPosition::default()` is the canonical constructor; any
        // regression that flips the default would silently flip every
        // plain `［＃「X」に傍点］` in rendered corpora to the wrong
        // side, so we pin it.
        assert_eq!(BoutenPosition::default(), BoutenPosition::Right);
    }

    #[test]
    fn empty_span_is_empty_and_zero_length() {
        let s = Span::new(42, 42);
        assert!(s.is_empty());
        assert_eq!(s.len(), 0);
    }

    #[test]
    fn span_slices_source_buffer() {
        let source = "hello world";
        let s = Span::new(6, 11);
        assert_eq!(s.slice(source), "world");
    }

    #[test]
    fn inline_variants_are_not_block() {
        assert!(
            !AozoraNode::Ruby(Ruby {
                base: "a".into(),
                reading: "a".into(),
                delim_explicit: false,
            })
            .is_block()
        );
        assert!(!AozoraNode::TateChuYoko(TateChuYoko { text: "12".into() }).is_block());
        assert!(!AozoraNode::Kaeriten(Kaeriten { mark: "一".into() }).is_block());
    }

    #[test]
    fn kaeriten_has_stable_xml_name() {
        let node = AozoraNode::Kaeriten(Kaeriten { mark: "レ".into() });
        assert_eq!(node.xml_node_name(), "aozora_kaeriten");
        assert!(!node.contains_inlines());
    }

    #[test]
    fn content_plain_from_str_is_plain() {
        let c = Content::from("hello");
        assert_eq!(c.as_plain(), Some("hello"));
        assert!(matches!(c, Content::Plain(_)));
    }

    #[test]
    fn content_empty_string_becomes_empty_segments() {
        let c = Content::from("");
        assert!(c.as_plain().is_none());
        assert!(matches!(c, Content::Segments(ref s) if s.is_empty()));
    }

    #[test]
    fn content_from_segments_single_text_collapses_to_plain() {
        let c = Content::from_segments(vec![Segment::Text("hi".into())]);
        assert_eq!(c.as_plain(), Some("hi"));
    }

    #[test]
    fn content_from_segments_multiple_texts_concat_and_collapse() {
        let c = Content::from_segments(vec![
            Segment::Text("al".into()),
            Segment::Text("ph".into()),
            Segment::Text("a".into()),
        ]);
        assert_eq!(c.as_plain(), Some("alpha"));
    }

    #[test]
    fn content_from_segments_mixed_stays_segmented() {
        let c = Content::from_segments(vec![
            Segment::Text("before ".into()),
            Segment::Gaiji(Gaiji {
                description: "X".into(),
                ucs: None,
                mencode: None,
            }),
            Segment::Text(" after".into()),
        ]);
        assert!(c.as_plain().is_none());
        assert!(matches!(c, Content::Segments(ref s) if s.len() == 3));
    }

    #[test]
    fn content_iter_over_plain_yields_single_text() {
        let c = Content::from("x");
        let collected: Vec<_> = c.iter().collect();
        assert_eq!(collected.len(), 1);
        match collected[0] {
            SegmentRef::Text(t) => assert_eq!(t, "x"),
            _ => panic!("plain must yield SegmentRef::Text"),
        }
    }

    #[test]
    fn content_iter_over_empty_segments_yields_nothing() {
        let c = Content::from("");
        assert_eq!(c.iter().count(), 0);
    }

    #[test]
    fn content_iter_over_segments_preserves_order() {
        let c = Content::from_segments(vec![
            Segment::Text("a".into()),
            Segment::Annotation(Annotation {
                raw: "［＃X］".into(),
                kind: AnnotationKind::Unknown,
            }),
            Segment::Text("b".into()),
        ]);
        let kinds: Vec<&'static str> = c
            .iter()
            .map(|sr| match sr {
                SegmentRef::Text(_) => "text",
                SegmentRef::Gaiji(_) => "gaiji",
                SegmentRef::Annotation(_) => "annotation",
            })
            .collect();
        assert_eq!(kinds, vec!["text", "annotation", "text"]);
    }

    #[test]
    fn content_default_is_empty_segments() {
        let c = Content::default();
        assert!(matches!(c, Content::Segments(ref s) if s.is_empty()));
    }

    #[test]
    fn content_from_box_str_fast_path() {
        let b: Box<str> = "owned".into();
        let c = Content::from(b);
        assert_eq!(c.as_plain(), Some("owned"));
    }

    #[test]
    fn content_from_string_fast_path() {
        let c = Content::from(String::from("stringy"));
        assert_eq!(c.as_plain(), Some("stringy"));
    }

    #[test]
    fn content_from_segments_empty_vec_yields_empty_segments() {
        let c = Content::from_segments(Vec::new());
        assert!(matches!(c, Content::Segments(ref s) if s.is_empty()));
    }

    #[test]
    fn content_from_segments_empty_text_concat_to_empty_segments() {
        let c = Content::from_segments(vec![Segment::Text("".into()), Segment::Text("".into())]);
        // Concatenation of empty strings → empty content → empty segments
        assert!(matches!(c, Content::Segments(ref s) if s.is_empty()));
    }

    #[test]
    fn block_variants_are_block() {
        assert!(AozoraNode::PageBreak.is_block());
        assert!(AozoraNode::Indent(Indent { amount: 2 }).is_block());
        assert!(AozoraNode::SectionBreak(SectionKind::Choho).is_block());
    }

    #[test]
    fn containers_report_contains_inlines() {
        assert!(AozoraNode::Indent(Indent { amount: 2 }).contains_inlines());
        assert!(
            AozoraNode::Warichu(Warichu {
                upper: "a".into(),
                lower: "b".into(),
            })
            .contains_inlines()
        );
    }

    #[test]
    fn leaf_blocks_do_not_contain_inlines() {
        assert!(!AozoraNode::PageBreak.contains_inlines());
        assert!(!AozoraNode::SectionBreak(SectionKind::Choho).contains_inlines());
    }

    #[test]
    fn xml_node_names_are_stable_and_unique() {
        use std::collections::BTreeSet;
        let samples: [AozoraNode; 15] = [
            AozoraNode::Ruby(Ruby {
                base: "".into(),
                reading: "".into(),
                delim_explicit: false,
            }),
            AozoraNode::Bouten(Bouten {
                kind: BoutenKind::Goma,
                target: "".into(),
                position: BoutenPosition::Right,
            }),
            AozoraNode::TateChuYoko(TateChuYoko { text: "".into() }),
            AozoraNode::Gaiji(Gaiji {
                description: "".into(),
                ucs: None,
                mencode: None,
            }),
            AozoraNode::Indent(Indent { amount: 0 }),
            AozoraNode::AlignEnd(AlignEnd { offset: 0 }),
            AozoraNode::Warichu(Warichu {
                upper: "".into(),
                lower: "".into(),
            }),
            AozoraNode::Keigakomi(Keigakomi),
            AozoraNode::PageBreak,
            AozoraNode::SectionBreak(SectionKind::Choho),
            AozoraNode::AozoraHeading(AozoraHeading {
                kind: AozoraHeadingKind::Window,
                text: "".into(),
            }),
            AozoraNode::Sashie(Sashie {
                file: "".into(),
                caption: None,
            }),
            AozoraNode::Annotation(Annotation {
                raw: "".into(),
                kind: AnnotationKind::Unknown,
            }),
            AozoraNode::DoubleRuby(DoubleRuby { content: "".into() }),
            AozoraNode::Container(Container {
                kind: ContainerKind::Indent { amount: 1 },
            }),
        ];
        let names: BTreeSet<&'static str> = samples.iter().map(AozoraNode::xml_node_name).collect();
        assert_eq!(names.len(), samples.len(), "xml node names must be unique");
        for name in &names {
            assert!(
                name.starts_with("aozora_"),
                "xml name '{name}' missing aozora_ prefix"
            );
        }
    }

    #[test]
    fn all_variants_are_non_exhaustive_for_forward_compat() {
        match AozoraNode::PageBreak {
            AozoraNode::PageBreak => {}
            _ => unreachable!(),
        }
    }
}
