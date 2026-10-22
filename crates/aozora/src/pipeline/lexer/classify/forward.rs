#![expect(
    clippy::expect_used,
    reason = "branch guards and the sanitize-stage size cap establish these invariants"
)]

use std::cell::RefCell;
use std::collections::HashMap;

use core::num::NonZeroI8;

use crate::spec::Diagnostic;
use crate::syntax::accent::{compose_accent, compose_accent_dots};
use crate::syntax::alloc::Allocator;
use crate::syntax::ast::{Content, Node, Segment};
use crate::syntax::degraded::degraded_directive;
use crate::syntax::format::ForwardOrigin;
use crate::syntax::lint::canonical_directive;
use crate::syntax::{
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
    static FORWARD_TARGET_INDEX: RefCell<ForwardTargetState> = RefCell::default();
}

#[derive(Default)]
struct ForwardTargetState {
    installed: bool,
    first_position: HashMap<String, u32>,
}

fn clear_forward_target_index() {
    FORWARD_TARGET_INDEX.with(|cell| {
        let mut state = cell.borrow_mut();
        state.installed = false;
        state.first_position.clear();
    });
}

fn clear_forward_target_index_if_installed() {
    FORWARD_TARGET_INDEX.with(|cell| {
        let mut state = cell.borrow_mut();
        if state.installed {
            state.installed = false;
            state.first_position.clear();
        }
    });
}

// The measured index construction cost exceeds the cost of the substring scans saved below this threshold.
const FORWARD_QUOTE_BODY_THRESHOLD: usize = 64;

pub(super) fn install_forward_target_index_from_source(source: &str) {
    use memchr::memmem;

    const QUOTE_OPEN: &[u8] = b"\xE3\x80\x8C";
    const QUOTE_CLOSE: &[u8] = b"\xE3\x80\x8D";

    let bytes = source.as_bytes();
    let mut count = 0usize;
    for _ in memmem::find_iter(bytes, QUOTE_OPEN) {
        count = count.saturating_add(1);
        if count.cmp(&FORWARD_QUOTE_BODY_THRESHOLD).is_ge() {
            break;
        }
    }
    if count.cmp(&FORWARD_QUOTE_BODY_THRESHOLD).is_lt() {
        clear_forward_target_index_if_installed();
        return;
    }
    let opens: Vec<usize> = memmem::find_iter(bytes, QUOTE_OPEN).collect();

    let mut first_positions: HashMap<String, u32> = HashMap::with_capacity(opens.len());
    let mut open_stack: Vec<usize> = Vec::new();
    let mut opens_iter = opens.iter().copied().peekable();
    for close_pos in memmem::find_iter(bytes, QUOTE_CLOSE) {
        while opens_iter
            .peek()
            .is_some_and(|&op| op.cmp(&close_pos).is_lt())
        {
            open_stack.push(opens_iter.next().expect("peeked Some"));
        }
        let Some(open_pos) = open_stack.pop() else {
            continue;
        };
        let body_start = open_pos.saturating_add(QUOTE_OPEN.len());
        let body = &source[body_start..close_pos];
        if body.is_empty() {
            continue;
        }
        first_positions.entry(body.to_owned()).or_insert_with(|| {
            let first = memmem::find(bytes, body.as_bytes()).unwrap_or(body_start);
            u32::try_from(first).expect("source positions fit in u32 per sanitize-stage cap")
        });
    }

    if first_positions
        .len()
        .cmp(&FORWARD_QUOTE_BODY_THRESHOLD)
        .is_lt()
    {
        clear_forward_target_index();
        return;
    }

    FORWARD_TARGET_INDEX.with(|cell| {
        let mut state = cell.borrow_mut();
        state.installed = true;
        state.first_position = first_positions;
    });
}

impl RecogniseCtx<'_, '_> {
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

        let hash_end = match events.get(open_idx + 1)? {
            PairEvent::Solo {
                kind: TriggerKind::Hash,
                span,
            } => span.end,
            _ => return None,
        };

