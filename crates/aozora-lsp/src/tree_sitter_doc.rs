//! Document-level tree-sitter wrapper. Kept as the control group for
//! the editor-burst microbenchmarks (`crates/aozora-lsp/benches/burst.rs`)
//! and as the home of the `input_edit` helper that
//! [`crate::state::DocBuffer`] reuses.
//!
//! ## Production state lives in `crate::paragraph`, not here
//!
//! Each open document is split at `\n\n` boundaries into a
//! `Vec<ParagraphBuffer>`, where every [`crate::paragraph::ParagraphBuffer`]
//! owns its own `Rope` text and tree-sitter `Tree`. Edits reparse only
//! the paragraph they hit. The reason is operational: tree-sitter's
//! parse on the aozora grammar is `O(doc-size)` (~33 ns/byte), so a
//! single document-wide reparse on a 6 MB input costs ~220 ms — too
//! slow for keystroke responsiveness. See [`crate::state`] for the
//! split-buffer architecture and the snapshot rebuild reuse story.
//!
//! ## What `TreeSitterDoc` is for
//!
//! The struct below predates the paragraph-first refactor. It is now
//! retained as the **measurement control** for the design decision:
//!
//! - `bench_ts_parse_full` measures whole-document parse cost on a
//!   6 MB input.
//! - `bench_ts_apply_edit_offset_0` and `bench_ts_apply_edit_mid_doc`
//!   measure tree-sitter's `apply_edit` cost depending on edit
//!   position.
//! - `bench_ts_parse_60kb_slice` / `bench_ts_parse_600kb_slice` measure
//!   parse cost on smaller slices, so the paragraph-first design's
//!   payoff (per-edit cost capped at paragraph size) stays quantifiable.
//!
//! The unit tests at the bottom of this module pin the tree-sitter
//! incremental contract — 1-shot parse ≡ initial parse + `apply_edit`,
//! the chunked-Rope parse path, char-boundary handling — invariants
//! the per-paragraph production code reuses through
//! [`crate::paragraph::ParagraphBuffer::apply_edit`].
//!
//! ## Semantic vs syntactic split
//!
//! The `aozora` Rust parser remains the source of truth for
//! diagnostics, HTML rendering, and formatting (see
//! [`crate::parse_cache::ParseCache`]). Tree-sitter is the
//! structural-only fast path that high-frequency LSP requests
//! (hover, completion, linkedEditingRange, semanticTokens, gaiji
//! span extraction) ride against the per-paragraph trees.

use std::fmt;
use std::sync::Mutex;

use ropey::Rope;
use tree_sitter::{InputEdit, Parser, Point, Tree};

/// Per-document tree-sitter state.
///
/// `Mutex` because [`Parser`] is `!Sync` (it carries internal
/// stacks). Wrapped in a single struct so the LSP backend's
/// `OpenDocument` doesn't have to synchronise the parser and tree
/// independently. Lock contention is negligible at LSP rates: each
/// edit takes microseconds on the incremental path.
pub struct TreeSitterDoc {
    inner: Mutex<Inner>,
}

struct Inner {
    parser: Parser,
    tree: Option<Tree>,
}

impl fmt::Debug for TreeSitterDoc {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TreeSitterDoc").finish_non_exhaustive()
    }
}

impl TreeSitterDoc {
    /// Build a fresh per-document parser. The tree-sitter language
    /// is set once; subsequent calls reuse the parser.
    ///
    /// # Panics
    /// If the bundled `tree-sitter-aozora` grammar is incompatible
    /// with the linked tree-sitter runtime (ABI version mismatch).
    /// In practice this only fires if the build script regenerated
    /// the parser against a different runtime than the one resolved
    /// by Cargo.
    #[must_use]
    pub fn new() -> Self {
        let mut parser = Parser::new();
        parser
            .set_language(&tree_sitter_aozora::LANGUAGE.into())
            .expect("tree-sitter-aozora language is compiled in");
        Self {
            inner: Mutex::new(Inner { parser, tree: None }),
        }
    }

