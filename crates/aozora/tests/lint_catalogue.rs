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
use aozora::render::{RenderOptions, SerializeOptions};
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

/// Build a source that gives `variant`'s canonical spelling a fair render
/// context. A forward-reference form only renders non-inert when its referent
/// is present in the preceding text, so prepend the forward target literal:
/// the first `「X」`'s inner text for a quoted forward, or — for the
/// bare-parenthesised 縦中横 target `（…）`, whose canonical `「（…）」は縦中横`
/// quotes the paren run — the whole `（…）` operand. Every non-forward form
/// (region open/close, alignment, marker synonyms) gets a neutral `あ` lead.
fn render_context_source(variant: &str) -> String {
    // Quoted forward target: prepend the first 「X」 inner text.
    if let Some(open) = variant.find('「') {
        let rest = &variant[open + '「'.len_utf8()..];
        if let Some(close) = rest.find('」') {
            return format!("{referent}［＃{variant}］", referent = &rest[..close]);
        }
    }
    // Bare-parenthesised 縦中横 target: prepend the whole （…） operand.
    if variant.starts_with('（')
        && let Some(close) = variant.find('）')
    {
        return format!(
            "{referent}［＃{variant}］",
            referent = &variant[..close + '）'.len_utf8()]
        );
    }
    format!("あ［＃{variant}］")
}

/// The seam pin (ADR-0022's fourth, opt-in render role): for every catalogue
/// variant, the normalise-render path replaces the inert `Unknown` directive
/// span with a real rendering of the canonical spelling. The existing
/// round-trip test proves a canonical is not itself a lint variant, but never
/// that it *renders* non-inert; this closes that gap. The default render stays
/// inert — the parser never reinterprets a near-miss — so the win is entirely
/// in the opt-in path.
#[test]
fn normalize_render_replaces_every_inert_variant() {
    let norm = RenderOptions {
        normalize_directives: true,
    };
    for &variant in CATALOGUE_SAMPLES {
        let source = render_context_source(variant);

        // (a) Without normalisation the near-miss is inert: the parser keeps it
        // Unknown and renders the hidden `aozora-directive` span.
        let default_html = Document::new(source.clone()).parse().to_html();
        assert!(
            default_html.contains("aozora-directive"),
            "variant {variant:?} should render inert by default; got {default_html:?}"
        );

        // (b) With normalisation the canonical replaces the near-miss: the
        // rendered HTML contains NEITHER `aozora-directive` NOR a ` hidden>`
        // span — the reader sees a visible element (or, for a referent-less /
        // region form, nothing), never an inert directive / heading-hint span.
        let normalized_html = Document::new(source.clone()).parse().to_html_with(norm);
        assert!(
            !normalized_html.contains("aozora-directive") && !normalized_html.contains(" hidden>"),
            "variant {variant:?} still renders inert after normalisation; \
             source {source:?} gave {normalized_html:?}"
        );
    }
}

/// The default render is unchanged by the new seam: `to_html_with` with the
/// default (opt-out) [`RenderOptions`] is byte-identical to `to_html()` across
/// plain text, a construct, a real directive, and a catalogue near-miss. Only
/// the opt-in flag can alter output.
#[test]
fn default_render_options_are_byte_identical_to_to_html() {
    for source in [
        "ただの平文です。",
        "｜青梅《おうめ》",
        "重要［＃「重要」は太字］",
        "あ［＃斜体字］",
    ] {
        let doc = Document::new(source.to_owned());
        let tree = doc.parse();
        assert_eq!(
            tree.to_html_with(RenderOptions::default()),
            tree.to_html(),
            "default RenderOptions must be byte-identical to to_html() for {source:?}"
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
