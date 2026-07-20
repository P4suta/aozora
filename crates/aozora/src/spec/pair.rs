//! Categories of balanced open/close delimiter pairs in Aozora notation.
//!
//! Trigger characters that always appear in isolation (`｜`, `＃`, `※`)
//! do not have a corresponding [`PairKind`]; the tokenize stage emits
//! them as a "solo" token.

use crate::spec::Span;

/// Pair kind. The variants enumerate every balanced delimiter Aozora
/// notation recognises.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "json", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub enum PairKind {
    /// `［ … ］` (U+FF3B / U+FF3D). Directive body container — always
    /// a bracket pair, with or without the leading `＃`.
    Bracket,

    /// `《 … 》` (U+300A / U+300B). Ruby reading.
    Ruby,

    /// `≪ … ≫` (U+226A / U+226B). Double-angle quotation — the aozora
    /// input encoding for a 底本's twin angle brackets, displayed back
    /// as `《…》`. Its own pair kind (a stray `》` never closes a `≪`).
    AngleQuote,

    /// `〔 … 〕` (U+3014 / U+3015). Accent-decomposition segment.
    Tortoise,

    /// `「 … 」` (U+300C / U+300D). Quoted literal inside annotation
    /// bodies (e.g. `［＃「青空」に傍点］`).
    Quote,
}

impl PairKind {
    /// Every variant in declaration order. Used by codegen so
    /// downstream artefacts (TypeScript types, CLI tables) track the
    /// enum without a hand-maintained parallel.
    ///
    /// A `&'static [Self]` (not a fixed-size array) so adding a variant to
    /// this `#[non_exhaustive]` enum stays source-compatible for callers that
    /// iterate `PairKind::ALL` — the length is not part of the contract.
    pub const ALL: &'static [Self] = &[
        Self::Bracket,
        Self::Ruby,
        Self::AngleQuote,
        Self::Tortoise,
        Self::Quote,
    ];

    /// Stable camelCase string identifier used by the driver wire
    /// formats. Centralised here so every driver agrees on the wire
    /// spelling without hand-maintaining a parallel match.
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

    /// The open delimiter literal for this pair (`［`, `《`, `≪`, `〔`, `「`).
    ///
    /// The single authority for the delimiter glyphs: diagnostic prose,
    /// quick-fix titles, and any renderer that needs to name a delimiter
    /// read them from here rather than hard-coding the character.
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

    /// The close delimiter literal for this pair (`］`, `》`, `≫`, `〕`, `」`).
    /// Counterpart of [`Self::open_str`].
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

/// Resolved open/close pair, as observed by the pair stage.
///
/// Both `open` and `close` are byte-spans in the original source.
/// Used downstream by editor surfaces such as LSP
/// `textDocument/linkedEditingRange` and `documentHighlight`.
///
/// `Unclosed` opens (no matching close was found before EOF) and stray
/// `Unmatched` closes are deliberately *not* represented here — they
/// have no partner span to link to and would only confuse the editor.
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
    /// Construct a link between an `open` and `close` span of the same
    /// `kind`. The caller guarantees the two spans are a genuinely
    /// matched pair; this constructor does no validation.
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
        // Both still usable — Copy semantics confirmed.
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
        // The wire spelling every driver agrees on — pin each variant's
        // tag so a rename (or a body-stubbing regression) is caught here
        // rather than silently drifting the JSON envelope.
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
