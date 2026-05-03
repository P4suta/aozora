//! Cross-cutting "kind" tag for AST nodes.
//!
//! [`NodeKind`] enumerates every wire-distinct tag the borrowed-AST
//! surfaces produce. It is used both for **internal** projection
//! ([`AozoraNode::kind`](crate::borrowed::AozoraNode::kind),
//! [`NodeRef::kind`](crate::borrowed::NodeRef::kind)) and for the
//! **driver wire format** ([`crate`]'s host crate `aozora` projects
//! the tag to a stable camelCase string via [`NodeKind::as_camel_case`]).
//!
//! The typed enum (rather than a `&'static str` constant) lets every
//! consumer pattern-match the tag exhaustively — the compiler points
//! out a new variant landing without a wire mapping — and concentrates
//! the camelCase string in a single authority.

/// Cross-cutting tag for an AST node or `NodeRef` projection.
///
/// The first 17 variants ([`Self::Ruby`] through [`Self::Container`])
/// project from [`crate::borrowed::AozoraNode`]'s discriminant. The
/// final two ([`Self::ContainerOpen`] / [`Self::ContainerClose`])
/// only arise from [`crate::borrowed::NodeRef`]'s container open /
/// close variants — the inline `Container` payload uses
/// [`Self::Container`].
///
/// `#[non_exhaustive]` so adding a new `AozoraNode` variant only needs
/// to land here and on the per-call `match` sites; existing wire
/// consumers see the new variant as an unrecognised tag and gracefully
/// degrade (the camelCase mapping is exhaustive within this crate;
/// downstream `match` over `NodeKind` is required to handle a `_` arm
/// for forward-compat).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum NodeKind {
    /// Ruby annotation (`｜base《reading》`).
    Ruby,
    /// Bouten (傍点) — emphasis dots over a span.
    Bouten,
    /// 縦中横 (tate-chu-yoko) — horizontal text inside vertical run.
    TateChuYoko,
    /// 外字 (gaiji) — non-Unicode character reference.
    Gaiji,
    /// Inline indent (字下げ) marker.
    Indent,
    /// Right-edge alignment (字上げ) marker.
    AlignEnd,
    /// 割注 (warichu) — split-line annotation.
    Warichu,
    /// 罫囲み (keigakomi) — ruled box.
    Keigakomi,
    /// 改ページ (page break).
    PageBreak,
    /// Section break (大見出し系統合).
    SectionBreak,
    /// Aozora heading (見出し).
    AozoraHeading,
    /// Heading hint that informs downstream rendering decisions.
    HeadingHint,
    /// 挿絵 (sashie) — illustration reference.
    Sashie,
    /// 返り点 (kaeriten) — kanbun reading marker.
    Kaeriten,
    /// Generic annotation that no specific recogniser claimed.
    Annotation,
    /// Double ruby (《《…》》).
    DoubleRuby,
    /// Inline-attached container (字下げ系の `AozoraNode` 包み込み).
    Container,
    /// `NodeRef::BlockOpen` projection — paired-container open
    /// sentinel position.
    ContainerOpen,
    /// `NodeRef::BlockClose` projection — paired-container close
    /// sentinel position.
    ContainerClose,
}

impl NodeKind {
    /// Every variant in declaration order.
    ///
    /// Used by `aozora kinds` (CLI introspection) and the
    /// TypeScript / JSON-Schema codegen so the artefact list
    /// tracks the enum without a hand-maintained parallel.
    pub const ALL: [Self; 19] = [
        Self::Ruby,
        Self::Bouten,
        Self::TateChuYoko,
        Self::Gaiji,
        Self::Indent,
        Self::AlignEnd,
        Self::Warichu,
        Self::Keigakomi,
        Self::PageBreak,
        Self::SectionBreak,
        Self::AozoraHeading,
        Self::HeadingHint,
        Self::Sashie,
        Self::Kaeriten,
        Self::Annotation,
        Self::DoubleRuby,
        Self::Container,
        Self::ContainerOpen,
        Self::ContainerClose,
    ];

    /// Stable camelCase string identifier for this kind.
    ///
    /// Driver crates (`aozora-ffi` / `aozora-wasm` / `aozora-py`) all
    /// emit JSON whose `kind` field equals this string verbatim, so
    /// downstream TypeScript / Python / C consumers can switch on the
    /// tag without consulting an out-of-band table.
    #[must_use]
    pub const fn as_camel_case(self) -> &'static str {
        match self {
            Self::Ruby => "ruby",
            Self::Bouten => "bouten",
            Self::TateChuYoko => "tateChuYoko",
            Self::Gaiji => "gaiji",
            Self::Indent => "indent",
            Self::AlignEnd => "alignEnd",
            Self::Warichu => "warichu",
            Self::Keigakomi => "keigakomi",
            Self::PageBreak => "pageBreak",
            Self::SectionBreak => "sectionBreak",
            Self::AozoraHeading => "heading",
            Self::HeadingHint => "headingHint",
            Self::Sashie => "sashie",
            Self::Kaeriten => "kaeriten",
            Self::Annotation => "annotation",
            Self::DoubleRuby => "doubleRuby",
            Self::Container => "container",
            Self::ContainerOpen => "containerOpen",
            Self::ContainerClose => "containerClose",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// camelCase strings are pinned — accidental rename of one breaks
    /// this test instead of silently breaking downstream tooling that
    /// switches on the tag.
    #[test]
    fn camel_case_strings_are_stable() {
        assert_eq!(NodeKind::Ruby.as_camel_case(), "ruby");
        assert_eq!(NodeKind::Bouten.as_camel_case(), "bouten");
        assert_eq!(NodeKind::TateChuYoko.as_camel_case(), "tateChuYoko");
        assert_eq!(NodeKind::Gaiji.as_camel_case(), "gaiji");
        assert_eq!(NodeKind::Indent.as_camel_case(), "indent");
        assert_eq!(NodeKind::AlignEnd.as_camel_case(), "alignEnd");
        assert_eq!(NodeKind::Warichu.as_camel_case(), "warichu");
        assert_eq!(NodeKind::Keigakomi.as_camel_case(), "keigakomi");
        assert_eq!(NodeKind::PageBreak.as_camel_case(), "pageBreak");
        assert_eq!(NodeKind::SectionBreak.as_camel_case(), "sectionBreak");
        assert_eq!(NodeKind::AozoraHeading.as_camel_case(), "heading");
        assert_eq!(NodeKind::HeadingHint.as_camel_case(), "headingHint");
        assert_eq!(NodeKind::Sashie.as_camel_case(), "sashie");
        assert_eq!(NodeKind::Kaeriten.as_camel_case(), "kaeriten");
        assert_eq!(NodeKind::Annotation.as_camel_case(), "annotation");
        assert_eq!(NodeKind::DoubleRuby.as_camel_case(), "doubleRuby");
        assert_eq!(NodeKind::Container.as_camel_case(), "container");
        assert_eq!(NodeKind::ContainerOpen.as_camel_case(), "containerOpen");
        assert_eq!(NodeKind::ContainerClose.as_camel_case(), "containerClose");
    }
}
