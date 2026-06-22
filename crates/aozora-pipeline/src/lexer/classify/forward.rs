//! Forward-reference recognisers.
//!
//! The `recognize_annotation` cascade and the per-construct
//! forward-reference recognisers (bouten, 縦中横, heading, emphasis,
//! left-ruby / side-note, caption-figure) plus the source-scanned
//! `FORWARD_TARGET_INDEX` that answers "does this target appear earlier
//! in the source?". These need the pair-stage event stream, unlike the
//! body-keyword `directive` classifier. Extracted verbatim from the
//! classify-stage classifier; recognised nodes build on the shared
//! `RecogniseCtx`.

#[cfg(feature = "classify-instrument")]
use super::super::instrumentation::{Subsystem, SubsystemGuard};

use std::cell::RefCell;
use std::collections::HashMap;

use aozora_spec::Diagnostic;
use aozora_syntax::alloc::BorrowedAllocator;
use aozora_syntax::borrowed;
use aozora_syntax::{BoutenPosition, DirectiveKind, EmphasisKind, MarginNoteKind, Span};

use super::super::pair::{PairEvent, PairKind};
use super::super::token::TriggerKind;
use super::directive::{
    bouten_kind_from_suffix, classify_annotation_body, classify_general_image_body,
    editorial_note_kind, parse_decimal_u8_prefix, parse_heading_keyword,
};
use super::{AnnotationMatch, BodyView, EmitKind, RecogniseCtx};

thread_local! {
    /// Forward-reference target → first byte offset in source.
    ///
    /// `state.installed = true` means the map is authoritative: every
    /// target queried by `forward_target_is_preceded` is either in the
    /// map or genuinely absent from source. `state.installed = false`
    /// means the lookup falls back to the legacy
    /// `source[..cutoff].contains` path for correctness.
    ///
    /// Pre-I-2 the streaming classify-stage entry point built this index from
    /// a complete event slice up-front. Streaming has no event slice,
    /// so the index is left empty: every `forward_target_is_preceded`
    /// query falls back to substring scan. The pathological doc
    /// (170 ms with substring, 20 ms with AC) regresses; the median
    /// document was already on the substring path so corpus
    /// throughput is unchanged. A future re-introduction can scan raw
    /// source bytes for `［＃「TARGET」` patterns (event-free) and
    /// re-populate the index without breaking the streaming pipeline.
    static FORWARD_TARGET_INDEX: RefCell<ForwardTargetState> = RefCell::default();
}

#[derive(Default)]
struct ForwardTargetState {
    installed: bool,
    first_position: HashMap<String, u32>,
}

/// Drop the per-classify forward-target index.
fn clear_forward_target_index() {
    FORWARD_TARGET_INDEX.with(|cell| {
        let mut state = cell.borrow_mut();
        state.installed = false;
        state.first_position.clear();
    });
}

/// Step A.1 fast path: most corpus docs never installed the index
/// in the first place (they're below the 64-quote threshold), so
/// the previous doc's `installed=false` carries forward and the
/// `clear()` call is a no-op. This wrapper short-circuits the
/// `borrow_mut()` + `HashMap::clear()` pair when there's nothing
/// to clear, saving ~10 ns per parse on the common path.
fn clear_forward_target_index_if_installed() {
    FORWARD_TARGET_INDEX.with(|cell| {
        let mut state = cell.borrow_mut();
        if state.installed {
            state.installed = false;
            state.first_position.clear();
        }
    });
}

/// Below this many distinct `「…」` quote bodies even the source-byte
/// pre-pass loses (build cost outpaces the substring scans saved).
/// The median corpus doc has < 100 quote bodies and skips the index
/// entirely; the pathological annotation-dense doc has thousands.
const FORWARD_QUOTE_BODY_THRESHOLD: usize = 64;

/// Build the forward-reference target index by scanning raw source
/// bytes for `「…」` quote pairs and recording the first byte position
/// of each unique body. Event-free — runs before the streaming
/// pipeline starts and replaces the legacy event-driven pre-pass that
/// I-2 deforestation made impossible to keep around.
pub(super) fn install_forward_target_index_from_source(source: &str) {
    use memchr::memmem;

    // `「` is U+300C, UTF-8 = E3 80 8C; `」` is U+300D, UTF-8 = E3 80 8D.
    const QUOTE_OPEN: &[u8] = b"\xE3\x80\x8C";
    const QUOTE_CLOSE: &[u8] = b"\xE3\x80\x8D";

    #[cfg(feature = "classify-instrument")]
    let _phase3_guard = SubsystemGuard::new(Subsystem::ForwardIndexInstall);

    let bytes = source.as_bytes();
    // Step A.1: cheap up-front count with early break at the
    // threshold. The previous shape unconditionally collected every
    // `「` offset into a `Vec<usize>` before checking the count —
    // wasted allocation on the >99 % of corpus docs that have far
    // fewer than 64 quote opens. memmem::find_iter is internally
    // SIMD-vectorised, so the count loop is bandwidth-bound; bailing
    // out at 64 keeps the work proportional to "found enough", not
    // "scanned the whole doc".
    let mut count = 0usize;
    for _ in memmem::find_iter(bytes, QUOTE_OPEN) {
        count += 1;
        if count >= FORWARD_QUOTE_BODY_THRESHOLD {
            break;
        }
    }
    if count < FORWARD_QUOTE_BODY_THRESHOLD {
        clear_forward_target_index_if_installed();
        return;
    }
    let opens: Vec<usize> = memmem::find_iter(bytes, QUOTE_OPEN).collect();

    // For each `「`, find the next `」` and slice the body. UTF-8
    // boundaries are guaranteed because both delimiters are 3-byte
    // sequences carved from `&str` source. The set of quote bodies is
    // the set of *candidate* forward-reference targets — every
    // `［＃「X」に傍点／は見出し／は縦中横］` names its target with a `「」`
    // pair, so the target appears as a quote body at least once (inside
    // its own directive).
    //
    // The stored value must be the target's first occurrence **as a
    // substring of the source text** — the same question the
    // `source[..cutoff].contains(target)` fallback answers — NOT the
    // position of the `「` that introduced this body. The canonical
    // forward reference is `語句［＃「語句」に傍点］`, where the referent
    // `語句` is *bare* text before the bracket and the only `「語句」`
    // pair lives *inside* the directive (after `［`). Recording the
    // quote position there would put `first_position` past every
    // directive's cutoff, so `first_pos < cutoff` is always false and
    // the bouten silently degrades to `Directive{Unknown}` — exactly
    // the bug that dropped ~half of all corpus 傍点/見出し once a
    // document crossed the 64-quote AC-install threshold. `memmem::find`
    // over the whole source picks up the bare referent.
    let mut first_positions: HashMap<String, u32> = HashMap::with_capacity(opens.len());
    for open_pos in opens {
        let body_start = open_pos + QUOTE_OPEN.len();
        let Some(rel_close) = memmem::find(&bytes[body_start..], QUOTE_CLOSE) else {
            // Unclosed `「` — nothing to index for this open.
            continue;
        };
        let body = &source[body_start..body_start + rel_close];
        if body.is_empty() {
            continue;
        }
        first_positions.entry(body.to_owned()).or_insert_with(|| {
            // First substring occurrence anywhere in the source. The
            // body provably occurs at `body_start` (its own quote), so
            // `find` never returns `None`; the fallback keeps it total.
            let first = memmem::find(bytes, body.as_bytes()).unwrap_or(body_start);
            u32::try_from(first).expect("source positions fit in u32 per sanitize-stage cap")
        });
    }

    if first_positions.len() < FORWARD_QUOTE_BODY_THRESHOLD {
        clear_forward_target_index();
        return;
    }

    FORWARD_TARGET_INDEX.with(|cell| {
        let mut state = cell.borrow_mut();
        state.installed = true;
        state.first_position = first_positions;
    });
}

