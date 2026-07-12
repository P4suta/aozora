//! Gaiji (外字) resolution — mapping `※［＃…、mencode］` references
//! to real Unicode characters.
//!
//! Two incoming shapes per the Aozora annotation manual:
//!
//! ```text
//!   ※［＃「description」、第3水準1-85-54］    ← JIS X 0213 plane-row-cell
//!   ※［＃「description」、U+XXXX、page-line］ ← explicit Unicode codepoint
//! ```
//!
//! The lexer's classify-stage recogniser (`aozora-pipeline::lexer::classify::recognize_gaiji`)
//! captures `description` and `mencode` verbatim and leaves `ucs = None`;
//! this module turns that reference into a concrete [`Resolved`] by
//! consulting two `phf::Map`s compiled into the binary
//! (one for the single-codepoint majority and one for the 25
//! combining-sequence cells) and, for `U+XXXX` shaped mencodes,
//! parsing the hex digits directly.
//!
//! ## Why a `Resolved` enum
//!
//! 25 cells in JIS X 0213:2004 plane 1 (Ainu か゚ family, IPA tone marks,
//! a handful of accented Latin) decode to a *combining sequence* — two
//! Unicode scalars that must travel together. A single `char` cannot
//! carry them, so the resolved value is either a [`char`] (the
//! ~99.4% common path) or a `&'static str` borrowed from the
//! generated combo table. Both variants are `Copy`, so embedding
//! `Option<Resolved>` in the parser's `Gaiji` payload does not
//! perturb its `Copy`-able tree.
//!
//! ## Lookup order
//!
//! 1. **`existing`** — the caller-provided codepoint (e.g. extracted
//!    by an earlier escape recogniser); short-circuit identity.
//! 2. **Combo table** — checked first for `mencode` because it is the
//!    only way to honour a 2-codepoint cell.
//! 3. **Single-char table** — the bulk path; one perfect-hash probe
//!    in `.rodata`.
//! 4. **`U+XXXX` prefix** — `U+` followed by 1–6 hex digits. Parsed
//!    as a hex integer, validated via [`char::from_u32`].
//! 5. **Description fallback** — small secondary table keyed by the
//!    literal description text (well-known shapes like 〓, 〻).
//! 6. **None** — unresolved. Renderer falls back to the raw
//!    `description` bytes.
//!
//! ## Why two PHF maps rather than one enum-valued map
//!
//! The single-char map is 4 329 entries; the combo map is 25.
//! Storing the common path as `phf::Map<&str, char>` keeps each value
//! at 4 bytes (vs 16-byte `&str`) and the cache footprint of the hot
//! lookup path tight. The combo map is consulted second; misses
//! there cost a single probe.

use core::fmt;

use crate::jisx0213_table::{
    DESCRIPTION_TO_CHAR, JISX0213_MENCODE_TO_CHAR, JISX0213_MENCODE_TO_STR, ROMAN_NUMERAL_LOWER,
    ROMAN_NUMERAL_UPPER,
};

/// Resolution outcome — either a single Unicode scalar or a static
/// string covering a combining sequence.
///
/// `Copy` so it can sit inside `Gaiji` without breaking the parser
/// tree's `Copy` chain.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Resolved {
    /// Common path: the mencode mapped to a single Unicode scalar
    /// (~99.4% of JIS X 0213:2004 cells, plus all `U+XXXX` shapes
    /// and the description fallback).
    Char(char),
    /// JIS X 0213 combining-sequence cell — 25 entries in plane 1
    /// (Ainu か゚ family, IPA tone marks, accented Latin). The string
    /// is borrowed from a static `phf::Map` value.
    Multi(&'static str),
}

impl Resolved {
    /// Convenience: write the resolved char(s) into any [`fmt::Write`].
    /// Renderer / hover / inlay-hint paths all take this shape.
    ///
    /// # Errors
    /// Propagates the writer's own errors verbatim.
    pub fn write_to<W: fmt::Write>(self, w: &mut W) -> fmt::Result {
        match self {
            Self::Char(c) => w.write_char(c),
            Self::Multi(s) => w.write_str(s),
        }
    }

    /// Returns the resolved single `char` if and only if this is a
    /// [`Resolved::Char`]. Combo cells return `None`.
    #[must_use]
    pub fn as_char(self) -> Option<char> {
        match self {
            Self::Char(c) => Some(c),
            Self::Multi(_) => None,
        }
    }

    /// Total UTF-8 length of the resolved value (1..=8 bytes in
    /// practice).
    #[must_use]
    pub fn utf8_len(self) -> usize {
        match self {
            Self::Char(c) => c.len_utf8(),
            Self::Multi(s) => s.len(),
        }
    }
}

/// Pure-function lookup used by `aozora-pipeline`'s classify stage
/// to populate the gaiji node's `ucs` field at construction time.
///
/// `existing` is the short-circuit for callers that already extracted
/// a codepoint from the source. Pass `None` to fall through to the
/// table layers.
#[must_use]
pub fn lookup(
    existing: Option<char>,
    mencode: Option<&str>,
    description: &str,
) -> Option<Resolved> {
    if let Some(ch) = existing {
        return Some(Resolved::Char(ch));
    }
    if let Some(m) = mencode {
        // Combo table first: the 25 multi-codepoint cells live only
        // here. A miss is a single PHF probe — cheap.
        if let Some(&s) = JISX0213_MENCODE_TO_STR.get(m) {
            return Some(Resolved::Multi(s));
        }
        if let Some(&ch) = JISX0213_MENCODE_TO_CHAR.get(m) {
            return Some(Resolved::Char(ch));
        }
        if let Some(ch) = parse_u_plus(m) {
            return Some(Resolved::Char(ch));
        }
    }
    if let Some(&ch) = DESCRIPTION_TO_CHAR.get(description) {
        return Some(Resolved::Char(ch));
    }
    // Bare `ローマ数字N` (#326) composes from the U+2160 block — the
    // N≥13 forms have no single JIS cell, so the dictionary above misses.
    if let Some(s) = roman_numeral_glyph(description) {
        return Some(Resolved::Multi(s));
    }
    // Smart fallback: a description that is *itself* a single
    // character resolves to that character. Common in real corpora
    // when the author CAN type the kanji (e.g. on a modern IME) but
    // wants the reader to see a `※[#…]` annotation pointing at the
    // JIS source. Mencode/dictionary tiers above already short-
    // circuited any case where the table had a more specific answer,
    // so this only fires when description is a one-glyph payload
    // and nothing else matched.
    //
    // Counts grapheme clusters by Unicode scalars: a base-plus-
    // combining sequence (e.g. アクセント分解) returns >1 char and
    // falls through to the final `None`. Surrogate halves can't
    // appear in `&str` so single-`char` is unambiguous here.
    let mut chars = description.chars();
    if let Some(only) = chars.next()
        && chars.next().is_none()
    {
        return Some(Resolved::Char(only));
    }
    None
}

/// Compose the Unicode roman numeral for a bare `ローマ数字N` /
/// `ローマ数字N小文字` gaiji description (#326), or `None` if the
/// description is not that shape or `N` is outside the composed range.
///
/// `N` accepts ASCII or full-width digits. The numeral is read from the
/// build-time [`ROMAN_NUMERAL_UPPER`] / [`ROMAN_NUMERAL_LOWER`] tables,
/// which spell `N` with the U+2160 / U+2170 single-letter blocks (so
/// `17` → `ⅩⅤⅠⅠ`). `N = 0` and out-of-range `N` resolve to `None` —
/// the gaiji then stays unresolved rather than rendering an empty glyph.
#[must_use]
fn roman_numeral_glyph(description: &str) -> Option<&'static str> {
    let rest = description.strip_prefix("ローマ数字")?;
    let (digits, lower) = rest
        .strip_suffix("小文字")
        .map_or((rest, false), |d| (d, true));
    if digits.is_empty() {
        return None;
    }
    let mut n: usize = 0;
    for ch in digits.chars() {
        let d = ch
            .to_digit(10)
            .or_else(|| ('０'..='９').contains(&ch).then(|| ch as u32 - '０' as u32))?;
        n = n.checked_mul(10)?.checked_add(d as usize)?;
    }
    let table = if lower {
        &ROMAN_NUMERAL_LOWER
    } else {
        &ROMAN_NUMERAL_UPPER
    };
    table.get(n).copied().filter(|s| !s.is_empty())
}

/// Parse a `U+XXXX` style mencode — 1 to 6 hex digits after the
/// literal `U+` prefix — and validate the result via
/// [`char::from_u32`]. Returns `None` for surrogates, non-characters,
/// and out-of-range integers, rather than panicking, so malformed
/// input falls cleanly through to the description fallback.
#[must_use]
fn parse_u_plus(mencode: &str) -> Option<char> {
    let hex = mencode.strip_prefix("U+")?;
    // Reject empty / oversized; `u32::from_str_radix` would accept
    // 10-digit inputs but those can't fit a Unicode scalar.
    if hex.is_empty() || hex.len() > 6 {
        return None;
    }
    let code = u32::from_str_radix(hex, 16).ok()?;
    char::from_u32(code)
}

