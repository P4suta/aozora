//! Attribute-major formatting model: one `Format` attribute, typed
//! per-scope sums.
//!
//! Aozora's typographic notations (太字, 傍点, 字下げ, …) each apply at one
//! or more *scopes* — forward-reference (`「X」は太字`), single line
//! (`［＃地付き］`), or a paired range / block (`［＃ここから太字］ …
//! ［＃ここで太字終わり］`). The legacy model defined the same attribute
//! once per scope (`EmphasisKind::Bold` + `ContainerKind::Bold`), letting an
//! illegal `(attribute, scope)` pair and placeholder close payloads
//! (`steps: ±1`, `width: 0`) be representable.
//!
//! This module separates the two axes:
//!
//! - [`Format`] names the **attribute** once, scope-independent — the source
//!   of the canonical keyword and the attribute-identity tag.
//! - [`ForwardAttr`], [`LineFormat`], [`RegionFormat`] each enumerate **only
//!   the attributes legal at that scope**, so an illegal pair is unrepresentable.
//!   Every scope sum projects back via `fn format() -> Format`.
//! - [`RegionClose`] is the close-marker discriminant; the open
//!   [`RegionFormat`] payload stays authoritative (see [`RegionClose::of`]).
//! - The scalar parameters ([`FontShift`], [`ColumnCount`], [`LineWidth`],
//!   [`Kumi`]) are `NonZero`, so the `0` / `±1` placeholders the close markers
//!   used to carry cannot be constructed.

use core::num::{NonZeroI8, NonZeroU8};

use crate::{BoutenKind, BoutenPosition, HeadingKind, HeadingStyle};

// ----------------------------------------------------------------------
// Scalar parameters — NonZero so placeholders are unconstructable
// ----------------------------------------------------------------------

/// Signed relative font-size shift (旧 `steps: i8`).
///
/// Positive = 大きな (larger), negative = 小さな (smaller). `NonZero` because
/// a zero-stage shift is not a font-size change — the close marker used to
/// carry a `±1` placeholder, which this type makes unrepresentable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct FontShift(pub NonZeroI8);

impl FontShift {
    /// `true` when the shift enlarges (大きな); `false` when it shrinks (小さな).
    #[must_use]
    pub const fn larger(self) -> bool {
        self.0.get() > 0
    }

    /// The unsigned stage count.
    #[must_use]
    pub const fn magnitude(self) -> u8 {
        self.0.get().unsigned_abs()
    }
}

/// Number of columns in a 段組 region. `1` is not a multi-column layout, so
/// `NonZero` rules out the degenerate case.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ColumnCount(pub NonZeroU8);

/// Full-width characters per line (字詰め / 字組み width). `NonZero` because a
/// zero-width line is meaningless — the close marker's `0` placeholder is gone.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct LineWidth(pub NonZeroU8);

/// The `L行W字組み` clause: `lines` lines of `width` full-width characters.
///
/// Both `NonZero` — the close marker used to re-emit `lines: 0`, which this
/// type makes unrepresentable (the open side is authoritative on close).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Kumi {
    /// Line count `L` from the `L行` clause.
    pub lines: NonZeroU8,
    /// Full-width characters per line `W` from the `W字組み` clause.
    pub width: NonZeroU8,
}

/// Secondary line-layout clause of an indent block (`、N字詰め` / `、L行W字組みで`).
// Deliberately NOT `#[non_exhaustive]`: serialize / render must handle every
// arm explicitly so a future layout is compiler-flagged at every site rather
// than silently dropped by a `_` fallback (the §7.6 param-drop bug class).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum IndentLayout {
    /// A plain `［＃ここから N字下げ］` block — no secondary layout.
    None,
    /// `［＃ここから N字下げ、M字詰め］` — also sets `M` full-width chars per line.
    LineWidth(LineWidth),
    /// `［＃ここから N字下げ、L行W字組みで］` — sets `L` lines of `W` chars.
    Kumi(Kumi),
}