/// Try to recognize a `［＃keyword…］` annotation at
/// `events[open_idx]`.
///
/// Requires the immediately-next event to be a [`TriggerKind::Hash`]
/// [`PairEvent::Solo`] — the shape `［` `＃` `body` `］`. Bodies
/// without a hash (plain `［…］`) are not annotations; bodies with a
/// hash whose keyword no specialised recogniser matches fall through
/// to the `Directive { Unknown }` catch-all so the bracket is
/// always consumed into some `Node`.
impl<'a> RecogniseCtx<'_, 'a, '_> {
    /// Forward-reference dispatch for a well-formed `［＃…］` bracket.
    ///
    /// Tries the body-keyword classifier first, then a fixed cascade of
    /// forward-reference recognisers, falling through to the
    /// `Directive{Unknown}` catch-all. Cascade order:
    ///
    /// 1. body keyword — `classify_annotation_body`
    /// 2. bouten (single) — `「X」に<kind>`
    /// 3. bouten (range) — `「X」～「Y」に<kind>`
    /// 4. left-ruby — `「X」の左に「Y」のルビ`
    /// 5. side-note — `「X」の左に「Y」の注記` / `「X」に「Y」の傍記`
    /// 6. 縦中横 — `「X」は縦中横`
    /// 7. heading — `「X」は…見出し`
    /// 8. emphasis — `「X」は太字` / `斜体`
    /// 9. caption-figure — `「cap」のキャプション付きの…（file）入る`
    /// 10. general image — `<desc>（file）入る`
    /// 11. empty / editorial-note / `Directive{Unknown}` catch-all
    ///
    /// # Ordering contract
    ///
    /// Most adjacent recognisers are **keyword-disjoint**: their bodies
    /// carry mutually exclusive particles/keywords, so reordering them
    /// leaves the output unchanged. These are deliberately *not* pinned
    /// by dedicated order tests (such a test would be vacuous):
    ///
    /// * bouten vs left-ruby vs side-note — `のルビ`, `の注記`, and `の傍記`
    ///   are not bouten kinds, so each declines before the next is tried.
    /// * 縦中横 / heading / emphasis — `縦中横`, `…見出し`, and
    ///   `太字`/`斜体` are mutually exclusive after the shared `は`
    ///   particle (縦中横's *diagnostic* threading is the one exception,
    ///   below).
    /// * bouten single vs range — separated by the `～` / `〜` infix.
    ///
    /// The orderings that **are** load-bearing (reordering changes the
    /// output) each have a regression test that pins them:
    ///
    /// * caption-figure ≺ general-image — both end in `（file）入る`; the
    ///   caption form is more specific and must win to keep its
    ///   figcaption. Pinned by `caption_before_figure_recognised`.
    /// * target-bearing recogniser ≺ editorial-note — `「ママ」に傍点`
    ///   must be claimed as bouten before the `ママ` editorial note would
    ///   type it as `Sic`. Pinned by
    ///   `mama_target_with_bouten_stays_bouten`.
    /// * editorial-note ≺ `Unknown` — the editorial kinds refine the
    ///   catch-all, which would otherwise claim every body. Pinned by
    ///   `editorial_notes_type_as_asis_and_textual_note`.
    /// * 縦中横 compound ≺ small-script range — `「X」は縦中横、行右小書き`
    ///   is 縦中横, not a small-script range nor `Unknown`. Pinned by
    ///   `tcy_small_script_compound_recognised_as_tcy`.
    /// * 縦中横 `ShapedNoTarget` diagnostic survives the fall-through —
    ///   when the target is absent the directive degrades to
    ///   `Directive{Unknown}`, but its `tcy_target_not_found` warning is
    ///   carried through the later arms via `tcy_pending`. Pinned by
    ///   `tcy_target_not_found_fires_as_warning` (node-absence by
    ///   `forward_tcy_without_preceding_target_falls_through`).
    #[allow(
        clippy::too_many_lines,
        reason = "a flat dispatch chain over the forward-reference recognisers \
                  (body / bouten / 縦中横 / heading / emphasis) — each block is \
                  the same shape and splitting them would scatter the ordered \
                  fall-through to the Directive{Unknown} catch-all"
    )]
    pub(super) fn recognize_annotation(
        &mut self,
        view: BodyView<'_>,
        open_idx: usize,
        close_idx: usize,
    ) -> Option<AnnotationMatch<'a>> {
        #[cfg(feature = "classify-instrument")]
        let _phase3_guard = SubsystemGuard::new(Subsystem::Directive);
        let events = view.events;
        let PairEvent::PairOpen {
            span: open_span, ..
        } = events[open_idx]
        else {
            return None;
        };
        let PairEvent::PairClose {
            span: close_span, ..
        } = events[close_idx]
        else {
            return None;
        };

        // The next event must be `＃`. `open_idx + 1 < close_idx` is
        // guaranteed whenever the hash exists, and `close_idx > open_idx`
        // always holds for a surviving PairOpen.
        let hash_end = match events.get(open_idx + 1)? {
            PairEvent::Solo {
                kind: TriggerKind::Hash,
                span,
            } => span.end,
            _ => return None,
        };

        // Body bytes are everything between `＃` and `］`. Trim leading /
        // trailing ASCII whitespace to be resilient to malformed input
        // like `［＃ 改ページ  ］`; Aozora spec does not officially allow
        // such whitespace but the corpus contains stragglers.
        let body = self.source[hash_end as usize..close_span.start as usize].trim();

        // Body-keyword classifier. Cannot be `or_else`d with the forward
        // ones because each step needs the same `&mut alloc` borrow; we
        // run them sequentially with explicit early returns instead.
        if let Some((emit, annotation_payload)) = classify_annotation_body(body, self.alloc) {
            return Some(AnnotationMatch {
                emit,
                // For Warichu open / close the body classifier hands back
                // a payload alongside the node; the body-builder uses it to
                // wrap as a `Segment::Directive` with the correct
                // `WarichuOpen` / `WarichuClose` kind instead of the
                // catch-all `Unknown` downgrade. Other body-keyword
                // families (PageBreak, Indent, …) leave the payload as
                // `None`, matching the legacy behaviour where the body-
                // builder fell through to its `Directive{Unknown}`
                // synthesis path.
                annotation_payload,
                consume_start: open_span.start,
                consume_end: close_span.end,
                pending_diagnostic: None,
            });
        }
        // Forward-reference warnings point at the whole `［＃…］` directive,
        // not at the (possibly pulled-back) consume span.
        let directive_span = Span::new(open_span.start, close_span.end);
        if let Some((node, consume_start, ambiguous)) =
            self.classify_forward_bouten(view, open_idx, close_idx)
        {
            return Some(AnnotationMatch {
                emit: EmitKind::Aozora(node),
                annotation_payload: None,
                consume_start,
                consume_end: close_span.end,
                pending_diagnostic: ambiguous
                    .then(|| Diagnostic::bouten_target_ambiguous(directive_span)),
            });
        }
        // Range bouten `「X」～「Y」に<kind>` — applies the marks to the whole
        // preceding run from X to Y.
        if let Some((node, consume_start)) =
            self.classify_forward_bouten_range(view, open_idx, close_idx)
        {
            return Some(AnnotationMatch {
                emit: EmitKind::Aozora(node),
                annotation_payload: None,
                consume_start,
                consume_end: close_span.end,
                pending_diagnostic: None,
            });
        }
        // `「X」の左に「Y」のルビ` — left-side ruby (saidoku building block). The
        // `の左に` prefix overlaps left-side bouten, but its `のルビ` keyword is
        // not a bouten kind, so the bouten classifier already returned `None`.
        if let Some((node, consume_start)) =
            self.classify_forward_left_ruby(view, open_idx, close_idx)
        {
            return Some(AnnotationMatch {
                emit: EmitKind::Aozora(node),
                annotation_payload: None,
                consume_start,
                consume_end: close_span.end,
                pending_diagnostic: None,
            });
        }
        // `「X」…の注記` / `「X」に「Y」の傍記` — side annotation (注記 / 傍記).
        // The `の注記` / `の傍記` keywords are disjoint from `のルビ` and every
        // bouten kind, so the bouten and left-ruby classifiers above have
        // already declined.
        if let Some((node, consume_start)) =
            self.classify_forward_side_note(view, open_idx, close_idx)
        {
            return Some(AnnotationMatch {
                emit: EmitKind::Aozora(node),
                annotation_payload: None,
                consume_start,
                consume_end: close_span.end,
                pending_diagnostic: None,
            });
        }
        // 縦中横 is 3-state: a shape-matched directive whose target has no
        // referent (`ShapedNoTarget`) carries a warning down the
        // fall-through path while still degrading to `Directive{Unknown}`.
        let tcy_pending = match self.classify_forward_tcy(view, open_idx, close_idx) {
            ForwardTcy::Recognised(node, consume_start) => {
                return Some(AnnotationMatch {
                    emit: EmitKind::Aozora(node),
                    annotation_payload: None,
                    consume_start,
                    consume_end: close_span.end,
                    pending_diagnostic: None,
                });
            }
            ForwardTcy::ShapedNoTarget => Some(Diagnostic::tcy_target_not_found(directive_span)),
            ForwardTcy::NotTcy => None,
        };
        if let Some((node, consume_start)) =
            self.classify_forward_heading(view, open_idx, close_idx)
        {
            return Some(AnnotationMatch {
                emit: EmitKind::Aozora(node),
                annotation_payload: None,
                consume_start,
                consume_end: close_span.end,
                pending_diagnostic: tcy_pending,
            });
        }
        if let Some((node, consume_start)) =
            self.classify_forward_emphasis(view, open_idx, close_idx)
        {
            return Some(AnnotationMatch {
                emit: EmitKind::Aozora(node),
                annotation_payload: None,
                consume_start,
                consume_end: close_span.end,
                pending_diagnostic: tcy_pending,
            });
        }

        // `「caption」のキャプション付きの(図|挿絵)（file）入る` — illustration
        // whose caption precedes the figure. Emits a Illustration (consumes the whole
        // bracket); checked here, after the styling recognisers.
        if let Some(node) = self.classify_caption_figure(view, open_idx, close_idx) {
            return Some(AnnotationMatch {
                emit: EmitKind::Aozora(node),
                annotation_payload: None,
                consume_start: open_span.start,
                consume_end: close_span.end,
                pending_diagnostic: None,
            });
        }

        // General image form `［＃<説明>（file［、横W×縦H］）入る］` (graphics.html)
        // — the free leading description (図 / 地図 / コンドル博士の図 …) is the
        // alt. Its description is arbitrary so it has no prefix needle in the
        // body dispatcher; checked here, after the more-specific 「caption」
        // form so that keeps its figcaption, and before the Unknown catch-all.
        if let Some(emit) = classify_general_image_body(body, self.alloc) {
            return Some(AnnotationMatch {
                emit,
                annotation_payload: None,
                consume_start: open_span.start,
                consume_end: close_span.end,
                pending_diagnostic: None,
            });
        }

        // Empty / placeholder directives from the file-header 凡例 that
        // prefixes nearly every work: `［＃］` (入力者注), `［＃…］` (返り点),
        // `［＃（…）］` (訓点送り仮名). The body is empty (post-trim) or an
        // ellipsis placeholder — a de-facto-standard symbol, not unrecognised
        // notation. Type it as `Empty` rather than the `Unknown` catch-all.
        if body.is_empty() || body == "…" || body == "（…）" {
            return Some(self.typed_annotation_match(directive_span, DirectiveKind::Empty));
        }
        // Input-editor notes (`「X」はママ`, `「X」は底本では「Y」`, …). These
        // do not restyle their target — X stays in the text — so they emit a
        // typed `Directive` consuming only the bracket, exactly like the
        // Unknown catch-all but with the correct kind. Checked here, after the
        // specialised recognisers, so `「ママ」に傍点` etc. are already claimed.
        if let Some(kind) = editorial_note_kind(body) {
            return Some(self.typed_annotation_match(directive_span, kind));
        }

        // Standalone external-character (#122): a no-`※` `［＃…］` whose body
        // is a gaiji description with a trailing mencode / 底本ページ-行
        // (`「※」は「祿－示」、第3水準1-84-27、144-上-9`, `「比」の「ヒ」に代えて「く」、
        // 第4水準2-1-23`). Checked after the editorial notes so a `底本では` note
        // with a coincidental code-like tail stays editorial, and just before
        // the Unknown catch-all. `classify_standalone_gaiji` requires a
        // resolvable glyph or a mencode/page-line tail, so an ordinary
        // `［＃「…」］` note is never claimed.
        if let Some((node, unresolved)) = self.classify_standalone_gaiji(view, open_idx) {
            return Some(AnnotationMatch {
                emit: EmitKind::Aozora(node),
                annotation_payload: None,
                consume_start: open_span.start,
                consume_end: close_span.end,
                pending_diagnostic: unresolved
                    .then(|| Diagnostic::unresolved_gaiji(directive_span)),
            });
        }

        // No specialised recogniser claimed the bracket — fall back to the
        // `Directive{Unknown}` catch-all.
        Some(self.unknown_annotation_match(directive_span, body, tcy_pending))
    }

    /// Standalone (no-`※`) external-character recogniser (#122). Reuses the
    /// gaiji body parser with the bracket open as the consume start, and
    /// declines unless the body resolves to a glyph or carries a mencode /
    /// 底本ページ-行 tail — so an ordinary `［＃「…」］` note is never claimed.
    /// Returns the gaiji node and whether it stayed unresolved.
    fn classify_standalone_gaiji(
        &mut self,
        view: BodyView<'_>,
        open_idx: usize,
    ) -> Option<(borrowed::Node<'a>, bool)> {
        let &PairEvent::PairOpen {
            span: open_span, ..
        } = view.events.get(open_idx)?
        else {
            return None;
        };
        let m = self.recognize_gaiji_core(view, open_span.start, true, open_idx)?;
        // Standalone has no `※` disambiguator, so an unqualified `［＃「…」］`
        // is an ordinary note. Require a resolved glyph or a mencode /
        // page-line tail before claiming it as a gaiji.
        if !m.payload.canonical.has_mencode() && m.payload.resolve().is_none() {
            return None;
        }
        let unresolved = m.payload.resolve().is_none();
        Some((self.alloc.gaiji(m.payload), unresolved))
    }

    /// Catch-all for any well-formed `［＃…］` whose body no specialised
    /// recogniser claimed — including empty bodies (`［＃］`), which real
    /// Aozora corpora occasionally use as illustrative glyphs. Emitting
    /// `Directive{Unknown}` with the raw source slice keeps the Tier-A
    /// canary (no bare `［＃` in HTML output) intact. `directive_span` is the
    /// `open.start..close.end` extent of the bracket.
    ///
    /// `tcy_pending` carries a 縦中横-target-not-found warning down from the
    /// fall-through path; otherwise a `ここから…` opener that no
    /// `ContainerKind` claimed surfaces as `unrecognised_container_directive`
    /// (`罫囲み` / `割り注` openers don't use the `ここから` prefix and are
    /// handled upstream, so the prefix uniquely flags a stray container open).
    fn unknown_annotation_match(
        &mut self,
        directive_span: Span,
        body: &str,
        tcy_pending: Option<Diagnostic>,
    ) -> AnnotationMatch<'a> {
        let raw = &self.source[directive_span.start as usize..directive_span.end as usize];
        // One payload for `emit`, one for `annotation_payload`, so the
        // body-builder can re-wrap without re-interning the raw string.
        let payload = self.alloc.make_directive(raw, DirectiveKind::Unknown);
        let node = self.alloc.annotation(payload);
        let payload_for_seg = self.alloc.make_directive(raw, DirectiveKind::Unknown);
        let pending_diagnostic = tcy_pending.or_else(|| {
            body.starts_with("ここから")
                .then(|| Diagnostic::unrecognised_container_directive(directive_span))
        });
        AnnotationMatch {
            emit: EmitKind::Aozora(node),
            annotation_payload: Some(payload_for_seg),
            consume_start: directive_span.start,
            consume_end: directive_span.end,
            pending_diagnostic,
        }
    }

    /// Emit a `［＃…］` as an `Directive` carrying a *specific* kind
    /// (`Sic`, `BaseTextVariant`, …) rather than the `Unknown` catch-all.
    /// Same span discipline as [`Self::unknown_annotation_match`] — the
    /// whole bracket is consumed and the target text is left in place — so
    /// the raw round-trips and the HTML output is unchanged (these kinds
    /// render hidden, as Unknown does); only the typed `DirectiveKind` the
    /// AST / wire surfaces differs.
    fn typed_annotation_match(
        &mut self,
        directive_span: Span,
        kind: DirectiveKind,
    ) -> AnnotationMatch<'a> {
        let raw = &self.source[directive_span.start as usize..directive_span.end as usize];
        let payload = self.alloc.make_directive(raw, kind);
        let node = self.alloc.annotation(payload);
        let payload_for_seg = self.alloc.make_directive(raw, kind);
        AnnotationMatch {
            emit: EmitKind::Aozora(node),
            annotation_payload: Some(payload_for_seg),
            consume_start: directive_span.start,
            consume_end: directive_span.end,
            pending_diagnostic: None,
        }
    }
}