// Gaiji descriptions (the text inside `「…」`) that resolve to a
// canonical character without depending on the mencode tail. Sourced
// from `crates/aozora-encoding/data/aozora-gaiji-chuki.tsv` (the
// official 8th-edition 外字注記辞書, ~8 800 entries) plus
// `aozora-gaiji-special.tsv` (hand-curated 〓 / 〻 placeholders).
// Generated by `xtask gaiji-gen` and exported from
// `crate::jisx0213_table::DESCRIPTION_TO_CHAR` (alias-imported at
// the top of this module).

/// Pretty-printer for tests and diagnostics. Returns
/// `(single_char_count, combo_count, description_count)`.
#[must_use]
pub fn table_sizes() -> (usize, usize, usize) {
    (
        JISX0213_MENCODE_TO_CHAR.len(),
        JISX0213_MENCODE_TO_STR.len(),
        DESCRIPTION_TO_CHAR.len(),
    )
}

// ────────────────────────────────────────────────────────────────────
// Full-document / cursor-local gaiji scan + resolution
// ────────────────────────────────────────────────────────────────────
//
// Single authority for pulling `※［＃…］` references out of raw source
// and resolving each to its glyph. Editor surfaces (`aozora-wasm`
// inlay hints / cursor hover) and batch callers (`aozora-py`
// `gaiji_resolutions`) both drive this; the `aozora::json` projection
// only serialises the [`GaijiResolution`] values produced here. Kept
// next to [`lookup`] so the scan and the table it consults share one
// home.

/// Opening delimiter of a *refmark* gaiji reference (`※［＃`).
pub const GAIJI_OPEN: &str = "※［＃";
/// The bracket-hash annotation opener (`［＃`).
///
/// Shared by the refmark form (`※` + this) and the standalone
/// external-character form (#122), which carries no `※`. The standalone form
/// needs the [`recognize_gaiji_body`] gate to tell it apart from a plain
/// directive (`［＃改ページ］`).
pub const BRACKET_HASH: &str = "［＃";
/// The refmark prefix (`※`) that distinguishes a refmark gaiji from the
/// standalone form.
pub const GAIJI_REFMARK: &str = "※";
/// Closing delimiter of a gaiji reference (`］`).
pub const GAIJI_CLOSE: &str = "］";
/// Window half-width (bytes) for the cursor-local [`find_span`] scan. A
/// real `※［＃…］` span is at most a few hundred bytes; capping the
/// search makes per-cursor resolution O(window) rather than O(doc).
pub const MAX_GAIJI_SPAN_LEN: usize = 512;

/// One resolved gaiji reference located in source.
///
/// Byte offsets index the source string. `mencode` / `codepoint` /
/// `resolved` are `None` when absent or unresolved; `codepoint` is also
/// `None` for combining-sequence cells (which have no single scalar).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GaijiResolution {
    /// Start byte offset of the `※［＃…］` span (inclusive).
    pub start: usize,
    /// End byte offset of the span (exclusive).
    pub end: usize,
    /// The `「…」` description text (or the whole body if no quotes).
    pub description: String,
    /// The mencode tail (`第3水準…` / `U+XXXX`), if present.
    pub mencode: Option<String>,
    /// Resolved codepoint as `u32`, when resolution is a single scalar.
    pub codepoint: Option<u32>,
    /// Resolved glyph(s), when [`lookup`] succeeds.
    pub resolved: Option<String>,
}

/// Split a gaiji body into `(description, mencode?)` — the single
/// authority shared by the parser's recogniser, the LSP resolution
/// view, and [`resolve_at`] (the `gaiji()` wire projection).
///
/// Two body shapes occur (Aozora annotation manual + corpus §6):
///
/// * **Simple quoted** — `「desc」` optionally followed by `、mencode`,
///   where `desc` has no nested `「」` and nothing but a `、mencode`
///   tail follows. The surrounding `「」` is stripped (the serializer
///   re-adds it).
/// * **Composed / bare** — everything else: the composed-glyph / 正字 /
///   屋号 forms (`「X」の「Y」に代えて「Z」、mencode`) and the quote-less
///   `二の字点、1-2-23`. The body is split on `、` and walked **from the
///   right**: the maximal trailing run of mencode-shaped tokens
///   ([`is_mencode_shaped`] / [`is_page_line_shaped`]) is the `mencode`;
///   everything before it is the `description`, kept verbatim (internal
///   `「」` and `、` preserved). Scanning right-to-left keeps a `、` that
///   belongs to the description (`…面から一、二画目をとったもの`) out of
///   the split — the bug a naive first-`、` split hits on composed forms.
///
/// Returns borrowed slices of `body`. A body with no trailing
/// mencode-shaped run (or none before it) yields the whole trimmed body
/// as the description and `None` mencode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GaijiBody<'a> {
    /// The description text — quotes stripped for the simple quoted form,
    /// kept verbatim for the composed / bare forms.
    pub description: &'a str,
    /// The trailing mencode reference (`第3水準…` / `U+XXXX`), if present.
    pub mencode: Option<&'a str>,
    /// `true` for the simple `「desc」` quoted form (a valid gaiji even
    /// without a mencode); `false` for the composed / bare forms, which
    /// the parser accepts only with a trailing mencode anchor.
    pub quoted: bool,
}

/// See the type-level docs on [`GaijiBody`].
#[must_use]
pub fn parse_gaiji_body(body: &str) -> GaijiBody<'_> {
    let body = body.trim();
    // Simple quoted form: 「desc」 with no nested quotes, tail empty or 、mencode.
    if let Some(rest) = body.strip_prefix('「')
        && let Some(close) = rest.find('」')
    {
        let desc = &rest[..close];
        let tail = rest[close + '」'.len_utf8()..].trim();
        // A simple quoted gaiji has an empty tail, a `、mencode` tail, or
        // (shape 2) a mencode-shaped tail fused directly after `」` with no
        // separating `、` (`「金＋夫」第3水準1-93-4`). The bare tail is gated on
        // it actually being a mencode, so a non-mencode tail (e.g. a `に傍点`
        // forward directive) falls through to the composed scan below.
        let bare_mencode_tail =
            !tail.is_empty() && is_mencode_shaped(mencode_resolution_token(tail));
        if !desc.is_empty()
            && !desc.contains(['「', '」'])
            && (tail.is_empty() || tail.starts_with('、') || bare_mencode_tail)
        {
            let mencode = tail.strip_prefix('、').map_or_else(
                || (!tail.is_empty()).then_some(tail),
                |m| {
                    let m = m.trim();
                    (!m.is_empty()).then_some(m)
                },
            );
            return GaijiBody {
                description: desc,
                mencode,
                quoted: true,
            };
        }
    }
    // Composed / bare form: right-to-left mencode scan. The run admits both
    // the canonical page-line forms and the near-miss ones (fused 上/中/下,
    // full-width minus, poetry locators).
    let shaped = |t: &str| is_mencode_shaped(t) || is_near_miss_page_line_shaped(t);
    let commas: Vec<usize> = body.match_indices('、').map(|(i, _)| i).collect();
    let tokens: Vec<&str> = body.split('、').map(str::trim).collect();
    let mut run_start = tokens.len();
    while run_start > 0 && shaped(tokens[run_start - 1]) {
        run_start -= 1;
    }
    // FP guard: a near-miss-only page-line token (one the canonical form
    // rejects) is admitted only when the run is anchored by a real mencode
    // token (第N水準 / U+ / bare N-N-N). This is what keeps a proofreader /
    // 段組 directive tail (`、58-下15`) from being promoted to a gaiji, while
    // the canonical description-anchored forms (`小書き片仮名ン、500-下-19`)
    // keep resolving without a men-ku-ten.
    let run = &tokens[run_start..];
    let uses_near_miss = run
        .iter()
        .any(|t| is_near_miss_page_line_shaped(t) && !is_page_line_shaped(t));
    let anchored = run.iter().any(|t| is_mencode_shaped(t));
    if run_start == tokens.len() || run_start == 0 || (uses_near_miss && !anchored) {
        return GaijiBody {
            description: body,
            mencode: None,
            quoted: false,
        };
    }
    let boundary = commas[run_start - 1];
    GaijiBody {
        description: body[..boundary].trim(),
        mencode: Some(body[boundary + '、'.len_utf8()..].trim()),
        quoted: false,
    }
}