/// Co-applied decorative styles stacked on a block-layout opener.
///
/// The `［＃ここから N字下げ、…］` compound lets several typographic attributes
/// ride one indent opener and close with the single generic `字下げ終わり`
/// (pairing is by family — the decorations never touch the close). This is the
/// *closed* set of those decorations.
///
/// A struct of named fields (not a bitset, not a `Vec`) is deliberate:
/// - `font` carries a magnitude no bit can hold (`小さい活字` = one step
///   smaller), and a single `Option<FontShift>` makes "two conflicting sizes"
///   and "the size flag set with no magnitude" both unrepresentable;
/// - the three `bool`s make a duplicated attribute unrepresentable;
/// - it stays `Copy` (a `Vec<Format>` would break the arena `Copy` chain).
///
/// Each field projects to the scope-independent [`Format`] identity via
/// [`Self::iter_formats`], so render / serialize / wire reuse the existing
/// per-[`Format`] machinery rather than re-deciding keyword/class/tag here.
///
/// The name is deliberately generic (not `IndentStyles`): a future block
/// anchor can carry the same decoration set without a second rearchitecture.
//
// Deliberately NOT `#[non_exhaustive]`: a fifth decoration must be
// compiler-flagged everywhere it is consumed, never silently defaulting to
// "absent" (the §7.6 param-drop bug class). The serialize and render sites
// destructure `{ bold, horizontal, framed, font }` with no `..` directly; the
// Pandoc kvs + JSON projection funnel through [`Self::iter_formats`], which
// destructures exhaustively too — so every channel is guarded.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct BlockStyles {
    /// `ゴシック体` — co-applied bold weight (`Format::Bold`).
    pub bold: bool,
    /// `横書き` / `横組み` — horizontal writing (`Format::Horizontal`).
    pub horizontal: bool,
    /// `罫囲み` — ruled box around the block (`Format::Framed`).
    pub framed: bool,
    /// `小さい活字` (= `FontShift(-1)`) / `N段階…文字` — relative font shift
    /// (`Format::FontSize`). `None` = no shift; a single `Option` forbids two
    /// conflicting sizes.
    pub font: Option<FontShift>,
}

impl BlockStyles {
    /// No decorations — the overwhelmingly common plain-indent case.
    pub const EMPTY: Self = Self {
        bold: false,
        horizontal: false,
        framed: false,
        font: None,
    };

    /// Whether no decoration is set (a plain indent that serializes byte-exact
    /// to today's `［＃ここから N字下げ］`).
    #[must_use]
    pub fn is_empty(self) -> bool {
        self == Self::EMPTY
    }

    /// Project the set to its [`Format`] identities in **canonical order**
    /// (`bold`, `horizontal`, `framed`, `font`). Serialize, render, and the
    /// Pandoc / wire `modifiers` path all consume this one order, so the
    /// canonical emission never drifts.
    ///
    /// This is the **single chokepoint** the Pandoc kvs + JSON projection feed
    /// through, so it destructures exhaustively (no `..`): a fifth decoration is
    /// compiler-flagged here, not silently dropped from those channels —
    /// completing the §7.6 param-drop guard the per-site destructures provide
    /// for serialize / render.
    pub fn iter_formats(self) -> impl Iterator<Item = Format> {
        let Self {
            bold,
            horizontal,
            framed,
            font,
        } = self;
        [
            bold.then_some(Format::Bold),
            horizontal.then_some(Format::Horizontal),
            framed.then_some(Format::Framed),
            font.map(Format::FontSize),
        ]
        .into_iter()
        .flatten()
    }
}

/// The block-only payload of an indent region.
///
/// `wrap` / `layout` / `styles` live here rather than on the single-line
/// `Indent` so the block-only clauses cannot leak into the line scope.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct IndentBlock {
    /// Full-width characters the block is indented by.
    pub amount: u8,
    /// Hanging-indent continuation width: `Some(M)` for `折り返して M字下げ`.
    pub wrap: Option<u8>,
    /// `true` for the combined `…、ページの左右中央` / `…、中央揃え` form
    /// (also page-centred).
    pub center: bool,
    /// Secondary line-layout clause; see [`IndentLayout`].
    pub layout: IndentLayout,
    /// Co-applied decorative styles (`ゴシック体` / `横書き` / `罫囲み` /
    /// `小さい活字`); see [`BlockStyles`].
    pub styles: BlockStyles,
}

// ----------------------------------------------------------------------
// Format — the attribute identity (scope-independent)
// ----------------------------------------------------------------------