/// Classify a `［＃「target」に<bouten-kind>］` forward-reference
/// bouten annotation.
///
/// Uses the event-stream layout to find the target quote pair,
/// avoiding the string-find-first-`」` pitfall when the target text
/// itself contains nested `「…」`. The pair stage has already balanced the
/// quotes so the target's extent is unambiguous.
///
/// Expected event layout for a valid forward bouten:
///
/// ```text
/// open_idx         PairOpen(Bracket)
/// open_idx + 1     Solo(Hash)                [already verified]
/// open_idx + 2     PairOpen(Quote, close=Q)
/// …                body events               [usually just Text]
/// Q                PairClose(Quote)
/// Q+1..close_idx   suffix events             [usually Text("に…")]
/// close_idx        PairClose(Bracket)
/// ```
impl<'a> RecogniseCtx<'_, 'a, '_> {
    /// Classify a range bouten `「X」～「Y」に<kind>` (also `〜`): apply the
    /// marks to the whole preceding run from the start of X to the end of Y,
    /// which butts against the bracket. Returns `(node, consume_start)` with
    /// `consume_start` at X so the styled span is the run's sole rendered copy.
    fn classify_forward_bouten_range(
        &mut self,
        view: BodyView<'_>,
        open_idx: usize,
        close_idx: usize,
    ) -> Option<(borrowed::Node<'a>, u32)> {
        let extracted = extract_forward_quote_targets(view, self.source, open_idx, close_idx)?;
        // Extraction stops at the `～`, so the first quote is X; the suffix is
        // `～「Y」に<kind>` (or `〜「Y」…`).
        let [start_target] = extracted.targets.as_slice() else {
            return None;
        };
        let rest = extracted
            .suffix
            .strip_prefix("～「")
            .or_else(|| extracted.suffix.strip_prefix("〜「"))?;
        let (end_target, after) = rest.split_once('」')?;
        if start_target.is_empty() || end_target.is_empty() {
            return None;
        }
        let (position, kind_suffix) = if let Some(r) = after.strip_prefix("に") {
            (BoutenPosition::Right, r)
        } else if let Some(r) = after.strip_prefix("の左に") {
            (BoutenPosition::Left, r)
        } else {
            return None;
        };
        let kind = bouten_kind_from_suffix(kind_suffix)?;
        let &PairEvent::PairOpen {
            span: open_span, ..
        } = view.events.get(open_idx)?
        else {
            return None;
        };
        // The run ends at the bracket with Y; its start is the last X before Y.
        let cutoff = open_span.start as usize;
        let prefix = &self.source[..cutoff];
        if !prefix.ends_with(end_target) {
            return None;
        }
        let y_start = cutoff - end_target.len();
        let x_start = self.source[..y_start].rfind(start_target)?;
        let phrase = &self.source[x_start..cutoff];
        let content = self.alloc.content_plain(phrase);
        Some((
            self.alloc.bouten(kind, content, position, true),
            u32::try_from(x_start).ok()?,
        ))
    }

