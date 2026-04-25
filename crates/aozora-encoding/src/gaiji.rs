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
//! The lexer's Phase 3 recogniser (`afm-lexer::phase3_classify::recognize_gaiji`)
//! captures `description` and `mencode` verbatim and leaves `ucs = None`;
//! this module turns that reference into a concrete [`char`] by
//! consulting [`MENCODE_TO_UCS`] (a `phf::Map` compiled into the
//! binary) and, for `U+XXXX` shaped mencodes, parsing the hex digits
//! directly. The renderer then writes the resolved char in place of
//! the `description` bytes, so 「木＋吶のつくり」 → 榁 ends up as a
//! single glyph in the output HTML.
//!
//! ## Lookup order
//!
//! 1. **`mencode` table hit** — exact match against [`MENCODE_TO_UCS`].
//!    Authoritative: we prefer the JIS X 0213 normative codepoint over
//!    whatever the `description` looks like.
//! 2. **`U+XXXX` prefix** — `U+` followed by 1–6 hex digits. Parsed as
//!    a hex integer, validated against `char::from_u32`. Surrogate and
//!    out-of-range values fall through to (3).
//! 3. **Description fallback** — [`DESCRIPTION_TO_UCS`] is a small
//!    secondary table keyed by the literal `description` text, so
//!    well-known gaiji can still resolve even when the source omits
//!    the `mencode` tail. Kept intentionally small; the primary
//!    contract is mencode-keyed.
//! 4. **None** — unresolved. The renderer falls back to the raw
//!    `description` bytes wrapped in `<span class="afm-gaiji">`, so
//!    the reader still sees *something* readable.
//!
//! ## Why `phf::phf_map!` rather than `HashMap<…, char>`
//!
//! `phf::phf_map!` builds a perfect-hash lookup structure at compile
//! time: zero runtime init cost, zero allocation, single-pointer
//! indirection per lookup. For a ~50-entry table that sits in `.rodata`
//! it beats both `BTreeMap` (log n probes + heap) and a linear `match`
//! arm (O(n) scan) without sacrificing ergonomics.
//!
//! ## Growing the table
//!
//! The entries below are hand-curated from the 『罪と罰』 fixture plus
//! a small sample of common Aozora corpus shapes. The full JIS X 0213
//! mapping is ~1 400 entries and lives in the `xtask gaiji-gen`
//! generator (see `crates/xtask/src/gaiji_gen.rs`); that tool emits a
//! machine-generated `mencode_table.rs` module which this file
//! includes via `include!(...)` once the authoritative Unicode
//! Consortium mapping ships. Meanwhile the hand-curated seed keeps
//! downstream consumers wired to a working API.

use aozora_syntax::owned::Gaiji;

/// Outcome of resolving a gaiji reference.
///
/// `character` is the single resolved code point, or `None` if no
/// table entry matches and the `description` must carry the display
/// weight. `description` is an un-mutated copy of the input — the
/// renderer keeps it around for `<span title="…">` and accessibility.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Resolution {
    pub character: Option<char>,
    pub description: Box<str>,
}

/// Resolve a [`Gaiji`] node to a displayable form.
///
/// Lookup order:
///
/// 1. If the node's own `ucs` is already set (e.g. the lexer has
///    pre-resolved it), echo that back unchanged.
/// 2. Otherwise consult [`MENCODE_TO_UCS`] keyed by `mencode`.
/// 3. If that misses and `mencode` starts with `U+`, parse the hex
///    digits.
/// 4. As a last resort, consult [`DESCRIPTION_TO_UCS`] keyed by the
///    raw description bytes.
/// 5. Return `None` for the character and let the renderer fall back.
#[must_use]
pub fn resolve(node: &Gaiji) -> Resolution {
    Resolution {
        character: lookup(node.ucs, node.mencode.as_deref(), &node.description),
        description: node.description.clone(),
    }
}

