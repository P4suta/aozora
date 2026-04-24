//! Property test — "run it twice, expect the same" invariants.
//!
//! Three structural properties that share a "fixed point / idempotent"
//! shape and benefit from living in one test binary:
//!
//! * **Determinism** — `render_to_string(src)` produces identical
//!   HTML across two independent parse runs (fresh arenas). Any
//!   divergence indicates ordering non-determinism in the lexer /
//!   post-process stack.
//!
//! * **Annotation-wrapper idempotence (Tier E)** —
//!   `strip_annotation_wrappers` is idempotent: running it twice on
//!   rendered HTML yields the same string both times. Already
//!   asserted on fixtures in `golden_56656.rs`; the proptest
//!   generalises to random input so a nested / malformed wrapper
//!   would surface under shrinking.
//!
//! * **Serialize round-trip fixed point (I3)** — `serialize ∘ parse`
//!   converges in one iteration: starting from an arbitrary `src`,
//!   `serialize(parse(src))` may not equal `src` (it canonicalises),
//!   but `serialize(parse(serialize(parse(src))))` must equal
//!   `serialize(parse(src))`. Catches classifier / serializer drift
//!   where the round-trip oscillates. Already hard-gated on the 17 k
//!   corpus (I3); the proptest gives the invariant a random-input
//!   sibling so regressions show up locally before a corpus sweep.

use afm_parser::html::render_to_string;
use afm_parser::test_support::strip_annotation_wrappers;
use afm_parser::{Options, parse, serialize};
use afm_test_utils::config::default_config;
use afm_test_utils::generators::{aozora_fragment, pathological_aozora};
use comrak::Arena;
use proptest::prelude::*;

proptest! {
    #![proptest_config(default_config())]

    /// Determinism — identical input produces identical output.
    /// Runs with aozora-shaped input because the trigger glyphs drive
    /// the widest non-determinism surface (ordering of recogniser
    /// emissions, registry allocation).
    #[test]
    fn render_is_deterministic_across_independent_arenas(src in aozora_fragment(16)) {
        let a = render_to_string(&src);
        let b = render_to_string(&src);
        prop_assert_eq!(a, b, "render_to_string must be deterministic");
    }

    /// Determinism on pathological input too — malformed brackets
    /// must not nondeterministically trigger different fallback paths.
    #[test]
    fn render_is_deterministic_for_pathological_input(src in pathological_aozora(6)) {
        let a = render_to_string(&src);
        let b = render_to_string(&src);
        prop_assert_eq!(a, b, "render_to_string must be deterministic on pathological input");
    }

    /// Tier E — `strip_annotation_wrappers` is idempotent. Proptest
    /// version of the existing fixture assertion in `golden_56656.rs`.
    #[test]
    fn strip_annotation_wrappers_is_idempotent(src in aozora_fragment(16)) {
        let html = render_to_string(&src);
        let once = strip_annotation_wrappers(&html);
        let twice = strip_annotation_wrappers(&once);
        prop_assert_eq!(
            &once, &twice,
            "strip_annotation_wrappers must be idempotent for src={:?}",
            src
        );
    }

    /// I3 — `serialize ∘ parse` is a fixed point after one iteration.
    ///
    /// First canonicalisation (`serialize(parse(src))`) may change
    /// `src` (whitespace normalisation, trailing newlines, etc.); a
    /// second canonicalisation must be byte-identical to the first.
    /// Any oscillation implies the classifier and serializer disagree
    /// on the canonical form.
    #[test]
    fn serialize_parse_is_fixed_point(src in aozora_fragment(16)) {
        let opts = Options::afm_default();
        let arena_a = Arena::new();
        let first = serialize(&parse(&arena_a, &src, &opts));
        let arena_b = Arena::new();
        let second = serialize(&parse(&arena_b, &first, &opts));
        prop_assert_eq!(
            &first, &second,
            "I3 fixed-point broken for src={:?}: first vs second differ",
            src
        );
    }
}
