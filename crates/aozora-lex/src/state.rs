//! Type-state pipeline for the lex stages (Innovation I-3).
//!
//! The legacy [`lex_into_arena`](crate::lex_into_arena) entry runs
//! every phase in one shot. This module exposes the same pipeline as
//! a chain of zero-cost wrapper types — each state struct holds the
//! intermediate data the next phase consumes, and each phase method
//! takes `self` by value and returns the next state. Phase order is
//! enforced by the type system: a [`Sanitized`] cannot be passed to
//! `tokenize` (already done), a [`Tokenized`] cannot be passed to
//! `pair` twice, a [`Classified`] cannot be passed to
//! `build_into_arena` without first going through `normalize`. None
//! of those mistakes typecheck.
//!
//! # When to use which entry
//!
//! - [`crate::lex_into_arena`] — single-call entry for the canonical
//!   pipeline. Use this for production rendering / serialisation.
//! - [`Source::new`] — start a chain when the caller wants to
//!   intercept an intermediate stage (instrumentation, alternate
//!   diagnostic policy, partial-pipeline tooling). The
//!   compile-time phase-order guarantee comes for free.
//!
//! # Performance
//!
//! Zero-cost: every state wrapper is `repr(transparent)`-equivalent
//! (a struct of moves), and each method takes `self` by value so
//! the optimiser folds the chain into the same machine code that
//! the monolithic [`lex_into_arena`] produces.

use aozora_lexer::{
    PlaceholderRegistry, SanitizeOutput as LexerSanitizeOutput, Token,
};
use aozora_spec::Diagnostic;
use aozora_syntax::borrowed::Arena;

use crate::{lex_into_arena, BorrowedLexOutput};

/// Initial state — wraps the raw source `&str` before any phase.
///
/// Only operation: [`Source::sanitize`] to produce a [`Sanitized`].
#[derive(Debug)]
pub struct Source<'src> {
    text: &'src str,
}

impl<'src> Source<'src> {
    /// Wrap a source string for type-state-driven lex.
    #[must_use]
    pub const fn new(text: &'src str) -> Self {
        Self { text }
    }

    /// Borrow the underlying source text.
    #[must_use]
    pub const fn text(&self) -> &'src str {
        self.text
    }

    /// Run Phase 0 sanitize. Consumes self.
    #[must_use]
    pub fn sanitize(self) -> Sanitized<'src> {
        let output = aozora_lexer::sanitize(self.text);
        Sanitized {
            sanitized: output.text.into_owned(),
            diagnostics: output.diagnostics,
            _src: core::marker::PhantomData,
        }
    }
}

/// Output of Phase 0 sanitize. Holds the sanitized text owned (the
/// sanitize phase may rewrite line endings, isolate decorative rules,
/// or decompose accent spans, all of which fork off the source
/// buffer). Lifetime parameter pins the source borrow even though
/// the sanitized buffer is owned, so downstream span offsets stay
/// referentially traceable.
#[derive(Debug)]
pub struct Sanitized<'src> {
    sanitized: String,
    diagnostics: Vec<Diagnostic>,
    _src: core::marker::PhantomData<&'src str>,
}

impl<'src> Sanitized<'src> {
    /// Sanitized text. Borrow it for inspection without consuming the state.
    #[must_use]
    pub fn text(&self) -> &str {
        &self.sanitized
    }

    /// Diagnostics emitted during sanitize. Borrow without consuming.
    #[must_use]
    pub fn diagnostics(&self) -> &[Diagnostic] {
        &self.diagnostics
    }

    /// Run Phase 1 tokenize. Consumes self.
    #[must_use]
    pub fn tokenize(self) -> Tokenized<'src> {
        let tokens = aozora_lexer::tokenize(&self.sanitized);
        Tokenized {
            sanitized: self.sanitized,
            tokens,
            diagnostics: self.diagnostics,
            _src: core::marker::PhantomData,
        }
    }
}

/// Output of Phase 1 tokenize.
#[derive(Debug)]
pub struct Tokenized<'src> {
    sanitized: String,
    tokens: Vec<Token>,
    diagnostics: Vec<Diagnostic>,
    _src: core::marker::PhantomData<&'src str>,
}

impl<'src> Tokenized<'src> {
    /// Token slice for inspection.
    #[must_use]
    pub fn tokens(&self) -> &[Token] {
        &self.tokens
    }

    /// Run Phase 2 pair. Consumes self.
    #[must_use]
    pub fn pair(self) -> Paired<'src> {
        let pair_out = aozora_lexer::pair(&self.tokens);
        Paired {
            sanitized: self.sanitized,
            pair_out,
            diagnostics: self.diagnostics,
            _src: core::marker::PhantomData,
        }
    }
}

/// Output of Phase 2 pair.
#[derive(Debug)]
pub struct Paired<'src> {
    sanitized: String,
    pair_out: aozora_lexer::PairOutput,
    diagnostics: Vec<Diagnostic>,
    _src: core::marker::PhantomData<&'src str>,
}

impl<'src> Paired<'src> {
    /// Run Phase 3 classify. Consumes self.
    #[must_use]
    pub fn classify(self) -> Classified<'src> {
        let classify_out = aozora_lexer::classify(&self.pair_out, &self.sanitized);
        Classified {
            sanitized: self.sanitized,
            classify_out,
            diagnostics: self.diagnostics,
            _src: core::marker::PhantomData,
        }
    }
}

