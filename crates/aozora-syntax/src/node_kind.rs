//! Cross-cutting "kind" tag for AST nodes.
//!
//! [`NodeKind`] enumerates every wire-distinct tag the owned-AST
//! surfaces produce. It is used both for **internal** projection
//! (`Node::kind`,
//! `NodeRef::kind`) and for the
//! **driver wire format** ([`crate`]'s host crate `aozora` projects
//! the tag to a stable camelCase string via [`NodeKind::as_json_tag`]).
//!
//! The typed enum (rather than a `&'static str` constant) lets every
//! consumer pattern-match the tag exhaustively — the compiler points
//! out a new variant landing without a wire mapping — and concentrates
//! the camelCase string in a single authority.

/// Cross-cutting tag for an AST node or `NodeRef` projection.
///
/// The first 18 variants ([`Self::Ruby`] through [`Self::Container`])
/// project from `NodeOwned`'s discriminant. The
/// final two ([`Self::ContainerOpen`] / [`Self::ContainerClose`])
/// only arise from `NodeRefOwned`'s container open /
/// close variants — the inline `Container` payload uses
/// [`Self::Container`].
///
/// `#[non_exhaustive]` so adding a new `Node` variant only needs
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
    CombineUpright,
    /// 外字 (gaiji) — non-Unicode character reference.
    Gaiji,
    /// Inline indent (字下げ) marker.
    Indent,
    /// Right-edge alignment (字上げ) marker.
    AlignEnd,
    /// Centring (中央) marker (`ページの左右中央` / `中央揃え`).
    Center,
    /// 割注 (warichu) — split-line annotation.
    Warichu,
    /// 罫囲み (keigakomi) — ruled box.
    Framed,
    /// 太字 line marker (`この行はゴシック体`) — bolds the line it sits on.
    LineBold,
    /// Absolute font-size line marker (`大文字` … `特大文字、太字`) — sizes the
    /// line it sits on.
    LineFontSize,
    /// 改ページ (page break).
    PageBreak,
    /// Section break (大見出し系統合).
    SectionBreak,
    /// 本文終わり (body-end marker) — main body ends, colophon follows.
    BodyEnd,
    /// 改行 (forced line break) — in-paragraph `<br />`.
    ForcedBreak,
    /// Aozora heading (見出し).
    Heading,
    /// Heading hint that informs downstream rendering decisions.
    HeadingHint,
    /// 挿絵 (sashie) — illustration reference.
    Illustration,
    /// 返り点 (kaeriten) — kanbun reading marker.
    Kaeriten,
    /// Generic annotation that no specific recogniser claimed.
    Directive,
    /// Double-angle quotation (input `≪…≫`, display `《…》`).
    AngleQuote,
    /// 太字 / 斜体 (bold / italic) — forward-reference emphasis leaf.
    Emphasis,
    /// Side annotation (注記) — `「X」の左に「Y」の注記`.
    MarginNote,
    /// Inline-attached container (字下げ系の `Node` 包み込み).
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
    pub const ALL: [Self; 26] = [
        Self::Ruby,
        Self::Bouten,
        Self::CombineUpright,
        Self::Gaiji,
        Self::Indent,
        Self::AlignEnd,
        Self::Center,
        Self::Warichu,
        Self::Framed,
        Self::LineBold,
        Self::LineFontSize,
        Self::PageBreak,
        Self::SectionBreak,
        Self::BodyEnd,
        Self::ForcedBreak,
        Self::Heading,
        Self::HeadingHint,
        Self::Illustration,
        Self::Kaeriten,
        Self::Directive,
        Self::AngleQuote,
        Self::Emphasis,
        Self::MarginNote,
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
    pub const fn as_json_tag(self) -> &'static str {
        match self {
            Self::Ruby => "ruby",
            Self::Bouten => "bouten",
            Self::CombineUpright => "combineUpright",
            Self::Gaiji => "gaiji",
            Self::Indent => "indent",
            Self::AlignEnd => "alignEnd",
            Self::Center => "center",
            Self::Warichu => "warichu",
            Self::Framed => "framed",
            Self::LineBold => "lineBold",
            Self::LineFontSize => "lineFontSize",
            Self::PageBreak => "pageBreak",
            Self::SectionBreak => "sectionBreak",
            Self::BodyEnd => "bodyEnd",
            Self::ForcedBreak => "forcedBreak",
            Self::Heading => "heading",
            Self::HeadingHint => "headingHint",
            Self::Illustration => "illustration",
            Self::Kaeriten => "kaeriten",
            Self::Directive => "directive",
            Self::AngleQuote => "angleQuote",
            Self::Emphasis => "emphasis",
            Self::MarginNote => "marginNote",
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
        assert_eq!(NodeKind::Ruby.as_json_tag(), "ruby");
        assert_eq!(NodeKind::Bouten.as_json_tag(), "bouten");
        assert_eq!(NodeKind::CombineUpright.as_json_tag(), "combineUpright");
        assert_eq!(NodeKind::Gaiji.as_json_tag(), "gaiji");
        assert_eq!(NodeKind::Indent.as_json_tag(), "indent");
        assert_eq!(NodeKind::AlignEnd.as_json_tag(), "alignEnd");
        assert_eq!(NodeKind::Center.as_json_tag(), "center");
        assert_eq!(NodeKind::Warichu.as_json_tag(), "warichu");
        assert_eq!(NodeKind::Framed.as_json_tag(), "framed");
        assert_eq!(NodeKind::PageBreak.as_json_tag(), "pageBreak");
        assert_eq!(NodeKind::SectionBreak.as_json_tag(), "sectionBreak");
        assert_eq!(NodeKind::BodyEnd.as_json_tag(), "bodyEnd");
        assert_eq!(NodeKind::ForcedBreak.as_json_tag(), "forcedBreak");
        assert_eq!(NodeKind::Heading.as_json_tag(), "heading");
        assert_eq!(NodeKind::HeadingHint.as_json_tag(), "headingHint");
        assert_eq!(NodeKind::Illustration.as_json_tag(), "illustration");
        assert_eq!(NodeKind::Kaeriten.as_json_tag(), "kaeriten");
        assert_eq!(NodeKind::Directive.as_json_tag(), "directive");
        assert_eq!(NodeKind::AngleQuote.as_json_tag(), "angleQuote");
        assert_eq!(NodeKind::Emphasis.as_json_tag(), "emphasis");
        assert_eq!(NodeKind::MarginNote.as_json_tag(), "marginNote");
        assert_eq!(NodeKind::Container.as_json_tag(), "container");
        assert_eq!(NodeKind::ContainerOpen.as_json_tag(), "containerOpen");
        assert_eq!(NodeKind::ContainerClose.as_json_tag(), "containerClose");
    }
}