    /// Whole-document parse. Drops any existing tree. Used on
    /// `didOpen` and on full-replacement `didChange` events (where
    /// the LSP client decided to send the entire buffer instead of
    /// an incremental range).
    ///
    /// # Panics
    /// If the inner `Mutex` was poisoned by a previous panicking
    /// caller. With the current single-handler-per-doc usage this
    /// is unreachable in practice.
    pub fn parse_full(&self, text: &str) {
        let mut inner = self.inner.lock().expect("parser mutex");
        inner.tree = inner.parser.parse(text, None);
    }

    /// Apply an incremental edit. The caller has already mutated the
    /// underlying buffer; here we tell tree-sitter where the edit
    /// happened (in byte coordinates) and let it reparse only the
    /// affected sub-trees.
    ///
    /// The `Point` fields default to zero — tree-sitter accepts that
    /// when the column count is genuinely unused (we never query
    /// row/column after the edit). If a future feature needs
    /// position-accurate tree info, populate the `Point`s here.
    ///
    /// # Panics
    /// On a poisoned inner `Mutex` (see [`Self::parse_full`]).
    pub fn apply_edit(&self, new_text: &str, edit: InputEdit) {
        let mut inner = self.inner.lock().expect("parser mutex");
        // Take the existing tree out so the borrow checker lets us
        // pass `&self.parser` and `&Tree` simultaneously into
        // `parse`. The new tree replaces it.
        let mut prior = inner.tree.take();
        if let Some(tree) = prior.as_mut() {
            tree.edit(&edit);
        }
        inner.tree = inner.parser.parse(new_text, prior.as_ref());
    }

    /// Whole-document parse driven by chunked input from a [`Rope`].
    /// `parser.parse_with` calls our callback repeatedly with byte
    /// offsets; we hand back the rope chunk containing each requested
    /// offset. Tree-sitter walks each chunk only once, so the parser
    /// never needs a contiguous `String` materialisation of the
    /// document.
    ///
    /// # Panics
    /// On a poisoned inner `Mutex`.
    pub fn parse_full_rope(&self, rope: &Rope) {
        let mut inner = self.inner.lock().expect("parser mutex");
        inner.tree = inner
            .parser
            .parse_with_options(&mut chunk_callback(rope), None, None);
    }

    /// Incremental edit + reparse driven by chunked input. Mirrors
    /// [`Self::apply_edit`] but accepts the post-edit buffer as a
    /// `Rope` so the parse step doesn't need a contiguous `String`.
    ///
    /// # Panics
    /// On a poisoned inner `Mutex`.
    pub fn apply_edit_rope(&self, rope: &Rope, edit: InputEdit) {
        let mut inner = self.inner.lock().expect("parser mutex");
        let mut prior = inner.tree.take();
        if let Some(tree) = prior.as_mut() {
            tree.edit(&edit);
        }
        inner.tree =
            inner
                .parser
                .parse_with_options(&mut chunk_callback(rope), prior.as_ref(), None);
    }

    /// Run a closure against the current tree. Returns `None` when
    /// no parse has been recorded yet (newly opened empty docs).
    ///
    /// The callback runs while the parser mutex is held — keep it
    /// short. For longer work (allocating Vecs, running multiple
    /// queries) consider cloning out the relevant nodes inside the
    /// closure and processing them after the lock is released.
    ///
    /// # Panics
    /// On a poisoned inner `Mutex` (see [`Self::parse_full`]).
    pub fn with_tree<R>(&self, f: impl FnOnce(&Tree) -> R) -> Option<R> {
        let inner = self.inner.lock().expect("parser mutex");
        inner.tree.as_ref().map(f)
    }
}

impl Default for TreeSitterDoc {
    fn default() -> Self {
        Self::new()
    }
}