/// The typographic attribute, independent of the scope it applies at.
///
/// The single source of the attribute-identity tag ([`Self::as_json_tag`]).
/// Each scope sum ([`ForwardAttr`] / [`LineFormat`] / [`RegionFormat`])
/// projects to this via its `format()` method, so cross-scope grouping
/// (e.g. "is this the same attribute as that one?") keys on one enum.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub enum Format {
    /// 太字 (bold).
    Bold,
    /// 斜体 (italic).
    Italic,
    /// 傍点 / 傍線 (emphasis dots / sidelines).
    Bouten(BoutenKind),
    /// 罫囲み (ruled box).
    Framed,
    /// 横組み (horizontal writing).
    Horizontal,
    /// N段階大きな / 小さな文字 (relative font size).
    FontSize(FontShift),
    /// キャプション (caption).
    Caption,
    /// 上付き小文字 (superscript).
    SuperScript,
    /// 下付き小文字 (subscript).
    SubScript,
    /// 行右 / 行左小書き (small side-script).
    SmallScript(BoutenPosition),
    /// 縦中横 (tate-chu-yoko).
    CombineUpright,
    /// 字下げ (indent).
    Indent,
    /// 地付き / 地から N 字上げ (end alignment).
    AlignEnd,
    /// 中央 (centring).
    Center,
    /// 字詰め (line width).
    LineWidth,
    /// 表 (table).
    Table,
    /// 段組 (multi-column).
    Columns(ColumnCount),
    /// 割り注 (split annotation).
    Warichu,
    /// 見出し (heading).
    Heading {
        /// The 大 / 中 / 小 outline level.
        level: HeadingKind,
        /// Standard / 同行 / 窓 style.
        style: HeadingStyle,
    },
}

impl Format {
    /// Stable camelCase attribute-identity tag (exhaustive — no `_` fallback,
    /// so a new attribute fails to build until it is given a tag).
    #[must_use]
    pub const fn as_json_tag(self) -> &'static str {
        match self {
            Self::Bold => "bold",
            Self::Italic => "italic",
            Self::Bouten(_) => "bouten",
            Self::Framed => "framed",
            Self::Horizontal => "horizontal",
            Self::FontSize(_) => "fontSize",
            Self::Caption => "caption",
            Self::SuperScript => "superScript",
            Self::SubScript => "subScript",
            Self::SmallScript(_) => "smallScript",
            Self::CombineUpright => "combineUpright",
            Self::Indent => "indent",
            Self::AlignEnd => "alignEnd",
            Self::Center => "center",
            Self::LineWidth => "lineWidth",
            Self::Table => "table",
            Self::Columns(_) => "columns",
            Self::Warichu => "warichu",
            Self::Heading { .. } => "heading",
        }
    }
}

// ----------------------------------------------------------------------
// Forward scope — `「X」は…` reference-attached emphasis
// ----------------------------------------------------------------------

/// The attributes legal at the forward-reference scope (`「X」は太字` etc.).
///
/// The content-carrying leaf that pairs an attribute with its target run is
/// the forward-reference leaf (`ForwardFormatOwned`); this enum is the
/// attribute alone.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub enum ForwardAttr {
    /// 太字.
    Bold,
    /// 斜体.
    Italic,
    /// 上付き小文字.
    SuperScript,
    /// 下付き小文字.
    SubScript,
    /// 行右 / 行左小書き.
    SmallScript(BoutenPosition),
    /// 罫囲み.
    Framed,
    /// 横組み.
    Horizontal,
    /// キャプション.
    Caption,
    /// N段階大きな / 小さな文字.
    FontSize(FontShift),
    /// 傍点 / 傍線. `position` records a `左に` left-side modifier.
    Bouten {
        /// The 傍点 / 傍線 mark.
        kind: BoutenKind,
        /// `Left` for the `左に` modifier, else `Right`.
        position: BoutenPosition,
    },
    /// 縦中横.
    CombineUpright,
}

/// A forward emphasis node's target-text provenance — whether `serialize`
/// must re-emit the leading literal to reconstruct the source.
///
/// A forward reference is recognized **only** when its target appears
/// *contiguously* in the source before the bracket. A directive whose quoted
/// target is absent (`（例）［＃「国境が消える」に傍点］`) or split by a ruby run
/// (`牛《ベゴ》の舌［＃「牛の舌」に傍点］`) is left an unresolved directive, not a
/// forward node, so it never reaches this type.
///
/// This is the one irreducible provenance the normalization waist could not
/// fold away — it cannot collapse to a constant in either direction:
/// - **Not always `Reclaimed`** (node owns the literal): when the recognized
///   target occurrence is itself a **ruby base**
///   (`我《が》…我［＃「我」に傍点］`; ≥34 in the 17,889-work `aozorabunko_text`
///   mirror, see #202) it cannot be pulled into a text-only forward leaf
///   (bouten-over-ruby is not representable), so it must stay `Referenced`.
/// - **Not always `Referenced`** (literal left upstream): an adjacent forward
///   had its literal pulled into the node and the surrounding plain run
///   truncated, so without re-emit the literal would be lost.
///
/// Deriving it at serialize from a non-local preceding-content scan would
/// re-introduce exactly the lookback the scope-free core removes, so it is
/// materialized here as explicit provenance. (The #180 unbounded-growth
/// pathology — a `Reclaimed` literal doubled in the plain tail — is separately
/// cured by the lowering pass's overlap-truncate.)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum ForwardOrigin {
    /// The classifier pulled the literal out of the immediately-preceding
    /// source (`青空［＃「青空」に傍点］`): the surrounding plain run was truncated,
    /// so the decorated run is the *sole* visible copy and the serializer
    /// re-emits the literal before the bracket.
    Reclaimed,
    /// The target is recognized contiguously before the bracket but is *not*
    /// byte-adjacent to it, so the literal stays in the preceding run; the
    /// serializer emits the bracket form alone.
    Referenced,
}