        let body = self.source[hash_end as usize..close_span.start as usize].trim();

        if let Some((emit, annotation_payload)) = classify_annotation_body(body, self.alloc) {
            return Some(AnnotationMatch {
                emit,
                annotation_payload,
                consume_start: open_span.start,
                consume_end: close_span.end,
                pending_diagnostic: None,
            });
        }
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

        if let Some(node) = self.classify_caption_figure(view, open_idx, close_idx) {
            return Some(AnnotationMatch {
                emit: EmitKind::Aozora(node),
                annotation_payload: None,
                consume_start: open_span.start,
                consume_end: close_span.end,
                pending_diagnostic: None,
            });
        }

        if let Some(emit) = classify_general_image_body(body, self.alloc) {
            return Some(AnnotationMatch {
                emit,
                annotation_payload: None,
                consume_start: open_span.start,
                consume_end: close_span.end,
                pending_diagnostic: None,
            });
        }

        if body.is_empty() || body == "…" || body == "（…）" {
            return Some(self.typed_annotation_match(directive_span, DirectiveKind::Empty));
        }
        if let Some(kind) = editorial_note_kind(body) {
            return Some(self.typed_annotation_match(directive_span, kind));
        }

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

        Some(self.unknown_annotation_match(directive_span, body, tcy_pending))
    }

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
        if !m.payload.canonical.has_mencode() && m.payload.resolve(self.alloc.store()).is_none() {
            return None;
        }
        let unresolved = m.payload.resolve(self.alloc.store()).is_none();
        Some((self.alloc.gaiji(m.payload), unresolved))
    }

    fn unknown_annotation_match(
        &mut self,
        directive_span: Span,
        body: &str,
        tcy_pending: Option<Diagnostic>,
    ) -> AnnotationMatch {
        let raw = &self.source[directive_span.start as usize..directive_span.end as usize];
        let canonical = canonical_directive(body);
        let kind = if canonical.is_some() || degraded_directive(body).is_some() {
            DirectiveKind::NonCanonical
        } else {
            DirectiveKind::Editorial
        };
        let payload = self.alloc.make_directive(raw, kind);
        let node = self.alloc.annotation(payload);
        let payload_for_seg = self.alloc.make_directive(raw, kind);
        let pending_diagnostic = canonical
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

impl RecogniseCtx<'_, '_> {
    fn classify_forward_bouten_range(
        &mut self,
        view: BodyView<'_>,
        open_idx: usize,
        close_idx: usize,
    ) -> Option<(Node, u32)> {
        let extracted = extract_forward_quote_targets(view, self.source, open_idx, close_idx)?;
        let [start_target] = extracted.targets.as_slice() else {
            return None;
        };
        let suffix = extracted.suffix;
        let rest = suffix
            .strip_prefix("～「")
            .or_else(|| suffix.strip_prefix("〜「"))?;
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
            self.alloc.bouten_range(kind, content, position),
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
        let suffix = extracted.suffix;
        let (position, kind_suffix) = if let Some(rest) = suffix.strip_prefix("に") {
            (BoutenPosition::Right, rest)
        } else if let Some(rest) = suffix.strip_prefix("の左に") {
            (BoutenPosition::Left, rest)
        } else {
            let rest = suffix.strip_prefix("の両側に")?;
            (BoutenPosition::Both, rest)
        };
        let kind = bouten_kind_from_suffix(kind_suffix)?;
        let &PairEvent::PairOpen {
            span: open_span, ..
        } = view.events.get(open_idx)?
        else {
            return None;
        };
        if let [only] = extracted.targets.as_slice()
            && !forward_target_is_preceded(view.events, self.source, open_idx, only)
        {
            let target = build_bouten_target(&extracted.targets, self.alloc);
            return Some((
                if extracted.nested_source {
                    self.alloc.forward_format_nested(
                        ForwardAttr::Bouten { kind, position },
                        target,
                        ForwardOrigin::SelfContained,
                    )
                } else {
                    self.alloc
                        .bouten(kind, target, position, ForwardOrigin::SelfContained)
                },
                open_span.start,
                ForwardDiag::None,
            ));
        }
        for target in &extracted.targets {
            if !forward_target_is_preceded(view.events, self.source, open_idx, target) {
                return None;
            }
        }
        let [only] = extracted.targets.as_slice() else {
            let target = build_bouten_target(&extracted.targets, self.alloc);
            return Some((
                self.alloc
                    .bouten(kind, target, position, ForwardOrigin::Referenced),
                open_span.start,
                ForwardDiag::NotStylable,
            ));
        };
        let (node, consume_start, diag) = self.resolve_forward_format(
            ForwardBracket {
                view,
                open_idx,
                open_span_start: open_span.start,
                nested_source: extracted.nested_source,
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

enum ForwardTcy {
    Recognised(Node, u32, ForwardDiag),
    ShapedNoTarget,
    NotTcy,
}

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
        if extracted.suffix != "は縦中横" {
            return ForwardTcy::NotTcy;
        }
        let [first] = extracted.targets.as_slice() else {
            return ForwardTcy::NotTcy;
        };
        if !forward_target_is_preceded(view.events, self.source, open_idx, first) {
            return ForwardTcy::ShapedNoTarget;
        }
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
                nested_source: extracted.nested_source,
            },
            ForwardAttr::CombineUpright,
            first,
        );
        ForwardTcy::Recognised(node, consume_start, diag)
    }
}

fn forward_target_is_preceded(
    events: &[PairEvent],
    source: &str,
    open_idx: usize,
    target: &str,
) -> bool {
    let Some(PairEvent::PairOpen { span, .. }) = events.get(open_idx) else {
        return false;
    };
    let cutoff = span.start;

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

    source[..cutoff as usize].contains(target)
}

// The corpus includes headings that quote 両頭 from 両頭《りやうとう》, so their look-back omits ruby readings.
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
    let stripped = strip_ruby_notation(prefix);
    let target = strip_ruby_notation(target);
    stripped.contains(&target)
}

