//! Type-state lex pipeline.
//!
//! `Pipeline<'src, 'a, S>` makes the lex phase order enforceable at
//! compile time. The state markers [`Source`], [`Sanitized`],
//! [`Tokenized`], [`Paired`] track which phases have run; methods
//! consume `self` and return the next state. Calling `.pair()` on a
//! `Source` is a type error; calling `.tokenize()` twice is a type
//! error; etc.
//!
//! # Two entry shapes
//!
//! - [`Pipeline::run_to_completion`] — one-shot, equivalent to
//!   [`crate::lex_into_arena`]. Used by `Document::parse` and the
//!   FFI / WASM / Python drivers.
//! - [`Pipeline::new`] → `.sanitize()` → `.tokenize()` → `.pair()` →
//!   `.build()` — explicit chain. Use for inspection / instrumentation:
//!   each intermediate state exposes accessors (`.sanitized_text()`,
//!   `.tokens()`, `.events()`, `.diagnostics()`) so callers can probe
//!   the partial output without re-running the pipeline.
//!
//! # Arena-batch passing
//!
//! Every inter-phase boundary materialises a [`bumpalo::collections::Vec`]
//! inside the pipeline's [`Arena`]. Phase 1 emits `BumpVec<'a, Token>`;
//! Phase 2 emits `BumpVec<'a, PairEvent>`; Phase 3 streams its
//! `ClassifiedSpan`s through the `ArenaNormalizer`
//! callback (no third Vec materialisation — the streaming `classify`
//! Iterator path is the cheapest shape on the corpus).
//!
//! Net effect on the corpus profile: per-parse `malloc`/`free`
//! traffic collapses into a single bump-pointer advance per element.
//!
//! # State carries its own payload
//!
//! Each state marker is a field-bound struct holding exactly the
//! phase outputs it has produced (`Sanitized` carries the arena
//! `&'a str`; `Tokenized` adds the token `BumpVec`; …). Reading
//! `.sanitized_text()` from `Pipeline<'_, '_, Sanitized>` is a
//! field projection on the state struct — no `Option::expect`
//! lives in production code. The compiler enforces "you cannot
//! ask for tokens unless you are in `Tokenized`" via method
//! resolution alone.
//!
//! # Lifetime model
//!
//! `'src` is the original source text lifetime; `'a` is the arena
//! lifetime. The sanitized text is materialised into the arena at the
//! `Sanitized` transition (cost: one `arena.alloc_str` of
//! `sanitize(source).text`), so all downstream phases borrow from the
//! arena rather than from in-Pipeline storage. This eliminates the
//! self-referential-struct problem `Tokenizer<'sanitized>` would
//! otherwise impose.
//!
//! # Compile-time phase-order enforcement
//!
//! Calling `.pair()` on a fresh [`Source`] (without going through
//! `.sanitize().tokenize()`) is a *type error*: there is no
//! `impl Pipeline<'_, '_, Source>::pair` method. The compile-fail
//! doctest below pins this contract — adding such an impl in the
//! future would silently break the type-state guarantee:
//!
//! ```compile_fail
//! use aozora_lex::Pipeline;
//! use aozora_syntax::borrowed::Arena;
//!
//! let arena = Arena::new();
//! // .pair() on Source skips Phase 0 + Phase 1 — must not compile.
//! let _ = Pipeline::new("plain", &arena).pair();
//! ```
//!
//! # Why `build` is the terminal transition
//!
//! Phase 3 (classify) requires `&mut BorrowedAllocator<'a>`. The
//! allocator owns the `Interner<'a>` whose internal `RefCell` makes
//! it `!Sync`; threading `&mut alloc` through Pipeline states would
//! force the allocator to live as long as the pipeline, blocking any
//! external pause-and-inspect between Phase 2 and Phase 3. We
//! collapse Phase 3 + the `ArenaNormalizer` fold
//! into a single terminal `.build()` call instead — inspection up
//! through `Paired` works freely; the final allocation pass is
//! atomic.

use core::marker::PhantomData;

use crate::lexer::{PairEvent, Token, classify, pair_in, sanitize, tokenize_in};
use aozora_spec::{Diagnostic, PairLink};
use aozora_syntax::ContainerKind;
use aozora_syntax::alloc::BorrowedAllocator;
use aozora_syntax::borrowed::{Arena, ContainerPair, Registry};
use bumpalo::collections::Vec as BumpVec;

