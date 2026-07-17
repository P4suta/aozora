//! CST `SyntaxKind` enum and `rowan::Language` impl.
//!
//! `SyntaxKind` is the discriminant carried by every node + token
//! in the CST. It is intentionally coarser than
//! `aozora::NodeKind`: the AST kind names every classified
//! construct, while CST kinds organise the tree shape (root,
//! containers, plain text, classified spans).

use rowan::Language;

/// Discriminant for every CST node + token.
///
/// `#[non_exhaustive]` so adding a new node kind in a minor
/// release does not break exhaustive matches in downstream
/// consumers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[non_exhaustive]
#[repr(u16)]
pub enum SyntaxKind {
    // ---- Root --------------------------------------------------------
    /// Document root. Always the outermost node.
    Document = 0,

    // ---- Branch nodes ------------------------------------------------
    /// Paired-container region (`［＃ここから...］...［＃ここで...終わり］`).
    /// Children include the `ContainerOpen` token, intervening
    /// blocks, and the `ContainerClose` token.
    Container,
    /// Single classified construct (Ruby, Bouten, Gaiji, …). One
    /// child token carrying the source slice for the construct.
    Construct,

    // ---- Tokens ------------------------------------------------------
    /// Plain text run not covered by any classifier.
    Plain,
    /// Source bytes of a classified construct (Inline / `BlockLeaf`).
    /// The owning `Construct` node carries the variant tag through
    /// rowan's attached metadata (or the parent walker, in MVP).
    ConstructText,
    /// Open boundary of a `Container` (`［＃ここから...］`).
    ContainerOpen,
    /// Close boundary of a `Container` (`［＃ここで...終わり］`).
    ContainerClose,
}

impl SyntaxKind {
    fn from_raw_u16(raw: u16) -> Self {
        match raw {
            0 => Self::Document,
            1 => Self::Container,
            2 => Self::Construct,
            3 => Self::Plain,
            4 => Self::ConstructText,
            5 => Self::ContainerOpen,
            6 => Self::ContainerClose,
            _ => unreachable!("SyntaxKind raw discriminant {raw} out of range"),
        }
    }
}

/// rowan `Language` impl wiring [`SyntaxKind`] to rowan's
/// `SyntaxKind` newtype.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum AozoraLanguage {}

impl Language for AozoraLanguage {
    type Kind = SyntaxKind;

    fn kind_from_raw(raw: rowan::SyntaxKind) -> SyntaxKind {
        SyntaxKind::from_raw_u16(raw.0)
    }

    fn kind_to_raw(kind: SyntaxKind) -> rowan::SyntaxKind {
        rowan::SyntaxKind(kind as u16)
    }
}

/// Typed alias for `rowan::SyntaxNode` parameterised on this CST's
/// language.
pub type SyntaxNode = rowan::SyntaxNode<AozoraLanguage>;
/// Typed alias for `rowan::SyntaxToken` parameterised on this CST's
/// language.
pub type SyntaxToken = rowan::SyntaxToken<AozoraLanguage>;

#[cfg(test)]
mod tests {
    use super::*;

    /// Every raw discriminant maps back to the exact variant it was
    /// assigned. Deleting any `from_raw_u16` arm drops that raw value
    /// to the `unreachable!` arm and panics here.
    #[test]
    fn from_raw_u16_maps_each_discriminant_to_its_variant() {
        let expected = [
            (0_u16, SyntaxKind::Document),
            (1, SyntaxKind::Container),
            (2, SyntaxKind::Construct),
            (3, SyntaxKind::Plain),
            (4, SyntaxKind::ConstructText),
            (5, SyntaxKind::ContainerOpen),
            (6, SyntaxKind::ContainerClose),
        ];
        for (raw, variant) in expected {
            assert_eq!(
                SyntaxKind::from_raw_u16(raw),
                variant,
                "raw {raw} must decode to its assigned variant",
            );
        }
    }

    /// `to_raw` -> `from_raw_u16` round-trips every variant to itself,
    /// pinning the concrete raw discriminant each variant emits.
    #[test]
    fn kind_raw_round_trip_is_identity() {
        let variants = [
            SyntaxKind::Document,
            SyntaxKind::Container,
            SyntaxKind::Construct,
            SyntaxKind::Plain,
            SyntaxKind::ConstructText,
            SyntaxKind::ContainerOpen,
            SyntaxKind::ContainerClose,
        ];
        for (index, variant) in variants.into_iter().enumerate() {
            let raw = AozoraLanguage::kind_to_raw(variant);
            assert_eq!(
                raw.0,
                u16::try_from(index).unwrap(),
                "variant at index {index} must emit its positional raw",
            );
            assert_eq!(
                AozoraLanguage::kind_from_raw(raw),
                variant,
                "raw {} must round-trip back to its variant",
                raw.0,
            );
        }
    }
}