    fn classify_forward_bouten(
        &mut self,
        view: BodyView<'_>,
        open_idx: usize,
        close_idx: usize,
    ) -> Option<(borrowed::Node<'a>, u32, bool)> {
        let extracted = extract_forward_quote_targets(view, self.source, open_idx, close_idx)?;
        // Shape 1: `に<kind>` — default right-side placement.
        // Shape 2: `の左に<kind>` — left-side placement (position flipped).
        let (position, kind_suffix) = if let Some(rest) = extracted.suffix.strip_prefix("に") {
            (BoutenPosition::Right, rest)
        } else if let Some(rest) = extracted.suffix.strip_prefix("の左に") {
            (BoutenPosition::Left, rest)
        } else {
            return None;
        };
        let kind = bouten_kind_from_suffix(kind_suffix)?;
        // A forward-reference bouten only makes sense when every named
        // target actually appears in the preceding text. Otherwise it
        // has no referent and we fall through to the Directive{Unknown}
        // catch-all so the reader sees the raw `［＃…］` rather than a
        // mysterious styling applied to nothing. Each target is checked
        // independently so a partially-valid multi-quote bracket (rare
        // but present in corpora) still fails cleanly.
        for target in &extracted.targets {
            if !forward_target_is_preceded(view.events, self.source, open_idx, target) {
                return None;
            }
        }
        // Pull the consume span back to swallow the preceding literal
        // target when the (single) target sits *immediately* before the
        // `［`. Without this the canonical
        //     <target>［＃「<target>」に傍点］
        // renders as `<target><em class="bouten">…<target>…</em>` —
        // the surrounding plain run still carries the raw literal and
        // the renderer faithfully emits the bouten's own content,
        // producing the visible duplication that bit the playground
        // welcome page. Letting `try_bracket_emit::flush_plain_up_to`
        // see the earlier `consume_start` is the same trick Ruby has
        // always used to claim its base text (see `try_ruby_emit`).
        //
        // Multi-target (`「A」「B」`) deliberately stays on the legacy
        // `open_span.start` consume — the targets are non-contiguous
        // in the source (e.g. `AとB`) and the current truncating
        // `flush_plain_up_to` API cannot splice a hole in the middle
        // of a pending plain run. That shape is the rarer corpus
        // pattern; we accept the duplication there until a future
        // change teaches the lexer to splice rather than truncate.
        let &PairEvent::PairOpen {
            span: open_span, ..
        } = view.events.get(open_idx)?
        else {
            return None;
        };
        let consume_start = if let [only] = extracted.targets.as_slice() {
            find_immediate_predecessor_target_position(view.events, self.source, open_idx, only)
                .unwrap_or(open_span.start)
        } else {
            open_span.start
        };
        let consumed_predecessor = consume_start < open_span.start;
        // Ambiguity: a *single* target that occurs more than once in the
        // look-back window has no unique referent. Multi-target brackets
        // (`「A」「B」`) name distinct runs and are not "ambiguous" in this
        // sense, so they never flag. `matches` counts non-overlapping
        // occurrences, which is the right notion of "candidate runs".
        let ambiguous = if let [only] = extracted.targets.as_slice() {
            self.source[..open_span.start as usize]
                .matches(only)
                .count()
                >= 2
        } else {
            false
        };
        let target = build_bouten_target(&extracted.targets, self.alloc);
        Some((
            self.alloc
                .bouten(kind, target, position, consumed_predecessor),
            consume_start,
            ambiguous,
        ))
    }
}

