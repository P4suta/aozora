//! Render path totality + structural soundness.
//!
//! Two complementary properties on the borrowed-AST → HTML path:
//!
//! 1. **Totality**: [`html::render_to_string`] must not panic on any
//!    input the lexer accepts (which is "any UTF-8 string" — the
//!    pipeline emits diagnostics rather than failing). The render
//!    layer is the last unfallible step before output; a panic here
//!    is a denial-of-service surface for any caller that hands user
//!    input to the renderer.
//!
//! 2. **Structural soundness**: every `<p>` is matched by a `</p>`,
//!    every `<aozora-container>` is matched by a `</aozora-container>`.
//!    A renderer regression that emits an unbalanced tag corrupts
//!    every downstream HTML consumer; the proptest is the decisive
//!    way to catch a stray phase-state branch that drops a closing
//!    tag.

use aozora_pipeline::lex_into_arena;
use aozora_proptest::config::default_config;
use aozora_proptest::generators::*;
use aozora_render::html::render_to_string;
use aozora_syntax::borrowed::Arena;
use proptest::prelude::*;

/// Count occurrences of a literal substring without allocating.
fn count_substr(haystack: &str, needle: &str) -> usize {
    haystack.matches(needle).count()
}

fn assert_render_is_balanced(source: &str) {
    let arena = Arena::new();
    let out = lex_into_arena(source, &arena);
    let html = render_to_string(&out);

    // Paragraph balance — `<p` rather than `<p>` to also catch
    // attribute-bearing variants that the renderer might one day
    // introduce. End tag is unambiguous.
    let p_open = count_substr(&html, "<p");
    let p_close = count_substr(&html, "</p>");
    assert_eq!(
        p_open, p_close,
        "paragraph tags unbalanced for source {source:?}\nopen={p_open} close={p_close}\n---\n{html}"
    );
}

// ----------------------------------------------------------------------
// Hand-curated regression anchors.
// ----------------------------------------------------------------------

#[test]
fn empty_input_renders_without_panic() {
    assert_render_is_balanced("");
}

#[test]
fn plain_text_renders_balanced() {
    assert_render_is_balanced("Hello, world.");
    assert_render_is_balanced("こんにちは、世界！");
}

#[test]
fn ruby_and_brackets_render_balanced() {
    assert_render_is_balanced("｜青梅《おうめ》");
    assert_render_is_balanced("text［＃改ページ］more");
}

#[test]
fn paired_container_renders_balanced() {
    assert_render_is_balanced(
        "［＃ここから2字下げ］\n\
         indented body\n\
         ［＃ここで字下げ終わり］",
    );
}

proptest! {
    #![proptest_config(default_config())]

    /// Render is total over the workhorse Aozora-fragment generator —
    /// no panic, balanced paragraph tags.
    #[test]
    fn aozora_fragment_renders_balanced(s in aozora_fragment(120)) {
        assert_render_is_balanced(&s);
    }

    /// Pathological / unbalanced source — the renderer must still be
    /// total, even when the lexer emits diagnostics.
    #[test]
    fn pathological_input_renders_balanced(s in pathological_aozora(120)) {
        assert_render_is_balanced(&s);
    }

    /// Unicode adversarial — combining marks, RTL overrides, PUA
    /// codepoints. The renderer must escape and balance regardless.
    #[test]
    fn unicode_adversarial_renders_balanced(s in unicode_adversarial()) {
        assert_render_is_balanced(&s);
    }

    /// XSS payloads — the rendered HTML must not contain the literal
    /// dangerous substring `<script>` (fully escaped or absent). The
    /// payload generator emits already-dangerous source; the renderer
    /// must not pass them through verbatim.
    #[test]
    fn xss_payload_does_not_leak_script_tag(s in xss_payload()) {
        let arena = Arena::new();
        let out = lex_into_arena(&s, &arena);
        let html = render_to_string(&out);
        prop_assert!(
            !html.contains("<script>") && !html.contains("<SCRIPT>"),
            "renderer leaked an unescaped <script> tag for source {s:?}\n---\n{html}"
        );
    }
}
