//! Canonicalization workspace command.

use aozora::Catalogue;
use tower_lsp::lsp_types::{
    DocumentChanges, OneOf, OptionalVersionedTextDocumentIdentifier, Range, TextDocumentEdit,
    TextEdit, Url, WorkspaceEdit,
};

pub(super) const COMMAND_CANONICALIZE_SLUG: &str = "aozora.canonicalizeSlug";

#[must_use]
pub(super) fn canonicalize_slug_edit(
    uri: Url,
    version: i32,
    range: Range,
    body_text: &str,
) -> Option<WorkspaceEdit> {
    let (trimmed, wrapped) = strip_brackets(body_text.trim());
    let canonical = Catalogue::canonical(trimmed)?;
    let new_text = if wrapped {
        format!("［＃{canonical}］")
    } else {
        canonical.to_owned()
    };
    if new_text == body_text {
        return None;
    }
    let edit = TextEdit { range, new_text };
    Some(WorkspaceEdit {
        changes: None,
        document_changes: Some(DocumentChanges::Edits(vec![TextDocumentEdit {
            text_document: OptionalVersionedTextDocumentIdentifier::new(uri, version),
            edits: vec![OneOf::Left(edit)],
        }])),
        change_annotations: None,
    })
}

fn strip_brackets(s: &str) -> (&str, bool) {
    for opener in ["［＃", "［#", "[＃", "[#"] {
        let Some(rest) = s.strip_prefix(opener) else {
            continue;
        };
        if let Some(body) = rest.strip_suffix('］').or_else(|| rest.strip_suffix(']')) {
            return (body, true);
        }
    }
    (s, false)
}

#[cfg(test)]
mod tests {
    use tower_lsp::lsp_types::Position;

    use super::*;

    fn fake_uri() -> Url {
        Url::parse("file:///fake.afm").expect("valid URL")
    }

    fn fake_range() -> Range {
        Range::new(Position::new(0, 0), Position::new(0, 4))
    }

    #[test]
    fn variant_input_yields_canonical_replacement() {
        let edit =
            canonicalize_slug_edit(fake_uri(), 7, fake_range(), "［＃ぼうてん］").expect("edit");
        let DocumentChanges::Edits(changes) = edit.document_changes.expect("document changes")
        else {
            panic!("expected text document edits");
        };
        assert_eq!(changes[0].text_document.version, Some(7));
        let OneOf::Left(edit) = &changes[0].edits[0] else {
            panic!("expected plain text edit");
        };
        assert_eq!(edit.new_text, "［＃傍点］");
    }

    #[test]
    fn unwrapped_variant_yields_unwrapped_canonical() {
        let edit = canonicalize_slug_edit(fake_uri(), 7, fake_range(), "ぼうてん").expect("edit");
        let DocumentChanges::Edits(changes) = edit.document_changes.expect("document changes")
        else {
            panic!("expected text document edits");
        };
        let OneOf::Left(edit) = &changes[0].edits[0] else {
            panic!("expected plain text edit");
        };
        assert_eq!(edit.new_text, "傍点");
    }

    #[test]
    fn accepted_bracket_variants_normalize_to_full_width() {
        for body in [
            "［＃ぼうてん］",
            "［＃ぼうてん]",
            "［#ぼうてん］",
            "［#ぼうてん]",
            "[＃ぼうてん］",
            "[＃ぼうてん]",
            "[#ぼうてん］",
            "[#ぼうてん]",
        ] {
            let workspace = canonicalize_slug_edit(fake_uri(), 7, fake_range(), body)
                .expect("variant canonicalizes");
            let DocumentChanges::Edits(changes) =
                workspace.document_changes.expect("document changes")
            else {
                panic!("expected text document edits");
            };
            let OneOf::Left(edit) = &changes[0].edits[0] else {
                panic!("expected plain text edit");
            };
            assert_eq!(edit.new_text, "［＃傍点］", "{body}");
        }
    }

    #[test]
    fn canonical_body_with_noncanonical_brackets_is_normalized() {
        let workspace = canonicalize_slug_edit(fake_uri(), 7, fake_range(), "[#傍点]")
            .expect("brackets canonicalize");
        let DocumentChanges::Edits(changes) = workspace.document_changes.expect("document changes")
        else {
            panic!("expected text document edits");
        };
        let OneOf::Left(edit) = &changes[0].edits[0] else {
            panic!("expected plain text edit");
        };
        assert_eq!(edit.new_text, "［＃傍点］");
    }

    #[test]
    fn already_canonical_returns_none() {
        let edit = canonicalize_slug_edit(fake_uri(), 7, fake_range(), "［＃傍点］");
        assert!(edit.is_none(), "no-op canonicalisation must return None");
    }

    #[test]
    fn unrecognised_input_returns_none() {
        assert!(canonicalize_slug_edit(fake_uri(), 7, fake_range(), "［＃なんだろう］").is_none());
    }
}