/// Fold a list of forward-bouten target strings into a single
/// `Content`. A one-element list takes the `Content::from(&str)`
/// fast path (the overwhelmingly common case); multi-target lists
/// build a `Segments` run where inter-target separators are modelled
/// as `Segment::Text("、")` so the renderer emits
/// `<em>A、B</em>` in document order.
///
/// Using `、` as the glue is a deliberate, lossy choice: the raw
/// source shape `「A」「B」` does not have an explicit separator, but
/// inserting one in the rendered output makes the targets readable
/// without requiring a dedicated `Segment::Separator` variant (which
/// would ripple through every renderer / serializer). Callers that
/// need the per-target list can walk `Content::iter` and filter on
/// `SegmentRef::Text`.
fn build_bouten_target<'a>(
    targets: &[&str],
    alloc: &mut BorrowedAllocator<'a>,
) -> borrowed::Content<'a> {
    match targets {
        [] => alloc.content_plain(""),
        [only] => alloc.content_plain(only),
        many => {
            let mut segs: Vec<borrowed::Segment<'a>> = Vec::with_capacity(many.len() * 2 - 1);
            for (i, t) in many.iter().enumerate() {
                if i > 0 {
                    segs.push(alloc.seg_text("、"));
                }
                segs.push(alloc.seg_text(t));
            }
            alloc.content_segments(&segs)
        }
    }
}

/// Outcome of [`RecogniseCtx::classify_forward_tcy`].
///
/// Distinguishes a recognised 縦中横 from a directive whose `は縦中横`
/// shape matched but whose target is absent from the look-back (which
/// still warrants a `tcy_target_not_found` warning even though the
/// bracket degrades to `Directive{Unknown}`), and from a bracket that is
/// not a 縦中横 directive at all (silent fall-through).
enum ForwardTcy<'a> {
    /// A 縦中横 with a located target — the node plus its consume start.
    Recognised(borrowed::Node<'a>, u32),
    /// `は縦中横` shape matched but the target has no preceding referent.
    ShapedNoTarget,
    /// Not a 縦中横 directive.
    NotTcy,
}

/// Classify a `［＃「target」は縦中横］` forward-reference
/// tate-chu-yoko (horizontal-in-vertical) annotation.
///
/// Same event-layout expectations as forward bouten, except the
/// suffix uses the particle `は` and the keyword `縦中横`. Paired
/// form (`［＃縦中横］…［＃縦中横終わり］`) is handled by the
/// paired-container classifier and not matched here.
///
/// Multi-quote `［＃「A」「B」は縦中横］` bodies are not standard Aozora
/// spec; we accept the first target's text and ignore the rest for
/// robustness rather than failing, so the bracket still consumes via
/// `classify_forward_tcy` instead of leaking to `Directive{Unknown}`.
impl<'a> RecogniseCtx<'_, 'a, '_> {
    fn classify_forward_tcy(
        &mut self,
        view: BodyView<'_>,
        open_idx: usize,
        close_idx: usize,
    ) -> ForwardTcy<'a> {
        let Some(extracted) = extract_forward_quote_targets(view, self.source, open_idx, close_idx)
        else {
            return ForwardTcy::NotTcy;
        };
        // `は縦中横` and the corpus compound `は縦中横、行右/左小書き` (numbered
        // list markers like 「１）」 set horizontal *and* small). Recognise the
        // compound as 縦中横 — the dominant transform; the small-script
        // fine-positioning normalises away on serialize (idempotent).
        if !matches!(
            extracted.suffix,
            "は縦中横" | "は縦中横、行右小書き" | "は縦中横、行左小書き"
        ) {
            return ForwardTcy::NotTcy;
        }
        let Some(first) = extracted.targets.first() else {
            return ForwardTcy::NotTcy;
        };
        // The shape is a 縦中横 directive. If its target has no referent in
        // the preceding text the styling is meaningless — flag it (the
        // caller turns this into `tcy_target_not_found`) and let the bracket
        // fall through to `Directive{Unknown}`.
        if !forward_target_is_preceded(view.events, self.source, open_idx, first) {
            return ForwardTcy::ShapedNoTarget;
        }
        // Same `consume_start` shrink as `classify_forward_bouten`: pull
        // the span back to swallow the immediately-preceding literal so
        // `昭和64［＃「64」は縦中横］年` renders as `昭和<span
        // class="tcy">64</span>年` instead of doubling the digits.
        let Some(&PairEvent::PairOpen {
            span: open_span, ..
        }) = view.events.get(open_idx)
        else {
            return ForwardTcy::NotTcy;
        };
        let consume_start =
            find_immediate_predecessor_target_position(view.events, self.source, open_idx, first)
                .unwrap_or(open_span.start);
        let consumed_predecessor = consume_start < open_span.start;
        let text = self.alloc.content_plain(first);
        ForwardTcy::Recognised(
            self.alloc.tate_chu_yoko(text, consumed_predecessor),
            consume_start,
        )
    }
}

/// Check whether `target` appears somewhere in the source preceding the
/// `［` event at `open_idx`. Used by forward-reference recognisers to
/// suppress `［＃「X」…］` spans whose target has no referent.
///
/// Returns `false` if the event shape isn't the expected `PairOpen`
/// (defensive — the caller is responsible for having picked a valid
/// bracket, so this only fails if invariants drift).
fn forward_target_is_preceded(
    events: &[PairEvent],
    source: &str,
    open_idx: usize,
    target: &str,
) -> bool {
    #[cfg(feature = "classify-instrument")]
    let _phase3_guard = SubsystemGuard::new(Subsystem::ForwardTargetCheck);
    let Some(PairEvent::PairOpen { span, .. }) = events.get(open_idx) else {
        return false;
    };
    let cutoff = span.start;

    // Hot path: a pre-built per-classify Aho-Corasick index covers the
    // target in O(1). Only installed when the doc has enough forward-
    // reference targets to amortise the AC build (see
    // `install_forward_target_index` and `FORWARD_AC_THRESHOLD`).
    let indexed = FORWARD_TARGET_INDEX.with(|cell| {
        let state = cell.borrow();
        if !state.installed {
            return None;
        }
        Some(matches!(state.first_position.get(target), Some(&first_pos) if first_pos < cutoff))
    });
    if let Some(decided) = indexed {
        return decided;
    }

    // Fallback: median corpus doc has too few forward-reference
    // targets to make the AC build worthwhile. Pay the legacy
    // substring scan instead.
    source[..cutoff as usize].contains(target)
}

