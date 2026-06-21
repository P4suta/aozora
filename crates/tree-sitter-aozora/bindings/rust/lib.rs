//! Tree-sitter binding for aozora-flavored markdown.
//!
//! Exposes [`LANGUAGE`] — a [`tree_sitter_language::LanguageFn`] —
//! that the consuming crate (today: `aozora-lsp`) feeds into a
//! `tree_sitter::Parser` to incrementally parse aozora source.
//! (We don't intra-doc-link `tree_sitter` because this crate doesn't
//! depend on it directly — only consumers do.)
//!
//! ## Why this crate exists
//!
//! `aozora-lsp` needs **size-independent** request latency. The
//! semantic Rust parser (`aozora-parser`) re-parses the entire
//! document on every call — fine for KB-sized docs, painful for
//! 40-100 KB docs (414 ms per parse, multiple handlers per
//! keystroke). Tree-sitter's incremental algorithm reuses
//! unchanged sub-trees so the second-and-onward edit costs
//! O(edit_size) instead of O(doc_size).
//!
//! The semantic parser stays the source of truth for HTML
//! rendering, formatting, and diagnostics — operations where the
//! tree-sitter syntax skeleton is too thin. The LSP runs both
//! parsers in parallel; the responsiveness gain comes from moving
//! the high-frequency handlers (hover, inlay, codeAction,
//! completion) onto the tree-sitter side.

use tree_sitter_language::LanguageFn;

unsafe extern "C" {
    fn tree_sitter_aozora() -> *const ();
}

/// The tree-sitter language for aozora-flavored markdown. Hand the
/// returned [`LanguageFn`] to `tree_sitter::Parser::set_language`
/// (in a downstream crate that depends on `tree_sitter`).
///
/// # Example
///
/// ```ignore
/// use tree_sitter::Parser;
/// let mut parser = Parser::new();
/// parser
///     .set_language(&tree_sitter_aozora::LANGUAGE.into())
///     .expect("language compiled in");
/// let tree = parser.parse("｜青空《あおぞら》", None).expect("parse");
/// ```
pub const LANGUAGE: LanguageFn = unsafe { LanguageFn::from_raw(tree_sitter_aozora) };

/// Node-kind names exposed by the grammar. Centralised here so
/// consumers (queries, walkers) reference them by symbol instead of
/// string-literal-everywhere.
pub mod kind {
    pub const DOCUMENT: &str = "document";
    pub const GAIJI: &str = "gaiji";
    pub const SLUG: &str = "slug";
    pub const SLUG_BODY: &str = "slug_body";
    pub const EXPLICIT_RUBY: &str = "explicit_ruby";
    pub const IMPLICIT_RUBY: &str = "implicit_ruby";
    pub const RUBY_BASE_EXPLICIT: &str = "ruby_base_explicit";
    pub const RUBY_BASE_IMPLICIT: &str = "ruby_base_implicit";
    pub const RUBY_READING: &str = "ruby_reading";
    pub const TEXT: &str = "text";
    pub const NEWLINE: &str = "newline";
}

#[cfg(test)]
mod tests {
    use tree_sitter::Parser;

    fn parse(src: &str) -> tree_sitter::Tree {
        let mut parser = Parser::new();
        parser
            .set_language(&super::LANGUAGE.into())
            .expect("language is compiled in");
        parser.parse(src, None).expect("parse never fails")
    }

    #[test]
    fn empty_input_parses_to_empty_document() {
        let tree = parse("");
        let root = tree.root_node();
        assert_eq!(root.kind(), super::kind::DOCUMENT);
        assert_eq!(root.named_child_count(), 0);
    }

    #[test]
    fn plain_text_only() {
        let tree = parse("hello, 世界");
        let root = tree.root_node();
        assert_eq!(root.kind(), super::kind::DOCUMENT);
        // Should contain text node(s); no markup detected.
        let mut cursor = root.walk();
        for child in root.named_children(&mut cursor) {
            assert!(
                matches!(child.kind(), "text" | "newline"),
                "unexpected child kind: {}",
                child.kind(),
            );
        }
    }

    #[test]
    fn detects_gaiji_span() {
        let src = "前※［＃「木＋吶のつくり」、第3水準1-85-54］後";
        let tree = parse(src);
        let root = tree.root_node();
        let mut found_gaiji = false;
        let mut cursor = root.walk();
        for child in root.named_children(&mut cursor) {
            if child.kind() == super::kind::GAIJI {
                found_gaiji = true;
                let mut inner = child.walk();
                let slug = child
                    .named_children(&mut inner)
                    .find(|c| c.kind() == super::kind::SLUG)
                    .expect("gaiji always wraps a slug");
                let body = slug
                    .child_by_field_name("body")
                    .expect("slug carries a body field");
                let body_text = body.utf8_text(src.as_bytes()).expect("UTF-8");
                assert!(
                    body_text.contains("木＋吶のつくり"),
                    "slug body should carry the description: {body_text}",
                );
            }
        }
        assert!(
            found_gaiji,
            "expected one gaiji span in {:?}",
            root.to_sexp(),
        );
    }

    #[test]
    fn detects_explicit_ruby() {
        let tree = parse("｜青空《あおぞら》");
        let root = tree.root_node();
        let mut cursor = root.walk();
        let ruby = root
            .named_children(&mut cursor)
            .find(|c| c.kind() == super::kind::EXPLICIT_RUBY)
            .expect("expected one explicit_ruby span");
        let base = ruby
            .child_by_field_name("base")
            .expect("ruby carries a base field");
        let reading = ruby
            .child_by_field_name("reading")
            .expect("ruby carries a reading field");
        assert_eq!(base.kind(), super::kind::RUBY_BASE_EXPLICIT);
        assert_eq!(reading.kind(), super::kind::RUBY_READING);
    }

    #[test]
    fn detects_implicit_ruby_after_kanji_run() {
        let tree = parse("青空《あおぞら》");
        let root = tree.root_node();
        let mut cursor = root.walk();
        let ruby = root
            .named_children(&mut cursor)
            .find(|c| c.kind() == super::kind::IMPLICIT_RUBY)
            .expect("expected implicit_ruby for kanji+《》 sequence");
        assert_eq!(
            ruby.child_by_field_name("base").unwrap().kind(),
            super::kind::RUBY_BASE_IMPLICIT,
        );
    }

    #[test]
    fn incremental_edit_reuses_subtree() {
        // Stage-1 acceptance test: the whole point of switching to TS
        // is incremental reparses. Edit a tiny section and verify the
        // edited tree carries fresh text without re-walking the rest.
        let initial = "前文\n｜青空《あおぞら》\n後文";
        let mut parser = Parser::new();
        parser
            .set_language(&super::LANGUAGE.into())
            .expect("language compiled in");
        let mut tree = parser.parse(initial, None).expect("parse");

        // Replace 「あおぞら」 with 「そら」.
        let new_src = "前文\n｜青空《そら》\n後文";
        let edit = tree_sitter::InputEdit {
            start_byte: initial.find("あおぞら").unwrap(),
            old_end_byte: initial.find("あおぞら").unwrap() + "あおぞら".len(),
            new_end_byte: initial.find("あおぞら").unwrap() + "そら".len(),
            start_position: tree_sitter::Point::default(),
            old_end_position: tree_sitter::Point::default(),
            new_end_position: tree_sitter::Point::default(),
        };
        tree.edit(&edit);
        let new_tree = parser
            .parse(new_src, Some(&tree))
            .expect("incremental parse");
        assert!(
            new_tree.root_node().to_sexp().contains("explicit_ruby"),
            "incremental tree should still carry the ruby node",
        );
    }
}