impl ForwardOrigin {
    /// Derive the provenance from the classifier's consume window:
    /// [`Reclaimed`](Self::Reclaimed) iff `consume_start` was pulled back
    /// before the directive's `［` at `bracket_start`, otherwise
    /// [`Referenced`](Self::Referenced).
    #[must_use]
    pub const fn from_consume(consume_start: u32, bracket_start: u32) -> Self {
        if consume_start < bracket_start {
            Self::Reclaimed
        } else {
            Self::Referenced
        }
    }
}

impl ForwardAttr {
    /// Project to the scope-independent [`Format`] attribute.
    #[must_use]
    pub const fn format(self) -> Format {
        match self {
            Self::Bold => Format::Bold,
            Self::Italic => Format::Italic,
            Self::SuperScript => Format::SuperScript,
            Self::SubScript => Format::SubScript,
            Self::SmallScript(p) => Format::SmallScript(p),
            Self::Framed => Format::Framed,
            Self::Horizontal => Format::Horizontal,
            Self::Caption => Format::Caption,
            Self::FontSize(f) => Format::FontSize(f),
            Self::Bouten { kind, .. } => Format::Bouten(kind),
            Self::CombineUpright => Format::CombineUpright,
        }
    }

    /// Canonical 青空文庫 keyword for the treatment (the body of
    /// `「X」は〈keyword〉` / `「X」に〈keyword〉`).
    ///
    /// [`Self::FontSize`] carries a magnitude and is serialized separately, so
    /// it falls through to the 太字 default here (the serializer never calls
    /// this for it).
    #[must_use]
    pub const fn keyword(self) -> &'static str {
        match self {
            Self::Italic => "斜体",
            Self::SuperScript => "上付き小文字",
            Self::SubScript => "下付き小文字",
            Self::SmallScript(BoutenPosition::Right) => "行右小書き",
            Self::SmallScript(BoutenPosition::Left) => "行左小書き",
            Self::Framed => "罫囲み",
            Self::Horizontal => "横組み",
            Self::Caption => "キャプション",
            Self::CombineUpright => "縦中横",
            Self::Bouten { kind, .. } => kind.keyword(),
            // Bold, FontSize, and any future weight default to 太字.
            _ => "太字",
        }
    }
}

// ----------------------------------------------------------------------
// Line scope — single-line layout directives
// ----------------------------------------------------------------------

/// The attributes legal at the single-line scope (`［＃地付き］` etc.).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub enum LineFormat {
    /// `［＃天から N字下げ］` — indent the line by `amount` full-width chars.
    Indent {
        /// Full-width characters to indent by.
        amount: u8,
    },
    /// `［＃地付き］` / `［＃地から N字上げ］` — end alignment.
    AlignEnd {
        /// Chars lifted off the foot edge. `0` = 地付き, `n` = 地から n 字上げ.
        offset: u8,
    },
    /// `中央揃え` / `ページの左右中央` — centring.
    Center {
        /// `true` for `ページの左右中央` (page centre), `false` for `中央揃え`.
        page: bool,
    },
    /// `［＃罫囲み］` — box the single line it sits on.
    Framed,
}

impl LineFormat {
    /// Project to the scope-independent [`Format`] attribute.
    #[must_use]
    pub const fn format(self) -> Format {
        match self {
            Self::Indent { .. } => Format::Indent,
            Self::AlignEnd { .. } => Format::AlignEnd,
            Self::Center { .. } => Format::Center,
            Self::Framed => Format::Framed,
        }
    }
}

// ----------------------------------------------------------------------
// Region scope — paired range / block containers
// ----------------------------------------------------------------------