/// Whether a gaiji `description` can be kept (it both serializes and
/// round-trips); otherwise the bracket falls through to a plain directive.
///
/// Rejects:
///   - a description embedding `［＃` (a nested annotation opener would leak a
///     bare `［＃` outside the directive wrapper, violating the Tier A canary),
///     and
///   - a description carrying structural `「…」` quotes, *except* the
///     composed-glyph / 正字 / 屋号 forms (balanced quotes anchored by a
///     trailing `、mencode`; a quote-bearing description without a mencode
///     anchor stays rejected, since the serializer's wrapper would unbalance
///     it).
#[must_use]
pub fn gaiji_description_serializable(description: &str, has_mencode: bool) -> bool {
    if description.contains("［＃") {
        return false;
    }
    if description.contains(['「', '」']) {
        let balanced = description.matches('「').count() == description.matches('」').count();
        return balanced && has_mencode;
    }
    true
}

/// Parse a `［＃…］` body and decide whether it is a recognised gaiji reference.
///
/// The single recognition gate shared by the parser's recogniser, the
/// standalone scan in [`gaiji_resolutions`] / [`resolve_at`], and the LSP /
/// wire resolution view.
///
/// Returns the parsed [`GaijiBody`] iff the body is a gaiji; `None` for a
/// plain directive (e.g. `改ページ`). A gaiji needs a non-empty description and
/// either the simple `「…」`-quoted form, a trailing mencode anchor, *or* a bare
/// description that is itself a known dictionary entry (the corpus form
/// `※［＃二重かっこ開く］`); the description must round-trip
/// ([`gaiji_description_serializable`]).
#[must_use]
pub fn recognize_gaiji_body(body: &str) -> Option<GaijiBody<'_>> {
    let parsed = parse_gaiji_body(body);
    // A bare body (no `「」` quotes, no mencode anchor) is normally an ordinary
    // directive — `改ページ`, `ここから2字下げ` — not a glyph reference, so it is
    // gated out. The exception is a description that is *itself* a known
    // dictionary entry (e.g. the corpus form `※［＃二重かっこ開く］`): an
    // unanchored but unambiguous glyph reference that resolves directly. Gating
    // on dictionary membership keeps real directives out while admitting these.
    let bare_unanchored = !parsed.quoted && parsed.mencode.is_none();
    let bare_resolvable = DESCRIPTION_TO_CHAR.contains_key(parsed.description)
        || roman_numeral_glyph(parsed.description).is_some();
    if parsed.description.is_empty()
        || (bare_unanchored && !bare_resolvable)
        || !gaiji_description_serializable(parsed.description, parsed.mencode.is_some())
    {
        return None;
    }
    Some(parsed)
}

/// The JIS / U+ token of a `mencode`, dropping any trailing 底本ページ-行
/// suffix (`第3水準1-84-27、144-上-9` → `第3水準1-84-27`, `U+74FC、372-10`
/// → `U+74FC`) so the resolver sees a clean men-ku-ten / codepoint.
#[must_use]
pub fn mencode_resolution_token(mencode: &str) -> &str {
    mencode
        .split_once('、')
        .map_or(mencode, |(token, _)| token.trim())
}

/// A structured JIS X 0213 面区点 (men-ku-ten) reference — the clean
/// `第N水準P-K-T` mencode form parsed into its components.
///
/// 水準 (level) is redundant with the plane (第3水準 = plane 1,
/// 第4水準 = plane 2), so only the plane is stored; [`Self::level`]
/// recovers it and [`fmt::Display`] reproduces the exact source form.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MenKuTen {
    /// Plane (面): 1 (第3水準) or 2 (第4水準).
    pub plane: u8,
    /// Row (区), 1..=94.
    pub ku: u8,
    /// Cell (点), 1..=94.
    pub ten: u8,
}

impl MenKuTen {
    /// JIS 水準 level: plane 1 → 3, plane 2 → 4.
    #[must_use]
    pub fn level(self) -> u8 {
        self.plane + 2
    }
}

impl fmt::Display for MenKuTen {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "第{}水準{}-{}-{}",
            self.level(),
            self.plane,
            self.ku,
            self.ten
        )
    }
}

/// Parse the clean `第N水準P-K-T` men-ku-ten form.
///
/// Returns `None` for anything else — bare `P-K-T`, page-line-suffixed
/// tails, leading-zero or inconsistent-level shapes — which the caller
/// keeps verbatim as [`GaijiCanonical::Unresolved`] so serialization
/// stays byte-exact for those.
#[must_use]
pub fn parse_menkuten(token: &str) -> Option<MenKuTen> {
    let after = token.strip_prefix('第')?;
    let suijun = after.find("水準")?;
    let level: u8 = after[..suijun].parse().ok()?;
    let mut parts = after[suijun + "水準".len()..].split('-');
    let plane: u8 = parts.next()?.parse().ok()?;
    let ku: u8 = parts.next()?.parse().ok()?;
    let ten: u8 = parts.next()?.parse().ok()?;
    // Exactly three components, a level consistent with the plane, and
    // no zero coordinate. The round-trip via `Display` is only exact
    // for the canonical shape, so reject everything else.
    if parts.next().is_some() || level != plane + 2 || plane == 0 || ku == 0 || ten == 0 {
        return None;
    }
    let mkt = MenKuTen { plane, ku, ten };
    // Guard the byte-exact round-trip: a leading-zero or otherwise
    // non-canonical source would not reproduce, so demand `token`
    // already be the canonical form.
    (mkt.to_string() == token).then_some(mkt)
}

/// The typed canonical value of a gaiji reference.
///
/// Replaces the former `(ucs, mencode)` pair on `Gaiji`: the resolved
/// glyph is derived on demand via [`Self::resolve`] and the source
/// mencode is reproduced via [`Self::write_mencode`], so there is a
/// single source of truth and the defensive `is_mencode_shaped` /
/// serializable validators dissolve into the variant choice.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GaijiCanonical<'src> {
    /// Structured `第N水準P-K-T` reference (clean form only).
    MenKuTen(MenKuTen),
    /// Explicit `U+XXXX` codepoint (clean form, no suffix).
    Unicode(char),
    /// Everything kept verbatim: bare / suffixed men-ku-ten, page-line
    /// references, description-only gaiji. `None` when the source had no
    /// mencode tail at all.
    Unresolved {
        /// The raw mencode tail, echoed unchanged on serialize.
        mencode: Option<&'src str>,
    },
}

impl<'src> GaijiCanonical<'src> {
    /// Classify a parsed mencode tail into its canonical form. Only the
    /// byte-exact `第N水準P-K-T` and `U+XXXX` shapes are structured; all
    /// else is retained verbatim as [`Self::Unresolved`].
    #[must_use]
    pub fn from_mencode(mencode: Option<&'src str>) -> Self {
        if let Some(m) = mencode {
            if let Some(mkt) = parse_menkuten(m) {
                return Self::MenKuTen(mkt);
            }
            if let Some(c) = parse_u_plus(m) {
                return Self::Unicode(c);
            }
        }
        Self::Unresolved { mencode }
    }

    /// Resolve to a concrete glyph via the single [`lookup`] authority,
    /// consulting the JIS tables and the `description` fallback.
    #[must_use]
    pub fn resolve(self, description: &str) -> Option<Resolved> {
        match self {
            Self::MenKuTen(m) => lookup(None, Some(&m.to_string()), description),
            Self::Unicode(c) => Some(Resolved::Char(c)),
            Self::Unresolved { mencode } => {
                lookup(None, mencode.map(mencode_resolution_token), description)
            }
        }
    }

    /// `true` when the source carried a mencode tail (drives the `、`
    /// separator in serialization).
    #[must_use]
    pub fn has_mencode(self) -> bool {
        !matches!(self, Self::Unresolved { mencode: None })
    }

    /// Write the canonical mencode token (without the leading `、`). The
    /// structured forms reproduce the clean `第N水準P-K-T` / `U+XXXX`
    /// shape; `Unresolved` echoes the raw tail verbatim.
    ///
    /// # Errors
    /// Propagates the writer's own errors.
    pub fn write_mencode<W: fmt::Write>(self, w: &mut W) -> fmt::Result {
        match self {
            Self::MenKuTen(m) => write!(w, "{m}"),
            Self::Unicode(c) => write!(w, "U+{:04X}", c as u32),
            Self::Unresolved { mencode } => mencode.map_or(Ok(()), |m| w.write_str(m)),
        }
    }
}