/// Build a tree-sitter `parse_with` callback that streams bytes from
/// a [`Rope`]. The callback returns the chunk containing the
/// requested byte offset, sliced from that offset onward; tree-sitter
/// stops calling once it sees an empty slice (`byte_idx >= len`).
///
/// The closure borrows the rope by reference; the resulting `&[u8]`
/// slices live for the duration of one callback invocation, which
/// fits the `parse_with` contract (the parser copies bytes it cares
/// about into its internal state before returning).
fn chunk_callback<'r>(rope: &'r Rope) -> impl FnMut(usize, Point) -> &'r [u8] {
    let len = rope.len_bytes();
    move |byte_idx, _pos| -> &'r [u8] {
        if byte_idx >= len {
            return &[];
        }
        let (chunk, chunk_byte_idx, _, _) = rope.chunk_at_byte(byte_idx);
        let local = byte_idx - chunk_byte_idx;
        &chunk.as_bytes()[local..]
    }
}

/// Translate an LSP-style "old text → new text" edit into a
/// tree-sitter [`InputEdit`].
///
/// The caller supplies byte offsets; the `Point` fields are left at
/// zero because the LSP backend's tree consumers never query row /
/// column positions on the TS tree (everything is byte-driven via
/// `crate::position`, private).
#[must_use]
pub fn input_edit(start_byte: usize, old_end_byte: usize, new_end_byte: usize) -> InputEdit {
    InputEdit {
        start_byte,
        old_end_byte,
        new_end_byte,
        start_position: Point::default(),
        old_end_position: Point::default(),
        new_end_position: Point::default(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tree_sitter_aozora::kind;

    #[test]
    fn full_parse_then_query() {
        let doc = TreeSitterDoc::new();
        doc.parse_full("｜青空《あおぞら》");
        let kind = doc
            .with_tree(|tree| tree.root_node().named_child(0).map(|n| n.kind().to_owned()))
            .flatten()
            .expect("tree exists");
        assert_eq!(kind, kind::EXPLICIT_RUBY);
    }

    #[test]
    fn incremental_edit_keeps_tree_in_sync() {
        let doc = TreeSitterDoc::new();
        let initial = "｜青空《あおぞら》";
        doc.parse_full(initial);

        // Replace 「あおぞら」 with 「そら」 — same shape, shorter
        // reading. Tree-sitter must reuse the surrounding ruby
        // structure rather than re-parse from scratch.
        let reading_start = initial.find("あおぞら").unwrap();
        let new_text = "｜青空《そら》";
        let edit = input_edit(
            reading_start,
            reading_start + "あおぞら".len(),
            reading_start + "そら".len(),
        );
        doc.apply_edit(new_text, edit);

        let kind = doc
            .with_tree(|tree| tree.root_node().named_child(0).map(|n| n.kind().to_owned()))
            .flatten()
            .expect("tree exists");
        assert_eq!(kind, kind::EXPLICIT_RUBY);
    }

    #[test]
    fn empty_doc_yields_none_for_with_tree_until_parse() {
        let doc = TreeSitterDoc::new();
        let result = doc.with_tree(|_| 42);
        assert_eq!(result, None);
    }

    #[test]
    fn full_replace_after_initial_parse_drops_old_tree() {
        let doc = TreeSitterDoc::new();
        doc.parse_full("plain text");
        doc.parse_full("｜青空《あおぞら》");
        let kind = doc
            .with_tree(|tree| tree.root_node().named_child(0).map(|n| n.kind().to_owned()))
            .flatten();
        assert_eq!(kind.as_deref(), Some(kind::EXPLICIT_RUBY));
    }

    #[test]
    fn default_constructs_a_usable_parser() {
        // `Default::default()` should produce a parser equivalent to
        // `TreeSitterDoc::new()` — the explicit `Default` impl exists
        // for derive-friendly callers, so a regression that drops the
        // language binding would surface here.
        let doc = TreeSitterDoc::default();
        doc.parse_full("｜空《そら》");
        let has_tree = doc.with_tree(|tree| tree.root_node().child_count() > 0);
        assert_eq!(has_tree, Some(true));
    }

    #[test]
    fn debug_format_does_not_leak_internals() {
        // The `Debug` impl uses `finish_non_exhaustive` so a future
        // refactor that exposes the parser / tree fields directly
        // would change the formatted output. Pin the current shape so
        // such a regression is loud.
        let doc = TreeSitterDoc::new();
        let s = format!("{doc:?}");
        assert!(s.starts_with("TreeSitterDoc"), "got {s:?}");
        assert!(
            s.contains(".."),
            "expected non_exhaustive marker, got {s:?}"
        );
    }

    // -------------------------------------------------------------
    // Rope-driven parse/edit paths (apply_edit_rope, parse_full_rope,
    // chunk_callback). These mirror the byte-driven tests above; the
    // backend always uses the rope path on real edits, so leaving them
    // untested would mean the byte-driven tests were the only thing
    // protecting the chunked-input contract.
    // -------------------------------------------------------------

    #[test]
    fn parse_full_rope_matches_string_parse() {
        let text = "｜青空《あおぞら》\n\n本文";
        let by_string = TreeSitterDoc::new();
        by_string.parse_full(text);
        let by_rope = TreeSitterDoc::new();
        by_rope.parse_full_rope(&Rope::from_str(text));

        let s_kinds = by_string.with_tree(collect_kinds).expect("tree");
        let r_kinds = by_rope.with_tree(collect_kinds).expect("tree");
        assert_eq!(s_kinds, r_kinds);
    }

    #[test]
    fn apply_edit_rope_matches_byte_apply_edit() {
        let initial = "｜青空《あおぞら》";
        let after = "｜青空《そら》";
        let reading_start = initial.find("あおぞら").unwrap();
        let edit = input_edit(
            reading_start,
            reading_start + "あおぞら".len(),
            reading_start + "そら".len(),
        );

        let via_str = TreeSitterDoc::new();
        via_str.parse_full(initial);
        via_str.apply_edit(after, edit);

        let via_rope = TreeSitterDoc::new();
        via_rope.parse_full_rope(&Rope::from_str(initial));
        via_rope.apply_edit_rope(&Rope::from_str(after), edit);

        let s = via_str.with_tree(collect_kinds).expect("tree");
        let r = via_rope.with_tree(collect_kinds).expect("tree");
        assert_eq!(s, r);
    }

    #[test]
    fn chunk_callback_returns_empty_past_end_of_rope() {
        // `chunk_callback` is internal but exercised only via the
        // rope-parse path, which never asks for an offset >= len in
        // practice. Pin the early-return contract so a future change
        // (e.g. wrapping vs panicking) can't slip in unnoticed.
        let rope = Rope::from_str("hello");
        let mut cb = chunk_callback(&rope);
        assert!(!cb(0, Point::default()).is_empty());
        assert!(cb(rope.len_bytes(), Point::default()).is_empty());
        assert!(cb(rope.len_bytes() + 100, Point::default()).is_empty());
    }

    #[test]
    fn chunk_callback_streams_multi_chunk_rope() {
        // A rope built from many small fragments creates multiple
        // internal chunks; the callback must hand back consecutive
        // slices that, concatenated, reproduce the source. If the
        // chunk-boundary math drifts by one byte, the parser would
        // see corrupt input — exactly the regression class to pin.
        let mut rope = Rope::new();
        for _ in 0..64 {
            rope.append(Rope::from_str("｜青空《あおぞら》\n"));
        }
        let len = rope.len_bytes();
        let mut cb = chunk_callback(&rope);
        let mut reconstructed = Vec::with_capacity(len);
        let mut offset = 0;
        while offset < len {
            let slice = cb(offset, Point::default());
            assert!(!slice.is_empty(), "callback returned empty mid-stream");
            reconstructed.extend_from_slice(slice);
            offset += slice.len();
        }
        assert_eq!(reconstructed.len(), len);
        // String round-trip: bytes must be valid UTF-8 again.
        let s = String::from_utf8(reconstructed).expect("valid utf8");
        let want: String = (0..64).map(|_| "｜青空《あおぞら》\n").collect();
        assert_eq!(s, want);
    }

    // -------------------------------------------------------------
    // Core invariant: 1-shot parse ≡ (initial parse + incremental
    // edit) for the same final text. This is the contract gaiji_spans
    // / linked_editing / hover all rely on. Each scenario probes a
    // different byte-range shape (insertion / deletion / replacement /
    // multi-paragraph edit). A regression that miscomputes
    // `InputEdit` byte boundaries would diverge the trees here.
    // -------------------------------------------------------------

    #[test]
    fn invariant_insertion_matches_full_parse() {
        let initial = "｜空《そら》";
        let edit_at = initial.len(); // append at EOF
        assert_incremental_matches_full_parse(initial, edit_at, 0, "\n本文");
    }

    #[test]
    fn invariant_deletion_matches_full_parse() {
        let initial = "｜青空《あおぞら》\n本文";
        // Drop the trailing 「\n本文」.
        let edit_at = initial.find("\n本文").unwrap();
        assert_incremental_matches_full_parse(initial, edit_at, "\n本文".len(), "");
    }

    #[test]
    fn invariant_replacement_matches_full_parse() {
        let initial = "｜青空《あおぞら》";
        // Replace the reading; same outer shape but different bytes.
        let reading_start = initial.find("あおぞら").unwrap();
        assert_incremental_matches_full_parse(initial, reading_start, "あおぞら".len(), "そらいろ");
    }

    #[test]
    fn invariant_cross_paragraph_replacement_matches_full_parse() {
        // Edit spans a `\n\n` paragraph boundary, the worst case for
        // an incremental parser that hopes to reuse subtrees.
        let initial = "段落1\n\n｜青空《あおぞら》\n\n段落3";
        let edit_at = initial.find("｜").unwrap();
        let span_end = initial.find("\n\n段落3").unwrap();
        assert_incremental_matches_full_parse(initial, edit_at, span_end - edit_at, "新しい本文");
    }

    // -------------------------------------------------------------
    // Test helpers
    // -------------------------------------------------------------

    fn collect_kinds(tree: &Tree) -> Vec<String> {
        let mut out = Vec::new();
        let mut cursor = tree.walk();
        walk_kinds(&mut cursor, &mut out);
        out
    }

    fn walk_kinds(cursor: &mut tree_sitter::TreeCursor<'_>, out: &mut Vec<String>) {
        out.push(cursor.node().kind().to_owned());
        if cursor.goto_first_child() {
            loop {
                walk_kinds(cursor, out);
                if !cursor.goto_next_sibling() {
                    break;
                }
            }
            let popped = cursor.goto_parent();
            debug_assert!(popped, "every goto_first_child must have a matching parent");
        }
    }

    /// Apply an `(edit_at, removed_len, inserted)` edit to `initial`
    /// in two ways — 1-shot full parse of the final text, and
    /// incremental `parse_full + apply_edit` from `initial` — and
    /// assert the resulting syntax-tree kind sequences are
    /// byte-for-byte identical.
    fn assert_incremental_matches_full_parse(
        initial: &str,
        edit_at: usize,
        removed_len: usize,
        inserted: &str,
    ) {
        assert!(
            initial.is_char_boundary(edit_at),
            "edit_at not at char boundary"
        );
        assert!(
            initial.is_char_boundary(edit_at + removed_len),
            "edit_at + removed_len not at char boundary"
        );
        let mut after = String::with_capacity(initial.len() - removed_len + inserted.len());
        after.push_str(&initial[..edit_at]);
        after.push_str(inserted);
        after.push_str(&initial[edit_at + removed_len..]);

        let full = TreeSitterDoc::new();
        full.parse_full(&after);
        let full_kinds = full.with_tree(collect_kinds).expect("full tree");

        let inc = TreeSitterDoc::new();
        inc.parse_full(initial);
        let edit = input_edit(edit_at, edit_at + removed_len, edit_at + inserted.len());
        inc.apply_edit(&after, edit);
        let inc_kinds = inc.with_tree(collect_kinds).expect("incremental tree");

        assert_eq!(
            inc_kinds, full_kinds,
            "incremental tree diverged from 1-shot parse for edit \
             at={edit_at} removed_len={removed_len} inserted={inserted:?}"
        );
    }
}
