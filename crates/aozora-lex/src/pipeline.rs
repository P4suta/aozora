//! Streaming type-state pipeline (Innovation I-3, post-deforestation).
//!
//! `Pipeline<'src, 'a, S, I>` makes the lex pipeline's phase order
//! enforceable at compile time. The state markers
//! [`Source`], [`Sanitized`], [`Tokenized`], [`Paired`] track which
//! phases have run; methods consume `self` and return the next
//! state. Calling `.pair()` on a `Source` is a type error; calling
//! `.tokenize()` twice is a type error; etc.
//!
//! # Two entry shapes
//!
//! - [`Pipeline::run_to_completion`] — one-shot, equivalent to
//!   [`crate::lex_into_arena`]. Used by `Document::parse` and the
//!   FFI / WASM / Python drivers.
//! - [`Pipeline::new`] → `.sanitize()` → `.tokenize()` → `.pair()` →
//!   `.build()` — explicit chain. Use for inspection / instrumentation:
//!   each intermediate state exposes accessors (`.sanitized_text()`,
//!   `.diagnostics()`) so callers can probe the partial output without
//!   having to re-run the pipeline.
//!
//! # Lifetime model
//!
//! `'src` is the original source text lifetime; `'a` is the arena
//! lifetime. The sanitized text is materialised into the arena at the
//! `Sanitized` transition (cost: one `arena.alloc_str` of
//! `sanitize(source).text`), so all downstream phases borrow from the
//! arena rather than from in-Pipeline storage. This eliminates the
//! self-referential-struct problem that `Tokenizer<'sanitized>` would
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
//! collapse Phase 3 + the [`crate::borrowed::ArenaNormalizer`] fold
//! into a single terminal `.build()` call instead — inspection up
//! through `Paired` works freely; the final allocation pass is
//! atomic.

use core::marker::PhantomData;

use aozora_lexer::{PairStream, Token, Tokenizer, classify, pair, sanitize, tokenize};
use aozora_spec::Diagnostic;
use aozora_syntax::ContainerKind;
use aozora_syntax::alloc::BorrowedAllocator;
use aozora_syntax::borrowed::{Arena, Registry};
use aozora_veb::EytzingerMap;

use crate::BorrowedLexOutput;
use crate::borrowed::ArenaNormalizer;

// =====================================================================
// State markers
// =====================================================================

/// Initial state — only `source` and `arena` are set.
#[derive(Debug)]
pub struct Source;

/// Phase 0 has run; sanitized text is materialised in the arena.
#[derive(Debug)]
pub struct Sanitized;

/// Phase 1 has been wired; the iterator produces [`Token`]s.
#[derive(Debug)]
pub struct Tokenized;

/// Phase 2 has been wired; the iterator produces [`PairEvent`]s.
#[derive(Debug)]
pub struct Paired;

// =====================================================================
// Pipeline
// =====================================================================

/// Streaming type-state lex pipeline.
///
/// `S` is one of [`Source`], [`Sanitized`], [`Tokenized`], [`Paired`];
/// `I` is the upstream iterator type carried for the post-Sanitize
/// states (defaults to `()` for states that haven't wired an iterator
/// yet). All public construction goes through [`Pipeline::new`] and
/// the per-state transition methods, never via the struct literal.
#[allow(missing_debug_implementations, reason = "I parameter is opaque iterator")]
pub struct Pipeline<'src, 'a, S, I = ()> {
    source: &'src str,
    arena: &'a Arena,
    /// `Some` after Phase 0 has run; arena-allocated. Borrowed by every
    /// downstream phase so the iterators don't refer back into the
    /// Pipeline struct itself.
    sanitized_text: Option<&'a str>,
    diagnostics: Vec<Diagnostic>,
    iter: I,
    _state: PhantomData<S>,
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
            sanitized_text: None,
            diagnostics: Vec::new(),
            iter: (),
            _state: PhantomData,
        }
    }

    /// One-shot driver: run every phase and return the final
    /// [`BorrowedLexOutput`]. Equivalent to [`crate::lex_into_arena`].
    #[must_use]
    pub fn run_to_completion(source: &'src str, arena: &'a Arena) -> BorrowedLexOutput<'a> {
        Self::new(source, arena).sanitize().tokenize().pair().build()
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
    pub fn sanitize(mut self) -> Pipeline<'src, 'a, Sanitized> {
        let out = sanitize(self.source);
        self.diagnostics.extend(out.diagnostics);
        let arena_text: &'a str = self.arena.alloc_str(&out.text);
        Pipeline {
            source: self.source,
            arena: self.arena,
            sanitized_text: Some(arena_text),
            diagnostics: self.diagnostics,
            iter: (),
            _state: PhantomData,
        }
    }
}

// ---------------------------------------------------------------------
// Sanitized
// ---------------------------------------------------------------------