/// Ruby-tolerant variant of [`forward_target_is_preceded`], for the heading
/// hint gate only. Tests whether `target` occurs in the look-back source
/// `source[..cutoff]` once ruby readings (`《…》`) and explicit-base `｜`
/// markers are stripped.
///
/// A heading whose quoted target is the ruby-*stripped* form of a preceding
/// run — `○　両頭《りやうとう》の蛇《へび》［＃「○　両頭の蛇」は中見出し］` — is not
/// a contiguous source substring, so the exact gate misses it (the AC index
/// misses it too, recording such a target at its own directive quote past
/// `cutoff`). Stripping is done over the raw source rather than the event
/// stream because the recogniser's [`BodyView`] is scoped to the bracket body
/// and does not carry the preceding run's ruby events. Headings are rare, so
/// the per-call `String` is immaterial.
///
/// Heading-only: bouten / emphasis / left-ruby pull their styled span back
/// over the literal predecessor (`find_immediate_predecessor_target_position`)
/// and must not see a ruby-stripped match, so they keep the exact gate.
fn forward_heading_target_is_preceded_ruby_stripped(
    view: BodyView<'_>,
    source: &str,
    open_idx: usize,
    target: &str,
) -> bool {
    let Some(&PairEvent::PairOpen { span, .. }) = view.events.get(open_idx) else {
        return false;
    };
    let prefix = &source[..span.start as usize];

    // Drop every `《reading》` span and explicit-base `｜`. A `《` with no
    // closing `》` leaves `in_ruby` set, dropping the tail — a ruby-less real
    // heading is unaffected, and an unmatched `《` only suppresses a match
    // (never invents one), so the gate stays conservative.
    let mut stripped = String::with_capacity(prefix.len());
    let mut in_ruby = false;
    for ch in prefix.chars() {
        match ch {
            '《' => in_ruby = true,
            '》' => in_ruby = false,
            '｜' => {}
            _ if in_ruby => {}
            _ => stripped.push(ch),
        }
    }
    stripped.contains(target)
}

/// Like [`forward_target_is_preceded`] but stricter: returns the byte
/// position where `target` begins **only if** that target's end butts
/// directly against the `［` event at `open_idx`. Used by the
/// `classify_forward_*` family to compute the `consume_start` they
/// hand back to `try_bracket_emit`, so the flush truncates the
/// pending plain run *just past* the matched literal and the styled
/// span becomes its sole rendered copy.
///
/// Returns `None` when:
/// - the event at `open_idx` is not a `PairOpen` (defensive),
/// - the bracket sits at byte offset < `target.len()` (no room),
/// - or the bytes immediately before the bracket differ from
///   `target` (the target lives mid-sentence or is not preceded at
///   all — leave the legacy duplicating behaviour in place rather
///   than splicing a hole into the middle of the plain run).
fn find_immediate_predecessor_target_position(
    events: &[PairEvent],
    source: &str,
    open_idx: usize,
    target: &str,
) -> Option<u32> {
    let &PairEvent::PairOpen { span, .. } = events.get(open_idx)? else {
        return None;
    };
    let cutoff = span.start as usize;
    let len = target.len();
    if cutoff < len {
        return None;
    }
    let candidate_start = cutoff - len;
    // Compare bytes; `target` is already canonical UTF-8 (extracted
    // from `extract_forward_quote_targets`), so equality of byte
    // slices is equivalent to equality of strings.
    if &source.as_bytes()[candidate_start..cutoff] == target.as_bytes() {
        Some(u32::try_from(candidate_start).ok()?)
    } else {
        None
    }
}

/// Result of walking the `［＃「…」「…」…<particle><keyword>］`
/// shape. `targets` holds each non-empty quote body in document order
/// (length `>= 1` when `Some(_)` is returned) and `suffix` is the
/// trimmed source between the last quote's `」` and the bracket's `］`,
/// ready for particle + keyword matching.
struct ForwardQuoteExtract<'s> {
    /// Inline capacity 4 covers the corpus 99th percentile — most
    /// forward-reference annotations have a single quoted target,
    /// the long tail rarely exceeds 2-3.
    targets: smallvec::SmallVec<[&'s str; 4]>,
    suffix: &'s str,
}

/// Shared helper for the `［＃「X」…<particle><keyword>］` shape.
///
/// Walks consecutive quote pairs immediately after the `＃` and
/// stops when the next event is *not* another `PairOpen(Quote)`.
/// Returns the collected target list together with the trimmed
/// suffix so callers can match on the particle + keyword portion.
///
/// Returns `None` if any shape assumption fails: no adjacent quote
/// pair, first quote empty, or the initial quote crossing out of the
/// bracket. Subsequent empty quote bodies are silently skipped
/// (defensive against `「」` placeholders in real corpora) rather
/// than aborting the recognition.
fn extract_forward_quote_targets<'s>(
    view: BodyView<'_>,
    source: &'s str,
    open_idx: usize,
    close_idx: usize,
) -> Option<ForwardQuoteExtract<'s>> {
    let events = view.events;
    let &PairEvent::PairClose {
        span: bracket_close_span,
        ..
    } = events.get(close_idx)?
    else {
        return None;
    };

    let mut targets: smallvec::SmallVec<[&'s str; 4]> = smallvec::SmallVec::new();
    let mut cursor = open_idx + 2; // skip `［` and `＃`
    let mut last_quote_end: u32 = 0;

    while let Some(&PairEvent::PairOpen {
        kind: PairKind::Quote,
        span: quote_open_span,
    }) = events.get(cursor)
    {
        // Look up the quote's matching close via the side-table. An
        // unmatched/orphan PairOpen has `links[cursor] == u32::MAX`,
        // which we treat as "not nested inside this bracket" and bail.
        let quote_close_link = *view.links.get(cursor)?;
        if quote_close_link == u32::MAX {
            return None;
        }
        let quote_close_idx = quote_close_link as usize;
        // The quote must close *before* the bracket — a cross-boundary
        // close would mean the quote is not nested inside the bracket.
        if quote_close_idx >= close_idx {
            return None;
        }
        let Some(&PairEvent::PairClose {
            span: quote_close_span,
            ..
        }) = events.get(quote_close_idx)
        else {
            return None;
        };
        // Empty quotes are tolerated in-position but not added to the
        // target list — they carry no semantic content.
        let body = &source[quote_open_span.end as usize..quote_close_span.start as usize];
        if !body.is_empty() {
            targets.push(body);
        }
        last_quote_end = quote_close_span.end;
        cursor = quote_close_idx + 1;
    }

    if targets.is_empty() {
        return None;
    }
    let suffix = source[last_quote_end as usize..bracket_close_span.start as usize].trim();
    Some(ForwardQuoteExtract { targets, suffix })
}

