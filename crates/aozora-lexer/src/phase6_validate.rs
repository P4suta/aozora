//! Phase 6 — self-check the Phase 4 output against four structural
//! invariants.
//!
//! The validator runs *after* normalization and is a sanity harness,
//! not a correctness fallback: in a well-behaved pipeline every V
//! check trivially passes. The value is in catching architectural
//! regressions (e.g. a Phase 3 recogniser that forgets to consume
//! its source span, leaving an `［＃` behind in the normalized
//! text; a Phase 4 driver bug that pushes a registry entry at the
//! wrong byte offset). Each violation becomes a [`Diagnostic`] so
//! the caller sees the full picture.
//!
//! ## Invariants
//!
//! * **V1 — No residual `［＃`.** If any `［＃` sequence survives
//!   into the normalized text, some `［＃…］` annotation escaped
//!   Phase 3/4 classification. This is the primary guardrail
//!   behind the 17 k-work corpus sweep's "leaked markers" count
//!   (ADR-0007).
//!
//! * **V2 — Every PUA character is a recorded sentinel.** Any
//!   `U+E001..=U+E004` codepoint in the normalized text must start
//!   at a byte offset recorded in the placeholder registry. Source-
//!   side PUA characters (flagged by Phase 0 via
//!   [`Diagnostic::SourceContainsPua`]) are accepted here only if
//!   the registry also records them — otherwise `post_process` would
//!   be unable to distinguish a real sentinel from a collision.
//!
//! * **V3 — Every registry entry has a matching PUA character in
//!   normalized.** The four registries must be strictly sorted by
//!   position and every recorded position must address a PUA char
//!   of the matching kind.
//!
//! * V4 (`SourceMap` coverage) — deferred until the `SourceMap`
//!   pass lands.
//!
//! The validator takes `&NormalizeOutput` and returns an extended
//! diagnostics vector; it never mutates or drops the normalized text.

use crate::diagnostic::Diagnostic;
use crate::phase4_normalize::{NormalizeOutput, PlaceholderRegistry};
use crate::{BLOCK_CLOSE_SENTINEL, BLOCK_LEAF_SENTINEL, BLOCK_OPEN_SENTINEL, INLINE_SENTINEL};

/// Output of Phase 6: the original normalized text + registry are
/// passed through unchanged, with any violation diagnostics appended.
#[derive(Debug, Clone)]
pub struct ValidateOutput {
    pub normalized: String,
    pub registry: PlaceholderRegistry,
    pub diagnostics: Vec<Diagnostic>,
}

/// Run the structural invariants V1..V3 against a Phase 4 output.
///
/// Pure function; no I/O. Consumes the `NormalizeOutput` by value so
/// the caller cannot observe the state between validate and any
/// downstream consumer.
#[must_use]
pub fn validate(input: NormalizeOutput) -> ValidateOutput {
    let NormalizeOutput {
        normalized,
        registry,
        mut diagnostics,
    } = input;

    check_v1(&normalized, &mut diagnostics);
    check_v2_v3(&normalized, &registry, &mut diagnostics);

    ValidateOutput {
        normalized,
        registry,
        diagnostics,
    }
}

/// V1 — the `［＃` digraph must not survive Phase 4. Any occurrence
/// means an annotation escaped classification; emit one diagnostic
/// per hit so the caller can surface all of them.
fn check_v1(normalized: &str, diagnostics: &mut Vec<Diagnostic>) {
    // `［＃` = U+FF3B U+FF03, 6 bytes total. Search for the paired
    // pattern; a lone `［` is not itself a leaked annotation.
    let needle = "［＃";
    let mut start = 0usize;
    while let Some(idx) = normalized[start..].find(needle) {
        let abs = start + idx;
        let abs_u32 = u32::try_from(abs).expect("sanitize caps source at u32");
        let end_u32 = abs_u32 + u32::try_from(needle.len()).expect("needle fits u32");
        diagnostics.push(Diagnostic::residual_annotation_marker(
            afm_syntax::Span::new(abs_u32, end_u32),
        ));
        start = abs + needle.len();
    }
}

