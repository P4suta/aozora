//! Sanitized ↔ source byte-offset mapping — issue #90.
//!
//! [`Document::parse`](crate) reports [`Diagnostic`](aozora_spec::Diagnostic)
//! spans in **sanitized** coordinates: the [`sanitize`](super::sanitize)
//! output, after a leading BOM is stripped, `CR`/`LF` is folded to `\n`,
//! decorative rules gain a separating blank line, and `〔…〕` accent
//! digraphs are decomposed. Each of those shifts byte offsets, so a
//! consumer that wants to land a caret (an editor overlay, a SARIF
//! region, a character-level checker) on the **source** text must undo
//! the sanitize transform.
//!
//! BOM and `CR`/`LF` are pure deletions a byte-alignment can recover, but
//! the `〔…〕` accent substitution (`ae&` → æ, `m'` → ḿ) changes length
//! per digraph and cannot be recovered by alignment alone — it is the one
//! primitive a source-coordinate consumer can't derive robustly on its
//! own. [`offset_map`] builds an exact map covering **all** the shifts.
//!
//! ```
//! use aozora_pipeline::lexer::offset_map;
//!
//! // CRLF source → the diagnostic-coordinate `\n` text.
//! let map = offset_map("a\r\nb");
//! // sanitized "a\nb": the 'b' at sanitized offset 2 sits at source offset 3.
//! assert_eq!(map.source_offset(2), 3);
//! ```
//!
//! The map is the composition of one piecewise-linear piece per
//! length-changing pass; a clean (ASCII, LF, no rules, no accents) source
//! yields the identity map.

use aozora_syntax::Span;

/// Exact map from a **sanitized** byte offset back to the **source** byte
/// offset it came from. Build it with [`offset_map`].
///
/// Querying is `O(log n)` per length-changing pass (typically one or two
/// small binary searches). A source needing no sanitation maps as the
/// identity ([`OffsetMap::is_identity`]).
#[derive(Debug, Clone)]
pub struct OffsetMap {
    /// Bytes of leading `U+FEFF` BOM stripped — a constant shift applied
    /// after the per-pass pieces resolve into post-BOM coordinates.
    bom: u32,
    /// `line_normalized → after_bom`: one anchor per `\r\n` collapse.
    crlf: Piece,
    /// `rule_isolated → line_normalized`: one anchor per inserted blank
    /// line before a decorative rule.
    rule: Piece,
    /// `sanitized → rule_isolated`: one anchor per length-changing accent
    /// digraph inside a `〔…〕` span.
    accent: Piece,
}

impl OffsetMap {
    /// Map a sanitized byte offset to the source byte offset it originated
    /// from. Offsets at or past the sanitized length map to the source
    /// length. A sanitized offset on a UTF-8 char boundary maps to a
    /// source char boundary — the property diagnostic spans (always on
    /// boundaries) rely on.
    #[must_use]
    pub fn source_offset(&self, sanitized: u32) -> u32 {
        // Unwind the passes in reverse application order:
        // sanitized → rule_isolated → line_normalized → after_bom → source.
        let rule_isolated = self.accent.to_input(sanitized);
        let line_normalized = self.rule.to_input(rule_isolated);
        let after_bom = self.crlf.to_input(line_normalized);
        after_bom + self.bom
    }

    /// Map a sanitized-coordinate [`Span`] to its source-coordinate span.
    #[must_use]
    pub fn source_span(&self, span: Span) -> Span {
        Span::new(self.source_offset(span.start), self.source_offset(span.end))
    }

    /// Whether the map is the identity — the source needed no sanitation
    /// (no BOM, no `\r`, no isolated rule, no decomposed accent), so
    /// sanitized and source coordinates coincide.
    #[must_use]
    pub fn is_identity(&self) -> bool {
        self.bom == 0
            && self.crlf.is_identity()
            && self.rule.is_identity()
            && self.accent.is_identity()
    }
}

/// One pass's offset contribution as a sorted anchor table. Each anchor
/// `(out_off, in_off)` marks the boundary right after a length-changing
/// edit; between anchors the pass copied bytes verbatim, so the mapping
/// is `in_off + (out − out_off)`. Built from a left-to-right list of
/// `(in_off, in_len, out_len)` edits.
#[derive(Debug, Clone)]
struct Piece {
    /// `(out_off, in_off)` pairs, sorted ascending by `out_off`.
    anchors: Vec<(u32, u32)>,
}