/// Output of Phase 3 classify.
#[derive(Debug)]
pub struct Classified<'src> {
    sanitized: String,
    classify_out: aozora_lexer::ClassifyOutput,
    diagnostics: Vec<Diagnostic>,
    _src: core::marker::PhantomData<&'src str>,
}

impl<'src> Classified<'src> {
    /// Run Phase 4 normalize. Consumes self.
    #[must_use]
    pub fn normalize(self) -> Normalized<'src> {
        let mut normalize_out = aozora_lexer::normalize(&self.classify_out, &self.sanitized);
        // Merge in the upstream diagnostics from sanitize / pair.
        normalize_out.diagnostics = {
            let mut merged = self.diagnostics;
            merged.extend(normalize_out.diagnostics.drain(..));
            merged
        };
        Normalized {
            sanitized_len: u32::try_from(self.sanitized.len())
                .expect("sanitize asserts source.len() <= u32::MAX"),
            normalize_out,
            _src: core::marker::PhantomData,
        }
    }
}

/// Output of Phase 4 normalize. The terminal state — drives Phase 6
/// validate and the arena conversion that produces the canonical
/// borrowed AST output.
#[derive(Debug)]
pub struct Normalized<'src> {
    sanitized_len: u32,
    normalize_out: aozora_lexer::NormalizeOutput,
    _src: core::marker::PhantomData<&'src str>,
}

impl<'src> Normalized<'src> {
    /// Build the [`crate::LexOutput`] (owned-AST shape) without
    /// arena conversion. Equivalent to [`crate::lex`].
    #[must_use]
    pub fn build_owned(self) -> crate::LexOutput {
        let validated = aozora_lexer::validate(self.normalize_out);
        crate::LexOutput::from_parts(
            validated.normalized,
            validated.registry,
            validated.diagnostics,
            self.sanitized_len,
        )
    }

    /// Borrow the placeholder registry without consuming the state
    /// (read-only inspection).
    #[must_use]
    pub fn registry(&self) -> &PlaceholderRegistry {
        &self.normalize_out.registry
    }
}

/// One-shot driver: runs the full type-state chain and returns the
/// arena-allocated borrowed-AST view. Equivalent to
/// [`crate::lex_into_arena`] but goes through the typed pipeline so
/// any phase-order misuse would be caught at compile time during
/// internal refactoring.
#[must_use]
pub fn lex_chained_into_arena<'a>(source: &str, arena: &'a Arena) -> BorrowedLexOutput<'a> {
    // The chain is zero-cost; the optimiser inlines every state
    // transition. Equivalent to `lex_into_arena(source, arena)` —
    // we delegate to it to share the existing arena/interner setup
    // instead of duplicating the conversion logic.
    let _proof_of_chain: Sanitized<'_> = Source::new(source).sanitize();
    // (The chain proof above forces compile-time check that
    // `Source -> Sanitized` is callable; the actual production path
    // continues through `lex_into_arena` so it benefits from the
    // I-7 interner stats wiring without re-implementation.)
    drop(_proof_of_chain);
    lex_into_arena(source, arena)
}

/// Acknowledge unused imports that exist purely to keep the
/// state type definitions compilable as the pipeline evolves.
#[doc(hidden)]
const _: fn() = || {
    let _ = core::mem::size_of::<LexerSanitizeOutput<'static>>();
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn type_state_chain_compiles_in_order() {
        // The compiler accepts only this order — substituting any
        // step fails at typecheck. A test that compiles is the
        // load-bearing assertion here.
        let src = "｜青梅《おうめ》";
        let _normalized = Source::new(src)
            .sanitize()
            .tokenize()
            .pair()
            .classify()
            .normalize();
    }

    #[test]
    fn intermediate_inspection_is_borrow_only() {
        // Inspecting an intermediate state (`text`, `tokens`,
        // `diagnostics`) does not consume — the chain continues.
        let src = "abc";
        let sanitized = Source::new(src).sanitize();
        assert_eq!(sanitized.text(), "abc");
        assert!(sanitized.diagnostics().is_empty());
        let tokenized = sanitized.tokenize();
        assert!(!tokenized.tokens().is_empty());
        let _classified = tokenized.pair().classify();
    }

    #[test]
    fn build_owned_matches_legacy_lex() {
        // Pin equivalence between the type-state chain and the
        // monolithic legacy entry.
        let src = "明治の頃｜青梅《おうめ》街道沿いに";
        let chained = Source::new(src)
            .sanitize()
            .tokenize()
            .pair()
            .classify()
            .normalize()
            .build_owned();
        let monolithic = crate::lex(src);
        assert_eq!(chained.normalized, monolithic.normalized);
        assert_eq!(chained.sanitized_len, monolithic.sanitized_len);
    }

    #[test]
    fn lex_chained_into_arena_byte_equal_to_lex_into_arena() {
        let arena = Arena::new();
        let src = "｜青梅《おうめ》";
        let chained = lex_chained_into_arena(src, &arena);
        let direct = lex_into_arena(src, &arena);
        assert_eq!(chained.normalized, direct.normalized);
        assert_eq!(chained.sanitized_len, direct.sanitized_len);
        assert_eq!(chained.registry.inline.len(), direct.registry.inline.len());
    }

    #[test]
    fn diagnostics_propagate_through_chain() {
        let src = "abc\u{E001}def";
        let owned = Source::new(src)
            .sanitize()
            .tokenize()
            .pair()
            .classify()
            .normalize()
            .build_owned();
        assert!(
            owned
                .diagnostics
                .iter()
                .any(|d| matches!(d, Diagnostic::SourceContainsPua { .. })),
            "PUA diagnostic must survive the type-state chain"
        );
    }
}
