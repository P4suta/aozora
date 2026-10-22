#[cfg(any(feature = "pandoc", test))]
use core::fmt;

use crate::encoding::gaiji::{GaijiCanonical, MenKuTen, Resolved};

use crate::syntax::format::{ForwardAttr, ForwardOrigin, LineFormat};
use crate::syntax::{
    DirectiveKind, HeadingKind, HeadingStyle, MarginNoteKind, NodeKind, RubyBaseClass, RubySide,
    SectionKind, ruby_base_class,
};

use super::intern::StrId;
use super::store::{ContentRange, NodeStore, SegRange};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub(crate) enum Content {
    Plain(StrId),
    Segments(SegRange),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub(crate) enum Segment {
    Text(StrId),
    Gaiji(Gaiji),
    Directive(Directive),
    Node(Node),
}

pub(crate) const fn node_is_content_segment(node: Node) -> bool {
    matches!(
        node,
        Node::Format(_)
            | Node::HeadingHint(_)
            | Node::Illustration(_)
            | Node::Kaeriten(_)
            | Node::AngleQuote(_)
            | Node::MarginNote(_)
    )
}

pub(crate) fn content_ruby_base_class(
    contents: &[Content],
    store: &NodeStore,
) -> Option<RubyBaseClass> {
    let mut class = None;
    for content in contents {
        match *content {
            Content::Plain(id) => absorb_text_class(store.resolve_str(id), &mut class)?,
            Content::Segments(range) => {
                for segment in store.resolve_seg_range(range) {
                    match *segment {
                        Segment::Text(id) => absorb_text_class(store.resolve_str(id), &mut class)?,
                        Segment::Gaiji(gaiji) => {
                            absorb_class(gaiji_ruby_base_class(gaiji, store)?, &mut class)?;
                        }
                        Segment::Directive(_) | Segment::Node(_) => return None,
                    }
                }
            }
        }
    }
    class
}

pub(crate) fn gaiji_ruby_base_class(gaiji: Gaiji, store: &NodeStore) -> Option<RubyBaseClass> {
    match gaiji.resolve(store)? {
        Resolved::Char(ch) => ruby_base_class(ch),
        Resolved::Multi(text) => {
            let mut class = None;
            for ch in text.chars().filter_map(ruby_base_class) {
                absorb_class(ch, &mut class)?;
            }
            class
        }
    }
}

fn absorb_text_class(text: &str, class: &mut Option<RubyBaseClass>) -> Option<()> {
    for ch in text.chars() {
        absorb_class(ruby_base_class(ch)?, class)?;
    }
    Some(())
}

fn absorb_class(candidate: RubyBaseClass, class: &mut Option<RubyBaseClass>) -> Option<()> {
    match class {
        Some(current) if *current != candidate => None,
        Some(_) => Some(()),
        slot @ None => {
            *slot = Some(candidate);
            Some(())
        }
    }
}

impl GaijiCanonicalOwned {
    fn to_canonical(self, store: &NodeStore) -> GaijiCanonical<'_> {
        match self {
            Self::MenKuTen(m) => GaijiCanonical::MenKuTen(m),
            Self::Unicode(c) => GaijiCanonical::Unicode(c),
            Self::Unresolved { mencode } => GaijiCanonical::Unresolved {
                mencode: mencode.map(|id| store.resolve_str(id)),
            },
        }
    }

    #[must_use]
    pub(crate) fn has_mencode(self) -> bool {
        !matches!(self, Self::Unresolved { mencode: None })
    }

    #[cfg(any(feature = "pandoc", test))]
    pub(crate) fn write_mencode<W: fmt::Write>(self, store: &NodeStore, w: &mut W) -> fmt::Result {
        self.to_canonical(store).write_mencode(w)
    }
}