/// Whether `s` is a JIS X 0213 men-ku-ten / `U+XXXX` mencode token:
/// `N-N-N`, `第N水準N-N-N`, or `U+XXXX` (1–6 ASCII hex digits).
#[must_use]
pub fn is_mencode_shaped(s: &str) -> bool {
    if let Some(hex) = s.strip_prefix("U+") {
        return !hex.is_empty() && hex.len() <= 6 && hex.chars().all(|c| c.is_ascii_hexdigit());
    }
    // Optional `第N水準` prefix: skip past the digits + `水準` token if
    // present, then validate the remainder as `N-N-N`.
    let rest = s
        .strip_prefix('第')
        .and_then(|after_dai| {
            let nondigit = after_dai.find(|c: char| !c.is_ascii_digit())?;
            let (_digits, tail) = after_dai.split_at(nondigit);
            tail.strip_prefix("水準")
        })
        .unwrap_or(s);
    !rest.is_empty()
        && rest.chars().all(|c| c.is_ascii_digit() || c == '-')
        && rest.chars().any(|c| c.is_ascii_digit())
}

/// Whether `s` is a canonical 底本ページ-行 reference — `-`-joined parts, each
/// a digit run, a 上 / 中 / 下 column marker, or a volume marker
/// (`144-上-9`, `372-10`, `7巻-42-下-10`).
///
/// This is the strict form the description-anchored gaiji rely on (a
/// `小書き片仮名ン、500-下-19` glyph carries only this provenance tail with no
/// men-ku-ten), so it stays exact. The looser near-miss spellings live in
/// the private `is_near_miss_page_line_shaped` (mencode-anchored).
#[must_use]
pub fn is_page_line_shaped(s: &str) -> bool {
    !s.is_empty() && s.split('-').all(is_page_line_part)
}

/// One `-`-separated component of a canonical 底本ページ-行 reference.
fn is_page_line_part(p: &str) -> bool {
    if let Some(volume) = p.strip_suffix('巻') {
        return matches!(volume, "上" | "中" | "下" | "前" | "後") || is_digit_run(volume);
    }
    matches!(p, "上" | "中" | "下") || is_digit_run(p)
}

/// The 上 / 中 / 下 column markers plus their 段 register variants, longest
/// first so a `下段` prefix/suffix strips before the shorter `下`.
const COLUMN_MARKERS: [&str; 6] = ["上段", "中段", "下段", "上", "中", "下"];

/// Whether `s` is a *near-miss* 底本ページ-行 reference — a superset of
/// [`is_page_line_shaped`] that also tolerates the corpus-attested near-miss
/// spellings: a full-width U+FF0D `－` separator (`94－11`), a 上/中/下(+段)
/// register fused directly to a digit run on either side (`下8`, `109下`,
/// `下段5`), and the poetry/register locator (`P61`, `下段5首目`).
///
/// These near-misses collide with proofreader / 段組 directive tails, so
/// [`parse_gaiji_body`] admits a near-miss-only token only when the run is
/// also anchored by a real mencode token. A bare 段 register (`上段`) is
/// deliberately *not* accepted — it is a 段組 directive operand, not a
/// page-line part.
fn is_near_miss_page_line_shaped(s: &str) -> bool {
    !s.is_empty() && s.split(['-', '－']).all(is_near_miss_page_line_part)
}

/// One separator-separated component of a near-miss 底本ページ-行 reference.
fn is_near_miss_page_line_part(p: &str) -> bool {
    if let Some(volume) = p.strip_suffix('巻') {
        return matches!(volume, "上" | "中" | "下" | "前" | "後") || is_digit_run(volume);
    }
    // Poetry/register locator: an optional leading `P` page prefix and an
    // optional trailing `首目` line-counter wrap the digit / column core
    // (`P61`, `下段5首目`).
    let core = p.strip_prefix('P').unwrap_or(p);
    let core = core.strip_suffix("首目").unwrap_or(core);
    // Bare marker stays the canonical set only; a bare 段 register (`上段`)
    // is a 段組 operand and must not read as a page-line part.
    if matches!(core, "上" | "中" | "下") {
        return true;
    }
    // Register marker fused before a digit run (`下8`, `下段5`) or after one
    // (`109下`, `109下段`).
    if let Some(rest) = COLUMN_MARKERS.iter().find_map(|m| core.strip_prefix(m)) {
        return is_digit_run(rest);
    }
    if let Some(rest) = COLUMN_MARKERS.iter().find_map(|m| core.strip_suffix(m)) {
        return is_digit_run(rest);
    }
    is_digit_run(core)
}

/// A non-empty run of ASCII or full-width decimal digits.
fn is_digit_run(p: &str) -> bool {
    !p.is_empty()
        && p.chars()
            .all(|c| c.is_ascii_digit() || ('０'..='９').contains(&c))
}

/// Resolve the gaiji span at `[start..end)` — either the refmark form
/// (`※［＃…］`) or the standalone form (`［＃…］`, #122).
///
/// The refmark `※` is itself the gaiji marker, so a `※［＃…］` span is always
/// resolved. A standalone `［＃…］` must pass [`recognize_gaiji_body`] (else it
/// is a plain directive like `［＃改ページ］` and yields `None`). Also `None`
/// if the delimiters don't bracket a parseable body — defensive, since
/// offsets from [`gaiji_resolutions`] / [`find_span`] always satisfy it.
#[must_use]
pub fn resolve_at(source: &str, start: usize, end: usize) -> Option<GaijiResolution> {
    let span = source.get(start..end)?;
    let (open, standalone) = if span.starts_with(GAIJI_OPEN) {
        (GAIJI_OPEN, false)
    } else if span.starts_with(BRACKET_HASH) {
        (BRACKET_HASH, true)
    } else {
        return None;
    };
    let body_start = start.checked_add(open.len())?;
    let body_end = end.checked_sub(GAIJI_CLOSE.len())?;
    if body_end <= body_start || body_end > source.len() {
        return None;
    }
    let body = source.get(body_start..body_end)?;
    let GaijiBody {
        description,
        mencode,
        ..
    } = if standalone {
        recognize_gaiji_body(body)?
    } else {
        parse_gaiji_body(body)
    };
    let (resolved, codepoint) = lookup(None, mencode.map(mencode_resolution_token), description)
        .map_or((None, None), |r| {
            let mut s = String::new();
            _ = r.write_to(&mut s);
            (Some(s), r.as_char().map(|c| c as u32))
        });
    Some(GaijiResolution {
        start,
        end,
        description: description.to_owned(),
        mencode: mencode.map(str::to_owned),
        codepoint,
        resolved,
    })
}

/// All gaiji references in `source`, resolved, in source order. Walks
/// the source linearly once; cost is `O(source)`.
#[must_use]
pub fn gaiji_resolutions(source: &str) -> Vec<GaijiResolution> {
    let mut out = Vec::new();
    let mut cursor = 0usize;
    // Scan the bracket-hash opener so both the refmark (`※［＃`) and the
    // standalone (`［＃`, #122) forms are seen; `resolve_at` folds a preceding
    // `※` into the span and gates the standalone form against recognition.
    while let Some(rel) = source[cursor..].find(BRACKET_HASH) {
        let hash_open = cursor + rel;
        let span_start = if source[..hash_open].ends_with(GAIJI_REFMARK) {
            hash_open - GAIJI_REFMARK.len()
        } else {
            hash_open
        };
        let body_start = hash_open + BRACKET_HASH.len();
        let Some(close_rel) = source[body_start..].find(GAIJI_CLOSE) else {
            break;
        };
        let span_end = body_start + close_rel + GAIJI_CLOSE.len();
        if let Some(res) = resolve_at(source, span_start, span_end) {
            out.push(res);
        }
        cursor = span_end;
    }
    out
}

/// Byte-range of the `※［＃…］` span containing `byte_offset`.
///
/// Scans only a bounded window around the cursor (cost independent of
/// doc size). For editor cursor-hover; full-document callers use
/// [`gaiji_resolutions`].
#[must_use]
pub fn find_span(source: &str, byte_offset: usize) -> Option<(usize, usize)> {
    if source.is_empty() {
        return None;
    }
    let win_start =
        snap_to_char_boundary_left(source, byte_offset.saturating_sub(MAX_GAIJI_SPAN_LEN));
    let win_end = snap_to_char_boundary_right(
        source,
        byte_offset
            .saturating_add(MAX_GAIJI_SPAN_LEN)
            .min(source.len()),
    );
    let window = &source[win_start..win_end];
    let win_offset = byte_offset.saturating_sub(win_start);

    for (hash_in_win, _) in window.match_indices(BRACKET_HASH) {
        let after_open = hash_in_win + BRACKET_HASH.len();
        let Some(end_rel) = window.get(after_open..).and_then(|s| s.find(GAIJI_CLOSE)) else {
            continue;
        };
        let end_in_win = after_open + end_rel + GAIJI_CLOSE.len();
        // Fold a preceding `※` into the span (refmark form).
        let start_in_win = if window[..hash_in_win].ends_with(GAIJI_REFMARK) {
            hash_in_win - GAIJI_REFMARK.len()
        } else {
            hash_in_win
        };
        if (start_in_win..end_in_win).contains(&win_offset) {
            return Some((win_start + start_in_win, win_start + end_in_win));
        }
    }
    None
}