fn strip_ruby_notation(source: &str) -> String {
    let mut stripped = String::with_capacity(source.len());
    let mut in_ruby = false;
    for ch in source.chars() {
        match ch {
            '《' => in_ruby = true,
            '》' => in_ruby = false,
            '｜' => {}
            _ if in_ruby => {}
            _ => stripped.push(ch),
        }
    }
    stripped
}

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
    if &source.as_bytes()[candidate_start..cutoff] == target.as_bytes() {
        Some(u32::try_from(candidate_start).ok()?)
    } else {
        None
    }
}

enum ForwardReferent {
    Adjacent(u32),
    Interior { start: u32, end: u32 },
    Unresolvable,
}

enum ForwardDiag {
    None,
    Ambiguous,
    NotStylable,
}

impl ForwardDiag {
    fn into_diagnostic(self, directive_span: Span) -> Option<Diagnostic> {
        match self {
            Self::None => None,
            Self::Ambiguous => Some(Diagnostic::bouten_target_ambiguous(directive_span)),
            Self::NotStylable => Some(Diagnostic::forward_referent_not_stylable(directive_span)),
        }
    }
}

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
    if cutoff >= len && &source.as_bytes()[cutoff - len..cutoff] == target.as_bytes() {
        return u32::try_from(cutoff - len)
            .map_or(ForwardReferent::Unresolvable, ForwardReferent::Adjacent);
    }
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

struct ForwardQuoteExtract<'s> {
    // This inline target capacity covers the measured 99th percentile in the corpus.
    targets: smallvec::SmallVec<[&'s str; 4]>,
    suffix: &'s str,
    nested_source: bool,
}