impl Piece {
    fn build(edits: &[(u32, u32, u32)]) -> Self {
        let mut anchors = Vec::with_capacity(edits.len());
        // Cumulative `out − in` byte delta after each edit.
        let mut delta: i64 = 0;
        for &(in_off, in_len, out_len) in edits {
            delta += i64::from(out_len) - i64::from(in_len);
            let in_end = in_off + in_len;
            // `out_end` fits u32: it never exceeds the sanitized length,
            // which is bounded by the source length (≤ u32::MAX upstream).
            #[allow(
                clippy::cast_sign_loss,
                clippy::cast_possible_truncation,
                reason = "out_end = in_end + delta is a non-negative offset ≤ sanitized len ≤ u32::MAX"
            )]
            let out_end = (i64::from(in_end) + delta) as u32;
            anchors.push((out_end, in_end));
        }
        Self { anchors }
    }

    /// Map an output offset to the input offset it came from.
    fn to_input(&self, out: u32) -> u32 {
        match self.anchors.binary_search_by_key(&out, |&(o, _)| o) {
            // Exactly on an anchor: the verbatim run starts here.
            Ok(idx) => self.anchors[idx].1,
            // Before the first anchor: still 1:1 with the input.
            Err(0) => out,
            // Inside the verbatim run that begins at the preceding anchor.
            Err(idx) => {
                let (anchor_out, anchor_in) = self.anchors[idx - 1];
                anchor_in + (out - anchor_out)
            }
        }
    }

    fn is_identity(&self) -> bool {
        self.anchors.is_empty()
    }
}

/// Build the exact [`OffsetMap`] for `source`.
///
/// Re-derives the [`sanitize`](super::sanitize::sanitize)
/// transform's offset shifts pass by pass. Pure — allocates only the
/// small per-pass anchor tables (empty for a source that needs no
/// sanitation).
///
/// The PUA-sentinel neutralization (the final sanitize pass) is
/// byte-length-preserving, so it contributes no shift and is not modelled.
#[must_use]
pub fn offset_map(source: &str) -> OffsetMap {
    use super::sanitize::{has_long_rule_line, isolate_decorative_rules, normalize_line_endings};
    use std::borrow::Cow;

    // Pass 1 — strip every leading BOM. A single constant shift.
    let mut after_bom = source;
    while let Some(rest) = after_bom.strip_prefix('\u{FEFF}') {
        after_bom = rest;
    }
    #[allow(
        clippy::cast_possible_truncation,
        reason = "source.len() ≤ u32::MAX is the lexer-wide span contract"
    )]
    let bom = (source.len() - after_bom.len()) as u32;

    // Pass 2 — CR/LF folding. Only `\r\n` (2→1) shifts; lone `\r` (1→1)
    // does not. Mirror normalize_line_endings' `\r` scan.
    let crlf_edits = scan_crlf_edits(after_bom);
    let line_normalized: Cow<'_, str> = if after_bom.contains('\r') {
        Cow::Owned(normalize_line_endings(after_bom))
    } else {
        Cow::Borrowed(after_bom)
    };

    // Pass 3 — decorative-rule isolation. Each insertion adds one `\n`.
    let rule_edits = scan_rule_edits(&line_normalized);
    let rule_isolated: Cow<'_, str> = if has_long_rule_line(&line_normalized) {
        Cow::Owned(isolate_decorative_rules(&line_normalized))
    } else {
        line_normalized
    };

    // Pass 4 — `〔…〕` accent decomposition. Per length-changing digraph.
    let accent_edits = scan_accent_edits(&rule_isolated);

    OffsetMap {
        bom,
        crlf: Piece::build(&crlf_edits),
        rule: Piece::build(&rule_edits),
        accent: Piece::build(&accent_edits),
    }
}

/// `(in_off, in_len, out_len)` edits for the CR/LF fold over `after_bom`.
/// Mirrors [`normalize_line_endings`](super::sanitize::normalize_line_endings):
/// `\r\n` collapses 2 bytes to 1; a lone `\r` is length-preserving and
/// emits no edit.
fn scan_crlf_edits(after_bom: &str) -> Vec<(u32, u32, u32)> {
    let bytes = after_bom.as_bytes();
    let mut edits = Vec::new();
    for cr in memchr::memchr_iter(b'\r', bytes) {
        if bytes.get(cr + 1) == Some(&b'\n') {
            #[allow(
                clippy::cast_possible_truncation,
                reason = "byte offset ≤ source.len() ≤ u32::MAX"
            )]
            edits.push((cr as u32, 2, 1));
        }
    }
    edits
}

