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
use aozora_syntax::lint::{CATALOGUE_SAMPLES, canonical_directive};

const LINT_CODE: &str = "aozora::lint::non_canonical_directive";

fn fires_lint(body: &str) -> bool {
    Document::new(format!("あ［＃{body}］"))
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

#[test]
fn genuine_editorial_unknown_does_not_fire() {
    for body in ["底本では「蒼空」", "入力者注", "未完", "「」は「」の「」"] {
        assert!(
            !fires_lint(body),
            "editorial Unknown {body:?} wrongly fired the notation-hygiene lint"
        );
    }
}