/// The attributes legal at the paired range / block scope.
///
/// The open marker of a `［＃ここから…］ … ［＃ここで…終わり］` (or bare
/// `［＃…］ … ［＃…終わり］`) pair. The matching close is a [`RegionClose`];
/// the open payload here stays authoritative when the pair round-trips.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub enum RegionFormat {
    /// 太字 range / block. `padded` = block-level (`<div>`, `\n\n` padded);
    /// `!padded` = inline bare range (`<b>`).
    Bold {
        /// `true` = `ここから` block; `false` = inline bare range.
        padded: bool,
    },
    /// 斜体 range / block — the slant counterpart of [`Self::Bold`].
    Italic {
        /// `true` = `ここから` block; `false` = inline bare range.
        padded: bool,
    },
    /// キャプション range / block.
    Caption {
        /// `true` = `ここから` block; `false` = inline bare range.
        padded: bool,
    },
    /// 見出し (delimited heading) range / block.
    Heading {
        /// The 大 / 中 / 小 outline level.
        level: HeadingKind,
        /// Standard / 同行 / 窓 style.
        style: HeadingStyle,
        /// `true` = `ここから` block; `false` = paired `窓中見出し…終わり`.
        padded: bool,
    },
    /// 傍点 / 傍線 range (`［＃傍点］ … ［＃傍点終わり］`, `［＃左に傍線］ …`).
    Bouten {
        /// The 傍点 / 傍線 mark; its 点/線 family drives the mismatch check.
        kind: BoutenKind,
        /// `Left` for the `左に` modifier, else `Right`.
        position: BoutenPosition,
    },
    /// 行右 / 行左小書き range.
    SmallScript(BoutenPosition),
    /// 縦中横 range (`［＃縦中横］ … ［＃縦中横終わり］`).
    CombineUpright,
    /// 字下げ block (`［＃ここから N字下げ］ …`).
    Indent(IndentBlock),
    /// 地付き / 地から N 字上げ block.
    AlignEnd {
        /// Chars lifted off the foot edge. `0` = 地付き, `n` = 地から n 字上げ.
        offset: u8,
    },
    /// 字詰め block (`［＃ここから N字詰め］ …`).
    LineWidth(LineWidth),
    /// 表 block.
    Table,
    /// 段組 block.
    Columns(ColumnCount),
    /// 横組み block.
    Horizontal,
    /// N段階大きな / 小さな文字 block.
    FontSize(FontShift),
    /// 罫囲み block / range.
    Framed,
    /// 割り注 block (multi-line `［＃割り注］ … ［＃割り注終わり］`).
    Warichu,
}

impl RegionFormat {
    /// Project to the scope-independent [`Format`] attribute.
    #[must_use]
    pub const fn format(self) -> Format {
        match self {
            Self::Bold { .. } => Format::Bold,
            Self::Italic { .. } => Format::Italic,
            Self::Caption { .. } => Format::Caption,
            Self::Heading { level, style, .. } => Format::Heading { level, style },
            Self::Bouten { kind, .. } => Format::Bouten(kind),
            Self::SmallScript(p) => Format::SmallScript(p),
            Self::CombineUpright => Format::CombineUpright,
            Self::Indent(_) => Format::Indent,
            Self::AlignEnd { .. } => Format::AlignEnd,
            Self::LineWidth(_) => Format::LineWidth,
            Self::Table => Format::Table,
            Self::Columns(c) => Format::Columns(c),
            Self::Horizontal => Format::Horizontal,
            Self::FontSize(f) => Format::FontSize(f),
            Self::Framed => Format::Framed,
            Self::Warichu => Format::Warichu,
        }
    }