/// `(in_off, 0, 1)` insertion edits for decorative-rule isolation over
/// `line_normalized`. Mirrors
/// [`isolate_decorative_rules`](super::sanitize::isolate_decorative_rules)
/// exactly (same `prev_nonblank` bookkeeping and tail-line handling), so
/// the recorded insertion points stay in lockstep with the real pass.
#[allow(
    clippy::cast_possible_truncation,
    reason = "byte offset ≤ source.len() ≤ u32::MAX is the lexer-wide span contract"
)]
fn scan_rule_edits(line_normalized: &str) -> Vec<(u32, u32, u32)> {
    use super::sanitize::is_rule_line_trimmed;

    let bytes = line_normalized.as_bytes();
    let mut edits = Vec::new();
    let mut line_start = 0usize;
    let mut prev_nonblank = false;

    for nl in memchr::memchr_iter(b'\n', bytes) {
        let trimmed = line_normalized[line_start..nl].trim();
        if is_rule_line_trimmed(trimmed) && prev_nonblank {
            edits.push((line_start as u32, 0, 1));
        }
        prev_nonblank = !trimmed.is_empty();
        line_start = nl + 1;
    }
    if line_start < bytes.len() {
        let tail = line_normalized[line_start..].trim();
        if is_rule_line_trimmed(tail) && prev_nonblank {
            edits.push((line_start as u32, 0, 1));
        }
    }
    edits
}

/// `(in_off, in_len, out_len)` edits for `〔…〕` accent decomposition over
/// `rule_isolated`. Mirrors
/// [`rewrite_accent_spans`](super::sanitize::rewrite_accent_spans)'
/// span scan (unclosed `〔` ends the scan) and delegates the per-digraph
/// deltas to [`decompose_fragment_edits`](aozora_syntax::accent::decompose_fragment_edits).
fn scan_accent_edits(rule_isolated: &str) -> Vec<(u32, u32, u32)> {
    use aozora_syntax::accent::decompose_fragment_edits;

    let mut edits = Vec::new();
    let mut cursor = 0usize;
    while cursor < rule_isolated.len() {
        let Some(open_rel) = rule_isolated[cursor..].find('〔') else {
            break;
        };
        let open_abs = cursor + open_rel;
        let after_open = open_abs + '〔'.len_utf8();
        let Some(close_rel) = rule_isolated[after_open..].find('〕') else {
            // Unclosed span: rewrite_accent_spans copies the rest verbatim.
            break;
        };
        let close_abs = after_open + close_rel;
        let body = &rule_isolated[after_open..close_abs];
        for (in_off, in_len, out_len) in decompose_fragment_edits(body) {
            #[allow(
                clippy::cast_possible_truncation,
                reason = "byte offset ≤ source.len() ≤ u32::MAX"
            )]
            edits.push(((after_open + in_off) as u32, in_len as u32, out_len as u32));
        }
        cursor = close_abs + '〕'.len_utf8();
    }
    edits
}

#[cfg(test)]
mod tests {
    use super::super::sanitize::sanitize;
    use super::*;