use crate::BorrowedLexOutput;
use crate::borrowed::{ArenaNormalizer, SourceNode};

// =====================================================================
// State markers (field-bound — each state carries the phase output it
// is responsible for. No `Option` / `expect` chain in production code:
// the type system guarantees the field is present whenever the state
// type can be named).
// =====================================================================

/// Initial state — no phase has run yet.
#[derive(Debug, Clone, Copy)]
pub struct Source;

/// Phase 0 has run; sanitized text is materialised in the arena.
#[derive(Debug, Clone, Copy)]
pub struct Sanitized<'a> {
    sanitized_text: &'a str,
}

/// Phase 1 has run; the token list is materialised inside the arena.
#[derive(Debug)]
pub struct Tokenized<'a> {
    sanitized_text: &'a str,
    tokens: BumpVec<'a, Token>,
}

/// Phase 2 has run; the event list and the resolved (open, close)
/// link side-table are materialised inside the arena.
#[derive(Debug)]
pub struct Paired<'a> {
    sanitized_text: &'a str,
    events: BumpVec<'a, PairEvent>,
    links: BumpVec<'a, PairLink>,
}

// =====================================================================
// Pipeline
// =====================================================================

/// Type-state lex pipeline. Each state's transition method consumes
/// `self`, materialises its phase output into the next state struct,
/// and returns a new pipeline in the next state.
#[derive(Debug)]
pub struct Pipeline<'src, 'a, S> {
    source: &'src str,
    arena: &'a Arena,
    diagnostics: Vec<Diagnostic>,
    state: S,
    // Tie the unused `'a` lifetime to the struct so the compiler
    // accepts state structs that reference the arena even when the
    // current state marker (`Source`) doesn't. Zero size at runtime.
    _arena: PhantomData<&'a Arena>,
}

// ---------------------------------------------------------------------
// Source
// ---------------------------------------------------------------------

impl<'src, 'a> Pipeline<'src, 'a, Source> {
    /// Wrap a source string for type-state-driven lex. Phase 0 has not
    /// yet run; only `source` and `arena` are set.
    #[must_use]
    pub fn new(source: &'src str, arena: &'a Arena) -> Self {
        Self {
            source,
            arena,
            diagnostics: Vec::new(),
            state: Source,
            _arena: PhantomData,
        }
    }

    /// One-shot driver: run every phase and return the final
    /// [`BorrowedLexOutput`]. Equivalent to [`crate::lex_into_arena`].
    #[must_use]
    pub fn run_to_completion(source: &'src str, arena: &'a Arena) -> BorrowedLexOutput<'a> {
        Self::new(source, arena)
            .sanitize()
            .tokenize()
            .pair()
            .build()
    }

    /// Borrow the original source text.
    #[must_use]
    pub fn source(&self) -> &'src str {
        self.source
    }

    /// Run Phase 0 (sanitize). Materialises the sanitized text in the
    /// arena so downstream phases borrow from the arena, not from the
    /// Pipeline struct (which would be self-referential).
    #[must_use]
    pub fn sanitize(mut self) -> Pipeline<'src, 'a, Sanitized<'a>> {
        let out = sanitize(self.source);
        self.diagnostics.extend(out.diagnostics);
        let arena_text: &'a str = self.arena.alloc_str(&out.text);
        Pipeline {
            source: self.source,
            arena: self.arena,
            diagnostics: self.diagnostics,
            state: Sanitized {
                sanitized_text: arena_text,
            },
            _arena: PhantomData,
        }
    }
}

// ---------------------------------------------------------------------
// Sanitized
// ---------------------------------------------------------------------

impl<'src, 'a> Pipeline<'src, 'a, Sanitized<'a>> {
    /// Sanitized text (arena-allocated).
    #[must_use]
    pub fn sanitized_text(&self) -> &'a str {
        self.state.sanitized_text
    }

    /// Diagnostics accumulated through Phase 0.
    #[must_use]
    pub fn diagnostics(&self) -> &[Diagnostic] {
        &self.diagnostics
    }

    /// Run Phase 1 (tokenize). Materialises the full
    /// `BumpVec<'a, Token>` inside `arena` via [`tokenize_in`].
    #[must_use]
    pub fn tokenize(self) -> Pipeline<'src, 'a, Tokenized<'a>> {
        let sanitized_text = self.state.sanitized_text;
        let tokens = tokenize_in(sanitized_text, self.arena);
        Pipeline {
            source: self.source,
            arena: self.arena,
            diagnostics: self.diagnostics,
            state: Tokenized {
                sanitized_text,
                tokens,
            },
            _arena: PhantomData,
        }
    }
}