    /// Stable camelCase wire tag — the machine-contract counterpart used by the
    /// `container_pairs` driver endpoint. Scope-specific (`boutenRange`,
    /// `combineUprightRange`), distinct from the attribute-level
    /// [`Format::as_json_tag`]; exhaustive so a new variant cannot fall through
    /// to a silent `"unknown"`.
    #[must_use]
    pub const fn as_json_tag(self) -> &'static str {
        match self {
            Self::Indent(_) => "indent",
            Self::Warichu => "warichu",
            Self::Framed => "framed",
            Self::AlignEnd { .. } => "alignEnd",
            Self::LineWidth(_) => "lineWidth",
            Self::Bouten { .. } => "boutenRange",
            Self::Bold { .. } => "bold",
            Self::Italic { .. } => "italic",
            Self::Heading { .. } => "heading",
            Self::Columns(_) => "columns",
            Self::Table => "table",
            Self::Horizontal => "horizontal",
            Self::FontSize(_) => "fontSize",
            Self::SmallScript(_) => "smallScript",
            Self::Caption { .. } => "caption",
            Self::CombineUpright => "combineUprightRange",
        }
    }

    /// Stable lowercase kebab family tag for human-facing diagnostics (the
    /// mismatched open/close pair names). Payload-independent.
    #[must_use]
    pub const fn kind_str(self) -> &'static str {
        match self {
            Self::Indent(_) => "indent",
            Self::Warichu => "warichu",
            Self::Framed => "framed",
            Self::AlignEnd { .. } => "align-end",
            Self::LineWidth(_) => "line-width",
            Self::Bouten { .. } => "bouten-range",
            Self::Bold { .. } => "bold",
            Self::Italic { .. } => "italic",
            Self::Heading { .. } => "heading",
            Self::Columns(_) => "columns",
            Self::Table => "table",
            Self::Horizontal => "horizontal",
            Self::FontSize(_) => "font-size",
            Self::SmallScript(_) => "small-script",
            Self::Caption { .. } => "caption",
            Self::CombineUpright => "combine-upright-range",
        }
    }

    /// Whether this region renders *inline* (within the current paragraph)
    /// rather than as a block wrapper.
    ///
    /// The 傍点 / 傍線 range, the bare-range 太字 / 斜体 / キャプション forms
    /// (`!padded`), the 小書き range, and the 縦中横 range sit within a line;
    /// every other region is block-level (gets `\n\n` padding + a `<div>`).
    #[must_use]
    pub const fn is_inline(self) -> bool {
        matches!(
            self,
            Self::Bouten { .. }
                | Self::Bold { padded: false }
                | Self::Italic { padded: false }
                | Self::SmallScript(_)
                | Self::Caption { padded: false }
                | Self::CombineUpright
        )
    }

    /// Whether this region's content is *phrasing* (rendered directly inside
    /// the block element rather than wrapped in `<p>` paragraphs).
    ///
    /// Only [`Self::Heading`] is phrasing: a heading element holds its title
    /// directly, so `<h1><p>…</p></h1>` would be invalid.
    #[must_use]
    pub const fn content_is_phrasing(self) -> bool {
        matches!(self, Self::Heading { .. })
    }

    /// Every variant, one representative instance per data-carrying variant —
    /// the payload is irrelevant to the discriminant-only tag projections. Lets
    /// the wire-tag exhaustiveness test and the codegen enumerate the family
    /// list without a hand-maintained parallel.
    pub const ALL: [Self; 16] = [
        Self::Indent(IndentBlock {
            amount: 0,
            wrap: None,
            center: false,
            layout: IndentLayout::None,
            styles: BlockStyles::EMPTY,
        }),
        Self::Warichu,
        Self::Framed,
        Self::AlignEnd { offset: 0 },
        Self::LineWidth(LineWidth(NonZeroU8::MIN)),
        Self::Bouten {
            kind: BoutenKind::Goma,
            position: BoutenPosition::Right,
        },
        Self::Bold { padded: false },
        Self::Italic { padded: false },
        Self::Heading {
            level: HeadingKind::Large,
            style: HeadingStyle::Standard,
            padded: false,
        },
        Self::Columns(ColumnCount(NonZeroU8::MIN)),
        Self::Table,
        Self::Horizontal,
        Self::FontSize(FontShift(NonZeroI8::MIN)),
        Self::SmallScript(BoutenPosition::Right),
        Self::Caption { padded: false },
        Self::CombineUpright,
    ];
}

// ----------------------------------------------------------------------
// Region close — self-sufficient; carries the close marker's own data
// ----------------------------------------------------------------------

/// The close marker of a paired region.
///
/// Carries exactly what the close marker (and HTML close tag) reproduce from
/// the **close** source text — never a placeholder. A close can appear without
/// a matching open (a stray `［＃…終わり］`) and a mismatched close keeps its
/// own family (`［＃傍線終わり］` closing a `［＃傍点］`), so the close must be
/// self-sufficient; it is *not* reconstructed from the open. `Option` /
/// `NonZero` / `bool` make the former `width: 0` / `steps: ±1` / `lines: 0`
/// placeholder states unrepresentable, which was Pillar 1's actual goal.
///
/// It stays meaningfully smaller than [`RegionFormat`] (the indent, line-width,
/// columns, align-end, and font-size closes all shed payload), so the separate
/// type still earns its keep.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub enum RegionClose {
    /// `字下げ終わり`, or the `字下げ、{W}字組み終わり` compound (the close
    /// carries `W`, so the marker round-trips byte-exact).
    Indent {
        /// The `W` of a `字組み終わり` compound; `None` for the generic
        /// `字下げ終わり` (plain / 字詰め / 折り返して / 中央 indents).
        kumi_width: Option<LineWidth>,
    },
    /// `割り注終わり`.
    Warichu,
    /// `罫囲み終わり`.
    Framed,
    /// `字上げ終わり` / 地付き close (no offset — the close marker carries none).
    AlignEnd,
    /// `字詰め終わり`.
    LineWidth,
    /// `傍点終わり` / `傍線終わり` / `波線終わり` … — the close's own mark and
    /// `左に` side. The 点/線 family (`kind.is_line()`) drives the
    /// `mismatched_bouten_container` diagnostic.
    Bouten {
        /// The close marker's own 傍点 / 傍線 mark.
        kind: BoutenKind,
        /// `Left` for the close's `左に` modifier, else `Right`.
        position: BoutenPosition,
    },
    /// `太字終わり` (`!padded`) / `ここで太字終わり` (`padded`).
    Bold {
        /// `true` = `ここで` block close; `false` = inline bare-range close.
        padded: bool,
    },
    /// `斜体終わり` / `ここで斜体終わり`.
    Italic {
        /// `true` = `ここで` block close; `false` = inline bare-range close.
        padded: bool,
    },
    /// `<見出し>終わり` / `ここで<見出し>終わり`.
    Heading {
        /// The 大 / 中 / 小 outline level.
        level: HeadingKind,
        /// Standard / 同行 / 窓 style.
        style: HeadingStyle,
        /// `true` = `ここで` block close; `false` = paired `…終わり` close.
        padded: bool,
    },
    /// `段組終わり`.
    Columns,
    /// `表終わり`.
    Table,
    /// `横組み終わり`.
    Horizontal,
    /// `大きな文字終わり` (`larger`) / `小さな文字終わり`.
    FontSize {
        /// `true` = 大きな (larger); `false` = 小さな (smaller).
        larger: bool,
    },
    /// `行右 / 行左小書き終わり`.
    SmallScript(BoutenPosition),
    /// `キャプション終わり` (`!padded`) / `ここでキャプション終わり` (`padded`).
    Caption {
        /// `true` = `ここで` block close; `false` = inline bare-range close.
        padded: bool,
    },
    /// `縦中横終わり`.
    CombineUpright,
}