impl<'src, 'a> Pipeline<'src, 'a, Sanitized> {
    /// Sanitized text (arena-allocated).
    #[must_use]
    pub fn sanitized_text(&self) -> &'a str {
        self.sanitized_text
            .expect("sanitized_text is always Some after Sanitized transition")
    }

    /// Diagnostics accumulated through Phase 0.
    #[must_use]
    pub fn diagnostics(&self) -> &[Diagnostic] {
        &self.diagnostics
    }

    /// Run Phase 1 (tokenize). Wraps the sanitized text in a
    /// [`Tokenizer`] iterator without materialising the token stream.
    #[must_use]
    pub fn tokenize(self) -> Pipeline<'src, 'a, Tokenized, Tokenizer<'a>> {
        let text = self.sanitized_text();
        Pipeline {
            source: self.source,
            arena: self.arena,
            sanitized_text: self.sanitized_text,
            diagnostics: self.diagnostics,
            iter: tokenize(text),
            _state: PhantomData,
        }
    }
}

// ---------------------------------------------------------------------
// Tokenized
// ---------------------------------------------------------------------

impl<'src, 'a> Pipeline<'src, 'a, Tokenized, Tokenizer<'a>> {
    /// Run Phase 2 (pair). Wraps the token iterator in a [`PairStream`].
    #[must_use]
    pub fn pair(self) -> Pipeline<'src, 'a, Paired, PairStream<Tokenizer<'a>>> {
        Pipeline {
            source: self.source,
            arena: self.arena,
            sanitized_text: self.sanitized_text,
            diagnostics: self.diagnostics,
            iter: pair(self.iter),
            _state: PhantomData,
        }
    }
}

// ---------------------------------------------------------------------
// Paired (terminal)
// ---------------------------------------------------------------------

impl<'src, 'a, T: Iterator<Item = Token>> Pipeline<'src, 'a, Paired, PairStream<T>> {
    /// Drive Phase 3 + the arena normalizer fold and return the final
    /// [`BorrowedLexOutput`]. This is the terminal transition because
    /// `&mut BorrowedAllocator` cannot be safely held across an external
    /// pause without locking the pipeline into a single thread for the
    /// allocator's lifetime.
    ///
    /// # Diagnostic order
    ///
    /// Sanitize (Phase 0) → Pair (Phase 2 unclosed/unmatched) →
    /// Classify (Phase 3 unknown annotations etc.). This matches the
    /// pre-Pipeline `lex_into_arena` ordering.
    #[must_use]
    pub fn build(mut self) -> BorrowedLexOutput<'a> {
        let sanitized_text = self
            .sanitized_text
            .expect("sanitized_text is always Some after Sanitized transition");
        let sanitized_len = u32::try_from(sanitized_text.len())
            .expect("sanitize asserts source.len() <= u32::MAX");

        // Allocator capacity hint: source.len()/32 is a rough upper bound
        // on the number of distinct strings the borrowed pipeline will
        // intern. `BorrowedAllocator::with_capacity` rounds up to the
        // next power of two; floor of 64 covers short documents.
        let interner_hint = (sanitized_text.len() / 32).max(64);
        let mut alloc = BorrowedAllocator::with_capacity(self.arena, interner_hint);
        let mut builder = ArenaNormalizer::new(sanitized_text, sanitized_text.len() / 64);

        // Inner block scope so the `&mut self.iter` borrow released
        // before the post-loop `self.iter.take_diagnostics()`.
        let classify_diagnostics: Vec<Diagnostic> = {
            let mut classify_stream = classify(&mut self.iter, sanitized_text, &mut alloc);
            for span in &mut classify_stream {
                builder.emit(&span);
            }
            classify_stream.take_diagnostics()
        };
        // Pair-stream diagnostics are complete only after the classify
        // pass has fully consumed the pair stream.
        self.diagnostics.extend(self.iter.take_diagnostics());
        self.diagnostics.extend(classify_diagnostics);

        let normalized: &'a str = self.arena.alloc_str(&builder.out);
        let registry = Registry {
            inline: EytzingerMap::from_sorted_slice(&builder.inline),
            block_leaf: EytzingerMap::from_sorted_slice(&builder.block_leaf),
            block_open: EytzingerMap::from_sorted_slice(&builder.block_open),
            block_close: EytzingerMap::from_sorted_slice(&builder.block_close),
        };
        let intern_stats = alloc.into_interner().stats;

        BorrowedLexOutput {
            normalized,
            registry,
            diagnostics: self.diagnostics,
            sanitized_len,
            intern_stats,
        }
    }
}

// Suppress an unused-import warning when the only use of `ContainerKind`
// is through the `Registry` field types — the import is still needed for
// the trait-impl resolution but the analyser doesn't see it.
const _: fn() = || {
    let _ = core::mem::size_of::<ContainerKind>();
};

#[cfg(test)]
mod tests {
    use super::*;
    use aozora_syntax::borrowed::Arena;

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
        assert_eq!(chain.registry.inline.len(), oneshot.registry.inline.len());
    }

    #[test]
    fn intermediate_inspection_at_sanitized() {
        let arena = Arena::new();
        let p = Pipeline::new("plain text", &arena).sanitize();
        assert_eq!(p.sanitized_text(), "plain text");
        assert!(p.diagnostics().is_empty());
        let _ = p.tokenize().pair().build();
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
        assert!(core::ptr::eq(p.source(), s));
    }
}