/// Classify a `［＃「target」は(大|中|小)見出し］` forward-reference
/// heading annotation.
///
/// Shares the event-stream extraction helper with `classify_forward_bouten`
/// — the quote-delimited target and the trailing keyword live in the same
/// `［＃「X」…］` shape. The suffix after the target must start with `は`
/// (unlike bouten's `に`), and the keyword selects the Markdown heading
/// level: `大見出し` → 1, `中見出し` → 2, `小見出し` → 3.
///
/// When the (single) referent is the bare line immediately above the
/// directive, the line is promoted in place to a block
/// `borrowed::Heading` (大→`<h1>` / 中→`<h2>` / 小→`<h3>`): the
/// consume span is pulled back over that line so the heading element is
/// its sole rendered copy. When the referent is not a clean preceding
/// line, the classifier keeps the inline `borrowed::HeadingHint` marker
/// at the directive position (information-preserving, never promoted to
/// an empty or misplaced heading).
///
/// Same `forward_target_is_preceded` gate as forward bouten: a heading
/// hint that names a target which does not appear in the preceding
/// source text is rejected — the annotation has no referent and the
/// paragraph would promote to an empty heading. Falling through lets
/// the catch-all emit `Directive { Unknown }` so the reader at least
/// sees the raw bracket text in diagnostics.
impl<'a> RecogniseCtx<'_, 'a, '_> {
    fn classify_forward_heading(
        &mut self,
        view: BodyView<'_>,
        open_idx: usize,
        close_idx: usize,
    ) -> Option<(borrowed::Node<'a>, u32)> {
        let extracted = extract_forward_quote_targets(view, self.source, open_idx, close_idx)?;
        let rest = extracted.suffix.strip_prefix("は")?;
        let (style, kind) = parse_heading_keyword(rest)?;

        // Reject hints whose targets are not preceded by matching text.
        // See `classify_forward_bouten` for the same rationale. Exact
        // look-back first (cheap; uses the AC index when installed); on a
        // miss, retry against a ruby-stripped copy of the look-back, since a
        // heading title carrying ruby (`両頭《りやうとう》`) has its
        // ruby-stripped target (`両頭`) quoted in the directive and so is not
        // a contiguous source substring.
        for target in &extracted.targets {
            if target.is_empty() {
                continue;
            }
            if forward_target_is_preceded(view.events, self.source, open_idx, target) {
                continue;
            }
            if forward_heading_target_is_preceded_ruby_stripped(view, self.source, open_idx, target)
            {
                continue;
            }
            return None;
        }

        let &PairEvent::PairOpen {
            span: open_span, ..
        } = view.events.get(open_idx)?
        else {
            return None;
        };

        // Promote when the single referent is the bare line right above
        // the directive: pull the consume span back over `序章\n` so the
        // `<hN>` (carrying the heading text) is the sole rendered copy.
        if let [only] = extracted.targets.as_slice()
            && let Some(consume_start) =
                find_heading_predecessor_position(view.events, self.source, open_idx, only)
        {
            let text = self.alloc.content_plain(only);
            let node = self.alloc.aozora_heading(kind, style, text);
            return Some((node, consume_start));
        }

        // Fallback: keep the inline hint marker at the directive position.
        // Concatenate targets in the (rare) multi-quote case so the full
        // named run drives the hint content. The 同行 / 窓 styles run into
        // the body on their own line, so they land here rather than promoting.
        let combined: String = extracted.targets.iter().copied().collect();
        if combined.is_empty() {
            return None;
        }
        Some((
            self.alloc.heading_hint(kind, style, &combined),
            open_span.start,
        ))
    }
}

/// Classify a `「X」の左に「Y」のルビ` forward-reference **left-side ruby** — the
/// building block of a 再読文字 (saidoku-moji). The target `X` is pulled back
/// (mirroring `classify_forward_bouten`); the reading `Y` attaches on the left.
/// Single-target only; the `の左に「…」のルビ` suffix shape is unique, so a
/// non-ruby `の左に…` (left-side bouten) never reaches here.
impl<'a> RecogniseCtx<'_, 'a, '_> {
    fn classify_forward_left_ruby(
        &mut self,
        view: BodyView<'_>,
        open_idx: usize,
        close_idx: usize,
    ) -> Option<(borrowed::Node<'a>, u32)> {
        let extracted = extract_forward_quote_targets(view, self.source, open_idx, close_idx)?;
        let [target] = extracted.targets.as_slice() else {
            return None;
        };
        // suffix == の左に「<reading>」のルビ
        let reading_text = extracted
            .suffix
            .strip_prefix("の左に「")?
            .strip_suffix("」のルビ")?;
        if reading_text.is_empty() {
            return None;
        }
        if !forward_target_is_preceded(view.events, self.source, open_idx, target) {
            return None;
        }
        let &PairEvent::PairOpen {
            span: open_span, ..
        } = view.events.get(open_idx)?
        else {
            return None;
        };
        let consume_start =
            find_immediate_predecessor_target_position(view.events, self.source, open_idx, target)
                .unwrap_or(open_span.start);
        let base = self.alloc.content_plain(target);
        let reading = self.alloc.content_plain(reading_text);
        Some((self.alloc.left_ruby(base, reading), consume_start))
    }
}

/// Classify a forward-reference **side annotation** — 注記 or 傍記. The
/// structural twin of [`Self::classify_forward_left_ruby`] (same
/// single-target pull-back), but the trailing keyword selects a distinct
/// [`borrowed::Node::MarginNote`] node and flavour:
/// - `「X」の左に「Y」の注記` / bare `「X」に「Y」の注記` →
///   [`MarginNoteKind::Gloss`] (editorial gloss; round-trips `の注記`).
/// - `「X」に「Y」の傍記` → [`MarginNoteKind::Marginal`] (the censorship-marker
///   form; round-trips bare `に…の傍記`).
///
/// The `の注記` / `の傍記` suffixes are disjoint from `のルビ` and every
/// bouten kind, so the bouten and left-ruby classifiers above have already
/// declined.
impl<'a> RecogniseCtx<'_, 'a, '_> {
    fn classify_forward_side_note(
        &mut self,
        view: BodyView<'_>,
        open_idx: usize,
        close_idx: usize,
    ) -> Option<(borrowed::Node<'a>, u32)> {
        let extracted = extract_forward_quote_targets(view, self.source, open_idx, close_idx)?;
        let [target] = extracted.targets.as_slice() else {
            return None;
        };
        // Pick the flavour by trailing keyword, then the note text:
        //   注記: explicit `の左に「Y」の注記` or bare `に「Y」の注記` — both map
        //         to the same node (`MarginNote` has no side axis).
        //   傍記: bare `に「Y」の傍記` only (the corpus's sole 傍記 shape; an
        //         unattested `の左に…の傍記` would be ambiguous to round-trip).
        let (kind, note_text) = if let Some(inner) = extracted.suffix.strip_suffix("」の注記") {
            let note = inner
                .strip_prefix("の左に「")
                .or_else(|| inner.strip_prefix("に「"))?;
            (MarginNoteKind::Gloss, note)
        } else if let Some(inner) = extracted.suffix.strip_suffix("」の傍記") {
            (MarginNoteKind::Marginal, inner.strip_prefix("に「")?)
        } else {
            return None;
        };
        if note_text.is_empty() {
            return None;
        }
        if !forward_target_is_preceded(view.events, self.source, open_idx, target) {
            return None;
        }
        let &PairEvent::PairOpen {
            span: open_span, ..
        } = view.events.get(open_idx)?
        else {
            return None;
        };
        let consume_start =
            find_immediate_predecessor_target_position(view.events, self.source, open_idx, target)
                .unwrap_or(open_span.start);
        let base = self.alloc.content_plain(target);
        let note = self.alloc.content_plain(note_text);
        Some((self.alloc.side_note(kind, base, note), consume_start))
    }