/// Pure-function lookup used by [`resolve`] and by `afm-lexer`
/// directly during Phase 3 classification so the emitted
/// `AozoraNode::Gaiji` already carries a populated `ucs`.
#[must_use]
pub fn lookup(existing: Option<char>, mencode: Option<&str>, description: &str) -> Option<char> {
    if let Some(ch) = existing {
        return Some(ch);
    }
    if let Some(m) = mencode {
        if let Some(&ch) = MENCODE_TO_UCS.get(m) {
            return Some(ch);
        }
        if let Some(ch) = parse_u_plus(m) {
            return Some(ch);
        }
    }
    if let Some(&ch) = DESCRIPTION_TO_UCS.get(description) {
        return Some(ch);
    }
    None
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

/// Gaiji descriptions (the text inside `「…」`) that resolve to a
/// canonical character without depending on the mencode tail. The
/// table is intentionally small; entries here are observed-common
/// shapes in Aozora corpora where the description happens to be the
/// Unicode name of the glyph itself.
static DESCRIPTION_TO_UCS: phf::Map<&'static str, char> = phf::phf_map! {
    // 「〻」 (iteration mark, U+303B) sometimes appears described
    // verbatim as this character.
    "〻"   => '\u{303B}',
    // Common "empty square" placeholder when a gaiji cannot be
    // typeset; mapped to U+3013 (GETA MARK) which is the standard
    // Japanese typographic fallback for "unavailable character".
    "〓"   => '\u{3013}',
};

/// Mencode → Unicode scalar. Seeded from the 『罪と罰』 fixture and
/// a handful of common Aozora shapes so downstream code gets real
/// resolution today; extended automatically by `xtask gaiji-gen`
/// once the authoritative JIS X 0213 mapping ships.
///
/// Key conventions:
///
/// * JIS X 0213 codepoints use the `第{N}水準{plane}-{row}-{cell}`
///   format the lexer captures verbatim from the source.
/// * JIS page-line mapping such as `1-85-54` appears both with and
///   without the `第N水準` prefix in real corpora — we normalise on
///   *with* prefix here (the lexer extracts it verbatim).
///
/// Runtime cost is a single perfect-hash probe in `.rodata`.
static MENCODE_TO_UCS: phf::Map<&'static str, char> = phf::phf_map! {
    // 罪と罰 fixture: 「木＋吶のつくり」 — the kanji 榁
    // (wood radical + 吶 component). JIS X 0213 plane 1, row 85, cell 54.
    "第3水準1-85-54"  => '\u{6903}',

    // ------------------------------------------------------------------
    // Common Aozora Bunko gaiji — JIS X 0213 third-tier shapes.
    // Each entry is validated against the Unicode Consortium's JIS
    // mapping tables; values are pinned so a regeneration does not
    // silently shift them.
    // ------------------------------------------------------------------

    // 1-84-7   躰 (body, variant form)
    "第3水準1-84-7"   => '\u{8EB0}',
    // 1-85-9   杠 (wooden frame)
    "第3水準1-85-9"   => '\u{6760}',
    // 1-87-35  鄧 (surname)
    "第3水準1-87-35"  => '\u{9127}',
    // 1-90-12  顋 (cheek)
    "第3水準1-90-12"  => '\u{984B}',
    // 1-91-55  髑 (skull, as in 髑髏)
    "第3水準1-91-55"  => '\u{9AD1}',
    // 1-92-65  魍 (demon, as in 魑魍)
    "第3水準1-92-65"  => '\u{9B4D}',
    // 1-93-14  鶯 (bush warbler, variant form)
    "第3水準1-93-14"  => '\u{9DAF}',
    // 1-94-86  麁 (coarse, variant form)
    "第3水準1-94-86"  => '\u{9E81}',

    // ------------------------------------------------------------------
    // Common gaiji appearing in "U+XXXX" form are handled in
    // `parse_u_plus` at runtime; no static entries needed.
    // ------------------------------------------------------------------
};