// ---------------------------------------------------------------------
// Tokenized
// ---------------------------------------------------------------------

impl<'src, 'a> Pipeline<'src, 'a, Tokenized<'a>> {
    /// Borrow the materialised token list. Useful for instrumentation.
    #[must_use]
    pub fn tokens(&self) -> &[Token] {
        &self.state.tokens
    }

    /// Run Phase 2 (pair). Materialises a paired-event stream
    /// inside `arena` via [`pair_in`]. Phase 2's
    /// diagnostics are drained into the pipeline's diagnostic
    /// accumulator immediately.
    #[must_use]
    pub fn pair(mut self) -> Pipeline<'src, 'a, Paired<'a>> {
        let Tokenized {
            sanitized_text,
            tokens,
        } = self.state;
        let out = pair_in(&tokens, self.arena);
        self.diagnostics.extend(out.diagnostics);
        Pipeline {
            source: self.source,
            arena: self.arena,
            diagnostics: self.diagnostics,
            state: Paired {
                sanitized_text,
                events: out.events,
                links: out.links,
            },
            _arena: PhantomData,
        }
    }
}

// ---------------------------------------------------------------------
// Paired (terminal)
// ---------------------------------------------------------------------

impl<'a> Pipeline<'_, 'a, Paired<'a>> {
    /// Borrow the materialised pair-event list. Useful for inspection
    /// before `.build()`.
    #[must_use]
    pub fn events(&self) -> &[PairEvent] {
        &self.state.events
    }

    /// Borrow the resolved (open, close) pair side-table. Useful for
    /// inspection before `.build()`.
    #[must_use]
    pub fn links(&self) -> &[PairLink] {
        &self.state.links
    }

    /// Drive Phase 3 + the arena normalizer fold and return the final
    /// [`BorrowedLexOutput`]. Terminal transition because
    /// `&mut BorrowedAllocator` cannot be safely held across an external
    /// pause without locking the pipeline into a single thread for the
    /// allocator's lifetime.
    ///
    /// # Diagnostic order
    ///
    /// Sanitize (Phase 0) → Pair (Phase 2 unclosed/unmatched) →
    /// Classify (Phase 3 unknown annotations etc.). Matches the
    /// pre-Pipeline `lex_into_arena` ordering.
    ///
    /// # Panics
    ///
    /// Panics if the sanitized source exceeds `u32::MAX` bytes
    /// (the lexer's `Span` width contract). In practice unreachable;
    /// Phase 0 caps source length at the same boundary.
    #[must_use]
    pub fn build(mut self) -> BorrowedLexOutput<'a> {
        let Paired {
            sanitized_text,
            events,
            links,
        } = self.state;
        let sanitized_len =
            u32::try_from(sanitized_text.len()).expect("sanitize asserts source.len() <= u32::MAX");

        // Allocator capacity hint: source.len()/32 is a rough upper bound
        // on the number of distinct strings the borrowed pipeline will
        // intern. `BorrowedAllocator::with_capacity` rounds up to the
        // next power of two; floor of 64 covers short documents.
        let interner_hint = (sanitized_text.len() / 32).max(64);
        let mut alloc = BorrowedAllocator::with_capacity(self.arena, interner_hint);
        let mut builder = ArenaNormalizer::new(sanitized_text, sanitized_text.len() / 64);

        // Drain the arena-allocated `BumpVec<PairEvent>` through the
        // streaming `classify` Iterator path.
        let mut events_iter = events.into_iter();
        let classify_diagnostics: Vec<Diagnostic> = {
            let mut classify_stream = classify(&mut events_iter, sanitized_text, &mut alloc);
            for span in &mut classify_stream {
                builder.emit(&span);
            }
            classify_stream.take_diagnostics()
        };
        self.diagnostics.extend(classify_diagnostics);