const fn snap_to_char_boundary_left(s: &str, mut idx: usize) -> usize {
    while idx > 0 && !s.is_char_boundary(idx) {
        idx -= 1;
    }
    idx
}

const fn snap_to_char_boundary_right(s: &str, mut idx: usize) -> usize {
    let len = s.len();
    while idx < len && !s.is_char_boundary(idx) {
        idx += 1;
    }
    idx
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_gaiji_body_splits_quoted_composed_and_bare() {
        // Simple quoted form: the 「」 wrapper is stripped.
        assert_eq!(
            parse_gaiji_body("「木＋吶のつくり」、第3水準1-85-54"),
            GaijiBody {
                description: "木＋吶のつくり",
                mencode: Some("第3水準1-85-54"),
                quoted: true,
            }
        );
        // Quoted form with no mencode is still a valid (quoted) gaiji.
        assert_eq!(
            parse_gaiji_body("「々」"),
            GaijiBody {
                description: "々",
                mencode: None,
                quoted: true,
            }
        );
        // Composed-glyph form (#181): the whole description is kept verbatim,
        // NOT cut at the first 、 — the right-to-left scan finds the mencode.
        assert_eq!(
            parse_gaiji_body("「廰」の「广」を「厂」に、第3水準1-15-94"),
            GaijiBody {
                description: "「廰」の「广」を「厂」に",
                mencode: Some("第3水準1-15-94"),
                quoted: false,
            }
        );
        // A 、 belonging to the description stays put (right-to-left scan).
        assert_eq!(
            parse_gaiji_body("面から一、二画目をとったもの、第3水準1-15-94"),
            GaijiBody {
                description: "面から一、二画目をとったもの",
                mencode: Some("第3水準1-15-94"),
                quoted: false,
            }
        );
        // Bare form, no quotes.
        assert_eq!(
            parse_gaiji_body("二の字点、1-2-23"),
            GaijiBody {
                description: "二の字点",
                mencode: Some("1-2-23"),
                quoted: false,
            }
        );
        // A trailing 底本ページ-行 suffix stays inside mencode verbatim
        // (resolution strips it via `mencode_resolution_token`).
        assert_eq!(
            parse_gaiji_body("「※」、第3水準1-84-27、144-上-9"),
            GaijiBody {
                description: "※",
                mencode: Some("第3水準1-84-27、144-上-9"),
                quoted: true,
            }
        );
        // No mencode anchor and no quotes: the whole body is the description.
        assert_eq!(
            parse_gaiji_body("二の字点"),
            GaijiBody {
                description: "二の字点",
                mencode: None,
                quoted: false,
            }
        );
    }

    #[test]
    fn near_miss_page_line_is_a_superset_of_canonical() {
        // Canonical forms are accepted by both predicates.
        for s in ["144-上-9", "7巻-42-下-10", "372-10"] {
            assert!(is_page_line_shaped(s), "canonical {s}");
            assert!(is_near_miss_page_line_shaped(s), "near-miss superset {s}");
        }
        // Near-miss-only forms: rejected by canonical, accepted by near-miss.
        // Shape 1: 上/中/下 fused directly before a digit run.
        // Shape 3: full-width minus U+FF0D separator; 下 fused after digits.
        // Shape 4: poetry/register locator (段 / 首目 / `P` page prefix).
        for s in [
            "383-下8",
            "2-下3",
            "323-上1",
            "94－11",
            "92－３",
            "109下－4",
            "69-下段9首目",
            "P61-下段5首目",
        ] {
            assert!(!is_page_line_shaped(s), "canonical must reject {s}");
            assert!(is_near_miss_page_line_shaped(s), "near-miss accepts {s}");
        }
        // A bare 段 register is a 段組 operand, NOT a page-line part — both
        // predicates reject it, so `［＃ここから２段組、上段］` stays a directive.
        for s in ["上段", "下段", "中段", "38-上段-1"] {
            assert!(!is_page_line_shaped(s), "canonical rejects bare 段 {s}");
            assert!(
                !is_near_miss_page_line_shaped(s),
                "near-miss rejects bare 段 {s}"
            );
        }
        // Neither accepts non-page-line tails.
        for s in ["U+304B", "巻-3-4", "X1－1", "漢字", ""] {
            assert!(!is_page_line_shaped(s));
            assert!(!is_near_miss_page_line_shaped(s));
        }
    }

    #[test]
    fn parse_gaiji_body_handles_near_miss_locators() {
        // Shape 1: 上/中/下 fused before a digit run in the page-line tail.
        assert_eq!(
            parse_gaiji_body("「※」は「てへん＋劣」、第3水準1-84-77、383-下8"),
            GaijiBody {
                description: "「※」は「てへん＋劣」",
                mencode: Some("第3水準1-84-77、383-下8"),
                quoted: false,
            }
        );
        // Shape 2: mencode fused directly after `」` with no separating `、`.
        assert_eq!(
            parse_gaiji_body("「金＋夫」第3水準1-93-4"),
            GaijiBody {
                description: "金＋夫",
                mencode: Some("第3水準1-93-4"),
                quoted: true,
            }
        );
        // Shape 3: full-width minus separator in a composed description's
        // page-line tail (the relaxed split is what makes the run shaped).
        assert_eq!(
            parse_gaiji_body(
                "「※」は「いしへん」＋「乏」、読みは「いしばり」、第3水準1-88-93、94－11"
            ),
            GaijiBody {
                description: "「※」は「いしへん」＋「乏」、読みは「いしばり」",
                mencode: Some("第3水準1-88-93、94－11"),
                quoted: false,
            }
        );
        // Shape 4: poetry/register locator with `P` prefix + 段 + 首目.
        assert_eq!(
            parse_gaiji_body("「※」は「王へん」に「干」、第3水準1-87-83、P61-下段5首目"),
            GaijiBody {
                description: "「※」は「王へん」に「干」",
                mencode: Some("第3水準1-87-83、P61-下段5首目"),
                quoted: false,
            }
        );
    }

    #[test]
    fn near_miss_locators_resolve_to_the_menkuten_glyph() {
        // The relaxed provenance tail is irrelevant to resolution: the glyph
        // comes from the men-ku-ten anchor. 第3水準1-93-4 = 鈇 (U+9207).
        let body = recognize_gaiji_body("「金＋夫」第3水準1-93-4").expect("shape 2 is a gaiji");
        assert_eq!(
            lookup(
                None,
                body.mencode.map(mencode_resolution_token),
                body.description
            ),
            Some(Resolved::Char('\u{9207}'))
        );
    }

    #[test]
    fn near_miss_page_line_requires_a_mencode_anchor() {
        // A near-miss-only tail with no men-ku-ten anchor is NOT enough on its
        // own — it collides with proofreader / 段組 directive tails, so it stays
        // a directive rather than being promoted to a gaiji.
        assert!(recognize_gaiji_body("「あ」に「い」、94－11").is_none());
        assert!(recognize_gaiji_body("「あ」に「い」、383-下8").is_none());
        // Real proofreader / layout directives with a near-miss-shaped tail.
        assert!(recognize_gaiji_body("ここから２段組、上段").is_none());
        assert!(recognize_gaiji_body("底本の閉じ括弧は「』」、58-下15").is_none());
        assert!(recognize_gaiji_body("底本ルビは「もら」と誤記、175-上段-4").is_none());
        // WITH a men-ku-ten anchor the same near-miss tail IS a gaiji.
        assert!(recognize_gaiji_body("「あ」に「い」、第3水準1-15-94、383-下8").is_some());
        // A canonical description-anchored gaiji keeps resolving with only a
        // page-line tail (no men-ku-ten) — the anchor rule never touches it.
        assert!(recognize_gaiji_body("小書き片仮名ン、500-下-19").is_some());
        // Plain directives stay directives.
        assert!(recognize_gaiji_body("改ページ").is_none());
        assert!(recognize_gaiji_body("ここから2字下げ").is_none());
    }

    #[test]
    fn gaiji_resolutions_empty_for_plain_text() {
        assert!(gaiji_resolutions("plain text, no gaiji").is_empty());
    }

    #[test]
    fn gaiji_resolutions_resolves_single_char_description() {
        // A description that is itself one glyph resolves via the smart
        // fallback in `lookup`.
        let src = "前※［＃「々」］後";
        let res = gaiji_resolutions(src);
        assert_eq!(res.len(), 1);
        let g = &res[0];
        assert_eq!(g.description, "々");
        assert_eq!(g.resolved.as_deref(), Some("々"));
        assert_eq!(g.codepoint, Some('々' as u32));
        // Offsets bracket the literal `※［＃…］` span.
        assert_eq!(&src[g.start..g.end], "※［＃「々」］");
    }

    #[test]
    fn gaiji_resolutions_includes_standalone_form() {
        // Standalone `［＃…］` (no `※`) external-character form (#122/#181):
        // previously invisible to the resolution view.
        let src = "前［＃「木＋吶のつくり」、第3水準1-85-54］後";
        let res = gaiji_resolutions(src);
        assert_eq!(res.len(), 1);
        let g = &res[0];
        assert_eq!(g.description, "木＋吶のつくり");
        assert_eq!(g.mencode.as_deref(), Some("第3水準1-85-54"));
        assert_eq!(g.resolved.as_deref(), Some("枘"));
        // The span has no `※`, so it starts at the `［`.
        assert_eq!(
            &src[g.start..g.end],
            "［＃「木＋吶のつくり」、第3水準1-85-54］"
        );
    }

    #[test]
    fn gaiji_resolutions_excludes_plain_directives() {
        // A bracket-hash directive is NOT a gaiji — the standalone gate
        // (recognize_gaiji_body) declines it, so it never enters the view.
        assert!(gaiji_resolutions("本文［＃改ページ］続き").is_empty());
        assert!(gaiji_resolutions("［＃ここから2字下げ］字下げ").is_empty());
        // A forward-reference bouten directive (balanced quotes, no mencode)
        // is likewise declined.
        assert!(gaiji_resolutions("東京［＃「東京」に傍点］へ").is_empty());
    }

    #[test]
    fn gaiji_resolutions_mixes_refmark_and_standalone_in_order() {
        // Both forms in one document, source order preserved.
        let src = "※［＃「々」］と［＃「木＋吶のつくり」、第3水準1-85-54］";
        let res = gaiji_resolutions(src);
        assert_eq!(res.len(), 2);
        assert_eq!(&src[res[0].start..res[0].end], "※［＃「々」］");
        assert_eq!(
            &src[res[1].start..res[1].end],
            "［＃「木＋吶のつくり」、第3水準1-85-54］"
        );
    }

    #[test]
    fn recognize_gaiji_body_gates_directives() {
        assert!(recognize_gaiji_body("改ページ").is_none());
        assert!(recognize_gaiji_body("ここから2字下げ").is_none());
        assert!(recognize_gaiji_body("「々」").is_some()); // quoted form, no mencode
        assert!(recognize_gaiji_body("「desc」、第3水準1-85-54").is_some());
        // #326: a bare `ローマ数字N` resolves, so its `※［＃…］` is gaiji,
        // not a plain directive.
        assert!(recognize_gaiji_body("ローマ数字17").is_some());
        assert!(recognize_gaiji_body("ローマ数字23").is_some());
        // A non-numeral `ローマ数字` body and out-of-range N stay directives.
        assert!(recognize_gaiji_body("ローマ数字").is_none());
        assert!(recognize_gaiji_body("ローマ数字0").is_none());
    }

    #[test]
    fn roman_numeral_glyph_composes_from_the_u2160_block() {
        // Standard roman spelling with single-letter atoms: 17 = X V I I.
        assert_eq!(
            roman_numeral_glyph("ローマ数字17"),
            Some("\u{2169}\u{2164}\u{2160}\u{2160}")
        );
        assert_eq!(
            roman_numeral_glyph("ローマ数字13"),
            Some("\u{2169}\u{2160}\u{2160}\u{2160}")
        );
        assert_eq!(
            roman_numeral_glyph("ローマ数字20"),
            Some("\u{2169}\u{2169}")
        );
        // Lowercase form (spec-complete; 0 corpus occurrences) uses U+2170.
        assert_eq!(
            roman_numeral_glyph("ローマ数字4小文字"),
            Some("\u{2170}\u{2174}")
        );
        // Full-width digits parse too.
        assert_eq!(
            roman_numeral_glyph("ローマ数字１７"),
            Some("\u{2169}\u{2164}\u{2160}\u{2160}")
        );
        // Not a roman numeral description.
        assert_eq!(roman_numeral_glyph("二重かっこ開く"), None);
        assert_eq!(roman_numeral_glyph("ローマ数字"), None);
        assert_eq!(roman_numeral_glyph("ローマ数字0"), None);
    }

    #[test]
    fn find_span_locates_enclosing_reference() {
        let src = "あ※［＃「々」］い";
        let open = src.find("※").unwrap();
        // A cursor inside the span finds the whole `※［＃…］` range.
        let span = find_span(src, open + GAIJI_OPEN.len()).unwrap();
        assert_eq!(&src[span.0..span.1], "※［＃「々」］");
        // A cursor outside any reference finds nothing.
        assert!(find_span(src, 0).is_none());
    }

    #[test]
    fn lookup_prefers_existing_ucs_when_already_set() {
        // The "existing" short-circuit returns the caller-provided
        // codepoint without consulting either table.
        assert_eq!(
            lookup(Some('\u{1234}'), Some("第3水準1-85-54"), "木＋吶のつくり"),
            Some(Resolved::Char('\u{1234}'))
        );
    }

    #[test]
    fn lookup_via_mencode_table_when_ucs_missing() {
        // 罪と罰 fixture: `木＋吶のつくり` with 第3水準1-85-54.
        // Per JIS X 0213:2004 plane 1, row 85, cell 54 = 枘 (U+6798).
        // ("吶のつくり" = right-side component of 吶 = 内, so 木+内 = 枘.)
        assert_eq!(
            lookup(None, Some("第3水準1-85-54"), "木＋吶のつくり"),
            Some(Resolved::Char('\u{6798}'))
        );
    }

    #[test]
    fn lookup_via_combo_table_returns_multi() {
        // 第3水準1-4-87 = か゚ = U+304B U+309A (combining handakuten).
        // The combo path is the *only* way to honour these 25 cells.
        assert_eq!(
            lookup(None, Some("第3水準1-4-87"), ""),
            Some(Resolved::Multi("\u{304B}\u{309A}"))
        );
    }

    #[test]
    fn combo_resolution_writes_both_codepoints() {
        // End-to-end: combo lookup + write_to should yield the full
        // 2-codepoint sequence (6 UTF-8 bytes for か + handakuten).
        let resolved = lookup(None, Some("第3水準1-4-87"), "").expect("combo resolves");
        let mut s = String::new();
        resolved
            .write_to(&mut s)
            .expect("write to String never fails");
        assert_eq!(s, "\u{304B}\u{309A}");
        assert_eq!(s.chars().count(), 2);
    }

    #[test]
    fn lookup_via_u_plus_form() {
        assert_eq!(
            lookup(None, Some("U+01F5"), "Latin Small Letter G With Acute"),
            Some(Resolved::Char('\u{01F5}'))
        );
    }

    #[test]
    fn lookup_via_u_plus_max_six_hex_digits() {
        // U+10FFFF is the Unicode max; any shape past 6 digits is rejected.
        assert_eq!(
            lookup(None, Some("U+10FFFF"), ""),
            Some(Resolved::Char('\u{10FFFF}'))
        );
    }

    #[test]
    fn lookup_rejects_u_plus_beyond_seven_hex_digits() {
        assert_eq!(lookup(None, Some("U+1234567"), ""), None);
    }

    #[test]
    fn lookup_rejects_u_plus_surrogate() {
        assert_eq!(lookup(None, Some("U+D800"), ""), None);
    }

    #[test]
    fn lookup_rejects_u_plus_non_hex() {
        assert_eq!(lookup(None, Some("U+GG12"), ""), None);
    }

    #[test]
    fn lookup_rejects_u_plus_without_digits() {
        assert_eq!(lookup(None, Some("U+"), ""), None);
    }

    #[test]
    fn lookup_via_description_fallback_when_mencode_absent() {
        assert_eq!(lookup(None, None, "〓"), Some(Resolved::Char('\u{3013}')));
    }

    #[test]
    fn lookup_returns_none_when_all_paths_miss() {
        // Multi-char description AND missing mencode → no resolution.
        assert_eq!(
            lookup(None, Some("not-in-any-table"), "unresolved gaiji"),
            None
        );
    }

    #[test]
    fn lookup_falls_back_to_description_self_when_single_char() {
        // 丂 is in the JIS X 0213 plane 2 table at row 1 cell 2 — but
        // a real-world author wrote `※[#「丂」、第4水準2-16-1]` with a
        // mencode that doesn't exist in the table. The description IS
        // the kanji itself, so the smart fallback resolves to it.
        assert_eq!(
            lookup(None, Some("第4水準2-16-1"), "丂"),
            Some(Resolved::Char('\u{4E02}'))
        );
        // Same for descriptions with no mencode at all.
        assert_eq!(lookup(None, None, "畺"), Some(Resolved::Char('\u{757A}')));
        assert_eq!(lookup(None, None, "龔"), Some(Resolved::Char('\u{9F94}')));
    }

    #[test]
    fn single_char_fallback_does_not_override_dictionary_hit() {
        // `〓` is in the special-placeholder table mapping to
        // `〓 U+3013`. (Yes, that's a no-op mapping, but it exercises
        // the dictionary path winning over the single-char fallback.)
        // If the fallback fired in spite of the table hit, the
        // dictionary's value would still match here — so the contract
        // is "fallback only fires when nothing else matched".
        assert_eq!(lookup(None, None, "〓"), Some(Resolved::Char('\u{3013}')));
    }

    #[test]
    fn single_char_fallback_does_not_fire_for_multi_char_descriptions() {
        // Multi-char description not in any table → must still be None.
        // Confirms the early-return on `chars.next().is_none()`.
        assert_eq!(lookup(None, None, "未知の字形"), None);
        assert_eq!(lookup(None, None, "ab"), None);
    }

    #[test]
    fn mencode_table_covers_the_fixture_gaiji() {
        // Pin the corrected 罪と罰 fixture mapping (枘 U+6798, not the
        // pre-regen hand-seed's wrong U+6903 椃).
        assert_eq!(
            JISX0213_MENCODE_TO_CHAR.get("第3水準1-85-54"),
            Some(&'\u{6798}')
        );
    }

    #[test]
    fn table_sizes_match_jisx0213_2004_spec() {
        // Pinned against the JIS X 0213:2004 normative count + the
        // 外字注記辞書 8th edition (8 881 entries) + 2 hand-curated
        // specials (〓 / 〻). Both data sources are checked into
        // `crates/aozora-encoding/data/`.
        use crate::jisx0213_table::{
            DESCRIPTION_COUNT, JISX0213_COMBO_COUNT, JISX0213_PLANE1_COUNT, JISX0213_PLANE2_COUNT,
        };
        let (single, combo, description) = table_sizes();
        assert_eq!(single, JISX0213_PLANE1_COUNT + JISX0213_PLANE2_COUNT);
        assert_eq!(combo, JISX0213_COMBO_COUNT);
        assert_eq!(description, DESCRIPTION_COUNT);
        assert_eq!(
            JISX0213_PLANE1_COUNT, 1893,
            "第3水準 must equal the spec count",
        );
        assert_eq!(
            JISX0213_PLANE2_COUNT, 2436,
            "第4水準 must equal the spec count",
        );
        assert_eq!(
            JISX0213_COMBO_COUNT, 25,
            "combining-sequence cells must equal spec",
        );
        assert!(
            description >= 8_000,
            "description-fallback table looks too small ({description}) — \
             did the gaiji-chuki extraction drop entries?",
        );
    }

    #[test]
    fn description_table_resolves_a_known_dictionary_entry() {
        // 「木＋吶のつくり」 is a hallmark fixture description for 枘
        // (U+6798, JIS X 0213 plane 1 row 85 cell 54). The dictionary
        // path resolves the same character as the mencode path, so a
        // test with description-only (no mencode) must hit U+6798.
        assert_eq!(
            lookup(None, None, "木＋吶のつくり"),
            Some(Resolved::Char('\u{6798}')),
        );
    }

    #[test]
    fn description_table_preserves_special_placeholders() {
        // 〓 / 〻 are hand-curated specials kept in
        // `aozora-gaiji-special.tsv` and merged into the generated map.
        assert_eq!(lookup(None, None, "〓"), Some(Resolved::Char('\u{3013}')));
        assert_eq!(lookup(None, None, "〻"), Some(Resolved::Char('\u{303B}')));
    }

    #[test]
    fn full_jisx0213_table_covers_a_known_plane1_third_tier_kanji() {
        // 第3水準1-85-9 = 敧 (U+6567) per JIS X 0213:2004.
        assert_eq!(
            JISX0213_MENCODE_TO_CHAR.get("第3水準1-85-9"),
            Some(&'\u{6567}')
        );
    }

    #[test]
    fn full_jisx0213_table_covers_a_known_plane2_fourth_tier_entry() {
        // 第4水準2-1-1 = 𠂉 (U+20089) — first plane-2 cell.
        assert_eq!(
            JISX0213_MENCODE_TO_CHAR.get("第4水準2-1-1"),
            Some(&'\u{20089}')
        );
    }

    #[test]
    fn resolved_utf8_len_matches_actual_encoding() {
        assert_eq!(Resolved::Char('A').utf8_len(), 1);
        assert_eq!(Resolved::Char('あ').utf8_len(), 3);
        assert_eq!(Resolved::Char('𠂉').utf8_len(), 4);
        assert_eq!(Resolved::Multi("\u{304B}\u{309A}").utf8_len(), 6);
    }

    #[test]
    fn resolved_as_char_returns_none_for_combos() {
        assert_eq!(Resolved::Char('A').as_char(), Some('A'));
        assert_eq!(Resolved::Multi("か゚").as_char(), None);
    }

    #[test]
    fn lookup_is_identity_on_the_ucs_input_when_set() {
        // The "existing" short-circuit honours the caller-provided
        // scalar without a wasted table probe.
        assert_eq!(
            lookup(Some('あ'), Some("anything"), "anything"),
            Some(Resolved::Char('あ'))
        );
    }

    #[test]
    fn menkuten_round_trips_through_display() {
        let m = parse_menkuten("第3水準1-85-54").expect("clean men-ku-ten parses");
        assert_eq!(
            m,
            MenKuTen {
                plane: 1,
                ku: 85,
                ten: 54
            }
        );
        assert_eq!(m.level(), 3);
        assert_eq!(m.to_string(), "第3水準1-85-54");
        let m4 = parse_menkuten("第4水準2-1-1").expect("plane-2 parses");
        assert_eq!(
            m4,
            MenKuTen {
                plane: 2,
                ku: 1,
                ten: 1
            }
        );
        assert_eq!(m4.to_string(), "第4水準2-1-1");
    }

    #[test]
    fn parse_menkuten_rejects_non_canonical_forms() {
        // Bare (no 第N水準 prefix) — kept verbatim, not structured.
        assert!(parse_menkuten("1-2-23").is_none());
        // Page-line suffix — not a clean men-ku-ten.
        assert!(parse_menkuten("第3水準1-84-27、144-上-9").is_none());
        // Level inconsistent with the plane (第3水準 ⇒ plane 1, not 2).
        assert!(parse_menkuten("第3水準2-1-1").is_none());
        // Leading zero would not byte-reproduce.
        assert!(parse_menkuten("第3水準1-05-4").is_none());
        // U+ form is not a men-ku-ten.
        assert!(parse_menkuten("U+74FC").is_none());
    }

    #[test]
    fn gaiji_canonical_classifies_only_clean_forms() {
        assert_eq!(
            GaijiCanonical::from_mencode(Some("第3水準1-85-54")),
            GaijiCanonical::MenKuTen(MenKuTen {
                plane: 1,
                ku: 85,
                ten: 54
            })
        );
        assert_eq!(
            GaijiCanonical::from_mencode(Some("U+74FC")),
            GaijiCanonical::Unicode('\u{74FC}')
        );
        // Suffixed / bare / absent stay verbatim for byte-exact serialize.
        assert_eq!(
            GaijiCanonical::from_mencode(Some("U+74FC、372-10")),
            GaijiCanonical::Unresolved {
                mencode: Some("U+74FC、372-10")
            }
        );
        assert_eq!(
            GaijiCanonical::from_mencode(Some("1-2-23")),
            GaijiCanonical::Unresolved {
                mencode: Some("1-2-23")
            }
        );
        assert_eq!(
            GaijiCanonical::from_mencode(None),
            GaijiCanonical::Unresolved { mencode: None }
        );
    }

    #[test]
    fn gaiji_canonical_resolve_matches_legacy_lookup() {
        // Men-ku-ten resolves through the table exactly as the raw-string
        // lookup did (枘 U+6798).
        assert_eq!(
            GaijiCanonical::from_mencode(Some("第3水準1-85-54")).resolve("木＋吶のつくり"),
            Some(Resolved::Char('\u{6798}'))
        );
        assert_eq!(
            GaijiCanonical::from_mencode(Some("U+74FC")).resolve(""),
            Some(Resolved::Char('\u{74FC}'))
        );
        // Suffixed form resolves via the resolution-token (suffix stripped).
        assert_eq!(
            GaijiCanonical::from_mencode(Some("U+74FC、372-10")).resolve(""),
            Some(Resolved::Char('\u{74FC}'))
        );
    }

    #[test]
    fn gaiji_canonical_write_mencode_reproduces_source() {
        let render = |c: GaijiCanonical<'_>| {
            let mut s = String::new();
            c.write_mencode(&mut s).unwrap();
            s
        };
        assert_eq!(
            render(GaijiCanonical::from_mencode(Some("第3水準1-85-54"))),
            "第3水準1-85-54"
        );
        assert_eq!(
            render(GaijiCanonical::from_mencode(Some("U+74FC"))),
            "U+74FC"
        );
        assert_eq!(
            render(GaijiCanonical::from_mencode(Some(
                "第3水準1-84-27、144-上-9"
            ))),
            "第3水準1-84-27、144-上-9"
        );
        GaijiCanonical::from_mencode(None)
            .write_mencode(&mut String::new())
            .unwrap();
        assert!(!GaijiCanonical::from_mencode(None).has_mencode());
    }

    #[test]
    fn parse_u_plus_length_guard_is_all_or_nothing() {
        // 1..=6 hex digits after `U+` parse to the scalar.
        assert_eq!(parse_u_plus("U+41"), Some('A'));
        assert_eq!(parse_u_plus("U+10FFFF"), Some('\u{10FFFF}'));
        // Empty hex is rejected.
        assert_eq!(parse_u_plus("U+"), None);
        // Seven hex digits are rejected even when the low bits name a valid
        // scalar (`U+00010FF` would parse to U+10FF once the length guard is
        // dropped). Pins `hex.is_empty() || hex.len() > 6` as an OR — an AND
        // there would stop rejecting the over-long form.
        assert_eq!(parse_u_plus("U+00010FF"), None);
        // Not a `U+` token at all.
        assert_eq!(parse_u_plus("第3水準1-85-54"), None);
    }

    #[test]
    fn parse_gaiji_body_run_consuming_the_whole_body_keeps_it_as_description() {
        // Every 、-token is mencode-shaped (here a single bare men-ku-ten), so
        // the right-to-left run empties `tokens`: `run_start` reaches 0 and the
        // whole body stays the description with no split. Pins the
        // `run_start > 0` loop guard — a `>=` there indexes `tokens[-1]` (a
        // usize underflow) once the run consumes everything.
        assert_eq!(
            parse_gaiji_body("1-2-3"),
            GaijiBody {
                description: "1-2-3",
                mencode: None,
                quoted: false,
            }
        );
    }

    #[test]
    fn gaiji_description_serializable_rejects_leaky_and_unbalanced() {
        // A plain description (no nested opener, no quotes) always serializes.
        assert!(gaiji_description_serializable("木＋吶のつくり", false));
        assert!(gaiji_description_serializable("木＋吶のつくり", true));
        // A nested `［＃` opener would leak a bare bracket-hash: rejected
        // regardless of the mencode anchor (kills the `-> true` stub).
        assert!(!gaiji_description_serializable("外字［＃注記", false));
        assert!(!gaiji_description_serializable("外字［＃注記", true));
        // Balanced quotes are serializable ONLY with a mencode anchor;
        // `balanced && has_mencode` is all-or-nothing.
        // balanced but unanchored → false (kills `&&`→`||`).
        assert!(!gaiji_description_serializable("「廰」の「广」", false));
        // balanced and anchored → true.
        assert!(gaiji_description_serializable("「廰」の「广」", true));
        // Unbalanced quotes stay rejected even WITH a mencode anchor
        // (kills `&&`→`||` on the other operand).
        assert!(!gaiji_description_serializable("「廰", true));
    }

    #[test]
    fn parse_menkuten_rejects_zero_coordinates() {
        // A zero in any of the three men-ku-ten coordinates is outside the
        // 1..=94 JIS range, so the clean-form parse must reject it. Each token
        // round-trips through `Display`, so ONLY the
        // `plane == 0 || ku == 0 || ten == 0` guard can reject it — this pins
        // those three OR nodes (an `&&` fuses two coordinates and lets a single
        // zero slip through).
        assert!(parse_menkuten("第2水準0-1-1").is_none(), "plane 0 rejected");
        assert!(parse_menkuten("第3水準1-0-1").is_none(), "ku 0 rejected");
        assert!(parse_menkuten("第3水準1-1-0").is_none(), "ten 0 rejected");
        // The all-nonzero canonical form still parses (guards do not
        // over-reject).
        assert_eq!(
            parse_menkuten("第3水準1-1-1"),
            Some(MenKuTen {
                plane: 1,
                ku: 1,
                ten: 1
            })
        );
    }

    #[test]
    fn gaiji_canonical_has_mencode_tracks_the_tail() {
        // A structured tail (men-ku-ten / U+) means a mencode is present.
        assert!(GaijiCanonical::from_mencode(Some("第3水準1-85-54")).has_mencode());
        assert!(GaijiCanonical::from_mencode(Some("U+74FC")).has_mencode());
        // A verbatim (unresolved) tail still counts as a present mencode.
        assert!(GaijiCanonical::from_mencode(Some("1-2-23")).has_mencode());
        // Only a truly absent tail is `false` (kills the `-> false` stub,
        // which the None case alone cannot).
        assert!(!GaijiCanonical::from_mencode(None).has_mencode());
    }

    #[test]
    fn is_mencode_shaped_u_plus_branch_boundaries() {
        // A well-formed U+ token with 1..=6 hex digits is mencode-shaped.
        assert!(is_mencode_shaped("U+41"));
        assert!(is_mencode_shaped("U+10FFFF"));
        // Empty hex is NOT shaped: pins `!hex.is_empty()` (kills `delete !`
        // and the first `&&`→`||`, both of which would admit the empty tail).
        assert!(!is_mencode_shaped("U+"));
        // 7+ hex digits exceed codepoint width: pins `hex.len() <= 6` on its
        // false side (kills `<=`→`>`).
        assert!(!is_mencode_shaped("U+1234567"));
        // Non-hex digits after `U+` are NOT shaped: pins the trailing
        // `all(is_ascii_hexdigit)` node (kills the second `&&`→`||`).
        assert!(!is_mencode_shaped("U+GG12"));
    }

    #[test]
    fn resolve_at_rejects_empty_body_span() {
        // `※［＃］` with nothing between the opener and `］`: body_end ==
        // body_start, so the `body_end <= body_start` guard fires and the span
        // is not a gaiji. Pins that guard as the reachable half of the `||` —
        // an `&&` there would admit the empty body as a hollow
        // (empty-description) resolution.
        let src = "※［＃］";
        assert!(resolve_at(src, 0, src.len()).is_none());
    }

    #[test]
    fn find_span_offset_arithmetic_adds_past_the_opener() {
        // A `※［＃…］` starting at byte 0 (no leading text): the `［＃` opener
        // sits at window byte 3, so `hash_in_win + ［＃.len()` must ADD to reach
        // the body. A subtraction underflows (or, without overflow checks,
        // indexes out of range and finds no `］`), losing the enclosing span.
        let src = "※［＃「々」］";
        let span = find_span(src, GAIJI_OPEN.len()).expect("cursor inside the span");
        assert_eq!(&src[span.0..span.1], "※［＃「々」］");
    }

    #[test]
    fn snap_left_finds_the_char_boundary_at_or_below() {
        // "あい" = two 3-byte chars; char boundaries at 0, 3, 6.
        let s = "あい";
        // A valid boundary is returned unchanged (an off-by-one in the guard
        // or a `delete !` would walk off it).
        assert_eq!(snap_to_char_boundary_left(s, 3), 3);
        assert_eq!(snap_to_char_boundary_left(s, 6), 6);
        // Index 0 is always a boundary.
        assert_eq!(snap_to_char_boundary_left(s, 0), 0);
        // A mid-char index snaps DOWN to the start of its char. Pins the `-=`
        // decrement (a `+=`/`/=` moves the wrong way or loops forever), the
        // `> 0` guard (`==`/`<` would never enter the loop), the `&&` (an `||`
        // would run past the boundary to 0), and the fn-body stub.
        assert_eq!(snap_to_char_boundary_left(s, 4), 3);
        assert_eq!(snap_to_char_boundary_left(s, 5), 3);
        assert_eq!(snap_to_char_boundary_left(s, 1), 0);
        assert_eq!(snap_to_char_boundary_left(s, 2), 0);
    }

    #[test]
    fn snap_right_finds_the_char_boundary_at_or_above() {
        // "あい" = two 3-byte chars; char boundaries at 0, 3, 6.
        let s = "あい";
        // A valid boundary is returned unchanged. Pins the `&&` (an `||` would
        // run to `len`) and the `delete !`.
        assert_eq!(snap_to_char_boundary_right(s, 3), 3);
        assert_eq!(snap_to_char_boundary_right(s, 0), 0);
        assert_eq!(snap_to_char_boundary_right(s, 6), 6);
        // A mid-char index snaps UP to the next boundary. Pins the `+=`
        // increment (a `-=`/`*=` moves the wrong way or loops forever) and the
        // `< len` guard (`==`/`>` would never enter the loop).
        assert_eq!(snap_to_char_boundary_right(s, 4), 6);
        assert_eq!(snap_to_char_boundary_right(s, 5), 6);
        assert_eq!(snap_to_char_boundary_right(s, 1), 3);
        assert_eq!(snap_to_char_boundary_right(s, 2), 3);
    }
}
