//! Categories of balanced open/close delimiter pairs in Aozora notation.
//!
//! Trigger characters that always appear in isolation (`｜`, `＃`, `※`)
//! do not have a corresponding [`PairKind`]; they emit a "solo" event in
//! the lex pipeline.

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

    /// `〔 … 〕` (U+3014 / U+3015). Accent-decomposition segment per
    /// ADR-0004.
    Tortoise,

    /// `「 … 」` (U+300C / U+300D). Quoted literal inside annotation
    /// bodies (e.g. `［＃「青空」に傍点］`).
    Quote,
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
}