impl RegionClose {
    /// The close that matches a given open [`RegionFormat`], preserving the
    /// open's payload.
    ///
    /// Used two ways: in the classifier, on the [`RegionFormat`] parsed from
    /// the *close* marker (so it carries the close's own data); and in the
    /// pairing-mismatch check, on the *open* (to derive the expected close).
    #[must_use]
    pub const fn of(region: RegionFormat) -> Self {
        match region {
            RegionFormat::Indent(block) => Self::Indent {
                kumi_width: match block.layout {
                    IndentLayout::Kumi(kumi) => Some(LineWidth(kumi.width)),
                    IndentLayout::LineWidth(_) | IndentLayout::None => None,
                },
            },
            RegionFormat::Warichu => Self::Warichu,
            RegionFormat::Framed => Self::Framed,
            RegionFormat::AlignEnd { .. } => Self::AlignEnd,
            RegionFormat::LineWidth(_) => Self::LineWidth,
            RegionFormat::Bouten { kind, position } => Self::Bouten { kind, position },
            RegionFormat::Bold { padded } => Self::Bold { padded },
            RegionFormat::Italic { padded } => Self::Italic { padded },
            RegionFormat::Heading {
                level,
                style,
                padded,
            } => Self::Heading {
                level,
                style,
                padded,
            },
            RegionFormat::Columns(_) => Self::Columns,
            RegionFormat::Table => Self::Table,
            RegionFormat::Horizontal => Self::Horizontal,
            RegionFormat::FontSize(shift) => Self::FontSize {
                larger: shift.larger(),
            },
            RegionFormat::SmallScript(side) => Self::SmallScript(side),
            RegionFormat::Caption { padded } => Self::Caption { padded },
            RegionFormat::CombineUpright => Self::CombineUpright,
        }
    }

