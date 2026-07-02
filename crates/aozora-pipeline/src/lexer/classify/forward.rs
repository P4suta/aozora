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

use core::num::NonZeroI8;

use aozora_spec::Diagnostic;
use aozora_syntax::accent::{compose_accent, compose_accent_dots};
use aozora_syntax::alloc_owned::OwnedAllocator;
use aozora_syntax::format::ForwardOrigin;
use aozora_syntax::lint::canonical_directive;
use aozora_syntax::owned::{ContentOwned, NodeOwned, SegmentOwned};
use aozora_syntax::{
    AbsoluteSize, AccentMark, BoutenPosition, DirectiveKind, EnclosureKind, FontShift, ForwardAttr,
    MarginNoteKind, Span,
};

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
impl RecogniseCtx<'_, '_> {
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
    ) -> Option<AnnotationMatch> {
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
        if let Some((node, consume_start, diag)) =
            self.classify_forward_bouten(view, open_idx, close_idx)
        {
            return Some(AnnotationMatch {
                emit: EmitKind::Aozora(node),
                annotation_payload: None,
                consume_start,
                consume_end: close_span.end,
                pending_diagnostic: diag.into_diagnostic(directive_span),
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
            ForwardTcy::Recognised(node, consume_start, diag) => {
                return Some(AnnotationMatch {
                    emit: EmitKind::Aozora(node),
                    annotation_payload: None,
                    consume_start,
                    consume_end: close_span.end,
                    pending_diagnostic: diag.into_diagnostic(directive_span),
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
        // `「X」は「□」囲み` — box-character enclosure (§6.7). Its `は「…」囲み`
        // suffix is disjoint from every emphasis keyword, so ordering versus
        // emphasis is free; kept adjacent as another `は`-form leaf.
        if let Some((node, consume_start, diag)) =
            self.classify_forward_box_enclosure(view, open_idx, close_idx)
        {
            return Some(AnnotationMatch {
                emit: EmitKind::Aozora(node),
                annotation_payload: None,
                consume_start,
                consume_end: close_span.end,
                pending_diagnostic: diag.into_diagnostic(directive_span).or(tcy_pending),
            });
        }
        if let Some((node, consume_start, diag)) =
            self.classify_forward_emphasis(view, open_idx, close_idx)
        {
            return Some(AnnotationMatch {
                emit: EmitKind::Aozora(node),
                annotation_payload: None,
                consume_start,
                consume_end: close_span.end,
                pending_diagnostic: diag.into_diagnostic(directive_span).or(tcy_pending),
            });
        }

        // `<run>［＃mは上ドット付き］` — dotted-letter composition (#331). Gated on
        // the `ドット付き` suffix so the reclaim look-back (which copies the
        // preceding run) never runs on ordinary directives. Unlike the styling
        // recognisers above it addresses a bare Latin letter in the preceding
        // run — no `「X」` quote — so it must run its own reclaim, not
        // `extract_forward_quote_targets`.
        if body.ends_with("ドット付き")
            && let Some((node, consume_start)) =
                self.classify_forward_accent_dot(view, open_idx, body)
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
    ) -> Option<(NodeOwned, bool)> {
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
        if !m.payload.canonical.has_mencode() && m.payload.resolve(self.alloc.store()).is_none() {
            return None;
        }
        let unresolved = m.payload.resolve(self.alloc.store()).is_none();
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
    ) -> AnnotationMatch {
        let raw = &self.source[directive_span.start as usize..directive_span.end as usize];
        // One payload for `emit`, one for `annotation_payload`, so the
        // body-builder can re-wrap without re-interning the raw string.
        let payload = self.alloc.make_directive(raw, DirectiveKind::Unknown);
        let node = self.alloc.annotation(payload);
        let payload_for_seg = self.alloc.make_directive(raw, DirectiveKind::Unknown);
        // Notation-hygiene lint: a body that is a verified near-miss of a
        // recognized directive (kept as Unknown) gets a canonical-form
        // suggestion. Catalogue bodies are disjoint from the ここから / 縦中横
        // cases below, so the priority is only defensive — never double-fires.
        let pending_diagnostic = canonical_directive(body)
            .map(|canonical| Diagnostic::non_canonical_directive(directive_span, canonical))
            .or(tcy_pending)
            .or_else(|| {
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
    ) -> AnnotationMatch {
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
impl RecogniseCtx<'_, '_> {
    /// Classify a range bouten `「X」～「Y」に<kind>` (also `〜`): apply the
    /// marks to the whole preceding run from the start of X to the end of Y,
    /// which butts against the bracket. Returns `(node, consume_start)` with
    /// `consume_start` at X so the styled span is the run's sole rendered copy.
    fn classify_forward_bouten_range(
        &mut self,
        view: BodyView<'_>,
        open_idx: usize,
        close_idx: usize,
    ) -> Option<(NodeOwned, u32)> {
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
        } else if let Some(r) = after.strip_prefix("の両側に") {
            (BoutenPosition::Both, r)
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
            self.alloc
                .bouten(kind, content, position, ForwardOrigin::Reclaimed),
            u32::try_from(x_start).ok()?,
        ))
    }

    fn classify_forward_bouten(
        &mut self,
        view: BodyView<'_>,
        open_idx: usize,
        close_idx: usize,
    ) -> Option<(NodeOwned, u32, ForwardDiag)> {
        let extracted = extract_forward_quote_targets(view, self.source, open_idx, close_idx)?;
        // Shape 1: `に<kind>` — default right-side placement.
        // Shape 2: `の左に<kind>` — left-side placement (position flipped).
        // Shape 3: `の両側に<kind>` — both sides.
        let (position, kind_suffix) = if let Some(rest) = extracted.suffix.strip_prefix("に") {
            (BoutenPosition::Right, rest)
        } else if let Some(rest) = extracted.suffix.strip_prefix("の左に") {
            (BoutenPosition::Left, rest)
        } else if let Some(rest) = extracted.suffix.strip_prefix("の両側に") {
            (BoutenPosition::Both, rest)
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
        // A *single* target with no referent is a self-contained bouten: the
        // quoted run is itself the marked text, so style it directly (consume the
        // whole bracket, no pull-back) instead of falling through to the hidden
        // Directive{Unknown}. #228-safe by construction — with no earlier copy
        // there is nothing to duplicate, and a no-referent target has zero
        // look-back occurrences so it is never ambiguous. (Multi-target keeps
        // falling through below: non-contiguous targets cannot be spliced into
        // one leaf.)
        if let [only] = extracted.targets.as_slice()
            && !forward_target_is_preceded(view.events, self.source, open_idx, only)
        {
            let target = build_bouten_target(&extracted.targets, self.alloc);
            return Some((
                self.alloc
                    .bouten(kind, target, position, ForwardOrigin::SelfContained),
                open_span.start,
                ForwardDiag::None,
            ));
        }
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
        // Multi-target `「A」「B」` names non-contiguous runs that cannot be
        // spliced into one leaf — keep the legacy `Referenced` consume (renders
        // nothing) and report the loss.
        let [only] = extracted.targets.as_slice() else {
            let target = build_bouten_target(&extracted.targets, self.alloc);
            return Some((
                self.alloc
                    .bouten(kind, target, position, ForwardOrigin::Referenced),
                open_span.start,
                ForwardDiag::NotStylable,
            ));
        };
        // Single target: shared #333 resolution (`build_bouten_target([x])` ==
        // `content_plain(x)`, so the bouten node is identical). Then overlay the
        // bouten ambiguity diagnostic when the styled target occurs ≥2 times in
        // the look-back (`matches` counts non-overlapping candidate runs).
        let (node, consume_start, diag) = self.resolve_forward_format(
            view,
            open_idx,
            open_span.start,
            ForwardAttr::Bouten { kind, position },
            only,
        );
        let diag = if matches!(diag, ForwardDiag::None)
            && self.source[..open_span.start as usize]
                .matches(only)
                .count()
                >= 2
        {
            ForwardDiag::Ambiguous
        } else {
            diag
        };
        Some((node, consume_start, diag))
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
fn build_bouten_target(targets: &[&str], alloc: &mut OwnedAllocator) -> ContentOwned {
    match targets {
        [] => alloc.content_plain(""),
        [only] => alloc.content_plain(only),
        many => {
            let mut segs: Vec<SegmentOwned> = Vec::with_capacity(many.len() * 2 - 1);
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
enum ForwardTcy {
    /// A 縦中横 with a located target — the node, its consume start, and the
    /// directive-level diagnostic (#333: `NotStylable` for a declined referent).
    Recognised(NodeOwned, u32, ForwardDiag),
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
impl RecogniseCtx<'_, '_> {
    fn classify_forward_tcy(
        &mut self,
        view: BodyView<'_>,
        open_idx: usize,
        close_idx: usize,
    ) -> ForwardTcy {
        let Some(extracted) = extract_forward_quote_targets(view, self.source, open_idx, close_idx)
        else {
            return ForwardTcy::NotTcy;
        };
        // `は縦中横` and the corpus compound `は縦中横、行右/左小書き` (numbered
        // list markers like 「１）」 set horizontal *and* small). Recognise the
        // compound as 縦中横 — the dominant transform; the small-script
        // fine-positioning normalises away on serialize (idempotent). `は横一列`
        // (a short punctuation run like `！？` set on one horizontal line) is a
        // corpus-attested prose synonym of 縦中横 and canonicalises to it.
        if !matches!(
            extracted.suffix,
            "は縦中横" | "は縦中横、行右小書き" | "は縦中横、行左小書き" | "は横一列"
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
        // Resolve the target position (#333), shared with the emphasis / box
        // families: `昭和64［＃「64」は縦中横］年` (adjacent) pulls `64` back into a
        // `Reclaimed` tcy; a non-adjacent plain referent splices a `Detached`
        // tcy decoration; a declined referent stays `Referenced` + reports it.
        let Some(&PairEvent::PairOpen {
            span: open_span, ..
        }) = view.events.get(open_idx)
        else {
            return ForwardTcy::NotTcy;
        };
        let (node, consume_start, diag) = self.resolve_forward_format(
            view,
            open_idx,
            open_span.start,
            ForwardAttr::CombineUpright,
            first,
        );
        ForwardTcy::Recognised(node, consume_start, diag)
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

/// Where a forward-reference target `X` resolves relative to its `［`, given
/// the current pending plain run — the three-way generalisation of
/// [`find_immediate_predecessor_target_position`] that drives #333.
///
/// - [`Adjacent`](ForwardReferent::Adjacent): `X` butts the bracket — pull it
///   back into a `Reclaimed` node (case A, unchanged behaviour).
/// - [`Interior`](ForwardReferent::Interior): `X` is a plain occurrence inside
///   the pending run but *not* adjacent — the caller materialises a `Detached`
///   decoration at `[start, end)` and keeps the bracket `Referenced` (case B).
/// - [`Unresolvable`](ForwardReferent::Unresolvable): `X` is present in the
///   look-back but not in the pending run (a ruby base, an earlier line, or
///   inside a prior construct) — keep it `Referenced`, declined.
///
/// The window `[pending_plain_start, ［)` is pure plain source by construction
/// (every completed node resets `pending_plain_start` to a byte past itself,
/// every newline flushes it), so a window `rfind` selects exactly the §7.5
/// most-recent-preceding *base-text* occurrence when one is representable and
/// returns `Unresolvable` otherwise — the ruby-base / cross-line decline falls
/// out for free, with no separate detector.
enum ForwardReferent {
    Adjacent(u32),
    Interior { start: u32, end: u32 },
    Unresolvable,
}

/// The directive-level diagnostic a forward recognizer asks the dispatch to
/// attach (#333). Orthogonal to the emitted node.
enum ForwardDiag {
    /// No diagnostic.
    None,
    /// A styled target that occurs more than once in the look-back — the
    /// chosen run may be unintended (`bouten_target_ambiguous`).
    Ambiguous,
    /// The target is present but not stylable in place — a ruby base, an
    /// earlier line, a prior construct, or one of several targets
    /// (`forward_referent_not_stylable`).
    NotStylable,
}

impl ForwardDiag {
    /// Build the concrete directive-span diagnostic this signal names, if any.
    fn into_diagnostic(self, directive_span: Span) -> Option<Diagnostic> {
        match self {
            Self::None => None,
            Self::Ambiguous => Some(Diagnostic::bouten_target_ambiguous(directive_span)),
            Self::NotStylable => Some(Diagnostic::forward_referent_not_stylable(directive_span)),
        }
    }
}

#[allow(
    clippy::too_many_arguments,
    reason = "each parameter is an independent input to the pure resolution — the events \
              table, source, bracket index, target text, and the pending-run window start."
)]
fn resolve_forward_referent(
    events: &[PairEvent],
    source: &str,
    open_idx: usize,
    target: &str,
    pending_plain_start: Option<u32>,
) -> ForwardReferent {
    let Some(&PairEvent::PairOpen { span, .. }) = events.get(open_idx) else {
        return ForwardReferent::Unresolvable;
    };
    let cutoff = span.start as usize;
    let len = target.len();
    // A: byte-adjacent — the most-recent occurrence by definition. `target`
    // is canonical UTF-8, so a byte-slice compare is a string compare.
    if cutoff >= len && &source.as_bytes()[cutoff - len..cutoff] == target.as_bytes() {
        return u32::try_from(cutoff - len)
            .map_or(ForwardReferent::Unresolvable, ForwardReferent::Adjacent);
    }
    // B: most-recent occurrence *within the current pending plain run*.
    let Some(window_start) = pending_plain_start.map(|p| p as usize) else {
        return ForwardReferent::Unresolvable;
    };
    if window_start >= cutoff {
        return ForwardReferent::Unresolvable;
    }
    source[window_start..cutoff]
        .rfind(target)
        .map_or(ForwardReferent::Unresolvable, |rel| {
            let start = window_start + rel;
            match (u32::try_from(start), u32::try_from(start + len)) {
                (Ok(start), Ok(end)) => ForwardReferent::Interior { start, end },
                _ => ForwardReferent::Unresolvable,
            }
        })
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
/// `HeadingOwned` (大→`<h1>` / 中→`<h2>` / 小→`<h3>`): the
/// consume span is pulled back over that line so the heading element is
/// its sole rendered copy. When the referent is not a clean preceding
/// line, the classifier keeps the inline `HeadingHintOwned` marker
/// at the directive position (information-preserving, never promoted to
/// an empty or misplaced heading).
///
/// Same `forward_target_is_preceded` gate as forward bouten, but a target
/// absent from the preceding source is no longer rejected: a single quoted
/// target with no referent is a *self-contained* forward heading — the
/// quoted run is itself the heading text, marked `self_contained` on the
/// hint so render shows it (serialize stays bracket-only, a fixed point,
/// the `ForwardOrigin::SelfContained` emphasis/bouten analogue). A
/// multi-quote hint with any missing referent still falls through to
/// `Directive { Unknown }`, since a non-contiguous referent cannot be
/// spliced into a single heading leaf.
impl RecogniseCtx<'_, '_> {
    fn classify_forward_heading(
        &mut self,
        view: BodyView<'_>,
        open_idx: usize,
        close_idx: usize,
    ) -> Option<(NodeOwned, u32)> {
        let extracted = extract_forward_quote_targets(view, self.source, open_idx, close_idx)?;
        let rest = extracted.suffix.strip_prefix("は")?;
        let (style, kind) = parse_heading_keyword(rest)?;

        let &PairEvent::PairOpen {
            span: open_span, ..
        } = view.events.get(open_idx)?
        else {
            return None;
        };

        // Whether `target` appears in the look-back. See `classify_forward_bouten`
        // for the same rationale. Exact look-back first (cheap; uses the AC index
        // when installed); on a miss, retry against a ruby-stripped copy of the
        // look-back, since a heading title carrying ruby (`両頭《りやうとう》`) has
        // its ruby-stripped target (`両頭`) quoted in the directive and so is not a
        // contiguous source substring.
        let preceded = |target: &str| {
            forward_target_is_preceded(view.events, self.source, open_idx, target)
                || forward_heading_target_is_preceded_ruby_stripped(
                    view,
                    self.source,
                    open_idx,
                    target,
                )
        };

        // A single quoted target absent from the look-back is a *self-contained*
        // forward heading: the quoted run is itself the heading text (the
        // `ForwardOrigin::SelfContained` emphasis/bouten analogue). A multi-quote
        // hint with any missing referent stays Unknown — a non-contiguous referent
        // cannot be spliced into a single heading leaf. An all-preceded hint renders
        // hidden and may promote to a block heading in the `promote_headings`
        // lowering pass when its referent is the bare line directly above it.
        //
        // `preceded` is evaluated at most once per target (the ruby-stripped
        // fallback copies the whole look-back, so a second call per heading would
        // double the parser's allocation pressure).
        let self_contained = match extracted.targets.as_slice() {
            [only] if !only.is_empty() => !preceded(only),
            targets if targets.iter().any(|t| !t.is_empty() && !preceded(t)) => return None,
            _ => false,
        };

        // Concatenate targets in the (rare) multi-quote case so the full named run
        // drives the hint content. The 同行 / 窓 styles run into the body on their
        // own line, so they too land here as hints.
        let combined: String = extracted.targets.iter().copied().collect();
        if combined.is_empty() {
            return None;
        }
        Some((
            self.alloc
                .heading_hint(kind, style, &combined, self_contained),
            open_span.start,
        ))
    }
}

/// Classify a `「X」の左に「Y」のルビ` forward-reference **left-side ruby** — the
/// building block of a 再読文字 (saidoku-moji). The target `X` is pulled back
/// (mirroring `classify_forward_bouten`); the reading `Y` attaches on the left.
/// Single-target only; the `の左に「…」のルビ` suffix shape is unique, so a
/// non-ruby `の左に…` (left-side bouten) never reaches here.
impl RecogniseCtx<'_, '_> {
    fn classify_forward_left_ruby(
        &mut self,
        view: BodyView<'_>,
        open_idx: usize,
        close_idx: usize,
    ) -> Option<(NodeOwned, u32)> {
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
/// [`NodeOwned::MarginNote`] node and flavour:
/// - `「X」の左に「Y」の注記` / bare `「X」に「Y」の注記` →
///   [`MarginNoteKind::Gloss`] (editorial gloss; round-trips `の注記`).
/// - `「X」に「Y」の傍記` → [`MarginNoteKind::Marginal`] (the censorship-marker
///   form; round-trips bare `に…の傍記`).
///
/// The `の注記` / `の傍記` suffixes are disjoint from `のルビ` and every
/// bouten kind, so the bouten and left-ruby classifiers above have already
/// declined.
impl RecogniseCtx<'_, '_> {
    fn classify_forward_side_note(
        &mut self,
        view: BodyView<'_>,
        open_idx: usize,
        close_idx: usize,
    ) -> Option<(NodeOwned, u32)> {
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
    ) -> Option<NodeOwned> {
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
impl RecogniseCtx<'_, '_> {
    /// Shared #333 resolution for the single-target `forward_format` families
    /// (emphasis / 縦中横 / box enclosure). The caller has already confirmed the
    /// target is preceded (not self-contained) and holds the `attr` + open
    /// span. Returns the bracket node, its consume start, and the diagnostic;
    /// for the interior case it also stashes the styled `Detached` decoration
    /// in `self.pending_decoration` for `try_bracket_emit` to splice.
    #[allow(
        clippy::too_many_arguments,
        reason = "the caller already holds the body view, bracket index, open-span start, \
                  resolved attribute, and target text — each an independent input."
    )]
    fn resolve_forward_format(
        &mut self,
        view: BodyView<'_>,
        open_idx: usize,
        open_span_start: u32,
        attr: ForwardAttr,
        only: &str,
    ) -> (NodeOwned, u32, ForwardDiag) {
        let text = self.alloc.content_plain(only);
        match resolve_forward_referent(
            view.events,
            self.source,
            open_idx,
            only,
            self.pending_plain_start,
        ) {
            ForwardReferent::Adjacent(consume_start) => (
                self.alloc
                    .forward_format(attr, text, ForwardOrigin::Reclaimed),
                consume_start,
                ForwardDiag::None,
            ),
            ForwardReferent::Interior { start, end } => {
                let deco = self
                    .alloc
                    .forward_format(attr, text, ForwardOrigin::Detached);
                self.pending_decoration = Some((deco, Span::new(start, end)));
                (
                    self.alloc
                        .forward_format(attr, text, ForwardOrigin::Referenced),
                    open_span_start,
                    ForwardDiag::None,
                )
            }
            ForwardReferent::Unresolvable => (
                self.alloc
                    .forward_format(attr, text, ForwardOrigin::Referenced),
                open_span_start,
                ForwardDiag::NotStylable,
            ),
        }
    }

    fn classify_forward_emphasis(
        &mut self,
        view: BodyView<'_>,
        open_idx: usize,
        close_idx: usize,
    ) -> Option<(NodeOwned, u32, ForwardDiag)> {
        let extracted = extract_forward_quote_targets(view, self.source, open_idx, close_idx)?;
        // The particle is tied to the decoration. `は` is the dominant emphasis
        // form (`「X」は太字`). The frame decoration also takes the "applied to"
        // particle `に` (`「X」に枠囲み`) — but 太字/斜体/… stay `は`-only, so a `に`
        // suffix is accepted only when it resolves to `Framed`. Bouten runs
        // earlier in the cascade and already claims `に〈bouten-kind〉`. The
        // serializer canonicalises both particles to `」は`.
        let attr = if let Some(rest) = extracted.suffix.strip_prefix("は") {
            forward_attr_from_suffix(rest)?
        } else if let Some(rest) = extracted.suffix.strip_prefix("に") {
            match forward_attr_from_suffix(rest)? {
                framed @ ForwardAttr::Framed(_) => framed,
                _ => return None,
            }
        } else {
            return None;
        };
        let [only] = extracted.targets.as_slice() else {
            return None;
        };
        // Forward accent (`「e」はアクサン（´）付き` → é): the quoted target must be a
        // single Latin letter composable with this mark. Word-qualified /
        // positional / multi-letter forms would misrender, so decline them to a
        // byte-exact `Directive{Unknown}` here (mirrors how
        // `classify_forward_accent_dot` validates via `compose_accent_dots`).
        if let ForwardAttr::Accent(mark) = attr {
            let mut cs = only.chars();
            match (cs.next(), cs.next()) {
                (Some(letter), None) if compose_accent(letter, mark).is_some() => {}
                _ => return None,
            }
        }
        let &PairEvent::PairOpen {
            span: open_span, ..
        } = view.events.get(open_idx)?
        else {
            return None;
        };
        if !forward_target_is_preceded(view.events, self.source, open_idx, only) {
            // No referent: the quoted target has no earlier copy, so it *is* the
            // styled run (`ForwardOrigin::SelfContained`) rather than falling
            // through to a hidden `Unknown` directive. Consume the whole bracket
            // — no pull-back — so the region tiling is byte-identical to the old
            // `Unknown` and the #228 double-render is structurally impossible.
            let text = self.alloc.content_plain(only);
            return Some((
                self.alloc
                    .forward_format(attr, text, ForwardOrigin::SelfContained),
                open_span.start,
                ForwardDiag::None,
            ));
        }
        Some(self.resolve_forward_format(view, open_idx, open_span.start, attr, only))
    }
}

/// Classify a `［＃「X」は「□」囲み］` box-character enclosure — the 「□」 box
/// member of the 罫囲み enclosure family ([`EnclosureKind::Box`], §6.7).
///
/// A single target `X`; the `は` particle and the quoted `□` glyph live in the
/// suffix, mirroring [`Self::classify_forward_left_ruby`] /
/// [`Self::classify_forward_side_note`]. Structurally it is an emphasis-style
/// treatment (it boxes the target run, like `「X」は太字` bolds it), so it reuses
/// the same `forward_target_is_preceded` pull-back / `SelfContained` logic as
/// [`Self::classify_forward_emphasis`]. The `は「` prefix + `」囲み` suffix shape
/// excludes every `の注記` / `のルビ` / `に…` form; only the canonical `□`
/// (U+25A1) glyph is claimed — any other glyph stays `Directive{Unknown}` until
/// it earns its own [`EnclosureKind`] member.
impl RecogniseCtx<'_, '_> {
    fn classify_forward_box_enclosure(
        &mut self,
        view: BodyView<'_>,
        open_idx: usize,
        close_idx: usize,
    ) -> Option<(NodeOwned, u32, ForwardDiag)> {
        let extracted = extract_forward_quote_targets(view, self.source, open_idx, close_idx)?;
        let [target] = extracted.targets.as_slice() else {
            return None;
        };
        let glyph = extracted
            .suffix
            .strip_prefix("は「")?
            .strip_suffix("」囲み")?;
        if glyph != "□" {
            return None;
        }
        let attr = ForwardAttr::Framed(EnclosureKind::Box);
        let &PairEvent::PairOpen {
            span: open_span, ..
        } = view.events.get(open_idx)?
        else {
            return None;
        };
        if !forward_target_is_preceded(view.events, self.source, open_idx, target) {
            // No referent: the quoted target is itself the boxed run. Consume the
            // whole bracket (no pull-back) so region tiling is byte-identical to
            // the old `Unknown` and the #228 double-render is impossible.
            let text = self.alloc.content_plain(target);
            return Some((
                self.alloc
                    .forward_format(attr, text, ForwardOrigin::SelfContained),
                open_span.start,
                ForwardDiag::None,
            ));
        }
        Some(self.resolve_forward_format(view, open_idx, open_span.start, attr, target))
    }
}

/// Classify a `<run>［＃mは上ドット付き］` dotted-letter directive (#331).
///
/// The directive addresses a base Latin letter *inside the immediately-
/// preceding run* — a bare word (`Padma-sambhava`) or a decomposed `〔…〕`
/// accent span — and asks for a combining dot above / below it. This is
/// sub-run occurrence addressing, not the `「X」は…` quote shape, so it reclaims
/// the preceding run directly (never `extract_forward_quote_targets`). The run
/// is pulled back (the styled span is the sole rendered copy, #228-safe); the
/// raw body is interned for byte-exact serialize and render-time composition.
/// `aozora_syntax::accent::compose_accent_dots` is the single authority for the
/// selector grammar and glyph table, shared with the renderer — a `Some`
/// result both validates the claim and (in the renderer) produces the glyphs.
impl RecogniseCtx<'_, '_> {
    fn classify_forward_accent_dot(
        &mut self,
        view: BodyView<'_>,
        open_idx: usize,
        body: &str,
    ) -> Option<(NodeOwned, u32)> {
        let &PairEvent::PairOpen {
            span: open_span, ..
        } = view.events.get(open_idx)?
        else {
            return None;
        };
        let bracket_start = open_span.start as usize;
        let run_start = reclaim_accent_run_start(&self.source[..bracket_start])?;
        let run = &self.source[run_start..bracket_start];
        // Validate the body against the run: the shared composer declines any
        // multi-clause / word-qualified / 段目 form and any unresolvable or
        // uncomposable letter, leaving those as `Directive{Unknown}`.
        compose_accent_dots(run, body)?;
        // Store the reclaimed run *uncomposed* — the renderer composes on the
        // fly and the serializer re-emits it verbatim before the raw body.
        let text = self.alloc.content_plain(run);
        let consume_start = u32::try_from(run_start).ok()?;
        let origin = ForwardOrigin::from_consume(consume_start, open_span.start);
        Some((self.alloc.accent_dot(text, body, origin), consume_start))
    }
}

/// Byte offset where the run immediately preceding a dotted-letter directive
/// begins, or `None` when the bracket is butted by a non-Latin character
/// (a `》` ruby close, Japanese text) — which declines the directive.
///
/// A prefix ending in `〕` reclaims the whole decomposed `〔…〕` accent span
/// (sanitize keeps its brackets, so it is a contiguous run); otherwise the
/// maximal trailing run of Latin letters / Latin-Extended glyphs / `-` word
/// joiners is reclaimed.
fn reclaim_accent_run_start(prefix: &str) -> Option<usize> {
    if prefix.ends_with('〕') {
        // `〔…〕` does not nest, so the matching open is the last `〔`.
        return prefix.rfind('〔');
    }
    let mut start = prefix.len();
    for (i, ch) in prefix.char_indices().rev() {
        if is_latin_run_char(ch) {
            start = i;
        } else {
            break;
        }
    }
    (start < prefix.len()).then_some(start)
}

/// A character that participates in a reclaimable Latin run: ASCII letters,
/// the Latin-1 / Latin-Extended-A ranges (accented vowels like ā, ç produced
/// by `〔…〕` decomposition), the Latin-Extended-Additional range (dotted
/// glyphs like ṁ), and the `-` word joiner (`Nara-sinha`).
fn is_latin_run_char(ch: char) -> bool {
    ch.is_ascii_alphabetic()
        || ch == '-'
        || matches!(ch, '\u{00C0}'..='\u{024F}' | '\u{1E00}'..='\u{1EFF}')
}

/// Map the keyword after `は` to a forward-scope [`ForwardAttr`].
///
/// 太字 → Bold, 斜体 → Italic (per
/// <https://www.aozora.gr.jp/annotation/emphasis.html>). ゴシック体 / ゴチック
/// are corpus spellings of bold — the guide writes 太字（ゴシック） — so both
/// map to Bold and canonicalise to `太字` on serialize. 上付き小文字 →
/// `SuperScript`, 下付き小文字 → `SubScript`, 行右小書き → `SmallScript(Right)`,
/// 行左小書き → `SmallScript(Left)`, and `N段階大きな/小さな文字` → `FontSize`
/// (per <https://www.aozora.gr.jp/annotation/etc.html>). 分数 → `Fraction`
/// (`「a/b」は分数`, the render arm typesets the `/`-split target). 罫囲み /
/// 枠囲み / 枠囲い / ○付き文字 / 点線丸囲み / 二重罫囲み → the corresponding
/// [`EnclosureKind`] under `Framed` (`「□」囲み` is claimed earlier, by the box
/// recogniser). Unknown suffixes return `None` (→ `Directive{Unknown}`).
pub(super) fn forward_attr_from_suffix(s: &str) -> Option<ForwardAttr> {
    Some(match s {
        "太字" | "ゴシック体" | "ゴチック" => ForwardAttr::Bold,
        "斜体" => ForwardAttr::Italic,
        "上付き小文字" => ForwardAttr::SuperScript,
        "下付き小文字" => ForwardAttr::SubScript,
        "行右小書き" => ForwardAttr::SmallScript(BoutenPosition::Right),
        "行左小書き" => ForwardAttr::SmallScript(BoutenPosition::Left),
        // 枠囲み / 枠囲い (okurigana variant) are corpus spellings of the frame
        // decoration; all canonicalise to 罫囲み on serialize.
        "罫囲み" | "枠囲み" | "枠囲い" => ForwardAttr::Framed(EnclosureKind::Rule),
        // Other single-target enclosures whose suffix carries no embedded glyph
        // (unlike 「□」囲み). Each has its own EnclosureKind so serialize
        // round-trips the exact keyword rather than folding onto 罫囲み.
        "○付き文字" => ForwardAttr::Framed(EnclosureKind::Circle),
        "点線丸囲み" => ForwardAttr::Framed(EnclosureKind::CircleDotted),
        "二重罫囲み" => ForwardAttr::Framed(EnclosureKind::DoubleRule),
        "横組み" => ForwardAttr::Horizontal,
        "キャプション" => ForwardAttr::Caption,
        // 絶対サイズ: `「X」は小文字` (corpus-attested) and its 特大/大/中 siblings.
        // Distinct from the relative `N段階…文字` (parse_font_size_suffix) and
        // from the script-glyph `上付き小文字`/`下付き小文字` (exact-match above).
        "特大文字" => ForwardAttr::FontSizeAbsolute(AbsoluteSize::ExtraLarge),
        "大文字" => ForwardAttr::FontSizeAbsolute(AbsoluteSize::Large),
        "中文字" => ForwardAttr::FontSizeAbsolute(AbsoluteSize::Medium),
        "小文字" => ForwardAttr::FontSizeAbsolute(AbsoluteSize::Small),
        // 分数: `「a/b」は分数`. Only the single-target form is matched here; a
        // comma-joined compound (`「3」は上付き小文字、「1/143」は分数`) yields a
        // suffix that is not exactly `分数`, so it stays `Directive{Unknown}`.
        "分数" => ForwardAttr::Fraction,
        _ => {
            return parse_font_size_suffix(s)
                .or_else(|| parse_align_end_suffix(s))
                .or_else(|| parse_accent_suffix(s));
        }
    })
}

/// Parse a forward accent-mark suffix (`アクサン（´）付き` / `アクサン（｀）付き` /
/// `ウムラウト（¨）付き`) into a [`ForwardAttr::Accent`].
///
/// The mark *word* does not distinguish acute from grave — the bracketed symbol
/// does (´ U+00B4 vs ｀ U+FF40); ウムラウト（¨） (¨ U+00A8) is the umlaut. Only the
/// canonical fullwidth-paren spelling (U+FF08 / U+FF09) is claimed; the
/// classifier separately requires the quoted target to be a *single composable
/// letter*, so word-qualified / positional / half-width-paren forms decline.
/// Any other suffix returns `None` (→ `Directive{Unknown}`).
fn parse_accent_suffix(s: &str) -> Option<ForwardAttr> {
    let mark = match s {
        "アクサン（´）付き" => AccentMark::Acute,
        "アクサン（｀）付き" => AccentMark::Grave,
        "ウムラウト（¨）付き" => AccentMark::Umlaut,
        _ => return None,
    };
    Some(ForwardAttr::Accent(mark))
}

/// Parse a `地付き` / `文末より N字上げ揃え` / `行末より N字上がり` end-alignment
/// suffix into a [`ForwardAttr::AlignEnd`] — the forward-scope analogue of the
/// line-form `AlignEndParamPrefix` (§7.6). Mirrors its verb set (`字上げ` /
/// `字上がり`, optional `揃え`) and, like the line-form `LineFormat::AlignEnd`,
/// collapses the 文末 / 行末 / 地より / 地から anchor to the offset alone. `地付き`
/// is the zero-lift form (`offset: 0`, flush to the text-end edge). Returns
/// `None` for a missing/zero magnitude or any other suffix (→
/// `Directive{Unknown}`).
fn parse_align_end_suffix(s: &str) -> Option<ForwardAttr> {
    // 地付き — flush to the text-end edge (zero lift); the forward analogue of
    // the `地付き` line form (LineFormat::AlignEnd { offset: 0 }). A distinct
    // lexeme with no magnitude, so it can't ride the prefix+digit path below.
    if s == "地付き" {
        return Some(ForwardAttr::AlignEnd { offset: 0 });
    }
    let rest = s
        .strip_prefix("文末より")
        .or_else(|| s.strip_prefix("行末より"))
        .or_else(|| s.strip_prefix("地より"))
        .or_else(|| s.strip_prefix("地から"))?;
    let (offset, tail) = parse_decimal_u8_prefix(rest)?;
    (matches!(tail, "字上げ" | "字上がり" | "字上げ揃え" | "字上がり揃え") && offset >= 1)
        .then_some(ForwardAttr::AlignEnd { offset })
}

/// Parse a `N段階大きな文字` / `N段階小さな文字` font-size suffix into a
/// [`ForwardAttr::FontSize`]. `大きな` yields a positive stage count, `小さな`
/// a negative one. Returns `None` for a missing/zero magnitude, an `i8`
/// overflow, or any other suffix (→ `Directive{Unknown}`).
fn parse_font_size_suffix(s: &str) -> Option<ForwardAttr> {
    let (magnitude, rest) = parse_decimal_u8_prefix(s)?;
    let steps = i8::try_from(magnitude).ok()?;
    let shift = FontShift(NonZeroI8::new(steps)?);
    match rest {
        "段階大きな文字" => Some(ForwardAttr::FontSize(shift)),
        "段階小さな文字" => Some(ForwardAttr::FontSize(FontShift(NonZeroI8::new(-steps)?))),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::{AccentMark, EnclosureKind, ForwardAttr, forward_attr_from_suffix};

    #[test]
    fn accent_suffixes_map_to_their_marks() {
        // The bracketed symbol distinguishes acute (´) from grave (｀); ウムラウト
        // (¨) is the umlaut. Fullwidth parens (U+FF08 / U+FF09) are canonical.
        assert_eq!(
            forward_attr_from_suffix("アクサン（´）付き"),
            Some(ForwardAttr::Accent(AccentMark::Acute))
        );
        assert_eq!(
            forward_attr_from_suffix("アクサン（｀）付き"),
            Some(ForwardAttr::Accent(AccentMark::Grave))
        );
        assert_eq!(
            forward_attr_from_suffix("ウムラウト（¨）付き"),
            Some(ForwardAttr::Accent(AccentMark::Umlaut))
        );
        // Half-width parens are not the canonical spelling — declined here (the
        // corpus half-width forms are all word-qualified anyway).
        assert_eq!(forward_attr_from_suffix("アクサン(´)付き"), None);
        // A bare mark word with no bracketed symbol is not a claimable suffix.
        assert_eq!(forward_attr_from_suffix("アクサン付き"), None);
    }

    #[test]
    fn enclosure_suffixes_map_to_their_kinds() {
        // Each glyph-free enclosure suffix picks its own EnclosureKind so
        // serialize can round-trip the exact keyword.
        assert_eq!(
            forward_attr_from_suffix("○付き文字"),
            Some(ForwardAttr::Framed(EnclosureKind::Circle))
        );
        assert_eq!(
            forward_attr_from_suffix("点線丸囲み"),
            Some(ForwardAttr::Framed(EnclosureKind::CircleDotted))
        );
        assert_eq!(
            forward_attr_from_suffix("二重罫囲み"),
            Some(ForwardAttr::Framed(EnclosureKind::DoubleRule))
        );
        // The ruled-frame spellings stay folded onto Rule.
        assert_eq!(
            forward_attr_from_suffix("罫囲み"),
            Some(ForwardAttr::Framed(EnclosureKind::Rule))
        );
        // 破線枠囲み is a block-compound segment, not a forward suffix — it must
        // NOT be claimed here (it stays Unknown).
        assert_eq!(forward_attr_from_suffix("破線枠囲み"), None);
        // A near-miss glyph-bearing form is not one of these bare suffixes.
        assert_eq!(forward_attr_from_suffix("丸囲み"), None);
    }

    #[test]
    fn fraction_suffix_matches_only_the_exact_single_form() {
        // `「a/b」は分数` → Fraction.
        assert_eq!(
            forward_attr_from_suffix("分数"),
            Some(ForwardAttr::Fraction)
        );
        // A comma-joined compound (`「3」は上付き小文字、「1/143」は分数`) reaches this
        // fn with a suffix that is not exactly `分数`, so it must NOT match — it
        // stays `Directive{Unknown}` until the multi-directive-per-bracket
        // grammar owns it (#321 out-of-scope).
        assert_eq!(
            forward_attr_from_suffix("上付き小文字、「1/143」は分数"),
            None
        );
        assert_eq!(forward_attr_from_suffix("分数、縦中横"), None);
    }

    #[test]
    fn align_end_suffix_parses_offset_and_verb_variants() {
        // The corpus-attested form: `「訳者」は文末より１字上げ揃え`.
        assert_eq!(
            forward_attr_from_suffix("文末より１字上げ揃え"),
            Some(ForwardAttr::AlignEnd { offset: 1 })
        );
        // Verb + anchor variants mirror the line-form recogniser.
        assert_eq!(
            forward_attr_from_suffix("行末より2字上がり"),
            Some(ForwardAttr::AlignEnd { offset: 2 })
        );
        assert_eq!(
            forward_attr_from_suffix("文末より3字上げ"),
            Some(ForwardAttr::AlignEnd { offset: 3 })
        );
        // 地付き is the zero-lift form; 地より / 地から are accepted anchors.
        assert_eq!(
            forward_attr_from_suffix("地付き"),
            Some(ForwardAttr::AlignEnd { offset: 0 })
        );
        assert_eq!(
            forward_attr_from_suffix("地より１字上げ"),
            Some(ForwardAttr::AlignEnd { offset: 1 })
        );
        assert_eq!(
            forward_attr_from_suffix("地より１１字上げ"),
            Some(ForwardAttr::AlignEnd { offset: 11 })
        );
        // Misspelling, zero magnitude, and 、-joined compound all stay Unknown.
        assert_eq!(forward_attr_from_suffix("地付け"), None);
        assert_eq!(forward_attr_from_suffix("地より0字上げ"), None);
        assert_eq!(forward_attr_from_suffix("地付き、地より３字アキ"), None);
        // Zero offset and unrelated suffixes stay Unknown.
        assert_eq!(forward_attr_from_suffix("文末より0字上げ"), None);
        assert_eq!(forward_attr_from_suffix("文末より字上げ"), None);
        assert_eq!(forward_attr_from_suffix("文頭より1字上げ"), None);
    }
}
