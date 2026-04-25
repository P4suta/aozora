//! Orchestrator that drives the lex pipeline.
//!
//! Stitches together [`aozora_lexer::sanitize`] (phase 0), our own
//! [`crate::tokenize::tokenize_with_scan`] (phase 1, replacing the
//! legacy character walker), and [`aozora_lexer::pair`] /
//! [`aozora_lexer::classify`] / [`aozora_lexer::normalize`] /
//! [`aozora_lexer::validate`] (phases 2, 3, 4, 6) to produce the
//! same [`LexOutput`] the legacy [`aozora_lexer::lex`] produces.
//!
//! The byte-identical proptest in `tests/property_byte_identical.rs`
//! pins this equivalence on every test run; subsequent commits that
//! pull more phases into this crate keep the test as the
//! load-bearing safety net.

use aozora_lexer::{
    LexOutput, classify, normalize, pair, sanitize, tokenize as legacy_tokenize, validate,
};

/// Run the lex pipeline over `source`, returning the same
/// [`LexOutput`] shape the legacy [`aozora_lexer::lex`] produces.
///
/// Phase ownership today:
///
/// | phase | implementation                                  |
/// |-------|-------------------------------------------------|
/// | 0     | `aozora_lexer::sanitize`                        |
/// | 1     | `crate::tokenize::tokenize_with_scan` (NEW)     |
/// | 2     | `aozora_lexer::pair`                            |
/// | 3     | `aozora_lexer::classify`                        |
/// | 4     | `aozora_lexer::normalize`                       |
/// | 6     | `aozora_lexer::validate`                        |
///
/// Each subsequent commit folds one more phase out of `aozora-lexer`
/// into this crate; the public [`LexOutput`] shape is fixed by ADR-0010.
///
/// # Panics
///
/// Panics if the sanitised text length exceeds `u32::MAX` bytes —
/// the same upper bound the legacy implementation enforces (`Span`
/// field width).
#[must_use]
pub(crate) fn run_pipeline(source: &str) -> LexOutput {
    // Phase 0: sanitize (BOM strip / CRLF normalisation / accent
    // decomposition / decorative-rule isolation / PUA collision scan).
    let sanitized = sanitize(source);

    // Phase 1: tokenise.
    //
    // We currently call the legacy character-walking tokenizer here.
    // An aozora-scan-driven alternative ([`crate::tokenize_with_scan`])
    // ships in this crate alongside it and is byte-identical to the
    // legacy output, but the corpus profile + the
    // `tokenize_compare` criterion bench (2026-04-26) showed it
    // **5.4× slower** on plain Japanese inputs:
    //
    //   band      legacy tokenize   aozora-scan tokenize
    //   plain     77 µs / 64 KiB    416 µs / 64 KiB
    //   sparse    80 µs / 64 KiB    426 µs / 64 KiB
    //   dense     93 µs / 64 KiB    363 µs / 64 KiB
    //
    // Root cause: `memchr3` over the trigger-leading-byte set
    // {0xE2, 0xE3, 0xEF} can't skip-forward in Japanese-heavy text
    // because 0xE3 is the leading byte of every hiragana / katakana
    // character. Each character produces a candidate that gets
    // PHF-classified and rejected — same per-character cost as the
    // legacy walker, plus the overhead of materialising candidate
    // offsets in a `Vec<u32>`. For the win to land, the scan needs
    // to compare against the full 3-byte trigger sequences in SIMD
    // (the simdjson "structural bitmap" approach) rather than just
    // the leading byte.
    //
    // Until that lands, the legacy tokenizer is faster on real
    // Japanese workloads. The scan-driven path is kept as
    // `crate::tokenize_with_scan` — its byte-identical proptest
    // gate, unit tests, and `tokenize_compare` bench provide the
    // foundation a future redesign will start from.
    let tokens = legacy_tokenize(&sanitized.text);

    // Phase 2: balanced-stack pairing.
    let pair_out = pair(&tokens);

    // Phase 3: classify each pair body / solo into AozoraNode
    // variants.
    let classify_out = classify(&pair_out, &sanitized.text);

    // Phase 4: PUA-sentinel substitution + placeholder registry build.
    let mut normalize_out = normalize(&classify_out, &sanitized.text);

    // Stitch phase 0 diagnostics back in front of the phase 4 output.
    // The legacy implementation does this exact prepend; preserving
    // the order keeps the byte-identical proptest honest.
    let mut diagnostics = sanitized.diagnostics;
    diagnostics.append(&mut normalize_out.diagnostics);
    normalize_out.diagnostics = diagnostics;

    // Phase 6: invariant validation (V1: residual ［＃, V2: PUA
    // sentinel registration, V3: registry sort + position match).
    let validated = validate(normalize_out);

    let sanitized_len = u32::try_from(sanitized.text.len())
        .expect("sanitized text fits in u32 (matches Span field width)");

    LexOutput::from_parts(
        validated.normalized,
        validated.registry,
        validated.diagnostics,
        sanitized_len,
    )
}