fn trim_forward_suffix(suffix: &str) -> &str {
    suffix.trim()
}

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
    let mut cursor = open_idx.saturating_add(2);
    let mut last_quote_end: u32 = 0;
    let mut nested_source = false;

    while let Some(&PairEvent::PairOpen {
        kind: PairKind::Quote,
        span: quote_open_span,
    }) = events.get(cursor)
    {
        let quote_close_link = *view.links.get(cursor)?;
        if quote_close_link == u32::MAX {
            return None;
        }
        let quote_close_idx = quote_close_link as usize;
        if quote_close_idx.cmp(&close_idx).is_ge() {
            return None;
        }
        let Some(&PairEvent::PairClose {
            span: quote_close_span,
            ..
        }) = events.get(quote_close_idx)
        else {
            return None;
        };
        let body = &source[quote_open_span.end as usize..quote_close_span.start as usize];
        if !body.is_empty() {
            targets.push(body);
            let adjacent_close = cursor.saturating_add(2);
            if quote_close_idx.cmp(&adjacent_close).is_ne() {
                nested_source = true;
            }
        }
        last_quote_end = quote_close_span.end;
        cursor = quote_close_idx.saturating_add(1);
    }

    if targets.is_empty() {
        return None;
    }
    let suffix =
        trim_forward_suffix(&source[last_quote_end as usize..bracket_close_span.start as usize]);
    Some(ForwardQuoteExtract {
        targets,
        suffix,
        nested_source,
    })
}

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

        let preceded = |target: &str| {
            forward_target_is_preceded(view.events, self.source, open_idx, target)
                || forward_heading_target_is_preceded_ruby_stripped(
                    view,
                    self.source,
                    open_idx,
                    target,
                )
        };

        let self_contained = match extracted.targets.as_slice() {
            [only] => !preceded(only),
            targets if targets.iter().any(|t| !t.is_empty() && !preceded(t)) => return None,
            _ => false,
        };

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
        let suffix = extracted.suffix;
        let reading_text = suffix.strip_prefix("の左に「")?.strip_suffix("」のルビ")?;
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
        let base = self.build_content_from_source_slice(view, target);
        let reading = self.build_content_from_source_slice(view, reading_text);
        Some((self.alloc.left_ruby(base, reading), consume_start))
    }
}

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
        let suffix = extracted.suffix;
        let (kind, note_text) = if let Some(inner) = suffix.strip_suffix("」の注記") {
            let note = inner
                .strip_prefix("の左に「")
                .or_else(|| inner.strip_prefix("に「"))?;
            (MarginNoteKind::Gloss, note)
        } else {
            let inner = suffix.strip_suffix("」の傍記")?;
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
        let base = self.build_content_from_source_slice(view, target);
        let note = self.build_content_from_source_slice(view, note_text);
        Some((self.alloc.side_note(kind, base, note), consume_start))
    }

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
        let caption_content = self.build_content_from_source_slice(view, caption);
        Some(self.alloc.sashie(file, None, None, Some(caption_content)))
    }
}

#[derive(Clone, Copy)]
struct ForwardBracket<'a> {
    view: BodyView<'a>,
    open_idx: usize,
    open_span_start: u32,
    nested_source: bool,
}