impl Gaiji {
    #[must_use]
    pub(crate) fn resolve(&self, store: &NodeStore) -> Option<Resolved> {
        self.canonical
            .to_canonical(store)
            .resolve(store.resolve_str(self.hint))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Ruby {
    pub base: ContentRange,
    pub reading: ContentRange,
    pub side: RubySide,
    pub base_emphasis: Option<ForwardAttr>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct MarginNote {
    pub kind: MarginNoteKind,
    pub base: ContentRange,
    pub note: ContentRange,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ForwardPayload {
    None,
    NestedSource,
    AccentBody(StrId),
    QuotedTarget(StrId),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ForwardFormat {
    pub attr: ForwardAttr,
    pub target: ContentRange,
    pub origin: ForwardOrigin,
    pub payload: ForwardPayload,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum GaijiCanonicalOwned {
    MenKuTen(MenKuTen),
    Unicode(char),
    Unresolved { mencode: Option<StrId> },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Gaiji {
    pub hint: StrId,
    pub canonical: GaijiCanonicalOwned,
    pub mencode_separator: bool,
    pub standalone: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Heading {
    pub kind: HeadingKind,
    pub style: HeadingStyle,
    pub text: ContentRange,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct HeadingHint {
    pub level: HeadingKind,
    pub style: HeadingStyle,
    pub target: StrId,
    pub self_contained: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Illustration {
    pub file: StrId,
    pub number: Option<StrId>,
    pub dimensions: Option<StrId>,
    pub caption: Option<Content>,
    pub description: Option<StrId>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Directive {
    pub raw: StrId,
    pub kind: DirectiveKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Kaeriten {
    pub mark: StrId,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct AngleQuote {
    pub content: ContentRange,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub(crate) enum Node {
    Ruby(Ruby),
    Format(ForwardFormat),
    Gaiji(Gaiji),
    Line(LineFormat),
    PageBreak,
    SectionBreak(SectionKind),
    BodyEnd,
    ForcedBreak,
    Heading(Heading),
    HeadingHint(HeadingHint),
    Illustration(Illustration),
    Kaeriten(Kaeriten),
    Directive(Directive),
    AngleQuote(AngleQuote),
    MarginNote(MarginNote),
}

impl Node {
    #[must_use]
    pub(crate) const fn kind(self) -> NodeKind {
        use crate::syntax::NodeKind;
        match self {
            Self::Ruby(_) => NodeKind::Ruby,
            Self::Format(f) => match f.attr {
                ForwardAttr::Bouten { .. } => NodeKind::Bouten,
                ForwardAttr::CombineUpright => NodeKind::CombineUpright,
                _ => NodeKind::Emphasis,
            },
            Self::Gaiji(_) => NodeKind::Gaiji,
            Self::Line(l) => match l {
                LineFormat::Indent { .. } => NodeKind::Indent,
                LineFormat::AlignEnd { .. } => NodeKind::AlignEnd,
                LineFormat::Center { .. } => NodeKind::Center,
                LineFormat::Gothic => NodeKind::LineGothic,
                LineFormat::FontSizeAbsolute { .. } => NodeKind::LineFontSize,
            },
            Self::PageBreak => NodeKind::PageBreak,
            Self::SectionBreak(_) => NodeKind::SectionBreak,
            Self::BodyEnd => NodeKind::BodyEnd,
            Self::ForcedBreak => NodeKind::ForcedBreak,
            Self::Heading(_) => NodeKind::Heading,
            Self::HeadingHint(_) => NodeKind::HeadingHint,
            Self::Illustration(_) => NodeKind::Illustration,
            Self::Kaeriten(_) => NodeKind::Kaeriten,
            Self::Directive(_) => NodeKind::Directive,
            Self::AngleQuote(_) => NodeKind::AngleQuote,
            Self::MarginNote(_) => NodeKind::MarginNote,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn payloads_are_copy() {
        const fn assert_copy<T: Copy>() {}
        assert_copy::<Content>();
        assert_copy::<Segment>();
        assert_copy::<Node>();
        assert_copy::<Ruby>();
        assert_copy::<Gaiji>();
    }

    #[test]
    fn node_kind_tag_matches_variant() {
        assert_eq!(Node::PageBreak.kind(), NodeKind::PageBreak);
        assert_eq!(
            Node::SectionBreak(SectionKind::Kaicho).kind(),
            NodeKind::SectionBreak
        );
    }

    #[test]
    fn gaiji_canonical_owned_has_mencode_tracks_the_tail() {
        let mut store = NodeStore::new();
        let id = store.intern("第3水準1-85-54");
        assert!(
            GaijiCanonicalOwned::MenKuTen(MenKuTen {
                plane: 1,
                ku: 85,
                ten: 54
            })
            .has_mencode()
        );
        assert!(GaijiCanonicalOwned::Unicode('竜').has_mencode());
        assert!(GaijiCanonicalOwned::Unresolved { mencode: Some(id) }.has_mencode());
        assert!(
            !GaijiCanonicalOwned::Unresolved { mencode: None }.has_mencode(),
            "no tail means has_mencode is false"
        );
    }

    #[test]
    fn gaiji_canonical_owned_write_mencode_emits_the_token() {
        let store = NodeStore::new();
        let mut buf = String::new();
        GaijiCanonicalOwned::MenKuTen(MenKuTen {
            plane: 1,
            ku: 85,
            ten: 54,
        })
        .write_mencode(&store, &mut buf)
        .unwrap();
        assert_eq!(buf, "第3水準1-85-54");

        let mut buf = String::new();
        GaijiCanonicalOwned::Unicode('A')
            .write_mencode(&store, &mut buf)
            .unwrap();
        assert_eq!(buf, "U+0041");
    }

    #[test]
    fn node_kind_distinguishes_forward_attrs() {
        use crate::syntax::alloc::Allocator;
        use crate::syntax::format::ForwardOrigin;
        use crate::syntax::{BoutenKind, BoutenPosition};

        let mut a = Allocator::new();
        let t = a.content_plain("青");
        let bouten = a.bouten(
            BoutenKind::Goma,
            t,
            BoutenPosition::Right,
            ForwardOrigin::Referenced,
        );
        assert_eq!(bouten.kind(), NodeKind::Bouten);

        let t2 = a.content_plain("12");
        let tcy = a.tate_chu_yoko(t2, ForwardOrigin::Referenced);
        assert_eq!(tcy.kind(), NodeKind::CombineUpright);

        let t3 = a.content_plain("重");
        let bold = a.forward_format(ForwardAttr::Bold, t3, ForwardOrigin::Reclaimed);
        assert_eq!(bold.kind(), NodeKind::Emphasis);
    }

    #[test]
    fn gaiji_resolve_matches_canonical_authority() {
        let mut store = NodeStore::new();
        for (hint, mencode) in [
            ("竜", Some("U+9F8D")),
            ("熙", Some("第3水準1-14-29")),
            ("謎の字", Some("未知の注記")),
            ("謎", None),
        ] {
            let canonical = GaijiCanonical::from_mencode(mencode);
            let hint_id = store.intern(hint);
            let owned_canonical = match canonical {
                GaijiCanonical::MenKuTen(m) => GaijiCanonicalOwned::MenKuTen(m),
                GaijiCanonical::Unicode(c) => GaijiCanonicalOwned::Unicode(c),
                GaijiCanonical::Unresolved { mencode } => GaijiCanonicalOwned::Unresolved {
                    mencode: mencode.map(|m| store.intern(m)),
                },
            };
            let owned = Gaiji {
                hint: hint_id,
                canonical: owned_canonical,
                mencode_separator: true,
                standalone: false,
            };
            assert_eq!(
                owned.resolve(&store),
                canonical.resolve(hint),
                "owned gaiji resolve diverged for hint={hint:?} mencode={mencode:?}",
            );
        }
    }
}