    /// Stable lowercase kebab family tag for human-facing diagnostics (mirrors
    /// [`RegionFormat::kind_str`] so an open/close mismatch names both sides
    /// consistently).
    #[must_use]
    pub const fn kind_str(self) -> &'static str {
        match self {
            Self::Indent { .. } => "indent",
            Self::Warichu => "warichu",
            Self::Framed => "framed",
            Self::AlignEnd => "align-end",
            Self::LineWidth => "line-width",
            Self::Bouten { .. } => "bouten-range",
            Self::Bold { .. } => "bold",
            Self::Italic { .. } => "italic",
            Self::Heading { .. } => "heading",
            Self::Columns => "columns",
            Self::Table => "table",
            Self::Horizontal => "horizontal",
            Self::FontSize { .. } => "font-size",
            Self::SmallScript(_) => "small-script",
            Self::Caption { .. } => "caption",
            Self::CombineUpright => "combine-upright-range",
        }
    }

    /// Whether the close renders inline (mirrors [`RegionFormat::is_inline`] so
    /// the close marker's `\n\n` padding / `<p>` handling matches the open).
    #[must_use]
    pub const fn is_inline(self) -> bool {
        matches!(
            self,
            Self::Bouten { .. }
                | Self::Bold { padded: false }
                | Self::Italic { padded: false }
                | Self::SmallScript(_)
                | Self::Caption { padded: false }
                | Self::CombineUpright
        )
    }

    /// Whether the close's content was *phrasing* — only [`Self::Heading`].
    #[must_use]
    pub const fn content_is_phrasing(self) -> bool {
        matches!(self, Self::Heading { .. })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `RegionFormat` stays small and `Copy` — pinned so a payload that bloats
    /// the registry's per-node footprint trips here.
    #[test]
    fn region_format_is_copy_and_small() {
        const fn assert_copy<T: Copy>() {}
        assert_copy::<RegionFormat>();
        assert_copy::<Format>();
        assert_copy::<ForwardAttr>();
        assert_copy::<LineFormat>();
        assert_copy::<RegionClose>();
        assert!(size_of::<RegionFormat>() <= 12);
    }

    /// The container-pairs wire tags are pinned: an accidental rename breaks
    /// this test instead of silently shifting the `SCHEMA_VERSION=1` wire.
    #[test]
    fn region_format_wire_tags_are_stable() {
        assert_eq!(
            RegionFormat::Bouten {
                kind: BoutenKind::Goma,
                position: BoutenPosition::Right,
            }
            .as_json_tag(),
            "boutenRange"
        );
        assert_eq!(
            RegionFormat::CombineUpright.as_json_tag(),
            "combineUprightRange"
        );
        assert_eq!(
            RegionFormat::LineWidth(LineWidth(NonZeroU8::MIN)).as_json_tag(),
            "lineWidth"
        );
        assert_eq!(RegionFormat::Bold { padded: true }.as_json_tag(), "bold");
        assert_eq!(RegionFormat::Bold { padded: false }.as_json_tag(), "bold");
    }

    /// Every open round-trips to a close discriminant and back to the same
    /// family kebab tag (the open/close diagnostic vocabulary agrees).
    #[test]
    fn region_close_of_round_trips_kind_str() {
        for open in RegionFormat::ALL {
            assert_eq!(
                RegionClose::of(open).kind_str(),
                open.kind_str(),
                "close family must mirror open family for {open:?}"
            );
        }
    }

    /// The 傍点/傍線 kind + position survive the open → close projection (so a
    /// mismatched close keeps its own family for round-trip and diagnostics).
    #[test]
    fn region_close_preserves_bouten_kind() {
        let line = RegionFormat::Bouten {
            kind: BoutenKind::UnderLine,
            position: BoutenPosition::Left,
        };
        assert_eq!(
            RegionClose::of(line),
            RegionClose::Bouten {
                kind: BoutenKind::UnderLine,
                position: BoutenPosition::Left,
            }
        );
    }

    /// The 字組み close keeps its own width; every other indent close is generic.
    #[test]
    fn region_close_indent_keeps_kumi_width() {
        let kumi = RegionFormat::Indent(IndentBlock {
            amount: 2,
            wrap: None,
            center: false,
            layout: IndentLayout::Kumi(Kumi {
                lines: NonZeroU8::MIN,
                width: NonZeroU8::new(20).unwrap(),
            }),
            styles: BlockStyles::EMPTY,
        });
        assert_eq!(
            RegionClose::of(kumi),
            RegionClose::Indent {
                kumi_width: Some(LineWidth(NonZeroU8::new(20).unwrap())),
            }
        );
        let plain = RegionFormat::Indent(IndentBlock {
            amount: 2,
            wrap: None,
            center: false,
            layout: IndentLayout::None,
            styles: BlockStyles::EMPTY,
        });
        assert_eq!(
            RegionClose::of(plain),
            RegionClose::Indent { kumi_width: None }
        );
    }

    /// Attribute-level tags are exhaustive and distinct from the scope wire
    /// tags (e.g. `bouten` vs `boutenRange`).
    #[test]
    fn format_attribute_tags() {
        assert_eq!(Format::Bouten(BoutenKind::Goma).as_json_tag(), "bouten");
        assert_eq!(Format::CombineUpright.as_json_tag(), "combineUpright");
        assert_eq!(Format::Warichu.as_json_tag(), "warichu");
        assert_eq!(Format::Center.as_json_tag(), "center");
    }

    /// Each scope sum projects onto a `Format` attribute (no panics / total).
    #[test]
    fn scope_projections_are_total() {
        assert_eq!(ForwardAttr::Bold.format(), Format::Bold);
        assert_eq!(LineFormat::Framed.format(), Format::Framed);
        assert_eq!(RegionFormat::Warichu.format(), Format::Warichu);
        assert_eq!(
            RegionFormat::Bold { padded: true }.format(),
            Format::Bold,
            "range and forward Bold share the attribute identity"
        );
    }
}
