/// Node kind emitted in parser projections and wire formats.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum NodeKind {
    /// Ruby annotation `｜base《reading》`.
    Ruby,
    /// Emphasis dots or lines on a text span.
    Bouten,
    /// Horizontal text within a vertical line.
    CombineUpright,
    /// External character reference.
    Gaiji,
    /// Line indentation marker.
    Indent,
    /// Alignment from the end of a line.
    AlignEnd,
    /// Center alignment marker.
    Center,
    /// Gothic font marker for a line.
    LineGothic,
    /// Absolute font-size marker for a line.
    LineFontSize,
    /// Page break.
    PageBreak,
    /// Section break.
    SectionBreak,
    /// End of the main body before the colophon.
    BodyEnd,
    /// Forced line break.
    ForcedBreak,
    /// Aozora heading.
    Heading,
    /// Hint for heading presentation.
    HeadingHint,
    /// Illustration reference.
    Illustration,
    /// Kanbun return mark.
    Kaeriten,
    /// Annotation without a more specific semantic kind.
    Directive,
    /// Quotation with source delimiters `≪…≫` and display delimiters `《…》`.
    AngleQuote,
    /// Styling directive for a preceding text span.
    Emphasis,
    /// Side annotation on a preceding text span.
    MarginNote,
    /// Opening marker of a paired container.
    ContainerOpen,
    /// Closing marker of a paired container.
    ContainerClose,
}

impl NodeKind {
    /// Every variant in declaration order.
    pub const ALL: [Self; 23] = [
        Self::Ruby,
        Self::Bouten,
        Self::CombineUpright,
        Self::Gaiji,
        Self::Indent,
        Self::AlignEnd,
        Self::Center,
        Self::LineGothic,
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
        Self::ContainerOpen,
        Self::ContainerClose,
    ];

    /// Stable `camelCase` identifier for this kind.
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
            Self::LineGothic => "lineGothic",
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
            Self::ContainerOpen => "containerOpen",
            Self::ContainerClose => "containerClose",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn camel_case_strings_are_stable() {
        assert_eq!(NodeKind::Ruby.as_json_tag(), "ruby");
        assert_eq!(NodeKind::Bouten.as_json_tag(), "bouten");
        assert_eq!(NodeKind::CombineUpright.as_json_tag(), "combineUpright");
        assert_eq!(NodeKind::Gaiji.as_json_tag(), "gaiji");
        assert_eq!(NodeKind::Indent.as_json_tag(), "indent");
        assert_eq!(NodeKind::AlignEnd.as_json_tag(), "alignEnd");
        assert_eq!(NodeKind::Center.as_json_tag(), "center");
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
        assert_eq!(NodeKind::ContainerOpen.as_json_tag(), "containerOpen");
        assert_eq!(NodeKind::ContainerClose.as_json_tag(), "containerClose");
    }
}
