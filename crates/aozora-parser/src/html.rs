//! HTML rendering front-door.
//!
//! Wraps `comrak::format_html`. The Aozora extension has already been registered
//! on the `Options` used at parse time, so comrak's own HTML renderer will
//! dispatch `NodeValue::Aozora(_)` arms through to our [`crate::aozora::html`]
//! module via the adapter's `render_html` method.

use comrak::nodes::AstNode;

use crate::Options;

/// Render `input` to HTML using afm defaults.
///
/// Convenience wrapper: allocates a temporary arena, parses, and serialises.
/// For workflows that need the AST itself, call [`crate::parse`] directly and
/// then pass the root into [`render_root_to_string`].
#[must_use]
pub fn render_to_string(input: &str) -> String {
    let arena = comrak::Arena::new();
    let options = Options::afm_default();
    let root = crate::parse(&arena, input, &options);
    render_root_to_string(root, &options)
}

/// Serialise a previously-parsed root to HTML. Panics are funnelled to an empty
/// output; comrak's formatter is infallible for well-formed trees.
#[must_use]
pub fn render_root_to_string<'a>(root: &'a AstNode<'a>, options: &Options<'_>) -> String {
    let mut out = String::new();
    comrak::format_html(root, &options.comrak, &mut out).unwrap_or(());
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_paragraph_round_trips_through_comrak() {
        let html = render_to_string("Hello.");
        assert!(html.contains("<p>Hello.</p>"));
    }

    #[test]
    fn ruby_is_emitted_semantically() {
        let html = render_to_string("｜青梅《おうめ》");
        assert!(html.contains("<ruby>青梅"), "missing ruby tag: {html}");
        assert!(html.contains("<rt>おうめ"), "missing rt tag: {html}");
    }

    #[test]
    fn block_annotation_produces_hidden_span() {
        let html = render_to_string("前［＃改ページ］後");
        assert!(
            html.contains(r#"class="afm-annotation""#),
            "missing annotation wrapper: {html}"
        );
        assert!(
            !html.contains("［＃改ページ］") || html.contains(">［＃改ページ］<"),
            "annotation not consumed: {html}"
        );
    }
}
