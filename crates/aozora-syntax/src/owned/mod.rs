//! Owned, no-lifetime mirror of the borrowed semantic AST.
//!
//! The sole AST consumers walk is the arena-backed `borrowed`
//! tree: every payload is `Copy` and borrows `&'src str` from the arena, so the
//! whole tree is tied to one lifetime and is **not** `Send + Sync` across an
//! arena drop. The #237 segment cache and an out-of-process LSP consumer need a
//! representation they can own, cache, and move between threads.
//!
//! This module is that representation. It mirrors the borrowed tree
//! variant-for-variant with three substitutions:
//!
//! - interned `&'src str` → [`StrId`] into a [`StrInterner`];
//! - `NonEmpty<Content>` → [`ContentRange`] into [`NodeStore`]'s content pool;
//! - `&'src [Segment]` → [`SegRange`] into [`NodeStore`]'s segment pool.
//!
//! Lifetime-free `Copy` payloads (`LineFormat`, `RegionFormat`, `Container`,
//! the scalar enums, `Span`, `Diagnostic`, …) are reused as-is, not duplicated.
//! [`OwnedLexOutput`] is the owned analogue of the pipeline's `LexOutput`, with
//! an added [`NodeStore`] that owns what the arena formerly held.
//!
//! # Status
//!
//! This is the **sole** AST representation: the lex pipeline's classify stage
//! builds it directly via [`OwnedAllocator`](crate::alloc_owned::OwnedAllocator)
//! and the fold records it into an [`OwnedLexOutput`] that every consumer reads.
//! The former arena-backed borrowed AST has been removed.

mod intern;
mod output;
mod payload;
mod registry;
mod store;

pub use intern::{InternStats, StrId, StrInterner};
pub use output::{OwnedLexOutput, SourceNodeOwned};
pub use payload::{
    AngleQuoteOwned, ContentOwned, DirectiveOwned, ForwardFormatOwned, GaijiCanonicalOwned,
    GaijiOwned, HeadingHintOwned, HeadingOwned, IllustrationOwned, KaeritenOwned, MarginNoteOwned,
    NodeOwned, RubyOwned, SegmentOwned, WarichuOwned,
};
pub use registry::{ContainerPair, NodeRefOwned, RegistryOwned};
pub use store::{ContentRange, NodeStore, SegRange};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::RubySide;

    // UNIT TEST 1: the Send + Sync property holds for the owned output.
    #[test]
    fn owned_lex_output_is_send_and_sync() {
        const fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<OwnedLexOutput>();
        assert_send_sync::<NodeStore>();
        assert_send_sync::<RegistryOwned>();
    }

    // UNIT TEST 3: build a tiny OwnedLexOutput by hand for a ruby node
    // (base "日本" reading "にほん") via the store API, then resolve the
    // StrIds / ranges back and assert the structure.
    #[test]
    fn hand_built_ruby_output_resolves_back() {
        let mut store = NodeStore::new();

        // Intern the base + reading and lay them down as single-element
        // content runs (a plain ruby base / reading is one `Plain` content).
        let base_id = store.intern("日本");
        let reading_id = store.intern("にほん");
        let base = store.push_contents(&[ContentOwned::Plain(base_id)]);
        let reading = store.push_contents(&[ContentOwned::Plain(reading_id)]);

        let ruby = RubyOwned {
            base,
            reading,
            side: RubySide::Right,
        };
        let node = NodeOwned::Ruby(ruby);

        // One inline registry entry at normalized position 0.
        let registry = RegistryOwned::from_sorted_slice(&[(0u32, NodeRefOwned::Inline(node))]);
        let source_nodes = vec![SourceNodeOwned {
            source_span: aozora_spec::Span::new(0, 12),
            node: NodeRefOwned::Inline(node),
        }];

        let out = OwnedLexOutput {
            normalized: String::from("\u{E001}"),
            sanitized: String::from("日本"),
            registry,
            diagnostics: Vec::new(),
            sanitized_len: 6,
            pairs: Vec::new(),
            source_nodes,
            container_pairs: Vec::new(),
            intern_stats: store.interner.stats,
            store,
        };

        // Recover the node from the registry and resolve its payloads.
        let hit = out
            .registry
            .node_at(aozora_spec::NormalizedOffset::new(0))
            .expect("registered inline node at position 0");
        let NodeRefOwned::Inline(NodeOwned::Ruby(got)) = hit else {
            panic!("expected an inline ruby node, got {hit:?}");
        };

        // Resolve the base run → single Plain → "日本".
        let base_run = out.store.resolve_content_range(got.base);
        assert_eq!(base_run.len(), 1, "ruby base is one content entry");
        let ContentOwned::Plain(got_base_id) = base_run[0] else {
            panic!("expected a Plain base content, got {:?}", base_run[0]);
        };
        assert_eq!(out.store.resolve_str(got_base_id), "日本", "base text");

        // Resolve the reading run → single Plain → "にほん".
        let reading_run = out.store.resolve_content_range(got.reading);
        assert_eq!(reading_run.len(), 1, "ruby reading is one content entry");
        let ContentOwned::Plain(got_reading_id) = reading_run[0] else {
            panic!("expected a Plain reading content, got {:?}", reading_run[0]);
        };
        assert_eq!(
            out.store.resolve_str(got_reading_id),
            "にほん",
            "reading text"
        );

        assert_eq!(got.side, RubySide::Right, "ruby side preserved");
    }
}
