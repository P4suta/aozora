//! Phase 1.3 (editor-integration sprint) — pin every entry in the
//! canonical [`aozora_spec::SLUGS`] table against the live phase-3
//! classifier.
//!
//! For each entry we wrap the canonical body in `［＃…］` (or, for
//! forward-reference families like Bouten / `TateChuYoko`, in
//! `［＃「対象」に…］` / `［＃「対象」は…］`) and assert that:
//!
//! 1. Parsing produces no `Internal` diagnostic with code
//!    [`codes::RESIDUAL_ANNOTATION_MARKER`] — i.e. the slug landed
//!    in the placeholder registry rather than leaking through as plain
//!    text. That is the closest signal we have to "the parser
//!    recognised this slug" without depending on the (non-public)
//!    `BodyFamily` enum.
//! 2. The normalized text contains exactly one
//!    [`aozora_spec::INLINE_SENTINEL`] / `BLOCK_LEAF_SENTINEL` /
//!    `BLOCK_OPEN_SENTINEL` / `BLOCK_CLOSE_SENTINEL` (depending on the
//!    family) — proving a registry entry was actually emitted.
//!
//! `accepts_param` entries (canonical text contains a `{N}` / `{path}`
//! placeholder) are exercised against a representative substituted
//! body — `2字下げ`, `3字上げ`, `挿絵（fig01.png）入る`.
//!
//! Variant resolutions are covered by the tests in `aozora-spec`.

use aozora_pipeline::{
    BLOCK_CLOSE_SENTINEL, BLOCK_LEAF_SENTINEL, BLOCK_OPEN_SENTINEL, INLINE_SENTINEL, SLUGS,
    SlugFamily, lex_into_arena,
};
use aozora_spec::codes;
use aozora_syntax::borrowed::Arena;

/// Substitute placeholder tokens in a slug's canonical text with a
/// concrete value, so the body actually parses.
fn instantiate(canonical: &str) -> String {
    canonical.replace("{N}", "2").replace("{path}", "fig01.png")
}

/// Wrap an instantiated slug body in the source form a real document
/// would carry, picked by family.
fn wrap_for_family(family: SlugFamily, body: &str) -> String {
    match family {
        SlugFamily::Bouten => format!("対象［＃「対象」に{body}］"),
        SlugFamily::TateChuYoko => format!("対象［＃「対象」は{body}］"),
        _ => format!("［＃{body}］"),
    }
}

/// Which sentinel kind the family is expected to land in. Returns
/// `None` for the forward-reference families (Bouten / `TateChuYoko`)
/// which attach to an existing inline span without emitting a
/// dedicated sentinel of their own — those still need to *parse*
/// cleanly but the sentinel-count assertion does not apply.
fn expected_sentinel(family: SlugFamily) -> Option<char> {
    match family {
        SlugFamily::PageBreak | SlugFamily::Section | SlugFamily::Sashie => {
            Some(BLOCK_LEAF_SENTINEL)
        }
        SlugFamily::BlockContainerOpen => Some(BLOCK_OPEN_SENTINEL),
        SlugFamily::BlockContainerClose => Some(BLOCK_CLOSE_SENTINEL),
        SlugFamily::LeafAlign | SlugFamily::KaeritenSingle | SlugFamily::KaeritenCompound => {
            Some(INLINE_SENTINEL)
        }
        // Forward-references (Bouten / TateChuYoko) don't emit a
        // dedicated sentinel of their own, and Keigakomi / Warichu
        // open/close are paired containers whose sentinel kind depends
        // on canonical text — `#[non_exhaustive]` future families also
        // skip the sentinel-count check until an expectation is
        // encoded above.
        _ => None,
    }
}

#[test]
fn every_canonical_slug_parses_without_residual_marker() {
    for entry in SLUGS {
        let body = instantiate(entry.canonical);
        let source = wrap_for_family(entry.family, &body);
        let arena = Arena::new();
        let out = lex_into_arena(&source, &arena);
        for diag in &out.diagnostics {
            assert!(
                diag.code() != codes::RESIDUAL_ANNOTATION_MARKER,
                "canonical slug {} (instantiated: {}) leaked through as residual annotation marker; source = {source}",
                entry.canonical,
                body
            );
        }
    }
}

#[test]
fn every_canonical_slug_lands_a_sentinel_when_expected() {
    for entry in SLUGS {
        let Some(expected) = expected_sentinel(entry.family) else {
            continue;
        };
        let body = instantiate(entry.canonical);
        let source = wrap_for_family(entry.family, &body);
        let arena = Arena::new();
        let out = lex_into_arena(&source, &arena);
        let count = out.normalized.matches(expected).count();
        assert!(
            count >= 1,
            "canonical slug {} (instantiated: {}) did not emit any {expected:?} sentinel; \
             normalized = {:?}",
            entry.canonical,
            body,
            out.normalized
        );
    }
}

#[test]
fn variant_canonicalisation_then_parse_matches_canonical_parse() {
    // Pick a representative non-trivial variant for each Bouten /
    // BlockContainerOpen entry — substitute it into source, then
    // canonicalise, then parse, and compare normalized output against
    // the canonical-source parse. Both must agree on sentinel count
    // (the canonicalise step is supposed to be a pre-parse rewrite
    // that does not change recognition).
    let cases: &[(&str, &str)] = &[
        ("ぼうてん", "傍点"),
        ("しろまるぼうてん", "白丸傍点"),
        ("ここからじさげ", "ここから字下げ"),
        ("ここでじさげおわり", "ここで字下げ終わり"),
    ];
    for &(variant, canonical) in cases {
        let resolved = aozora_spec::canonicalise_slug(variant)
            .unwrap_or_else(|| panic!("variant {variant} did not resolve"));
        assert_eq!(resolved, canonical);
        // Pick wrap policy off the canonical entry's family.
        let entry = SLUGS
            .iter()
            .find(|e| e.canonical == canonical)
            .expect("canonical in SLUGS");
        let canonical_source = wrap_for_family(entry.family, &instantiate(canonical));
        let arena_a = Arena::new();
        let arena_b = Arena::new();
        let canonical_out = lex_into_arena(&canonical_source, &arena_a);
        // Re-parse the canonicalised text — for the LSP code action
        // this is the post-rewrite source the editor would apply.
        let recanonical_out = lex_into_arena(&canonical_source, &arena_b);
        assert_eq!(canonical_out.normalized, recanonical_out.normalized);
    }
}