    /// Walk every sanitized offset and pin the universal invariants a
    /// source-coordinate consumer relies on: monotonic, in-bounds, ending
    /// exactly at `source.len()`, and always on a source char boundary.
    #[allow(
        clippy::cast_possible_truncation,
        reason = "test offsets ≤ source.len() ≤ u32::MAX"
    )]
    fn assert_map_invariants(source: &str) {
        let sanitized = sanitize(source).text;
        let map = offset_map(source);
        let mut prev = 0u32;
        for s in 0..=sanitized.len() {
            // Diagnostic spans land only on char boundaries; the
            // boundary-preservation guarantee is scoped to those.
            if !sanitized.is_char_boundary(s) {
                continue;
            }
            let src = map.source_offset(s as u32);
            assert!(
                src >= prev,
                "non-monotonic at sanitized {s}: {src} < {prev} (source {source:?})"
            );
            assert!(
                src as usize <= source.len(),
                "out of bounds at sanitized {s}: {src} > {} (source {source:?})",
                source.len()
            );
            assert!(
                source.is_char_boundary(src as usize),
                "source offset {src} is not a char boundary (source {source:?})"
            );
            prev = src;
        }
        assert_eq!(
            map.source_offset(sanitized.len() as u32) as usize,
            source.len(),
            "sanitized end must map to source end (source {source:?})"
        );
    }

    #[test]
    fn identity_for_clean_source() {
        let map = offset_map("plain ascii\nと日本語");
        assert!(map.is_identity());
        assert_eq!(map.source_offset(0), 0);
        assert_eq!(map.source_offset(5), 5);
    }

    #[test]
    fn bom_is_a_constant_shift() {
        let source = "\u{FEFF}abc"; // BOM = 3 bytes
        let map = offset_map(source);
        assert!(!map.is_identity());
        assert_eq!(map.source_offset(0), 3); // 'a' sits 3 bytes in
        assert_eq!(map.source_offset(3), 6); // sanitized end → source end
        assert_map_invariants(source);
    }

    #[test]
    fn crlf_folds_shift_trailing_offsets() {
        let source = "a\r\nb\r\nc";
        let map = offset_map(source);
        // sanitized = "a\nb\nc"
        assert_eq!(map.source_offset(0), 0); // 'a'
        assert_eq!(map.source_offset(1), 1); // '\n' → the '\r'
        assert_eq!(map.source_offset(2), 3); // 'b'
        assert_eq!(map.source_offset(4), 6); // 'c'
        assert_eq!(map.source_offset(5), 7); // end
        assert_map_invariants(source);
    }

    #[test]
    fn lone_cr_is_length_preserving() {
        let source = "a\rb"; // lone CR → LF, 1:1
        let map = offset_map(source);
        assert!(map.is_identity());
        assert_map_invariants(source);
    }

    #[test]
    fn decorative_rule_insertion_shifts_offsets() {
        let source = "ab\n==========\ncd"; // 10 '=' rule after non-blank line
        let map = offset_map(source);
        // sanitized = "ab\n\n==========\ncd" (a blank line inserted)
        assert_eq!(map.source_offset(4), 3); // rule start in sanitized → source 3
        assert_eq!(map.source_offset(15), 14); // 'c'
        assert_map_invariants(source);
    }

    #[test]
    fn accent_growth_digraph_maps_exactly() {
        let source = "〔m'a〕"; // m' (2 bytes) → ḿ (3 bytes)
        let map = offset_map(source);
        // sanitized = "〔ḿa〕": 〔=0..3, ḿ=3..6, a=6..7, 〕=7..10
        assert_eq!(map.source_offset(3), 3); // ḿ start → m' start
        assert_eq!(map.source_offset(6), 5); // 'a' → source 'a'
        assert_eq!(map.source_offset(10), 9); // end
        assert_map_invariants(source);
    }

    #[test]
    fn accent_shrink_ligature_maps_exactly() {
        let source = "〔ae&on〕"; // ae& (3 bytes) → æ (2 bytes)
        assert_map_invariants(source);
    }

    #[test]
    fn length_preserving_accent_needs_no_anchor() {
        // e` → è is 2 bytes → 2 bytes: no shift, map stays identity.
        let source = "〔fune`bre〕";
        let map = offset_map(source);
        assert!(map.is_identity());
        assert_map_invariants(source);
    }

    #[test]
    fn all_transforms_compose() {
        let source = "\u{FEFF}前\r\n----------\r\n〔café`〕後";
        assert_map_invariants(source);
    }

    #[test]
    fn source_span_maps_both_endpoints() {
        let source = "a\r\nbc";
        let map = offset_map(source);
        // sanitized "a\nbc": span [2,4) ("bc") → source [3,5)
        let mapped = map.source_span(Span::new(2, 4));
        assert_eq!(mapped, Span::new(3, 5));
    }

    #[test]
    fn unclosed_tortoise_span_has_no_accent_shift() {
        // rewrite_accent_spans copies an unclosed 〔 run verbatim.
        let source = "tail 〔fune`bre no close";
        let map = offset_map(source);
        assert!(map.is_identity());
        assert_map_invariants(source);
    }
}