/// V2 — every PUA sentinel in the normalized text must be recorded
/// in the registry of the matching kind. V3 — every registry entry
/// must address a matching PUA char.
fn check_v2_v3(
    normalized: &str,
    registry: &PlaceholderRegistry,
    diagnostics: &mut Vec<Diagnostic>,
) {
    // V2: walk the normalized text char-by-char, flag unrecorded PUA
    // chars.
    let mut byte_pos: u32 = 0;
    for ch in normalized.chars() {
        let ch_len = u32::try_from(ch.len_utf8()).expect("char len 1..=4");
        let recorded = match ch {
            INLINE_SENTINEL => registry.inline_at(byte_pos).is_some(),
            BLOCK_LEAF_SENTINEL => registry.block_leaf_at(byte_pos).is_some(),
            BLOCK_OPEN_SENTINEL => registry.block_open_at(byte_pos).is_some(),
            BLOCK_CLOSE_SENTINEL => registry.block_close_at(byte_pos).is_some(),
            _ => true,
        };
        if !recorded {
            diagnostics.push(Diagnostic::unregistered_sentinel(
                afm_syntax::Span::new(byte_pos, byte_pos + ch_len),
                ch,
            ));
        }
        byte_pos += ch_len;
    }

    // V3: confirm every registry entry's position matches a PUA char
    // of the matching kind. An out-of-order vector, duplicate key, or
    // stale offset bubbles out here.
    check_registry_slice(normalized, &registry.inline, INLINE_SENTINEL, diagnostics);
    check_registry_slice(
        normalized,
        &registry.block_leaf,
        BLOCK_LEAF_SENTINEL,
        diagnostics,
    );
    check_registry_slice(
        normalized,
        &registry.block_open,
        BLOCK_OPEN_SENTINEL,
        diagnostics,
    );
    check_registry_slice(
        normalized,
        &registry.block_close,
        BLOCK_CLOSE_SENTINEL,
        diagnostics,
    );
}