impl RecogniseCtx<'_, '_> {
    fn resolve_forward_format(
        &mut self,
        bracket: ForwardBracket<'_>,
        attr: ForwardAttr,
        target: &str,
    ) -> (Node, u32, ForwardDiag) {
        let ForwardBracket {
            view,
            open_idx,
            open_span_start,
            nested_source,
        } = bracket;
        let text = self.alloc.content_plain(target);
        match resolve_forward_referent(
            ReferentSearch {
                events: view.events,
                source: self.source,
                open_idx,
                pending_plain_start: self.pending_plain_start,
            },
            target,
        ) {
            ForwardReferent::Adjacent(consume_start) => (
                if nested_source {
                    self.alloc
                        .forward_format_nested(attr, text, ForwardOrigin::Reclaimed)
                } else {
                    self.alloc
                        .forward_format(attr, text, ForwardOrigin::Reclaimed)
                },
                consume_start,
                ForwardDiag::None,
            ),
            ForwardReferent::Interior { start, end } => {
                let deco = if nested_source {
                    self.alloc
                        .forward_format_nested(attr, text, ForwardOrigin::Detached)
                } else {
                    self.alloc
                        .forward_format(attr, text, ForwardOrigin::Detached)
                };
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
        let suffix = extracted.suffix;
        let attr = if let Some(rest) = suffix.strip_prefix("は") {
            forward_attr_from_suffix(rest)?
        } else {
            let rest = suffix.strip_prefix("に")?;
            match forward_attr_from_suffix(rest)? {
                framed @ ForwardAttr::Framed(_) => framed,
                _ => return None,
            }
        };
        let [only] = extracted.targets.as_slice() else {
            return None;
        };
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
            let text = self.alloc.content_plain(only);
            return Some((
                if extracted.nested_source {
                    self.alloc
                        .forward_format_nested(attr, text, ForwardOrigin::SelfContained)
                } else {
                    self.alloc
                        .forward_format(attr, text, ForwardOrigin::SelfContained)
                },
                open_span.start,
                ForwardDiag::None,
            ));
        }
        Some(self.resolve_forward_format(
            ForwardBracket {
                view,
                open_idx,
                open_span_start: open_span.start,
                nested_source: extracted.nested_source,
            },
            attr,
            only,
        ))
    }
}

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
            let text = self.alloc.content_plain(target);
            return Some((
                if extracted.nested_source {
                    self.alloc
                        .forward_format_nested(attr, text, ForwardOrigin::SelfContained)
                } else {
                    self.alloc
                        .forward_format(attr, text, ForwardOrigin::SelfContained)
                },
                open_span.start,
                ForwardDiag::None,
            ));
        }
        Some(self.resolve_forward_format(
            ForwardBracket {
                view,
                open_idx,
                open_span_start: open_span.start,
                nested_source: extracted.nested_source,
            },
            attr,
            target,
        ))
    }
}

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
        compose_accent_dots(run, body)?;
        let text = self.alloc.content_plain(run);
        let consume_start = u32::try_from(run_start).ok()?;
        let origin = ForwardOrigin::from_consume(consume_start, open_span.start);
        Some((self.alloc.accent_dot(text, body, origin), consume_start))
    }
}

fn reclaim_accent_run_start(prefix: &str) -> Option<usize> {
    if prefix.ends_with('〕') {
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

fn is_latin_run_char(ch: char) -> bool {
    ch.is_ascii_alphabetic()
        || ch == '-'
        || matches!(ch, '\u{00C0}'..='\u{024F}' | '\u{1E00}'..='\u{1EFF}')
}

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
        "○付き文字" => ForwardAttr::Framed(EnclosureKind::Circle),
        "点線丸囲み" => ForwardAttr::Framed(EnclosureKind::CircleDotted),
        "二重罫囲み" => ForwardAttr::Framed(EnclosureKind::DoubleRule),
        "横組み" => ForwardAttr::Horizontal,
        "キャプション" => ForwardAttr::Caption,
        "特大文字" => ForwardAttr::FontSizeAbsolute(AbsoluteSize::ExtraLarge),
        "大文字" => ForwardAttr::FontSizeAbsolute(AbsoluteSize::Large),
        "中文字" => ForwardAttr::FontSizeAbsolute(AbsoluteSize::Medium),
        "小文字" => ForwardAttr::FontSizeAbsolute(AbsoluteSize::Small),
        "分数" => ForwardAttr::Fraction,
        _ => {
            return parse_font_size_suffix(s)
                .or_else(|| parse_align_end_suffix(s))
                .or_else(|| parse_accent_suffix(s));
        }
    })
}

fn parse_accent_suffix(s: &str) -> Option<ForwardAttr> {
    let mark = match s {
        "アクサン（´）付き" => AccentMark::Acute,
        "アクサン（｀）付き" => AccentMark::Grave,
        "ウムラウト（¨）付き" => AccentMark::Umlaut,
        _ => return None,
    };
    Some(ForwardAttr::Accent(mark))
}