    /// Classify a `「caption」のキャプション付きの(図|挿絵)（file）入る`
    /// illustration whose caption *precedes* the figure (distinct from the
    /// trailing `挿絵（file）「caption」入る` form `classify_sashie_body`
    /// handles). Emits a `Illustration` with the leading quote as its caption and
    /// the parenthesised path as its file; consumes only the bracket.
    fn classify_caption_figure(
        &mut self,
        view: BodyView<'_>,
        open_idx: usize,
        close_idx: usize,
    ) -> Option<borrowed::Node<'a>> {
        let extracted = extract_forward_quote_targets(view, self.source, open_idx, close_idx)?;
        let [caption] = extracted.targets.as_slice() else {
            return None;
        };
        if caption.is_empty() {
            return None;
        }
        // suffix == のキャプション付きの(図|挿絵|写真)（file）入る
        let rest = extracted.suffix.strip_prefix("のキャプション付きの")?;
        let rest = rest
            .strip_prefix("図")
            .or_else(|| rest.strip_prefix("挿絵"))
            .or_else(|| rest.strip_prefix("写真"))?;
        let rest = rest.strip_prefix('（')?;
        let close_off = rest.find('）')?;
        let file = &rest[..close_off];
        if file.is_empty() || &rest[close_off + '）'.len_utf8()..] != "入る" {
            return None;
        }
        let caption_content = self.alloc.content_plain(caption);
        Some(self.alloc.sashie(file, None, None, Some(caption_content)))
    }
}

/// Classify a `［＃「target」は太字／斜体］` forward-reference emphasis.
///
/// The `は`-form leaf counterpart of the `［＃太字］…［＃太字終わり］`
/// range container (`parse_emphasis_body`). Shares the quote-extraction
/// and predecessor-pull-back machinery with `classify_forward_tcy` /
/// `classify_forward_heading`: the suffix after the target must start with
/// `は`, and the keyword selects 太字 (`<b>`) or 斜体 (`<i>`).
///
/// Single-target only — `「A」「B」は太字` is not a real Aozora shape and
/// would not round-trip byte-exactly, so it falls through to
/// `Directive{Unknown}`. The `forward_target_is_preceded` gate rejects a
/// target with no referent (emphasis over nothing); the
/// `find_immediate_predecessor_target_position` pull-back swallows the
/// immediately-preceding literal (the dominant
/// `作者附記［＃「作者附記」は太字］` shape) so the `<b>` / `<i>` is its sole
/// rendered copy.
impl<'a> RecogniseCtx<'_, 'a, '_> {
    fn classify_forward_emphasis(
        &mut self,
        view: BodyView<'_>,
        open_idx: usize,
        close_idx: usize,
    ) -> Option<(borrowed::Node<'a>, u32)> {
        let extracted = extract_forward_quote_targets(view, self.source, open_idx, close_idx)?;
        let rest = extracted.suffix.strip_prefix("は")?;
        let kind = emphasis_kind_from_suffix(rest)?;
        let [only] = extracted.targets.as_slice() else {
            return None;
        };
        if !forward_target_is_preceded(view.events, self.source, open_idx, only) {
            return None;
        }
        let &PairEvent::PairOpen {
            span: open_span, ..
        } = view.events.get(open_idx)?
        else {
            return None;
        };
        let consume_start =
            find_immediate_predecessor_target_position(view.events, self.source, open_idx, only)
                .unwrap_or(open_span.start);
        let consumed_predecessor = consume_start < open_span.start;
        let text = self.alloc.content_plain(only);
        Some((
            self.alloc.emphasis(kind, text, consumed_predecessor),
            consume_start,
        ))
    }
}

/// Byte position where `target` begins, **only if** it is the bare line
/// immediately preceding the `［` at `open_idx` — i.e. `target` followed
/// by a single `\n`, and itself starting at a line boundary (BOF or after
/// a `\n`). The promoted heading's consume span is pulled back to this
/// position so `序章\n［＃「序章」は…見出し］` collapses into one
/// `Heading`; the mandatory `\n` keeps the serializer's round-trip
/// (`<text>\n［＃…］`) byte-identical to the source. Returns `None`
/// (→ inline `HeadingHint` fallback) for any other shape.
fn find_heading_predecessor_position(
    events: &[PairEvent],
    source: &str,
    open_idx: usize,
    target: &str,
) -> Option<u32> {
    let &PairEvent::PairOpen { span, .. } = events.get(open_idx)? else {
        return None;
    };
    let bytes = source.as_bytes();
    let cutoff = span.start as usize;
    // The heading sits on its own line directly above the directive.
    if cutoff == 0 || bytes[cutoff - 1] != b'\n' {
        return None;
    }
    let text_end = cutoff - 1;
    let len = target.len();
    if text_end < len {
        return None;
    }
    let candidate_start = text_end - len;
    if &bytes[candidate_start..text_end] != target.as_bytes() {
        return None;
    }
    // The target must occupy the whole line (BOF or preceded by `\n`).
    if candidate_start != 0 && bytes[candidate_start - 1] != b'\n' {
        return None;
    }
    u32::try_from(candidate_start).ok()
}

/// Map the keyword after `は` to an [`EmphasisKind`].
///
/// 太字 → Bold, 斜体 → Italic (per
/// <https://www.aozora.gr.jp/annotation/emphasis.html>). ゴシック体 / ゴチック
/// are corpus spellings of bold — the guide writes 太字（ゴシック） — so both
/// map to Bold and canonicalise to `太字` on serialize. 上付き小文字 →
/// `SuperScript`, 下付き小文字 → `SubScript`, 行右小書き → `SmallRight`,
/// 行左小書き → `SmallLeft`, and `N段階大きな/小さな文字` → `FontSize`
/// (per <https://www.aozora.gr.jp/annotation/etc.html>). Unknown suffixes
/// return `None` (→ `Directive{Unknown}`).
pub(super) fn emphasis_kind_from_suffix(s: &str) -> Option<EmphasisKind> {
    Some(match s {
        "太字" | "ゴシック体" | "ゴチック" => EmphasisKind::Bold,
        "斜体" => EmphasisKind::Italic,
        "上付き小文字" => EmphasisKind::SuperScript,
        "下付き小文字" => EmphasisKind::SubScript,
        "行右小書き" => EmphasisKind::SmallRight,
        "行左小書き" => EmphasisKind::SmallLeft,
        "罫囲み" => EmphasisKind::KeigakomiInline,
        "横組み" => EmphasisKind::HorizontalInline,
        "キャプション" => EmphasisKind::Caption,
        _ => return parse_font_size_suffix(s),
    })
}

/// Parse a `N段階大きな文字` / `N段階小さな文字` font-size suffix into an
/// [`EmphasisKind::FontSize`]. `大きな` yields a positive stage count,
/// `小さな` a negative one. Returns `None` for a missing/zero magnitude,
/// an `i8` overflow, or any other suffix (→ `Directive{Unknown}`).
fn parse_font_size_suffix(s: &str) -> Option<EmphasisKind> {
    let (magnitude, rest) = parse_decimal_u8_prefix(s)?;
    let steps = i8::try_from(magnitude).ok()?;
    if steps == 0 {
        return None;
    }
    match rest {
        "段階大きな文字" => Some(EmphasisKind::FontSize { steps }),
        "段階小さな文字" => Some(EmphasisKind::FontSize { steps: -steps }),
        _ => None,
    }
}