fn check_registry_slice<T>(
    normalized: &str,
    slice: &[(u32, T)],
    expected_char: char,
    diagnostics: &mut Vec<Diagnostic>,
) {
    for window in slice.windows(2) {
        if window[0].0 >= window[1].0 {
            // Build the span with the earlier position as `start` so
            // `Span::new` stays well-formed (start <= end). Callers of
            // the diagnostic care about *both* positions, not their
            // order: the miette snippet only highlights the invalid
            // pair's textual range.
            let (lo, hi) = if window[0].0 <= window[1].0 {
                (window[0].0, window[1].0)
            } else {
                (window[1].0, window[0].0)
            };
            diagnostics.push(Diagnostic::registry_out_of_order(afm_syntax::Span::new(
                lo, hi,
            )));
        }
    }
    for &(pos, _) in slice {
        let idx = pos as usize;
        let ch_len = expected_char.len_utf8();
        let actual = normalized
            .get(idx..idx + ch_len)
            .and_then(|s| s.chars().next());
        if actual != Some(expected_char) {
            diagnostics.push(Diagnostic::registry_position_mismatch(
                afm_syntax::Span::new(pos, pos + u32::try_from(ch_len).expect("utf8 len")),
                expected_char,
            ));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::phase1_events::tokenize;
    use crate::phase2_pair::pair;
    use crate::phase3_classify::classify;
    use crate::phase4_normalize::normalize;

    fn run(src: &str) -> ValidateOutput {
        let tokens = tokenize(src);
        let pair_out = pair(&tokens);
        let classify_out = classify(&pair_out, src);
        let normalize_out = normalize(&classify_out, src);
        validate(normalize_out)
    }

    #[test]
    fn plain_text_has_no_diagnostics() {
        let out = run("hello world こんにちは");
        assert!(
            !out.diagnostics
                .iter()
                .any(|d| matches!(d, Diagnostic::ResidualAnnotationMarker { .. })),
            "unexpected V1 violation: {:?}",
            out.diagnostics
        );
    }

    #[test]
    fn well_formed_inline_passes_v2_v3() {
        let out = run("｜漢《かん》");
        assert!(
            !out.diagnostics
                .iter()
                .any(|d| matches!(d, Diagnostic::UnregisteredSentinel { .. })),
        );
        assert!(
            !out.diagnostics
                .iter()
                .any(|d| matches!(d, Diagnostic::RegistryPositionMismatch { .. })),
        );
    }

    #[test]
    fn page_break_block_passes_all_invariants() {
        let out = run("前\n［＃改ページ］\n後");
        for diag in &out.diagnostics {
            match diag {
                Diagnostic::ResidualAnnotationMarker { .. }
                | Diagnostic::UnregisteredSentinel { .. }
                | Diagnostic::RegistryOutOfOrder { .. }
                | Diagnostic::RegistryPositionMismatch { .. } => {
                    panic!("unexpected validate diagnostic: {diag:?}")
                }
                _ => {}
            }
        }
    }

    #[test]
    fn registry_is_sorted_after_validate() {
        let out = run("｜a《あ》｜b《い》｜c《う》");
        assert!(out.registry.is_sorted_strictly());
    }

    #[test]
    fn block_paired_container_passes_all_invariants() {
        let out = run("［＃ここから字下げ］本文［＃ここで字下げ終わり］");
        for diag in &out.diagnostics {
            assert!(
                !matches!(
                    diag,
                    Diagnostic::ResidualAnnotationMarker { .. }
                        | Diagnostic::UnregisteredSentinel { .. }
                        | Diagnostic::RegistryOutOfOrder { .. }
                        | Diagnostic::RegistryPositionMismatch { .. }
                ),
                "unexpected diag: {diag:?}",
            );
        }
    }

    #[test]
    fn early_diagnostics_survive_validate() {
        // A stray `］` triggers Phase 2's UnmatchedClose. Validate
        // must not drop it.
        let out = run("stray］text");
        assert!(
            out.diagnostics
                .iter()
                .any(|d| matches!(d, Diagnostic::UnmatchedClose { .. })),
        );
    }

    // ---------------------------------------------------------------
    // Synthetic registries exercising V2 / V3 paths that are not
    // reachable from the `lex() → normalize()` pipeline because
    // the upstream phases are well-behaved. Tests construct
    // deliberately malformed `NormalizeOutput` values and feed them
    // into `validate()` to cover the diagnostic-emission arms.
    // ---------------------------------------------------------------

    use afm_syntax::{AozoraNode, Ruby};

    /// Construct a minimal [`NormalizeOutput`] that satisfies V1 (no
    /// residual `［＃`) but can be tweaked per-test to trigger V2/V3
    /// violations.
    fn synthetic_output(normalized: &str, inline: Vec<(u32, AozoraNode)>) -> NormalizeOutput {
        NormalizeOutput {
            normalized: normalized.to_owned(),
            registry: PlaceholderRegistry {
                inline,
                ..PlaceholderRegistry::default()
            },
            diagnostics: Vec::new(),
        }
    }

    fn ruby_node() -> AozoraNode {
        AozoraNode::Ruby(Ruby {
            base: "x".into(),
            reading: "y".into(),
            delim_explicit: false,
        })
    }

    #[test]
    fn v3_registry_out_of_order_emits_diagnostic() {
        // Two inline entries whose byte positions go backwards (5
        // after 10). validate() must emit RegistryOutOfOrder and
        // leave the rest of the stream intact.
        let norm = "a\u{E001}bc\u{E001}d";
        // Real positions: first sentinel at 1, second at 4. Swap
        // them to create the out-of-order violation.
        let out = validate(synthetic_output(
            norm,
            vec![(4, ruby_node()), (1, ruby_node())],
        ));
        assert!(
            out.diagnostics
                .iter()
                .any(|d| matches!(d, Diagnostic::RegistryOutOfOrder { .. })),
            "RegistryOutOfOrder must fire for descending positions: {:?}",
            out.diagnostics
        );
    }

    #[test]
    fn v3_registry_position_mismatch_emits_diagnostic() {
        // Claim an inline sentinel sits at byte 0 of normalized, but
        // byte 0 is actually `a` (not U+E001). validate() must emit
        // RegistryPositionMismatch with the expected codepoint.
        let norm = "abc";
        let out = validate(synthetic_output(norm, vec![(0, ruby_node())]));
        let mismatch = out
            .diagnostics
            .iter()
            .find(|d| matches!(d, Diagnostic::RegistryPositionMismatch { .. }));
        assert!(
            mismatch.is_some(),
            "RegistryPositionMismatch must fire: {:?}",
            out.diagnostics
        );
        if let Some(Diagnostic::RegistryPositionMismatch { expected, .. }) = mismatch {
            assert_eq!(
                *expected, INLINE_SENTINEL,
                "mismatch must name the inline sentinel as expected"
            );
        }
    }

    #[test]
    fn v3_registry_position_off_end_emits_mismatch() {
        // A registry position past the normalized-string length is
        // treated as a position mismatch (the codepoint-at-position
        // lookup returns None). Guards against drift between
        // `block_close.push` and the output string length.
        let norm = "short";
        let output = NormalizeOutput {
            normalized: norm.to_owned(),
            registry: PlaceholderRegistry {
                block_close: vec![(99, afm_syntax::ContainerKind::Keigakomi)],
                ..PlaceholderRegistry::default()
            },
            diagnostics: Vec::new(),
        };
        let out = validate(output);
        assert!(
            out.diagnostics
                .iter()
                .any(|d| matches!(d, Diagnostic::RegistryPositionMismatch { .. })),
            "out-of-bounds registry position must report mismatch: {:?}",
            out.diagnostics
        );
    }
}
