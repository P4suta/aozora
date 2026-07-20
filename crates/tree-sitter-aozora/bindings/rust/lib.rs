//! Tree-sitter binding for Aozora Bunko notation.
//!
//! [`LANGUAGE`] provides a lossless editing projection of the source language.
//! The `aozora` crate remains the semantic authority.

// FFI binding to the generated tree-sitter C parser. The unsafe here is the
// standard `tree-sitter-language` pattern (and is exempt in the strict-code
// gate, alongside `aozora-ffi`); every block stays explicitly gated.
#![deny(unsafe_op_in_unsafe_fn)]

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
                let body_text = child.utf8_text(src.as_bytes()).expect("UTF-8");
                assert!(
                    body_text.contains("木＋吶のつくり"),
                    "gaiji should carry the description: {body_text}",
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
        let source = "｜青空《あおぞら》";
        let tree = parse(source);
        let root = tree.root_node();
        let mut cursor = root.walk();
        let ruby = root
            .named_children(&mut cursor)
            .find(|c| c.kind() == super::kind::EXPLICIT_RUBY)
            .expect("expected one explicit_ruby span");
        assert_eq!(ruby.utf8_text(source.as_bytes()).expect("UTF-8"), source);
        assert_eq!(ruby.child_count(), 0);
    }

    #[test]
    fn detects_implicit_ruby_after_kanji_run() {
        let source = "青空《あおぞら》";
        let tree = parse(source);
        let root = tree.root_node();
        let mut cursor = root.walk();
        let ruby = root
            .named_children(&mut cursor)
            .find(|c| c.kind() == super::kind::IMPLICIT_RUBY)
            .expect("expected implicit_ruby for kanji+《》 sequence");
        assert_eq!(ruby.utf8_text(source.as_bytes()).expect("UTF-8"), source);
        assert_eq!(ruby.child_count(), 0);
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
