use crate::spec::Span;

/// Balanced delimiter family in Aozora notation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "json", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub enum PairKind {
    /// Directive body delimiters (`［ … ］`).
    Bracket,

    /// Ruby reading delimiters (`《 … 》`).
    Ruby,

    /// Double-angle quotation delimiters (`≪ … ≫`), displayed as `《…》`.
    AngleQuote,

    /// Accent-decomposition delimiters (`〔 … 〕`).
    Tortoise,

    /// Quoted literal delimiters inside annotations (`「 … 」`).
    Quote,
}

impl PairKind {
    /// Every variant in declaration order.
    pub const ALL: &'static [Self] = &[
        Self::Bracket,
        Self::Ruby,
        Self::AngleQuote,
        Self::Tortoise,
        Self::Quote,
    ];

    /// Stable camelCase wire-format identifier.
    #[must_use]
    pub const fn as_json_tag(self) -> &'static str {
        match self {
            Self::Bracket => "bracket",
            Self::Ruby => "ruby",
            Self::AngleQuote => "angleQuote",
            Self::Tortoise => "tortoise",
            Self::Quote => "quote",
        }
    }

    /// Opening delimiter literal.
    #[must_use]
    pub const fn open_str(self) -> &'static str {
        match self {
            Self::Bracket => "［",
            Self::Ruby => "《",
            Self::AngleQuote => "≪",
            Self::Tortoise => "〔",
            Self::Quote => "「",
        }
    }

    /// Closing delimiter literal.
    #[must_use]
    pub const fn close_str(self) -> &'static str {
        match self {
            Self::Bracket => "］",
            Self::Ruby => "》",
            Self::AngleQuote => "≫",
            Self::Tortoise => "〕",
            Self::Quote => "」",
        }
    }
}

/// Matched delimiters with byte spans in the original source.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "json", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub struct PairLink {
    /// Which delimiter family this pair belongs to.
    pub kind: PairKind,
    /// Byte-span of the open delimiter in the original source.
    pub open: Span,
    /// Byte-span of the close delimiter in the original source.
    pub close: Span,
}

impl PairLink {
    /// Construct a link without checking whether the delimiters match.
    #[must_use]
    pub const fn new(kind: PairKind, open: Span, close: Span) -> Self {
        Self { kind, open, close }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pair_kind_is_copy() {
        let k = PairKind::Bracket;
        let copy = k;
        assert_eq!(k, copy);
    }

    #[test]
    fn pair_kind_variants_are_distinct() {
        let variants = [
            PairKind::Bracket,
            PairKind::Ruby,
            PairKind::AngleQuote,
            PairKind::Tortoise,
            PairKind::Quote,
        ];
        for (i, a) in variants.iter().enumerate() {
            for b in &variants[i + 1..] {
                assert_ne!(a, b);
            }
        }
    }

    #[test]
    fn as_json_tag_is_stable_per_pair_kind() {
        let cases = [
            (PairKind::Bracket, "bracket"),
            (PairKind::Ruby, "ruby"),
            (PairKind::AngleQuote, "angleQuote"),
            (PairKind::Tortoise, "tortoise"),
            (PairKind::Quote, "quote"),
        ];
        for (kind, tag) in cases {
            assert_eq!(kind.as_json_tag(), tag, "as_json_tag for {kind:?}");
        }
    }

    #[test]
    fn open_and_close_str_cover_every_pair_kind() {
        let cases = [
            (PairKind::Bracket, "［", "］"),
            (PairKind::Ruby, "《", "》"),
            (PairKind::AngleQuote, "≪", "≫"),
            (PairKind::Tortoise, "〔", "〕"),
            (PairKind::Quote, "「", "」"),
        ];
        for (kind, open, close) in cases {
            assert_eq!(kind.open_str(), open, "open_str for {kind:?}");
            assert_eq!(kind.close_str(), close, "close_str for {kind:?}");
        }
    }

    #[test]
    fn pair_link_records_kind_and_endpoints() {
        let link = PairLink::new(PairKind::Bracket, Span::new(0, 3), Span::new(10, 13));
        assert_eq!(link.kind, PairKind::Bracket);
        assert_eq!(link.open, Span::new(0, 3));
        assert_eq!(link.close, Span::new(10, 13));
    }

    #[test]
    fn pair_link_is_copy() {
        let l = PairLink::new(PairKind::Ruby, Span::new(0, 3), Span::new(6, 9));
        let copy = l;
        assert_eq!(l.open, copy.open);
    }
}