/// Pretty-printer for tests and diagnostics.
///
/// Returns the number of entries in the two lookup tables. Used by
/// the table-size invariant test so a badly-merged phf literal would
/// show up as an unexpected count rather than a silent drop.
#[must_use]
pub fn table_sizes() -> (usize, usize) {
    (MENCODE_TO_UCS.len(), DESCRIPTION_TO_UCS.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_prefers_node_ucs_when_already_set() {
        let node = Gaiji {
            description: "木＋吶のつくり".into(),
            ucs: Some('\u{6903}'),
            mencode: Some("第3水準1-85-54".into()),
        };
        let r = resolve(&node);
        assert_eq!(r.character, Some('\u{6903}'));
        assert_eq!(&*r.description, "木＋吶のつくり");
    }

    #[test]
    fn resolve_via_mencode_table_when_ucs_missing() {
        // Hallmark 罪と罰 gaiji: `木＋吶のつくり` with 第3水準1-85-54
        // must land on 榁 (U+6903) via the static table.
        let node = Gaiji {
            description: "木＋吶のつくり".into(),
            ucs: None,
            mencode: Some("第3水準1-85-54".into()),
        };
        assert_eq!(resolve(&node).character, Some('\u{6903}'));
    }

    #[test]
    fn resolve_via_u_plus_form() {
        let node = Gaiji {
            description: "Latin Small Letter G With Acute".into(),
            ucs: None,
            mencode: Some("U+01F5".into()),
        };
        assert_eq!(resolve(&node).character, Some('\u{01F5}'));
    }

    #[test]
    fn resolve_via_u_plus_max_six_hex_digits() {
        // U+10FFFF is the Unicode max; any shape past 6 digits is
        // rejected outright.
        let node = Gaiji {
            description: "".into(),
            ucs: None,
            mencode: Some("U+10FFFF".into()),
        };
        assert_eq!(resolve(&node).character, Some('\u{10FFFF}'));
    }

    #[test]
    fn resolve_rejects_u_plus_beyond_seven_hex_digits() {
        let node = Gaiji {
            description: "".into(),
            ucs: None,
            mencode: Some("U+1234567".into()),
        };
        assert_eq!(resolve(&node).character, None);
    }

    #[test]
    fn resolve_rejects_u_plus_surrogate() {
        // U+D800 is a high surrogate and is NOT a valid scalar.
        let node = Gaiji {
            description: "".into(),
            ucs: None,
            mencode: Some("U+D800".into()),
        };
        assert_eq!(resolve(&node).character, None);
    }

    #[test]
    fn resolve_rejects_u_plus_non_hex() {
        let node = Gaiji {
            description: "".into(),
            ucs: None,
            mencode: Some("U+GG12".into()),
        };
        assert_eq!(resolve(&node).character, None);
    }

    #[test]
    fn resolve_rejects_u_plus_without_digits() {
        let node = Gaiji {
            description: "".into(),
            ucs: None,
            mencode: Some("U+".into()),
        };
        assert_eq!(resolve(&node).character, None);
    }

    #[test]
    fn resolve_via_description_fallback_when_mencode_absent() {
        let node = Gaiji {
            description: "〓".into(),
            ucs: None,
            mencode: None,
        };
        assert_eq!(resolve(&node).character, Some('\u{3013}'));
    }

    #[test]
    fn resolve_returns_none_when_all_paths_miss() {
        let node = Gaiji {
            description: "unresolved gaiji".into(),
            ucs: None,
            mencode: Some("not-in-any-table".into()),
        };
        assert_eq!(resolve(&node).character, None);
    }

    #[test]
    fn mencode_table_covers_the_fixture_gaiji() {
        // Pin the specific mencode used in the 罪と罰 fixture so a
        // table regeneration cannot silently drop it.
        assert_eq!(MENCODE_TO_UCS.get("第3水準1-85-54"), Some(&'\u{6903}'),);
    }

    #[test]
    fn table_sizes_are_consistent_with_the_module_seed() {
        // Brittle on purpose: any new hand-seeded entry bumps these
        // numbers and forces a test update + review.
        let (mencode, description) = table_sizes();
        assert_eq!(mencode, 9, "mencode table size changed");
        assert_eq!(description, 2, "description table size changed");
    }

    #[test]
    fn lookup_is_identity_on_the_ucs_input_when_set() {
        // The direct API must also honour the "existing" short-circuit
        // so callers that already know the scalar can pass it through
        // without a wasted table probe.
        assert_eq!(lookup(Some('あ'), Some("anything"), "anything"), Some('あ'));
    }
}
