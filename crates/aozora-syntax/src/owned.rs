//! Owned-AST type definitions (legacy / lex-internal).
//!
//! As of Plan B, the canonical user-facing AST is the borrowed-AST
//! defined in [`crate::borrowed`]. The owned types here remain only
//! as the lex pipeline's internal output shape:
//! `aozora-lexer`'s Phase 3 classifier produces these `Box`-allocated
//! nodes; `aozora_syntax::convert::to_borrowed_with` then converts
//! them into the arena-backed borrowed shape.
//!
//! New consumers SHOULD use `borrowed::AozoraNode<'_>` via the
//! `aozora::Document::parse` entry rather than constructing or
//! matching on these owned types directly.

use core::slice;

use crate::{
    AlignEnd, AnnotationKind, AozoraHeadingKind, BoutenKind, BoutenPosition, Container, Indent,
    Keigakomi, SectionKind,
};

/// Every Aozora-specific AST node (owned variant). Embedded into
/// comrak's `NodeValue` tree as a single `NodeValue::Aozora(AozoraNode)`
/// variant so the upstream diff stays at one line.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub enum AozoraNode {
    /// Ruby (furigana). Ex: `｜青梅《おうめ》` or `日本《にほん》`.
    Ruby(Ruby),
    /// Emphasis dots / sidelines. Ex: `［＃「X」に傍点］`.
    Bouten(Bouten),
    /// Tate-chu-yoko (horizontal embedding inside vertical text).
    TateChuYoko(TateChuYoko),
    /// Gaiji (out-of-range character).
    Gaiji(Gaiji),
    /// `［＃ここから字下げ］ ... ［＃ここで字下げ終わり］` block.
    Indent(Indent),
    /// `［＃地付き］` / `［＃地から2字上げ］` block.
    AlignEnd(AlignEnd),
    /// Split annotation (`割り注`).
    Warichu(Warichu),
    /// Boxed block (`罫囲み`).
    Keigakomi(Keigakomi),
    /// Page break.
    PageBreak,
    /// Section break.
    SectionBreak(SectionKind),
    /// Aozora-specific heading (窓見出し, 副見出し).
    AozoraHeading(AozoraHeading),
    /// Heading-hint marker.
    HeadingHint(HeadingHint),
    /// Illustration (`［＃挿絵（fig.png）入る］`).
    Sashie(Sashie),
    /// Chinese-reading order mark.
    Kaeriten(Kaeriten),
    /// Generic annotation when no more specific recogniser matched.
    Annotation(Annotation),
    /// Double angle-bracket notation — `《《X》》`.
    DoubleRuby(DoubleRuby),
    /// Paired block container.
    Container(Container),
}

impl AozoraNode {
    /// Whether this node occupies a block (paragraph-level) position.
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

    /// Whether children of this node (if any) are inline content.
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

    /// Stable XML node name used by snapshots / pretty-printers.
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
            Self::HeadingHint(_) => "aozora_heading_hint",
            Self::Sashie(_) => "aozora_sashie",
            Self::Kaeriten(_) => "aozora_kaeriten",
            Self::Annotation(_) => "aozora_annotation",
            Self::DoubleRuby(_) => "aozora_double_ruby",
            Self::Container(_) => "aozora_container",
        }
    }
}

/// `《《X》》` payload.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct DoubleRuby {
    pub content: Content,
}

/// Body-content type for nodes whose textual payload may contain
/// nested Aozora constructs.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub enum Content {
    Plain(Box<str>),
    Segments(Box<[Segment]>),
}

impl Content {
    /// Construct from an arbitrary segment list. Empty input →
    /// `Segments([])`; all-text input collapses into a single `Plain`.
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

    /// Plain-text fast path: returns `Some` for `Plain`, `None` for `Segments`.
    #[must_use]
    pub fn as_plain(&self) -> Option<&str> {
        match self {
            Self::Plain(s) => Some(s),
            Self::Segments(_) => None,
        }
    }

    /// Iterate segments left-to-right.
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
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub enum Segment {
    Text(Box<str>),
    Gaiji(Gaiji),
    Annotation(Annotation),
}

/// Borrowed view yielded by [`Content::iter`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum SegmentRef<'a> {
    Text(&'a str),
    Gaiji(&'a Gaiji),
    Annotation(&'a Annotation),
}

/// Iterator returned by [`Content::iter`].
#[derive(Debug)]
pub enum ContentIter<'a> {
    Plain(Option<&'a str>),
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
    pub base: Content,
    pub reading: Content,
    pub delim_explicit: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Bouten {
    pub kind: BoutenKind,
    pub target: Content,
    pub position: BoutenPosition,
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct TateChuYoko {
    pub text: Content,
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Gaiji {
    pub description: Box<str>,
    pub ucs: Option<char>,
    pub mencode: Option<Box<str>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Warichu {
    pub upper: Content,
    pub lower: Content,
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct AozoraHeading {
    pub kind: AozoraHeadingKind,
    pub text: Content,
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct HeadingHint {
    pub level: u8,
    pub target: Box<str>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Sashie {
    pub file: Box<str>,
    pub caption: Option<Content>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Annotation {
    pub raw: Box<str>,
    pub kind: AnnotationKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Kaeriten {
    pub mark: Box<str>,
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
    fn content_from_segments_collapses_to_plain() {
        let c = Content::from_segments(vec![Segment::Text("hi".into())]);
        assert_eq!(c.as_plain(), Some("hi"));
    }

    #[test]
    fn content_iter_yields_synthesised_text_for_plain() {
        let c = Content::from("x");
        let v: Vec<_> = c.iter().collect();
        assert_eq!(v.len(), 1);
        assert!(matches!(v[0], SegmentRef::Text("x")));
    }

    #[test]
    fn xml_node_names_share_prefix() {
        assert!(AozoraNode::PageBreak.xml_node_name().starts_with("aozora_"));
    }
}
