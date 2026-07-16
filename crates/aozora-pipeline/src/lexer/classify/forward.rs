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
use aozora_syntax::alloc::Allocator;
use aozora_syntax::ast::{Content, Node, Segment};
use aozora_syntax::format::ForwardOrigin;
use aozora_syntax::lint::canonical_directive;
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
    let _classify_guard = SubsystemGuard::new(Subsystem::ForwardIndexInstall);

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

    // Pair each `「` with its BALANCED `」` via a quote-only stack,
    // mirroring the pair stage (`PairKind::Quote`, pair.rs) so a quote
    // body is sliced at the same extent the recognisers see through
    // `view.links`. A target whose text embeds a nested `「…」` — a
    // forward-referenced gaiji base such as `冢※［＃「土へん＋冢」…］` — is
    // captured whole instead of truncated at the inner `」` (the naive
    // "first `」`" scan recorded `冢※［＃「土へん＋冢`, a body no recogniser
    // queries, so the real target was absent from the index and every
    // installed-doc lookup wrongly returned "not preceded"). When quote
    // depth never exceeds 1 (every non-nested document) the innermost
    // open is always the immediately-preceding one, so this yields the
    // identical body set the first-`」` scan did — byte-for-byte
    // unchanged wherever there are no nested quotes.
    //
    // The stored value is the body's first occurrence **as a substring
    // of the source** (`memmem::find` over the whole source) — the same
    // question the `source[..cutoff].contains(target)` fallback answers —
    // NOT the position of the `「` that introduced this body. The
    // canonical reference is `語句［＃「語句」に傍点］`, whose referent is
    // *bare* text before the bracket while the only `「語句」` pair lives
    // *inside* the directive. Because the position is a body-keyed global
    // substring find, it is independent of which `「` inserted the entry,
    // so the stack pairing never shifts a stored position.
    let mut first_positions: HashMap<String, u32> = HashMap::with_capacity(opens.len());
    let mut open_stack: Vec<usize> = Vec::new();
    let mut opens_iter = opens.iter().copied().peekable();
    for close_pos in memmem::find_iter(bytes, QUOTE_CLOSE) {
        // Push every `「` that opens before this `」`.
        while opens_iter.peek().is_some_and(|&op| op < close_pos) {
            open_stack.push(opens_iter.next().expect("peeked Some"));
        }
        // Match this `」` to the innermost still-open `「`; a stray close
        // with an empty stack has nothing to index.
        let Some(open_pos) = open_stack.pop() else {
            continue;
        };
        let body_start = open_pos + QUOTE_OPEN.len();
        let body = &source[body_start..close_pos];
        if body.is_empty() {
            continue;
        }
        first_positions.entry(body.to_owned()).or_insert_with(|| {
            // First substring occurrence anywhere in the source. The body
            // provably occurs at `body_start` (its own quote), so `find`
            // never returns `None`; the fallback keeps it total.
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
    /// * 縦中横 single-target compound → `Unknown` — `「X」は縦中横、行右小書き`
    ///   matches neither the exact `は縦中横` tcy shape (the `、`-suffix declines
    ///   at [`Self::classify_forward_tcy`]) nor any emphasis keyword, so it falls
    ///   through to `Directive{Unknown}` (lossless verbatim), served render-only
    ///   by Tier2 `--degraded` (drop the small-script axis; ADR-0027 A5). Pinned
    ///   by `tcy_small_script_compound_declines_to_unknown`.
    /// * 縦中横 `ShapedNoTarget` diagnostic survives the fall-through —
    ///   when the target is absent the directive degrades to
    ///   `Directive{Unknown}`, but its `tcy_target_not_found` warning is
    ///   carried through the later arms via `tcy_pending`. Pinned by
    ///   `tcy_target_not_found_fires_as_warning` (node-absence by
    ///   `forward_tcy_without_preceding_target_falls_through`).
    #[expect(
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
        let _classify_guard = SubsystemGuard::new(Subsystem::Directive);
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
    ) -> Option<(Node, bool)> {
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
    ) -> Option<(Node, u32)> {
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
            let r = after.strip_prefix("の両側に")?;
            (BoutenPosition::Both, r)
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
    ) -> Option<(Node, u32, ForwardDiag)> {
        let extracted = extract_forward_quote_targets(view, self.source, open_idx, close_idx)?;
        // Shape 1: `に<kind>` — default right-side placement.
        // Shape 2: `の左に<kind>` — left-side placement (position flipped).
        // Shape 3: `の両側に<kind>` — both sides.
        let (position, kind_suffix) = if let Some(rest) = extracted.suffix.strip_prefix("に") {
            (BoutenPosition::Right, rest)
        } else if let Some(rest) = extracted.suffix.strip_prefix("の左に") {
            (BoutenPosition::Left, rest)
        } else {
            let rest = extracted.suffix.strip_prefix("の両側に")?;
            (BoutenPosition::Both, rest)
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
            ForwardBracket {
                view,
                open_idx,
                open_span_start: open_span.start,
            },
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
fn build_bouten_target(targets: &[&str], alloc: &mut Allocator) -> Content {
    match targets {
        [] => alloc.content_plain(""),
        [only] => alloc.content_plain(only),
        many => {
            let mut segs: Vec<Segment> = Vec::with_capacity(many.len() * 2 - 1);
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
    Recognised(Node, u32, ForwardDiag),
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
/// Only the exact `「X」は縦中横` shape is recognised (spec §6.3, #435).
/// Multi-quote `［＃「A」「B」は縦中横］` bodies and the non-canonical
/// `は縦中横、行右/左小書き` / `は横一列` variants decline to
/// `Directive{Unknown}` (lossless verbatim) rather than being silently
/// folded — each is served by an opt-in Tier1 lint / Tier2 render instead.
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
        // Only the exact spec form `「X」は縦中横` (spec §6.3) is recognised
        // (#435). The non-canonical corpus variants decline to
        // `Directive{Unknown}` (lossless verbatim), each served by an opt-in
        // layer instead of a silent parser fold:
        //   - `は縦中横、行右/左小書き` dropped the small-script axis silently
        //     (data loss); it now stays Unknown.
        //   - `は横一列` (a punctuation run set horizontal) → Unknown + Tier1
        //     lint suggests `は縦中横`.
        if extracted.suffix != "は縦中横" {
            return ForwardTcy::NotTcy;
        }
        // Single target only — a multi-quote `「A」「B」は縦中横` is not a real
        // Aozora shape and used to keep only the first target (silent data
        // loss); it now declines to `Directive{Unknown}`, mirroring emphasis.
        let [first] = extracted.targets.as_slice() else {
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
            ForwardBracket {
                view,
                open_idx,
                open_span_start: open_span.start,
            },
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
    let _classify_guard = SubsystemGuard::new(Subsystem::ForwardTargetCheck);
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

/// The look-back a forward-reference resolution searches for a target's
/// referent: the paired-event table together with the directive bracket's
/// event index (which give the cutoff), the full sanitized source, and the
/// start of the current pending plain run (`None` when no plain run is open
/// to search within). A self-documenting parameter object for the pure
/// resolution below.
#[derive(Clone, Copy)]
struct ReferentSearch<'a> {
    events: &'a [PairEvent],
    source: &'a str,
    open_idx: usize,
    pending_plain_start: Option<u32>,
}

fn resolve_forward_referent(search: ReferentSearch<'_>, target: &str) -> ForwardReferent {
    let ReferentSearch {
        events,
        source,
        open_idx,
        pending_plain_start,
    } = search;
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
/// `Heading` (大→`<h1>` / 中→`<h2>` / 小→`<h3>`): the
/// consume span is pulled back over that line so the heading element is
/// its sole rendered copy. When the referent is not a clean preceding
/// line, the classifier keeps the inline `HeadingHint` marker
/// at the directive position (information-preserving, never promoted to
/// an empty heading, or one bound to the wrong line).
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
    ) -> Option<(Node, u32)> {
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
    ) -> Option<(Node, u32)> {
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
/// [`Node::MarginNote`] node and flavour:
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
    ) -> Option<(Node, u32)> {
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
        } else {
            let inner = extracted.suffix.strip_suffix("」の傍記")?;
            (MarginNoteKind::Marginal, inner.strip_prefix("に「")?)
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
    ) -> Option<Node> {
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

/// The directive bracket a forward-format resolution operates over: the body
/// view, the bracket's open-event index, and its already-resolved open-span
/// start. Bundled so the single-target `forward_format` families
/// (emphasis / bouten / 縦中横 / box enclosure) share one location argument.
#[derive(Clone, Copy)]
struct ForwardBracket<'a> {
    view: BodyView<'a>,
    open_idx: usize,
    open_span_start: u32,
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
    fn resolve_forward_format(
        &mut self,
        bracket: ForwardBracket<'_>,
        attr: ForwardAttr,
        only: &str,
    ) -> (Node, u32, ForwardDiag) {
        let ForwardBracket {
            view,
            open_idx,
            open_span_start,
        } = bracket;
        let text = self.alloc.content_plain(only);
        match resolve_forward_referent(
            ReferentSearch {
                events: view.events,
                source: self.source,
                open_idx,
                pending_plain_start: self.pending_plain_start,
            },
            only,
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
    ) -> Option<(Node, u32, ForwardDiag)> {
        let extracted = extract_forward_quote_targets(view, self.source, open_idx, close_idx)?;
        // The particle is tied to the decoration. `は` is the dominant emphasis
        // form (`「X」は太字`). The frame decoration also takes the "applied to"
        // particle `に` (`「X」に枠囲み`) — but 太字/斜体/… stay `は`-only, so a `に`
        // suffix is accepted only when it resolves to `Framed`. Bouten runs
        // earlier in the cascade and already claims `に〈bouten-kind〉`. The
        // serializer canonicalises both particles to `」は`.
        let attr = if let Some(rest) = extracted.suffix.strip_prefix("は") {
            forward_attr_from_suffix(rest)?
        } else {
            let rest = extracted.suffix.strip_prefix("に")?;
            match forward_attr_from_suffix(rest)? {
                framed @ ForwardAttr::Framed(_) => framed,
                _ => return None,
            }
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
        Some(self.resolve_forward_format(
            ForwardBracket {
                view,
                open_idx,
                open_span_start: open_span.start,
            },
            attr,
            only,
        ))
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
    ) -> Option<(Node, u32, ForwardDiag)> {
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
        Some(self.resolve_forward_format(
            ForwardBracket {
                view,
                open_idx,
                open_span_start: open_span.start,
            },
            attr,
            target,
        ))
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
    ) -> Option<(Node, u32)> {
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
/// 太字 → Bold, ゴシック体 → Gothic (a distinct typeface, **not** a fold to
/// 太字 — #435), 斜体 → Italic (per
/// <https://www.aozora.gr.jp/annotation/emphasis.html>). ゴチック (1 corpus
/// work) is not recognised — it declines to `Directive{Unknown}` and a Tier1
/// lint suggests ゴシック体. 上付き小文字 →
/// `SuperScript`, 下付き小文字 → `SubScript`, 行右小書き → `SmallScript(Right)`,
/// 行左小書き → `SmallScript(Left)`, and `N段階大きな/小さな文字` → `FontSize`
/// (per <https://www.aozora.gr.jp/annotation/etc.html>). 分数 → `Fraction`
/// (`「a/b」は分数`, the render arm typesets the `/`-split target). 罫囲み /
/// ○付き文字 / 点線丸囲み / 二重罫囲み → the corresponding [`EnclosureKind`]
/// under `Framed` (`「□」囲み` is claimed earlier, by the box recogniser). The
/// non-canonical 枠囲み / 枠囲い decline to `Directive{Unknown}` (Tier1 → 罫囲み).
/// Unknown suffixes return `None` (→ `Directive{Unknown}`).
pub(super) fn forward_attr_from_suffix(s: &str) -> Option<ForwardAttr> {
    Some(match s {
        "太字" => ForwardAttr::Bold,
        "ゴシック体" => ForwardAttr::Gothic,
        "斜体" => ForwardAttr::Italic,
        "上付き小文字" => ForwardAttr::SuperScript,
        "下付き小文字" => ForwardAttr::SubScript,
        "行右小書き" => ForwardAttr::SmallScript(BoutenPosition::Right),
        "行左小書き" => ForwardAttr::SmallScript(BoutenPosition::Left),
        "罫囲み" => ForwardAttr::Framed(EnclosureKind::Rule),
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
    use core::fmt::Write;

    use super::*;

    // --- shared builders -------------------------------------------------

    /// Build a `<prefix>［＃「t0」「t1」…<suffix>］` directive: the source, the
    /// pair-event stream, the `links` side-table, and the `(open_idx,
    /// close_idx)` the recognisers take. Each `targets` entry gets a real
    /// `PairOpen(Quote)`/`PairClose(Quote)` pair (linked in `links`); the
    /// suffix is raw source (no events) exactly as the recognisers see it.
    fn build(
        prefix: &str,
        targets: &[&str],
        suffix: &str,
    ) -> (String, Vec<PairEvent>, Vec<u32>, usize, usize) {
        let mut src = String::new();
        let mut events: Vec<PairEvent> = Vec::new();
        let mut links: Vec<u32> = Vec::new();

        src.push_str(prefix);
        let b_open_start = u32::try_from(src.len()).unwrap();
        src.push('［');
        let b_open_end = u32::try_from(src.len()).unwrap();
        let open_idx = events.len();
        events.push(PairEvent::PairOpen {
            kind: PairKind::Bracket,
            span: Span::new(b_open_start, b_open_end),
        });
        links.push(u32::MAX);

        let h_start = u32::try_from(src.len()).unwrap();
        src.push('＃');
        let h_end = u32::try_from(src.len()).unwrap();
        events.push(PairEvent::Solo {
            kind: TriggerKind::Hash,
            span: Span::new(h_start, h_end),
        });
        links.push(u32::MAX);

        for t in targets {
            let q_open_start = u32::try_from(src.len()).unwrap();
            src.push('「');
            let q_open_end = u32::try_from(src.len()).unwrap();
            let q_open_idx = events.len();
            events.push(PairEvent::PairOpen {
                kind: PairKind::Quote,
                span: Span::new(q_open_start, q_open_end),
            });
            links.push(u32::MAX);

            src.push_str(t);
            let q_close_start = u32::try_from(src.len()).unwrap();
            src.push('」');
            let q_close_end = u32::try_from(src.len()).unwrap();
            let q_close_idx = events.len();
            events.push(PairEvent::PairClose {
                kind: PairKind::Quote,
                span: Span::new(q_close_start, q_close_end),
            });
            links.push(u32::try_from(q_open_idx).unwrap());
            links[q_open_idx] = u32::try_from(q_close_idx).unwrap();
        }

        src.push_str(suffix);
        let b_close_start = u32::try_from(src.len()).unwrap();
        src.push('］');
        let b_close_end = u32::try_from(src.len()).unwrap();
        let close_idx = events.len();
        events.push(PairEvent::PairClose {
            kind: PairKind::Bracket,
            span: Span::new(b_close_start, b_close_end),
        });
        links.push(u32::try_from(open_idx).unwrap());
        links[open_idx] = u32::try_from(close_idx).unwrap();

        (src, events, links, open_idx, close_idx)
    }

    /// A single `PairOpen(Bracket)` whose `span.start` is `cutoff` — the only
    /// event the position helpers (`find_immediate_predecessor_target_position`,
    /// `resolve_forward_referent`, `forward_target_is_preceded`) read.
    fn open_at(cutoff: u32) -> Vec<PairEvent> {
        vec![PairEvent::PairOpen {
            kind: PairKind::Bracket,
            span: Span::new(cutoff, cutoff + 3),
        }]
    }

    // --- forward-target source index (install / clear / lookup) ----------

    #[test]
    fn forward_index_install_uses_map_for_bare_predecessor() {
        clear_forward_target_index();
        // A *bare* (non-quoted) predecessor followed by 64 distinct quote
        // bodies — enough to install the index.
        let mut src = String::from("BAREPRED");
        for i in 0..64u32 {
            src.push('「');
            write!(src, "q{i:02}").unwrap();
            src.push('」');
        }
        install_forward_target_index_from_source(&src);
        let ev = open_at(u32::try_from(src.len()).unwrap());
        // Installed: the map has no `BAREPRED` key (it was never a quote body),
        // so the target is NOT preceded. Any mutation that fails to install
        // falls back to the substring scan, which *does* find `BAREPRED`.
        assert!(!forward_target_is_preceded(&ev, &src, 0, "BAREPRED"));
        clear_forward_target_index();
    }

    #[test]
    fn forward_index_skips_install_when_few_distinct_bodies() {
        clear_forward_target_index();
        let mut src = String::from("BAREPRED");
        for _ in 0..64 {
            src.push_str("「dup」");
        }
        install_forward_target_index_from_source(&src);
        let ev = open_at(u32::try_from(src.len()).unwrap());
        // 64 opens but a single distinct body (< threshold) → NOT installed →
        // the substring fallback finds `BAREPRED`. A mutant that installs the
        // tiny map would miss the bare key and wrongly answer "not preceded".
        assert!(forward_target_is_preceded(&ev, &src, 0, "BAREPRED"));
        clear_forward_target_index();
    }

    #[test]
    fn forward_index_records_quote_body_positions() {
        clear_forward_target_index();
        // ASCII prefix so the body slice offsets are on clean boundaries.
        let mut src = String::from("start");
        for i in 0..64u32 {
            src.push('「');
            write!(src, "q{i:02}").unwrap();
            src.push('」');
        }
        install_forward_target_index_from_source(&src);
        let ev = open_at(u32::try_from(src.len()).unwrap());
        // The map keys are the exact quote bodies. A body-slice offset bug maps
        // the wrong bytes and loses the real key.
        assert!(forward_target_is_preceded(&ev, &src, 0, "q05"));
        // A body absent from the source is not preceded.
        assert!(!forward_target_is_preceded(&ev, &src, 0, "zz99"));
        clear_forward_target_index();
    }

    #[test]
    fn forward_index_cleared_after_dense_then_low_distinct_doc() {
        clear_forward_target_index();
        // Doc A installs (64 distinct bodies including `KEEPME`).
        let mut a = String::from("「KEEPME」");
        for i in 0..63u32 {
            a.push('「');
            write!(a, "a{i:02}").unwrap(); // a00..a62 — 63 more, 64 distinct total
            a.push('」');
        }
        install_forward_target_index_from_source(&a);
        // Doc B: 64 opens but 1 distinct body → the len-below-threshold tail
        // (`clear_forward_target_index`) must reset the installed flag.
        let mut b = String::from("bbb");
        for _ in 0..64 {
            b.push_str("「z」");
        }
        install_forward_target_index_from_source(&b);
        let ev = open_at(u32::try_from(b.len()).unwrap());
        // Cleared: `KEEPME` is absent from B's substring fallback. A stubbed
        // clear leaves A's map installed and wrongly answers "preceded".
        assert!(!forward_target_is_preceded(&ev, &b, 0, "KEEPME"));
        clear_forward_target_index();
    }

    #[test]
    fn forward_index_cleared_if_installed_after_small_doc() {
        clear_forward_target_index();
        let mut a = String::from("「KEEPME」");
        for i in 0..63u32 {
            a.push('「');
            write!(a, "a{i:02}").unwrap();
            a.push('」');
        }
        install_forward_target_index_from_source(&a);
        // Doc B has fewer than threshold opens → the early
        // `clear_forward_target_index_if_installed` branch must reset installed.
        let b = String::from("bbb「x」「y」「z」");
        install_forward_target_index_from_source(&b);
        let ev = open_at(u32::try_from(b.len()).unwrap());
        assert!(!forward_target_is_preceded(&ev, &b, 0, "KEEPME"));
        clear_forward_target_index();
    }

    // --- predecessor position helpers ------------------------------------

    #[test]
    fn find_immediate_predecessor_pins_offset_and_boundaries() {
        // cutoff > len, adjacent match → the byte where the target begins.
        assert_eq!(
            find_immediate_predecessor_target_position(&open_at(5), "aaaXY", 0, "XY"),
            Some(3)
        );
        // cutoff == len, adjacent match → offset 0 (equal-boundary case).
        assert_eq!(
            find_immediate_predecessor_target_position(&open_at(2), "XY", 0, "XY"),
            Some(0)
        );
        // cutoff > len, no match → None.
        assert_eq!(
            find_immediate_predecessor_target_position(&open_at(5), "aaaZZ", 0, "XY"),
            None
        );
        // cutoff < len → no room, None.
        assert_eq!(
            find_immediate_predecessor_target_position(&open_at(1), "X", 0, "XY"),
            None
        );
    }

    #[test]
    fn resolve_forward_referent_locates_interior_occurrence() {
        // `XY` occurs at byte 5 inside the pending run [3, 9) but is not
        // byte-adjacent to the bracket at cutoff 9, so it is an interior span.
        match resolve_forward_referent(
            ReferentSearch {
                events: &open_at(9),
                source: "pppqqXYZZ",
                open_idx: 0,
                pending_plain_start: Some(3),
            },
            "XY",
        ) {
            ForwardReferent::Interior { start, end } => assert_eq!((start, end), (5, 7)),
            _ => panic!("expected an interior referent"),
        }
    }

    // --- quote extraction ------------------------------------------------

    #[test]
    fn extract_collects_every_consecutive_quote_target() {
        let (src, ev, links, oi, ci) = build("", &["A", "B"], "に傍点");
        let view = BodyView {
            events: &ev,
            links: &links,
        };
        let ex = extract_forward_quote_targets(view, &src, oi, ci).expect("two targets");
        assert_eq!(ex.targets.to_vec(), vec!["A", "B"]);
        assert_eq!(ex.suffix, "に傍点");
    }

    #[test]
    fn build_bouten_target_glues_multi_targets_with_ideographic_comma() {
        let mut alloc = Allocator::new();
        let c = build_bouten_target(&["A", "B"], &mut alloc);
        let Content::Plain(id) = c else {
            panic!("all-text multi-target folds to a single Plain run");
        };
        assert_eq!(alloc.store().resolve_str(id), "A、B");
    }

    // --- range bouten ----------------------------------------------------

    #[test]
    fn range_bouten_reclaims_run_from_x_to_y() {
        let (src, ev, links, oi, ci) = build("AqABB", &["A"], "～「BB」に傍点");
        let view = BodyView {
            events: &ev,
            links: &links,
        };
        let mut alloc = Allocator::new();
        let (node, cs) = {
            let mut c = RecogniseCtx {
                alloc: &mut alloc,
                source: &src,
                diagnostics: Vec::new(),
                pending_plain_start: None,
                pending_decoration: None,
            };
            c.classify_forward_bouten_range(view, oi, ci)
                .expect("range bouten recognised")
        };
        // Run start is the last `A` before `BB` (byte 2), not byte 0.
        assert_eq!(cs, 2);
        let Node::Format(f) = node else {
            panic!("expected a Format node, got {node:?}");
        };
        assert!(matches!(
            f.attr,
            ForwardAttr::Bouten {
                position: BoutenPosition::Right,
                ..
            }
        ));
        assert_eq!(alloc.store().content_range_as_plain(f.target), Some("ABB"));
    }

    #[test]
    fn range_bouten_declines_empty_end_target() {
        let (src, ev, links, oi, ci) = build("AqA", &["A"], "～「」に傍点");
        let view = BodyView {
            events: &ev,
            links: &links,
        };
        let mut alloc = Allocator::new();
        let is_none = {
            let mut c = RecogniseCtx {
                alloc: &mut alloc,
                source: &src,
                diagnostics: Vec::new(),
                pending_plain_start: None,
                pending_decoration: None,
            };
            c.classify_forward_bouten_range(view, oi, ci).is_none()
        };
        assert!(is_none, "an empty Y target declines the range");
    }

    // --- ruby-stripped heading gate --------------------------------------

    #[test]
    fn heading_ruby_stripped_gate_drops_readings_and_bars() {
        let cases: &[(&str, &str, bool)] = &[
            ("A《x》B", "AB", true), // `《x》` reading stripped
            ("A｜B", "AB", true),    // explicit-base `｜` stripped
            ("AB", "AB", true),      // ruby-free run matches directly
            ("XY", "AB", false),     // genuinely absent
        ];
        for (prefix, target, want) in cases {
            let (src, ev, links, oi, _ci) = build(prefix, &["z"], "は大見出し");
            let view = BodyView {
                events: &ev,
                links: &links,
            };
            assert_eq!(
                forward_heading_target_is_preceded_ruby_stripped(view, &src, oi, target),
                *want,
                "prefix {prefix:?} target {target:?}"
            );
        }
    }

    // --- forward heading -------------------------------------------------

    fn run_heading(prefix: &str, targets: &[&str]) -> Option<bool> {
        clear_forward_target_index();
        let (src, ev, links, oi, ci) = build(prefix, targets, "は大見出し");
        let view = BodyView {
            events: &ev,
            links: &links,
        };
        let mut alloc = Allocator::new();
        let node = {
            let mut c = RecogniseCtx {
                alloc: &mut alloc,
                source: &src,
                diagnostics: Vec::new(),
                pending_plain_start: None,
                pending_decoration: None,
            };
            c.classify_forward_heading(view, oi, ci).map(|(n, _)| n)
        };
        node.map(|n| match n {
            Node::HeadingHint(h) => h.self_contained,
            other => panic!("expected a HeadingHint, got {other:?}"),
        })
    }

    #[test]
    fn heading_preceded_ruby_stripped_target_is_not_self_contained() {
        // Exact look-back misses (`両頭《り》の蛇《へ》` is not a substring of
        // `両頭の蛇`) but the ruby-stripped look-back hits → preceded.
        assert_eq!(
            run_heading("両頭《り》の蛇《へ》", &["両頭の蛇"]),
            Some(false)
        );
    }

    #[test]
    fn heading_missing_single_target_is_self_contained() {
        assert_eq!(run_heading("", &["序章"]), Some(true));
    }

    #[test]
    fn heading_multi_target_all_preceded_is_not_self_contained() {
        assert_eq!(run_heading("甲乙", &["甲", "乙"]), Some(false));
    }

    #[test]
    fn heading_multi_target_with_missing_referent_declines() {
        assert_eq!(run_heading("甲", &["甲", "丙"]), None);
    }

    // --- left-side ruby / side note --------------------------------------

    #[test]
    fn left_ruby_pulls_back_preceded_target() {
        clear_forward_target_index();
        let (src, ev, links, oi, ci) = build("漢", &["漢"], "の左に「かん」のルビ");
        let view = BodyView {
            events: &ev,
            links: &links,
        };
        let mut alloc = Allocator::new();
        let (node, cs) = {
            let mut c = RecogniseCtx {
                alloc: &mut alloc,
                source: &src,
                diagnostics: Vec::new(),
                pending_plain_start: None,
                pending_decoration: None,
            };
            c.classify_forward_left_ruby(view, oi, ci)
                .expect("left ruby recognised")
        };
        assert_eq!(cs, 0);
        assert!(matches!(node, Node::Ruby(_)));
    }

    #[test]
    fn side_note_pulls_back_preceded_target() {
        clear_forward_target_index();
        let (src, ev, links, oi, ci) = build("語", &["語"], "に「注」の注記");
        let view = BodyView {
            events: &ev,
            links: &links,
        };
        let mut alloc = Allocator::new();
        let (node, cs) = {
            let mut c = RecogniseCtx {
                alloc: &mut alloc,
                source: &src,
                diagnostics: Vec::new(),
                pending_plain_start: None,
                pending_decoration: None,
            };
            c.classify_forward_side_note(view, oi, ci)
                .expect("side note recognised")
        };
        assert_eq!(cs, 0);
        assert!(matches!(node, Node::MarginNote(_)));
    }

    // --- caption figure --------------------------------------------------

    #[test]
    fn caption_figure_captures_file_and_caption() {
        let (src, ev, links, oi, ci) = build("", &["図一"], "のキャプション付きの図（f.png）入る");
        let view = BodyView {
            events: &ev,
            links: &links,
        };
        let mut alloc = Allocator::new();
        let node = {
            let mut c = RecogniseCtx {
                alloc: &mut alloc,
                source: &src,
                diagnostics: Vec::new(),
                pending_plain_start: None,
                pending_decoration: None,
            };
            c.classify_caption_figure(view, oi, ci)
                .expect("caption figure recognised")
        };
        match node {
            Node::Illustration(ill) => {
                assert_eq!(alloc.store().resolve_str(ill.file), "f.png");
                assert!(ill.caption.is_some());
            }
            other => panic!("expected an Illustration, got {other:?}"),
        }
    }

    #[test]
    fn caption_figure_declines_when_tail_is_not_iru() {
        let (src, ev, links, oi, ci) = build("", &["図一"], "のキャプション付きの図（f.png）NG");
        let view = BodyView {
            events: &ev,
            links: &links,
        };
        let mut alloc = Allocator::new();
        let is_none = {
            let mut c = RecogniseCtx {
                alloc: &mut alloc,
                source: &src,
                diagnostics: Vec::new(),
                pending_plain_start: None,
                pending_decoration: None,
            };
            c.classify_caption_figure(view, oi, ci).is_none()
        };
        assert!(is_none, "a non-`入る` tail declines the caption figure");
    }

    // --- forward emphasis ------------------------------------------------

    #[test]
    fn emphasis_is_reclaimed_for_adjacent_bold_target() {
        clear_forward_target_index();
        let (src, ev, links, oi, ci) = build("太字", &["太字"], "は太字");
        let view = BodyView {
            events: &ev,
            links: &links,
        };
        let mut alloc = Allocator::new();
        let (node, cs) = {
            let mut c = RecogniseCtx {
                alloc: &mut alloc,
                source: &src,
                diagnostics: Vec::new(),
                pending_plain_start: None,
                pending_decoration: None,
            };
            let (n, s, _d) = c
                .classify_forward_emphasis(view, oi, ci)
                .expect("は太字 recognised");
            (n, s)
        };
        assert_eq!(cs, 0, "an adjacent target pulls the consume start back");
        let Node::Format(f) = node else {
            panic!("expected a Format node, got {node:?}");
        };
        assert_eq!(f.attr, ForwardAttr::Bold);
        assert_eq!(f.origin, ForwardOrigin::Reclaimed);
        assert_eq!(alloc.store().content_range_as_plain(f.target), Some("太字"));
    }

    #[test]
    fn emphasis_accepts_framed_via_ni_particle() {
        clear_forward_target_index();
        let (src, ev, links, oi, ci) = build("枠", &["枠"], "に罫囲み");
        let view = BodyView {
            events: &ev,
            links: &links,
        };
        let mut alloc = Allocator::new();
        let (node, cs) = {
            let mut c = RecogniseCtx {
                alloc: &mut alloc,
                source: &src,
                diagnostics: Vec::new(),
                pending_plain_start: None,
                pending_decoration: None,
            };
            let (n, s, _d) = c
                .classify_forward_emphasis(view, oi, ci)
                .expect("に罫囲み recognised as a frame");
            (n, s)
        };
        assert_eq!(cs, 0);
        let Node::Format(f) = node else {
            panic!("expected a Format node, got {node:?}");
        };
        assert_eq!(f.attr, ForwardAttr::Framed(EnclosureKind::Rule));
        assert_eq!(f.origin, ForwardOrigin::Reclaimed);
    }

    #[test]
    fn emphasis_accent_needs_a_single_composable_letter() {
        // A composable single Latin letter with no referent → self-contained.
        clear_forward_target_index();
        let (src, ev, links, oi, ci) = build("", &["e"], "はアクサン（´）付き");
        let view = BodyView {
            events: &ev,
            links: &links,
        };
        let mut alloc = Allocator::new();
        let (node, _cs) = {
            let mut c = RecogniseCtx {
                alloc: &mut alloc,
                source: &src,
                diagnostics: Vec::new(),
                pending_plain_start: None,
                pending_decoration: None,
            };
            let (n, s, _d) = c
                .classify_forward_emphasis(view, oi, ci)
                .expect("composable accent recognised");
            (n, s)
        };
        let Node::Format(f) = node else {
            panic!("expected a Format node, got {node:?}");
        };
        assert_eq!(f.attr, ForwardAttr::Accent(AccentMark::Acute));
        assert_eq!(f.origin, ForwardOrigin::SelfContained);

        // A non-composable target declines to `Directive{Unknown}`.
        clear_forward_target_index();
        let (src2, ev2, links2, oi2, ci2) = build("", &["の"], "はアクサン（´）付き");
        let view2 = BodyView {
            events: &ev2,
            links: &links2,
        };
        let mut alloc2 = Allocator::new();
        let is_none = {
            let mut c = RecogniseCtx {
                alloc: &mut alloc2,
                source: &src2,
                diagnostics: Vec::new(),
                pending_plain_start: None,
                pending_decoration: None,
            };
            c.classify_forward_emphasis(view2, oi2, ci2).is_none()
        };
        assert!(is_none, "a non-composable accent letter declines");
    }

    // --- box enclosure ---------------------------------------------------

    #[test]
    fn box_enclosure_reclaims_adjacent_box_target() {
        clear_forward_target_index();
        let (src, ev, links, oi, ci) = build("四", &["四"], "は「□」囲み");
        let view = BodyView {
            events: &ev,
            links: &links,
        };
        let mut alloc = Allocator::new();
        let (node, cs) = {
            let mut c = RecogniseCtx {
                alloc: &mut alloc,
                source: &src,
                diagnostics: Vec::new(),
                pending_plain_start: None,
                pending_decoration: None,
            };
            let (n, s, _d) = c
                .classify_forward_box_enclosure(view, oi, ci)
                .expect("box enclosure recognised");
            (n, s)
        };
        assert_eq!(cs, 0);
        let Node::Format(f) = node else {
            panic!("expected a Format node, got {node:?}");
        };
        assert_eq!(f.attr, ForwardAttr::Framed(EnclosureKind::Box));
        assert_eq!(f.origin, ForwardOrigin::Reclaimed);
    }

    // --- standalone gaiji ------------------------------------------------

    #[test]
    fn standalone_gaiji_claims_unresolvable_mencode_form() {
        // `未知の字形` is not in any table and the bogus `第3水準9-99-99`
        // mencode neither parses (plane 9 ≠ level 3) nor resolves — so the
        // gaiji stays claimed (has a mencode tail) but unresolved.
        let (src, ev, links, oi, _ci) = build("", &[], "「未知の字形」、第3水準9-99-99");
        let view = BodyView {
            events: &ev,
            links: &links,
        };
        let mut alloc = Allocator::new();
        let (node, unresolved) = {
            let mut c = RecogniseCtx {
                alloc: &mut alloc,
                source: &src,
                diagnostics: Vec::new(),
                pending_plain_start: None,
                pending_decoration: None,
            };
            c.classify_standalone_gaiji(view, oi)
                .expect("standalone gaiji with a mencode tail is claimed")
        };
        assert!(matches!(node, Node::Gaiji(_)));
        assert!(unresolved, "the bogus mencode does not resolve to a glyph");
    }

    // --- recognize_annotation empty / ellipsis catch ---------------------

    #[test]
    fn empty_and_ellipsis_bodies_type_as_empty_directive() {
        clear_forward_target_index();
        for body in ["", "…", "（…）"] {
            let (src, ev, links, oi, ci) = build("", &[], body);
            let view = BodyView {
                events: &ev,
                links: &links,
            };
            let mut alloc = Allocator::new();
            let emit = {
                let mut c = RecogniseCtx {
                    alloc: &mut alloc,
                    source: &src,
                    diagnostics: Vec::new(),
                    pending_plain_start: None,
                    pending_decoration: None,
                };
                c.recognize_annotation(view, oi, ci)
                    .expect("directive recognised")
                    .emit
            };
            match emit {
                EmitKind::Aozora(Node::Directive(d)) => {
                    assert_eq!(d.kind, DirectiveKind::Empty, "body {body:?}");
                }
                _ => panic!("expected an Empty directive for body {body:?}"),
            }
        }
    }

    // --- pure suffix / char helpers --------------------------------------

    #[test]
    fn forward_attr_suffix_maps_each_styling_keyword() {
        assert_eq!(forward_attr_from_suffix("太字"), Some(ForwardAttr::Bold));
        assert_eq!(
            forward_attr_from_suffix("ゴシック体"),
            Some(ForwardAttr::Gothic)
        );
        assert_eq!(forward_attr_from_suffix("斜体"), Some(ForwardAttr::Italic));
        assert_eq!(
            forward_attr_from_suffix("上付き小文字"),
            Some(ForwardAttr::SuperScript)
        );
        assert_eq!(
            forward_attr_from_suffix("下付き小文字"),
            Some(ForwardAttr::SubScript)
        );
        assert_eq!(
            forward_attr_from_suffix("横組み"),
            Some(ForwardAttr::Horizontal)
        );
        assert_eq!(
            forward_attr_from_suffix("キャプション"),
            Some(ForwardAttr::Caption)
        );
        assert_eq!(
            forward_attr_from_suffix("特大文字"),
            Some(ForwardAttr::FontSizeAbsolute(AbsoluteSize::ExtraLarge))
        );
        assert_eq!(
            forward_attr_from_suffix("大文字"),
            Some(ForwardAttr::FontSizeAbsolute(AbsoluteSize::Large))
        );
        assert_eq!(
            forward_attr_from_suffix("中文字"),
            Some(ForwardAttr::FontSizeAbsolute(AbsoluteSize::Medium))
        );
        assert_eq!(
            forward_attr_from_suffix("小文字"),
            Some(ForwardAttr::FontSizeAbsolute(AbsoluteSize::Small))
        );
    }

    #[test]
    fn parse_font_size_suffix_signs_and_magnitude() {
        assert_eq!(
            parse_font_size_suffix("3段階大きな文字"),
            Some(ForwardAttr::FontSize(FontShift(NonZeroI8::new(3).unwrap())))
        );
        assert_eq!(
            parse_font_size_suffix("2段階小さな文字"),
            Some(ForwardAttr::FontSize(FontShift(
                NonZeroI8::new(-2).unwrap()
            )))
        );
        assert_eq!(
            parse_font_size_suffix("5段階小さな文字"),
            Some(ForwardAttr::FontSize(FontShift(
                NonZeroI8::new(-5).unwrap()
            )))
        );
        assert_eq!(parse_font_size_suffix("nope"), None);
    }

    #[test]
    fn is_latin_run_char_admits_latin_and_joiners() {
        assert!(is_latin_run_char('a'));
        assert!(is_latin_run_char('Z'));
        assert!(is_latin_run_char('-'));
        assert!(is_latin_run_char('\u{0101}')); // ā — Latin-Extended-A
        assert!(is_latin_run_char('\u{1E41}')); // ṁ — Latin-Extended-Additional
        assert!(!is_latin_run_char('あ'));
        assert!(!is_latin_run_char('0'));
        assert!(!is_latin_run_char('、'));
    }

    #[test]
    fn reclaim_accent_run_start_finds_the_latin_run() {
        assert_eq!(reclaim_accent_run_start("abc"), Some(0));
        assert_eq!(reclaim_accent_run_start("あ"), None);
        assert_eq!(reclaim_accent_run_start("あabc"), Some(3));
        // A `〕`-terminated prefix reclaims the whole decomposed accent span.
        assert_eq!(reclaim_accent_run_start("x〔y〕"), Some(1));
    }

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
    fn tcy_small_script_compound_declines_to_unknown() {
        // `「X」は縦中横、行右/左小書き` reaches this emphasis suffix mapper (after
        // the exact-`は縦中横` tcy shape declines the `、`-suffix upstream) with a
        // compound keyword that is not a claimable emphasis attribute, so it stays
        // `Directive{Unknown}` rather than being misclaimed as a small-script —
        // the dropped small-script axis is served render-only by Tier2 --degraded
        // (ADR-0027 A5), never on the default parse path.
        assert_eq!(forward_attr_from_suffix("縦中横、行右小書き"), None);
        assert_eq!(forward_attr_from_suffix("縦中横、行左小書き"), None);
        // The bare small-script words ARE claimable alone — proving the compound
        // declines because of the leading `縦中横、`, not the small-script word.
        assert!(forward_attr_from_suffix("行右小書き").is_some());
        assert!(forward_attr_from_suffix("行左小書き").is_some());
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