        let normalized: &'a str = self.arena.alloc_str(&builder.out);
        // Single-table Registry: classifier emits in source order so
        // `entries` is already sorted by position; from_sorted_slice
        // skips the redundant sort pass.
        let registry = Registry::from_sorted_slice(&builder.entries);
        // Freeze the arena `BumpVec<PairLink>` into a `&'a [PairLink]`.
        // `BumpVec::into_bump_slice` consumes self and returns a slice
        // alive for the bump allocator's lifetime, exactly the lifetime
        // we need on `BorrowedLexOutput::pairs`.
        let pairs: &'a [PairLink] = links.into_bump_slice();
        // Move the source-keyed side table out of the heap-backed
        // `Vec<SourceNode>` and into the arena, in one allocation.
        let source_nodes: &'a [SourceNode<'a>] = self.arena.alloc_slice_copy(&builder.source_nodes);
        // Same dance for the container-pair side table — close-order
        // (matches the close events as the open-stack drains).
        let container_pairs: &'a [ContainerPair] =
            self.arena.alloc_slice_copy(&builder.container_pairs);
        let intern_stats = alloc.into_interner().stats;

        BorrowedLexOutput {
            normalized,
            registry,
            diagnostics: self.diagnostics,
            sanitized_len,
            pairs,
            source_nodes,
            container_pairs,
            intern_stats,
        }
    }
}

// Suppress an unused-import warning when the only use of `ContainerKind`
// is through the `Registry` field types — the import is still needed for
// the trait-impl resolution but the analyser doesn't see it.
const _CONTAINER_KIND_USE_MARKER: usize = size_of::<ContainerKind>();

#[cfg(test)]
mod tests {
    use core::ptr;

    use aozora_syntax::borrowed::Arena;

    use super::*;

    #[test]
    fn type_state_chain_compiles() {
        let arena = Arena::new();
        let _final = Pipeline::new("｜青梅《おうめ》", &arena)
            .sanitize()
            .tokenize()
            .pair()
            .build();
    }

    #[test]
    fn run_to_completion_matches_chain() {
        let arena1 = Arena::new();
        let arena2 = Arena::new();
        let chain = Pipeline::new("｜青梅《おうめ》", &arena1)
            .sanitize()
            .tokenize()
            .pair()
            .build();
        let oneshot = Pipeline::run_to_completion("｜青梅《おうめ》", &arena2);
        assert_eq!(chain.normalized, oneshot.normalized);
        assert_eq!(chain.sanitized_len, oneshot.sanitized_len);
        assert_eq!(
            chain.registry.count_kind(aozora_spec::Sentinel::Inline),
            oneshot.registry.count_kind(aozora_spec::Sentinel::Inline)
        );
    }

    #[test]
    fn intermediate_inspection_at_sanitized() {
        let arena = Arena::new();
        let p = Pipeline::new("plain text", &arena).sanitize();
        assert_eq!(p.sanitized_text(), "plain text");
        assert!(p.diagnostics().is_empty());
        drop(p.tokenize().pair().build());
    }

    #[test]
    fn intermediate_inspection_at_tokenized() {
        let arena = Arena::new();
        let p = Pipeline::new("a｜b《c》", &arena).sanitize().tokenize();
        // Token sanity: at least Text+Trigger+Text+Trigger+Text+Trigger.
        assert!(p.tokens().len() >= 5);
        drop(p.pair().build());
    }

    #[test]
    fn intermediate_inspection_at_paired() {
        let arena = Arena::new();
        let p = Pipeline::new("a｜b《c》", &arena)
            .sanitize()
            .tokenize()
            .pair();
        assert!(!p.events().is_empty());
        drop(p.build());
    }

    #[test]
    fn sanitize_pua_collision_diagnostic_propagates() {
        let arena = Arena::new();
        let out = Pipeline::run_to_completion("abc\u{E001}def", &arena);
        assert!(
            out.diagnostics
                .iter()
                .any(|d| matches!(d, Diagnostic::SourceContainsPua { .. })),
            "expected SourceContainsPua, got {:?}",
            out.diagnostics
        );
    }

    #[test]
    fn empty_source_round_trips() {
        let arena = Arena::new();
        let out = Pipeline::run_to_completion("", &arena);
        assert!(out.normalized.is_empty());
        assert!(out.registry.is_empty());
        assert_eq!(out.sanitized_len, 0);
    }

    #[test]
    fn source_accessor_returns_original() {
        let arena = Arena::new();
        let s = "the original";
        let p = Pipeline::new(s, &arena);
        assert!(ptr::eq(p.source(), s));
    }
}
