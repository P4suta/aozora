//! CST `SyntaxKind` enum and `rowan::Language` impl.
//!
//! `SyntaxKind` is the discriminant carried by every node + token
//! in the CST. It is intentionally coarser than
//! [`aozora::NodeKind`]: the AST kind names every classified
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

const SYNTAX_KIND_COUNT: u16 = 7;

impl SyntaxKind {
    fn from_raw_u16(raw: u16) -> Self {
        assert!(
            raw < SYNTAX_KIND_COUNT,
            "SyntaxKind raw discriminant {raw} out of range"
        );
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