fn parse_align_end_suffix(s: &str) -> Option<ForwardAttr> {
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

    fn open_at(cutoff: u32) -> Vec<PairEvent> {
        vec![PairEvent::PairOpen {
            kind: PairKind::Bracket,
            span: Span::new(cutoff, cutoff + 3),
        }]
    }

    #[test]
    fn forward_index_install_uses_map_for_bare_predecessor() {
        clear_forward_target_index();
        let mut src = String::from("BAREPRED");
        for i in 0..64u32 {
            src.push('「');
            write!(src, "q{i:02}").unwrap();
            src.push('」');
        }
        install_forward_target_index_from_source(&src);
        let ev = open_at(u32::try_from(src.len()).unwrap());
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
        assert!(forward_target_is_preceded(&ev, &src, 0, "BAREPRED"));
        clear_forward_target_index();
    }

    #[test]
    fn forward_index_records_quote_body_positions() {
        clear_forward_target_index();
        let mut src = String::from("start");
        for i in 0..64u32 {
            src.push('「');
            write!(src, "q{i:02}").unwrap();
            src.push('」');
        }
        install_forward_target_index_from_source(&src);
        let ev = open_at(u32::try_from(src.len()).unwrap());
        assert!(forward_target_is_preceded(&ev, &src, 0, "q05"));
        assert!(!forward_target_is_preceded(&ev, &src, 0, "zz99"));
        clear_forward_target_index();
    }

    #[test]
    fn forward_index_cleared_after_dense_then_low_distinct_doc() {
        clear_forward_target_index();
        let mut a = String::from("「KEEPME」");
        for i in 0..63u32 {
            a.push('「');
            write!(a, "a{i:02}").unwrap();
            a.push('」');
        }
        install_forward_target_index_from_source(&a);
        let mut b = String::from("bbb");
        for _ in 0..64 {
            b.push_str("「z」");
        }
        install_forward_target_index_from_source(&b);
        let ev = open_at(u32::try_from(b.len()).unwrap());
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
        let b = String::from("bbb「x」「y」「z」");
        install_forward_target_index_from_source(&b);
        let ev = open_at(u32::try_from(b.len()).unwrap());
        assert!(!forward_target_is_preceded(&ev, &b, 0, "KEEPME"));
        clear_forward_target_index();
    }

    #[test]
    fn find_immediate_predecessor_pins_offset_and_boundaries() {
        assert_eq!(
            find_immediate_predecessor_target_position(&open_at(5), "aaaXY", 0, "XY"),
            Some(3)
        );
        assert_eq!(
            find_immediate_predecessor_target_position(&open_at(2), "XY", 0, "XY"),
            Some(0)
        );
        assert_eq!(
            find_immediate_predecessor_target_position(&open_at(5), "aaaZZ", 0, "XY"),
            None
        );
        assert_eq!(
            find_immediate_predecessor_target_position(&open_at(1), "X", 0, "XY"),
            None
        );
    }

    #[test]
    fn resolve_forward_referent_locates_interior_occurrence() {
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

    #[test]
    fn forward_suffix_trims_unicode_whitespace() {
        assert_eq!(trim_forward_suffix("\u{2003}に傍点\u{205f}"), "に傍点");
        assert_eq!(trim_forward_suffix("\u{3000}は太字\u{00a0}"), "は太字");
    }

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

    fn run_forward_bouten(prefix: &str) -> ForwardDiag {
        let (src, events, links, open_idx, close_idx) = build(prefix, &["A"], "に傍点");
        let view = BodyView {
            events: &events,
            links: &links,
        };
        let mut alloc = Allocator::new();
        let mut context = RecogniseCtx {
            alloc: &mut alloc,
            source: &src,
            diagnostics: Vec::new(),
            pending_plain_start: None,
            pending_decoration: None,
        };
        context
            .classify_forward_bouten(view, open_idx, close_idx)
            .expect("forward bouten recognised")
            .2
    }

    #[test]
    fn forward_bouten_ambiguity_starts_at_the_second_candidate() {
        assert!(matches!(run_forward_bouten("A"), ForwardDiag::None));
        assert!(matches!(run_forward_bouten("A A"), ForwardDiag::Ambiguous));
    }

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

    #[test]
    fn heading_ruby_stripped_gate_drops_readings_and_bars() {
        let cases: &[(&str, &str, bool)] = &[
            ("A《x》B", "AB", true),
            ("A｜B", "AB", true),
            ("A《x》B《y》", "A《x》B", true),
            ("AB", "AB", true),
            ("XY", "AB", false),
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

    #[test]
    fn standalone_gaiji_claims_unresolvable_mencode_form() {
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
        assert!(is_latin_run_char('\u{0101}'));
        assert!(is_latin_run_char('\u{1E41}'));
        assert!(!is_latin_run_char('あ'));
        assert!(!is_latin_run_char('0'));
        assert!(!is_latin_run_char('、'));
    }

    #[test]
    fn reclaim_accent_run_start_finds_the_latin_run() {
        assert_eq!(reclaim_accent_run_start("abc"), Some(0));
        assert_eq!(reclaim_accent_run_start("あ"), None);
        assert_eq!(reclaim_accent_run_start("あabc"), Some(3));
        assert_eq!(reclaim_accent_run_start("x〔y〕"), Some(1));
    }

    #[test]
    fn accent_suffixes_map_to_their_marks() {
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
        assert_eq!(forward_attr_from_suffix("アクサン(´)付き"), None);
        assert_eq!(forward_attr_from_suffix("アクサン付き"), None);
    }

    #[test]
    fn enclosure_suffixes_map_to_their_kinds() {
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
        assert_eq!(
            forward_attr_from_suffix("罫囲み"),
            Some(ForwardAttr::Framed(EnclosureKind::Rule))
        );
        assert_eq!(forward_attr_from_suffix("破線枠囲み"), None);
        assert_eq!(forward_attr_from_suffix("丸囲み"), None);
    }

    #[test]
    fn fraction_suffix_matches_only_the_exact_single_form() {
        assert_eq!(
            forward_attr_from_suffix("分数"),
            Some(ForwardAttr::Fraction)
        );
        assert_eq!(
            forward_attr_from_suffix("上付き小文字、「1/143」は分数"),
            None
        );
        assert_eq!(forward_attr_from_suffix("分数、縦中横"), None);
    }

    #[test]
    fn tcy_small_script_compound_declines_to_unknown() {
        assert_eq!(forward_attr_from_suffix("縦中横、行右小書き"), None);
        assert_eq!(forward_attr_from_suffix("縦中横、行左小書き"), None);
        assert!(forward_attr_from_suffix("行右小書き").is_some());
        assert!(forward_attr_from_suffix("行左小書き").is_some());
    }

    #[test]
    fn align_end_suffix_parses_offset_and_verb_variants() {
        assert_eq!(
            forward_attr_from_suffix("文末より１字上げ揃え"),
            Some(ForwardAttr::AlignEnd { offset: 1 })
        );
        assert_eq!(
            forward_attr_from_suffix("行末より2字上がり"),
            Some(ForwardAttr::AlignEnd { offset: 2 })
        );
        assert_eq!(
            forward_attr_from_suffix("文末より3字上げ"),
            Some(ForwardAttr::AlignEnd { offset: 3 })
        );
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
        assert_eq!(forward_attr_from_suffix("地付け"), None);
        assert_eq!(forward_attr_from_suffix("地より0字上げ"), None);
        assert_eq!(forward_attr_from_suffix("地付き、地より３字アキ"), None);
        assert_eq!(forward_attr_from_suffix("文末より0字上げ"), None);
        assert_eq!(forward_attr_from_suffix("文末より字上げ"), None);
        assert_eq!(forward_attr_from_suffix("文頭より1字上げ"), None);
    }
}
