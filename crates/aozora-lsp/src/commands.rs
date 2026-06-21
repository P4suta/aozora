//! `workspace/executeCommand` handler.
//!
//! Single command: `aozora.canonicalizeSlug`. Argument shape:
//!
//! ```json
//! {
//!   "uri":   "file:///…",
//!   "range": { "start": {"line": 0, "character": 4 },
//!              "end":   {"line": 0, "character": 12 } }
//! }
//! ```
//!
//! The handler:
//!
//! 1. Reads the `range` slice of the document text (the LSP backend
//!    looks up the document by `uri` and passes the slice in).
//! 2. Strips an optional surrounding `［＃` / `］`.
//! 3. Calls [`aozora::canonicalise_slug`] to snap the body to its
//!    canonical form.
//! 4. Returns a [`WorkspaceEdit`] that replaces `range` with the
//!    canonical text (or `None` when no canonicalisation applies).
//!
//! The completion handler also wires this command into its
//! `additional_text_edits` flow when the user accepts a slug variant —
//! see `completion.rs` for the fan-in side.

use std::collections::HashMap;

use aozora::canonicalise_slug;
use tower_lsp::lsp_types::{Range, TextEdit, Url, WorkspaceEdit};

/// Canonical slug-canonicalize command identifier exchanged with the
/// editor via `workspace/executeCommand`. Picked to mirror the
/// `VSCode` `contributes.commands` convention (`<extension>.<verb>`).
pub const COMMAND_CANONICALIZE_SLUG: &str = "aozora.canonicalizeSlug";

/// Compute the [`WorkspaceEdit`] that canonicalises the slug body.
///
/// `body_text` is the substring the editor has already extracted from
/// `range` against `uri` — the LSP backend looks up the document and
/// slices.
///
/// Returns `None` if no canonicalisation applies (the body is already
/// canonical, or it's not a recognised slug). The caller maps `None`
/// to a no-op response.
#[must_use]
pub fn canonicalize_slug_edit(uri: Url, range: Range, body_text: &str) -> Option<WorkspaceEdit> {
    let trimmed = strip_brackets(body_text.trim());
    let canonical = canonicalise_slug(trimmed)?;
    if canonical == trimmed {
        // Already canonical → no edit.
        return None;
    }
    // Preserve the surrounding ［＃ / ］ if the input had them.
    let new_text = if body_text.trim().starts_with("［＃") {
        format!("［＃{canonical}］")
    } else {
        canonical.to_owned()
    };
    let edit = TextEdit { range, new_text };
    let mut changes: HashMap<Url, Vec<TextEdit>> = HashMap::new();
    changes.insert(uri, vec![edit]);
    Some(WorkspaceEdit {
        changes: Some(changes),
        document_changes: None,
        change_annotations: None,
    })
}

fn strip_brackets(s: &str) -> &str {
    s.strip_prefix("［＃")
        .and_then(|s| s.strip_suffix('］'))
        .unwrap_or(s)
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
            canonicalize_slug_edit(fake_uri(), fake_range(), "［＃ぼうてん］").expect("edit");
        let changes = edit.changes.expect("changes");
        let edits = changes.values().next().expect("one entry");
        assert_eq!(edits[0].new_text, "［＃傍点］");
    }

    #[test]
    fn unwrapped_variant_yields_unwrapped_canonical() {
        let edit = canonicalize_slug_edit(fake_uri(), fake_range(), "ぼうてん").expect("edit");
        let changes = edit.changes.unwrap();
        let edits = changes.values().next().unwrap();
        assert_eq!(edits[0].new_text, "傍点");
    }

    #[test]
    fn already_canonical_returns_none() {
        let edit = canonicalize_slug_edit(fake_uri(), fake_range(), "［＃傍点］");
        assert!(edit.is_none(), "no-op canonicalisation must return None");
    }

    #[test]
    fn unrecognised_input_returns_none() {
        assert!(canonicalize_slug_edit(fake_uri(), fake_range(), "［＃なんだろう］").is_none());
    }
}
