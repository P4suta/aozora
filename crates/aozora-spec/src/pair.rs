//! Categories of balanced open/close delimiter pairs in Aozora notation.
//!
//! Trigger characters that always appear in isolation (`｜`, `＃`, `※`)
//! do not have a corresponding [`PairKind`]; they emit a "solo" event in
//! the lex pipeline.

use crate::Span;

/// Pair kind. The variants enumerate every balanced delimiter Aozora
/// notation recognises.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub enum PairKind {
    /// `［ … ］` (U+FF3B / U+FF3D). Annotation body container — always
    /// a bracket pair, with or without the leading `＃`.
    Bracket,

    /// `《 … 》` (U+300A / U+300B). Ruby reading.
    Ruby,

    /// `《《 … 》》`. Double-bracket bouten. Open/close are merged into
    /// single trigger tokens upstream, so the stack treats them as an
    /// independent kind (a stray inner `》` never closes a `《《`).
    DoubleRuby,

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
    pub const ALL: [Self; 5] = [
        Self::Bracket,
        Self::Ruby,
        Self::DoubleRuby,
        Self::Tortoise,
        Self::Quote,
    ];

    /// Stable camelCase string identifier used by the driver wire
    /// formats. Centralised here so every driver agrees on the wire
    /// spelling without hand-maintaining a parallel match.
    #[must_use]
    pub const fn as_camel_case(self) -> &'static str {
        match self {
            Self::Bracket => "bracket",
            Self::Ruby => "ruby",
            Self::DoubleRuby => "doubleRuby",
            Self::Tortoise => "tortoise",
            Self::Quote => "quote",
        }
    }
}

/// Resolved open/close pair, as observed by Phase 2.
///
/// Both `open` and `close` are byte-spans in the *sanitized* source
/// (the same coordinate system every other phase-2 / phase-3 `Span`
/// lives in). Used downstream by editor surfaces such as LSP
/// `textDocument/linkedEditingRange` and `documentHighlight`.
///
/// `Unclosed` opens (no matching close was found before EOF) and stray
/// `Unmatched` closes are deliberately *not* represented here — they
/// have no partner span to link to and would only confuse the editor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct PairLink {
    pub kind: PairKind,
    pub open: Span,
    pub close: Span,
}

impl PairLink {
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
            PairKind::DoubleRuby,
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
