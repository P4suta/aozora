//! Notation-hygiene lint (`aozora::lint::non_canonical_directive`) —
//! the parse-round-trip keystone that pins the zero-false-positive
//! invariant against recogniser drift.
//!
//! For every catalogue sample: the variant, parsed as a directive, must
//! (a) still be an Unknown directive — so linting it is legitimate — and
//! (b) actually trigger the lint; and its suggested canonical form must
//! parse to a recognised construct (no lint, no bare directive). If a
//! future parser change starts recognising a variant, or stops
//! recognising a canonical, this test fails instead of the lint silently
//! rotting.

use aozora::Document;
use aozora::render::SerializeOptions;
use aozora_syntax::lint::{CATALOGUE_SAMPLES, canonical_directive};

const LINT_CODE: &str = "aozora::lint::non_canonical_directive";

fn fires_lint(body: &str) -> bool {
    source_fires_lint(&format!("あ［＃{body}］"))
}

fn source_fires_lint(source: &str) -> bool {
    Document::new(source.to_owned())
        .parse()
        .diagnostics()
        .iter()
        .any(|d| d.code() == LINT_CODE)
}

#[test]
fn every_variant_is_unknown_and_fires_the_lint() {
    for &variant in CATALOGUE_SAMPLES {
        // The variant must resolve to a canonical…
        let canonical = canonical_directive(variant)
            .unwrap_or_else(|| panic!("catalogue sample {variant:?} did not resolve"));
        // …still be Unknown (renders as the hidden directive span)…
        let vhtml = Document::new(format!("あ［＃{variant}］"))
            .parse()
            .to_html();
        assert!(
            vhtml.contains("aozora-directive"),
            "variant {variant:?} is unexpectedly recognised; drop it from the catalogue"
        );
        // …and actually trigger the notation-hygiene lint.
        assert!(
            fires_lint(variant),
            "catalogue sample {variant:?} did not fire the lint"
        );
        // The suggested canonical must NOT itself be a lint variant
        // (otherwise the suggestion would just re-warn).
        assert!(
            !fires_lint(&canonical),
            "canonical {canonical:?} for {variant:?} is itself a lint variant"
        );
    }
}

/// The `aozora fmt --fix-notation` autofix is the third consumer of the
/// canonical catalogue (after the pipeline lint and the LSP quick-fix). For
/// every catalogue sample it must: keep the variant verbatim (still
/// lint-flagged) by default, resolve the near-miss when opted in (so the lint
/// no longer fires), and stay a second-pass fixed point — the `write_back`
/// idempotency guard depends on the last property.
#[test]
fn fix_notation_resolves_every_variant_and_is_idempotent() {
    let fix = SerializeOptions { fix_notation: true };
    for &variant in CATALOGUE_SAMPLES {
        let input = format!("あ［＃{variant}］");

        // Default fmt keeps the flagged near-miss verbatim and still flagged
        // (the parser stays lossless; only opt-in fmt rewrites).
        let plain = Document::new(input.clone()).parse().to_source();
        assert!(
            plain.contains(&format!("［＃{variant}］")),
            "default fmt must preserve variant {variant:?} verbatim; got {plain:?}"
        );
        assert!(
            source_fires_lint(&plain),
            "default fmt output should still fire the lint for {variant:?}"
        );

        // --fix-notation rewrites the near-miss to canonical form, so the
        // notation-hygiene lint no longer fires on the result.
        let fixed = Document::new(input.clone()).parse().to_source_with(fix);
        assert!(
            !source_fires_lint(&fixed),
            "fix-notation should resolve the near-miss {variant:?}; got {fixed:?}"
        );

        // The rewrite is a second-pass fixed point: every directive is now
        // canonical (non-Unknown), so a further fix pass is a no-op.
        let again = Document::new(fixed.clone()).parse().to_source_with(fix);
        assert_eq!(
            fixed, again,
            "fix-notation must be idempotent for {variant:?}"
        );
    }
}

#[test]
fn genuine_editorial_unknown_does_not_fire() {
    for body in ["底本では「蒼空」", "入力者注", "未完", "「」は「」の「」"] {
        assert!(
            !fires_lint(body),
            "editorial Unknown {body:?} wrongly fired the notation-hygiene lint"
        );
    }
}
