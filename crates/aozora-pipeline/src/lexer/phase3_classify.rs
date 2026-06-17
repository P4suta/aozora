//! Phase 3 — classify the Phase 2 event stream into [`borrowed::AozoraNode`] spans.
//!
//! Walks the cross-linked [`PairEvent`] stream produced by Phase 2 and
//! produces a contiguous vector of [`ClassifiedSpan`] whose
//! `source_span` values tile every byte of the sanitized source
//! end-to-end, in byte-offset order.
//!
//! The span kinds are:
//!
//! * [`SpanKind::Plain`] — a run of text that carries no Aozora
//!   construct. Adjacent un-classified events (text, stray triggers,
//!   unclosed opens, unmatched closes) are merged into one span so
//!   Phase 4 can emit them verbatim in a single write.
//! * [`SpanKind::Aozora`] — a classified Aozora construct, carrying the
//!   concrete [`borrowed::AozoraNode`] that Phase 4 will replace with a PUA
//!   placeholder sentinel (see [`crate::INLINE_SENTINEL`] and friends).
//! * [`SpanKind::Newline`] — a `\n` in the sanitized text, kept as its
//!   own span kind because block-level annotations (Phase 4 block
//!   sentinel substitution) care about line boundaries.
//!
//! ## Span-coverage invariant
//!
//! When `source.len() > 0`:
//!
//! 1. `spans[0].source_span.start == 0`
//! 2. `spans[i].source_span.end == spans[i + 1].source_span.start`
//! 3. `spans[last].source_span.end == source.len()`
//!
//! When `source.is_empty()`, `spans` is empty.
//!
//! Phase 4 relies on this invariant to emit `normalized` text without
//! ever re-scanning `source`.
//!
//! ## Recogniser layout
//!
//! Every recogniser is a narrow function that inspects a
//! `&[PairEvent]` slice (often one pair's `body_events`) plus the
//! sanitized source. The driver loop's `Classifier::try_recognize`
//! dispatches based on the leading event kind:
//!
//! * Ruby (`｜X《Y》` explicit, trailing-kanji implicit)
//! * Bracket annotations, dispatched on the body keyword:
//!   fixed keyword (`改ページ` / `地付き` / ...), kaeriten
//!   (`一`/`二`/... plus okurigana `（X）`), indent / align-end
//!   (`N字下げ` / `地からN字上げ`), sashie (`挿絵`), forward-ref
//!   bouten, forward-ref TCY, paired-container open / close, and
//!   an `Annotation{Unknown}` catch-all.
//! * Gaiji — `※［＃...］` reference-mark + bracket combos.
//! * Double-angle quotation `≪…≫` (displayed as `《…》`) (`AngleQuote`).
//!
//! The catch-all makes every well-formed `［＃…］` bracket produce
//! *some* `AozoraNode`, so the Tier-A canary (no bare `［＃` in the
//! HTML output outside an `aozora-annotation` wrapper) holds regardless
//! of which specialised recogniser claims the bracket.
//!
//! ## Inlining note (negative result)
//!
//! `phase3_subsystems` (instrumented) reports 88 % of classify wall in
//! "iterator-dispatch overhead" and only 9.4 % in actual recogniser
//! leaves. The straightforward fix — sprinkle `#[inline]` on
//! `recognize_and_emit` / `try_ruby_emit` / `try_bracket_emit` /
//! `try_gaiji_emit` / `process_event` / `handle_top_level` /
//! `handle_stream_event` / `Iterator::next` — was tried and reverted:
//! aggressive inlining regressed throughput by 1–6 % across all
//! bands. Selective inline (only the *small* helpers `push_output` /
//! `flush_plain_up_to` / `append_to_frame` / `pending_outputs_pop_front`
//! plus Phase 1 `flush_text` / `pair_text_then` / `try_merge_double`)
//! brought it within ±1.3 % of baseline — neutral.
//!
//! Conclusion: **LLVM's default inline judgement is already optimal
//! on this code at -O3 + fat-LTO.** Forcing inline on
//! recogniser-wrapping dispatchers regresses via i-cache thrash
//! (the wrappers expand into the per-call hot path and code bloat
//! dominates the dispatch saving). The 88 % "overhead" reported by
//! the instrumented build is partly the instrumentation itself
//! (`Instant::now()` per guard); the production overhead is real but
//! attacking it with attributes alone doesn't move it.
//!
//! The remaining headroom requires *structural* changes (Vec-passing
//! between phases, removing iterator chains) rather than attribute
//! hints.

use core::mem;
use core::ops::Range;
use std::collections::VecDeque;

#[cfg(feature = "phase3-instrument")]
use super::instrumentation::{
    Subsystem, SubsystemGuard, YieldKind, record_pending_size, record_replay_body_size,
    record_yield,
};

use aozora_encoding::gaiji as gaiji_resolve;
// Phase 3 builds borrowed AST directly via `BorrowedAllocator`'s
// inherent methods. The `NodeAllocator` trait abstraction was retired
// in F.4 once the owned-AST path was gone.
use aozora_syntax::alloc::BorrowedAllocator;
use aozora_syntax::borrowed;
use aozora_syntax::{
    AlignEnd, AnnotationKind, AozoraHeadingKind, AozoraHeadingStyle, BoutenKind, BoutenPosition,
    Center, ContainerKind, EmphasisKind, Indent, SectionKind, Span,
};

use super::phase2_pair::{PairEvent, PairKind};
use super::token::TriggerKind;
use aozora_spec::Diagnostic;

/// One classified slice of the sanitized source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClassifiedSpan<'a> {
    pub kind: SpanKind<'a>,
    pub source_span: Span,
}

/// Classification of a [`ClassifiedSpan`].
///
/// Phase 4 (now folded into `aozora_lex::lex_into_arena`'s
/// `ArenaNormalizer` walk) maps the variants to PUA sentinels as
/// follows:
///
/// | variant        | sentinel              | `post_process` role |
/// |----------------|-----------------------|-------------------|
/// | `Plain`        | verbatim source bytes | — |
/// | `Newline`      | verbatim `\n`         | — |
/// | `Aozora(n)`    | `E001` if inline, `E002` if block-leaf | splice Aozora node into the AST |
/// | `BlockOpen`    | `E003`                | pair with matching `BlockClose` |
/// | `BlockClose`   | `E004`                | close nearest unclosed `BlockOpen` |
///
/// The `BlockOpen` / `BlockClose` split exists because paired
/// containers (`ここから字下げ` … `ここで字下げ終わり`) span arbitrary
/// content between the two markers. The lexer emits both markers as
/// independent spans and lets `post_process` walk the AST to wrap
/// sibling nodes in the container.
///
/// # Memory layout
///
/// The `Aozora(borrowed::AozoraNode<'a>)` variant is *not* boxed —
/// `borrowed::AozoraNode<'a>` is `Copy` and 16 bytes, so storing it
/// inline keeps `SpanKind` to `Aozora`-variant size while avoiding
/// the `Box` indirection the legacy owned shape paid.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum SpanKind<'a> {
    /// Source bytes that carry no Aozora construct. Emitted verbatim
    /// by the normalizer.
    Plain,
    /// Classified Aozora construct (inline span or block-leaf line).
    /// The normalizer replaces the source span with an `E001` (inline)
    /// or `E002` (block-leaf) sentinel and records the node in the
    /// placeholder registry keyed at the sentinel's normalized
    /// position.
    Aozora(borrowed::AozoraNode<'a>),
    /// Paired-container opener — `［＃ここから字下げ］`, `［＃罫囲み］`,
    /// etc. The normalizer emits an `E003` sentinel line; `post_process`
    /// matches it to the corresponding `BlockClose` via a balanced
    /// stack walk of the AST.
    BlockOpen(ContainerKind),
    /// Paired-container closer — `［＃ここで字下げ終わり］`,
    /// `［＃罫囲み終わり］`, etc. The normalizer emits an `E004`
    /// sentinel line; the carried `ContainerKind` is a hint used by
    /// `post_process` to diagnose `［＃罫囲み終わり］` closing an
    /// `Indent` opener (kind mismatch).
    BlockClose(ContainerKind),
    /// A `\n` in the sanitized text. Retained as its own span kind
    /// because block-level recognizers need line boundaries.
    Newline,
}

/// Classify a streaming Phase 2 [`PairEvent`] iterator against the
/// sanitized source.
///
/// Returns a [`ClassifyStream`] iterator yielding one [`ClassifiedSpan`]
/// per call to [`Iterator::next`]. After exhaustion, call
/// [`ClassifyStream::take_diagnostics`] to drain non-fatal observations
/// accumulated during recognition. The upstream pair stream's
/// diagnostics are NOT forwarded automatically — the caller is
/// responsible for calling `pair_stream.take_diagnostics()` after the
/// classify stream is dropped (the fused pipeline in `aozora-lex` does
/// this).
///
/// Pure function; no I/O. The yielded spans byte-contiguously cover
/// `source` — see the module-level span-coverage invariant.
#[must_use]
pub fn classify<'src, 'al, 'a, I>(
    events: I,
    source: &'src str,
    alloc: &'al mut BorrowedAllocator<'a>,
) -> ClassifyStream<'src, 'al, 'a, I::IntoIter>
where
    I: IntoIterator<Item = PairEvent>,
{
    // Pre-pass: scan raw source bytes for `「…」` quote bodies and
    // record the FIRST byte position of each unique body. The streaming
    // pipeline never materialises a `Vec<PairEvent>`, so the legacy
    // event-driven AC pre-pass (which walked the event slice to collect
    // forward-reference targets) doesn't fit; this source-byte variant
    // is event-free and pays one extra `memmem` sweep per document
    // before classification starts. Only installed when the source has
    // enough quote bodies to amortise the build (the median corpus doc
    // skips the index entirely; the pathological annotation-dense
    // 252-occurrence doc reclaims the 170 ms → 20 ms classify win this
    // index used to give the legacy event-driven pre-pass).
    install_forward_target_index_from_source(source);
    ClassifyStream::new(events.into_iter(), source, alloc)
}

use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::OnceLock;

use aho_corasick::{AhoCorasick, AhoCorasickBuilder, Anchored, Input, MatchKind, StartKind};

thread_local! {
    /// Forward-reference target → first byte offset in source.
    ///
    /// `state.installed = true` means the map is authoritative: every
    /// target queried by `forward_target_is_preceded` is either in the
    /// map or genuinely absent from source. `state.installed = false`
    /// means the lookup falls back to the legacy
    /// `source[..cutoff].contains` path for correctness.
    ///
    /// Pre-I-2 the streaming Phase 3 entry point built this index from
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
fn install_forward_target_index_from_source(source: &str) {
    use memchr::memmem;

    // `「` is U+300C, UTF-8 = E3 80 8C; `」` is U+300D, UTF-8 = E3 80 8D.
    const QUOTE_OPEN: &[u8] = b"\xE3\x80\x8C";
    const QUOTE_CLOSE: &[u8] = b"\xE3\x80\x8D";

    #[cfg(feature = "phase3-instrument")]
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
    // sequences carved from `&str` source.
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
            u32::try_from(open_pos).expect("source positions fit in u32 per Phase 0 cap")
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

// ----------------------------------------------------------------------
// Body-keyword dispatcher: single anchored Aho-Corasick DFA covering
// every fixed-string and prefix-with-parameter family in one pass.
//
// Replaces the prior 7-step `or_else` chain in `recognize_annotation`
// (fixed_keyword / kaeriten / indent_or_align / sashie / inline_warichu
// / container_open / container_close), each of which used to scan the
// body bytes from start in its own `match` or `strip_prefix`. The DFA
// runs once, anchored at byte 0, and its `pattern_id` indexes into a
// constant family table. Per-family branches finish the work — exact
// families verify `match_end == body.len()`, prefix families parse the
// remainder for parameters.
//
// Forward classifiers (bouten / TCY / heading) and the `Annotation
// {Unknown}` catch-all stay in `recognize_annotation` because they
// need event-stream context, not body bytes.
// ----------------------------------------------------------------------

/// One row of [`BODY_PATTERNS`]: the byte sequence the DFA matches at
/// `body[0..match_end]`, and the family that decides what to emit.
#[derive(Clone, Copy)]
struct BodyPattern {
    needle: &'static str,
    family: BodyFamily,
}

/// Outcome category for an anchored AC match against the annotation
/// body. Each variant carries enough information to either emit a
/// constant `EmitKind` directly (when the family is exact-match) or to
/// dispatch to a small per-family parser for the body remainder.
#[derive(Clone, Copy)]
enum BodyFamily {
    // === Exact-match (body must equal needle) ===
    PageBreak,
    SectionKaicho,
    SectionKaidan,
    SectionKaimihiraki,
    AlignEnd0,         // 地付き
    CenterMarker,      // ページの左右中央 / 中央揃え
    KeigakomiOpen,     // 罫囲み
    KeigakomiClose,    // 罫囲み終わり
    IndentBlock1,      // ここから字下げ → Indent { amount: 1 }
    AlignEndBlock0,    // ここから地付き → AlignEnd { offset: 0 }
    IndentBlockEnd,    // ここで字下げ終わり
    AlignEndBlockEnd,  // ここで地付き終わり
    LineWidthBlockEnd, // ここで字詰め終わり
    TableBlockOpen,    // ここから表
    TableBlockEnd,     // ここで表終わり
    ColumnsBlockEnd,   // ここで段組(み)終わり
    WarichuOpen,       // 割り注
    WarichuClose,      // 割り注終わり
    KaeritenSingle,    // body must equal one of 12 single-char marks
    KaeritenCompound,  // body must equal one of 6 compound marks

    // === Prefix-with-parameter (parse body[match_end..]) ===
    AlignEndParamPrefix,      // 地から → 地から{N}字上げ
    SashiePrefix,             // 挿絵（ → 挿絵（X）入る
    IndentBlockParamPrefix,   // ここから → ここから{N}字下げ
    AlignEndBlockParamPrefix, // ここから地から → ここから地から{N}字上げ
    OkuriganaPrefix,          // （ → kaeriten okurigana （X）

    // === Body-equals-pattern then parse from body[0] ===
    IndentParamPrefix, // {digit} → {N}字下げ (re-parse from body[0])

    /// 傍点 / 傍線 range form (`傍点` / `白丸傍点` / `二重傍線` / `左に傍線`
    /// …, with optional `終わり` close suffix). The needle matches the
    /// variant (or the `左に` prefix); `parse_bouten_range_body` reads the
    /// full body for the kind, the `左に` position, and the `終わり` close.
    BoutenRange,

    /// 太字 / 斜体 range / block form (`太字` / `斜体` inline,
    /// `ここから太字` / `ここで斜体終わり` block, with `終わり` close). The
    /// needle anchors the body; `parse_emphasis_body` reads the full body
    /// for the kind, the block vs inline form, and open vs close.
    Emphasis,
}

/// Static pattern table. Order is irrelevant for behavior because the
/// DFA is built with [`MatchKind::LeftmostLongest`]: the longer needle
/// always wins (so `罫囲み終わり` beats `罫囲み`, `ここから字下げ` beats
/// `ここから`, `一レ` beats `一`, etc.). Keeping families together for
/// readability instead of sorting by length.
static BODY_PATTERNS: &[BodyPattern] = &[
    // Block container with full-keyword bodies.
    BodyPattern {
        needle: "ここから字下げ",
        family: BodyFamily::IndentBlock1,
    },
    BodyPattern {
        needle: "ここから地付き",
        family: BodyFamily::AlignEndBlock0,
    },
    BodyPattern {
        needle: "ここから地から",
        family: BodyFamily::AlignEndBlockParamPrefix,
    },
    BodyPattern {
        needle: "ここから",
        family: BodyFamily::IndentBlockParamPrefix,
    },
    BodyPattern {
        needle: "ここで字下げ終わり",
        family: BodyFamily::IndentBlockEnd,
    },
    BodyPattern {
        needle: "ここで地付き終わり",
        family: BodyFamily::AlignEndBlockEnd,
    },
    // The 字上げ block (［＃ここから地から N 字上げ］) is closed by either
    // ［＃ここで字上げ終わり］ or ［＃ここで地付き終わり］ — both end the same
    // AlignEnd container. The open-side offset is authoritative when
    // pairing, so this closer reuses AlignEndBlockEnd.
    BodyPattern {
        needle: "ここで字上げ終わり",
        family: BodyFamily::AlignEndBlockEnd,
    },
    BodyPattern {
        needle: "ここで字詰め終わり",
        family: BodyFamily::LineWidthBlockEnd,
    },
    BodyPattern {
        needle: "ここから表",
        family: BodyFamily::TableBlockOpen,
    },
    BodyPattern {
        needle: "ここで表終わり",
        family: BodyFamily::TableBlockEnd,
    },
    BodyPattern {
        needle: "ここで段組終わり",
        family: BodyFamily::ColumnsBlockEnd,
    },
    BodyPattern {
        needle: "ここで段組み終わり",
        family: BodyFamily::ColumnsBlockEnd,
    },
    // Section / page break (exact).
    BodyPattern {
        needle: "改ページ",
        family: BodyFamily::PageBreak,
    },
    BodyPattern {
        needle: "改丁",
        family: BodyFamily::SectionKaicho,
    },
    BodyPattern {
        needle: "改段",
        family: BodyFamily::SectionKaidan,
    },
    BodyPattern {
        needle: "改見開き",
        family: BodyFamily::SectionKaimihiraki,
    },
    // Geographic alignment.
    BodyPattern {
        needle: "地から",
        family: BodyFamily::AlignEndParamPrefix,
    },
    BodyPattern {
        needle: "地付き",
        family: BodyFamily::AlignEnd0,
    },
    BodyPattern {
        needle: "ページの左右中央",
        family: BodyFamily::CenterMarker,
    },
    BodyPattern {
        needle: "中央揃え",
        family: BodyFamily::CenterMarker,
    },
    // Other inline / block.
    BodyPattern {
        needle: "挿絵（",
        family: BodyFamily::SashiePrefix,
    },
    BodyPattern {
        needle: "罫囲み終わり",
        family: BodyFamily::KeigakomiClose,
    },
    BodyPattern {
        needle: "罫囲み",
        family: BodyFamily::KeigakomiOpen,
    },
    BodyPattern {
        needle: "割り注終わり",
        family: BodyFamily::WarichuClose,
    },
    BodyPattern {
        needle: "割り注",
        family: BodyFamily::WarichuOpen,
    },
    // 傍点 / 傍線 range form openers (`［＃傍点］ … ［＃傍点終わり］`). One
    // needle per emphasis variant `bouten_kind_from_suffix` recognises,
    // plus the `左に` left-side prefix. LeftmostLongest disambiguates
    // overlaps (`二重丸傍点` vs `丸傍点`, `白丸傍点` vs `丸傍点`); the close
    // form (`…終わり`) matches the same variant needle as a prefix and is
    // re-parsed in full by `parse_bouten_range_body`.
    BodyPattern {
        needle: "左に",
        family: BodyFamily::BoutenRange,
    },
    BodyPattern {
        needle: "白ゴマ傍点",
        family: BodyFamily::BoutenRange,
    },
    BodyPattern {
        needle: "白丸傍点",
        family: BodyFamily::BoutenRange,
    },
    BodyPattern {
        needle: "二重丸傍点",
        family: BodyFamily::BoutenRange,
    },
    BodyPattern {
        needle: "蛇の目傍点",
        family: BodyFamily::BoutenRange,
    },
    BodyPattern {
        needle: "ばつ傍点",
        family: BodyFamily::BoutenRange,
    },
    BodyPattern {
        needle: "白三角傍点",
        family: BodyFamily::BoutenRange,
    },
    BodyPattern {
        needle: "丸傍点",
        family: BodyFamily::BoutenRange,
    },
    BodyPattern {
        needle: "傍点",
        family: BodyFamily::BoutenRange,
    },
    BodyPattern {
        needle: "二重傍線",
        family: BodyFamily::BoutenRange,
    },
    BodyPattern {
        needle: "波線",
        family: BodyFamily::BoutenRange,
    },
    BodyPattern {
        needle: "傍線",
        family: BodyFamily::BoutenRange,
    },
    // 太字 / 斜体 emphasis. The inline-range openers (`太字` / `斜体`)
    // also anchor their `…終わり` closers (re-parsed in full by
    // `parse_emphasis_body`); the block forms need their own anchors
    // (`ここから太字` beats the generic `ここから` via LeftmostLongest;
    // `ここで太字終わり` has no shorter generic anchor).
    BodyPattern {
        needle: "太字",
        family: BodyFamily::Emphasis,
    },
    BodyPattern {
        needle: "斜体",
        family: BodyFamily::Emphasis,
    },
    BodyPattern {
        needle: "ここから太字",
        family: BodyFamily::Emphasis,
    },
    BodyPattern {
        needle: "ここから斜体",
        family: BodyFamily::Emphasis,
    },
    BodyPattern {
        needle: "ここで太字終わり",
        family: BodyFamily::Emphasis,
    },
    BodyPattern {
        needle: "ここで斜体終わり",
        family: BodyFamily::Emphasis,
    },
    // Kaeriten okurigana opener (full-width left paren U+FF08).
    BodyPattern {
        needle: "（",
        family: BodyFamily::OkuriganaPrefix,
    },
    // Kaeriten compound marks (6) — must precede the single forms in
    // the table only for documentation; LeftmostLongest does the
    // actual disambiguation (`一レ` 6 bytes > `一` 3 bytes).
    BodyPattern {
        needle: "一レ",
        family: BodyFamily::KaeritenCompound,
    },
    BodyPattern {
        needle: "上レ",
        family: BodyFamily::KaeritenCompound,
    },
    BodyPattern {
        needle: "下レ",
        family: BodyFamily::KaeritenCompound,
    },
    BodyPattern {
        needle: "中レ",
        family: BodyFamily::KaeritenCompound,
    },
    BodyPattern {
        needle: "二レ",
        family: BodyFamily::KaeritenCompound,
    },
    BodyPattern {
        needle: "三レ",
        family: BodyFamily::KaeritenCompound,
    },
    // Kaeriten single marks (12).
    BodyPattern {
        needle: "一",
        family: BodyFamily::KaeritenSingle,
    },
    BodyPattern {
        needle: "丁",
        family: BodyFamily::KaeritenSingle,
    },
    BodyPattern {
        needle: "三",
        family: BodyFamily::KaeritenSingle,
    },
    BodyPattern {
        needle: "上",
        family: BodyFamily::KaeritenSingle,
    },
    BodyPattern {
        needle: "下",
        family: BodyFamily::KaeritenSingle,
    },
    BodyPattern {
        needle: "中",
        family: BodyFamily::KaeritenSingle,
    },
    BodyPattern {
        needle: "丙",
        family: BodyFamily::KaeritenSingle,
    },
    BodyPattern {
        needle: "乙",
        family: BodyFamily::KaeritenSingle,
    },
    BodyPattern {
        needle: "二",
        family: BodyFamily::KaeritenSingle,
    },
    BodyPattern {
        needle: "四",
        family: BodyFamily::KaeritenSingle,
    },
    BodyPattern {
        needle: "甲",
        family: BodyFamily::KaeritenSingle,
    },
    BodyPattern {
        needle: "レ",
        family: BodyFamily::KaeritenSingle,
    },
    // {N}字下げ — anchored on each digit (ASCII + full-width).
    BodyPattern {
        needle: "0",
        family: BodyFamily::IndentParamPrefix,
    },
    BodyPattern {
        needle: "1",
        family: BodyFamily::IndentParamPrefix,
    },
    BodyPattern {
        needle: "2",
        family: BodyFamily::IndentParamPrefix,
    },
    BodyPattern {
        needle: "3",
        family: BodyFamily::IndentParamPrefix,
    },
    BodyPattern {
        needle: "4",
        family: BodyFamily::IndentParamPrefix,
    },
    BodyPattern {
        needle: "5",
        family: BodyFamily::IndentParamPrefix,
    },
    BodyPattern {
        needle: "6",
        family: BodyFamily::IndentParamPrefix,
    },
    BodyPattern {
        needle: "7",
        family: BodyFamily::IndentParamPrefix,
    },
    BodyPattern {
        needle: "8",
        family: BodyFamily::IndentParamPrefix,
    },
    BodyPattern {
        needle: "9",
        family: BodyFamily::IndentParamPrefix,
    },
    BodyPattern {
        needle: "０",
        family: BodyFamily::IndentParamPrefix,
    },
    BodyPattern {
        needle: "１",
        family: BodyFamily::IndentParamPrefix,
    },
    BodyPattern {
        needle: "２",
        family: BodyFamily::IndentParamPrefix,
    },
    BodyPattern {
        needle: "３",
        family: BodyFamily::IndentParamPrefix,
    },
    BodyPattern {
        needle: "４",
        family: BodyFamily::IndentParamPrefix,
    },
    BodyPattern {
        needle: "５",
        family: BodyFamily::IndentParamPrefix,
    },
    BodyPattern {
        needle: "６",
        family: BodyFamily::IndentParamPrefix,
    },
    BodyPattern {
        needle: "７",
        family: BodyFamily::IndentParamPrefix,
    },
    BodyPattern {
        needle: "８",
        family: BodyFamily::IndentParamPrefix,
    },
    BodyPattern {
        needle: "９",
        family: BodyFamily::IndentParamPrefix,
    },
];

/// Build the annotation-body Aho-Corasick automaton from `BODY_PATTERNS`.
///
/// This DFA build is the bulk of parser boot cost (~150 microseconds, as
/// the `boot` bench measures). It is exposed under `#[doc(hidden)]` so
/// that bench can build it in isolation without making `BODY_PATTERNS`
/// public — the same pattern as `aozora-scan`'s hidden `NaiveScanner`
/// export. The process-lifetime cache lives in `body_dispatcher`;
/// `prewarm` warms it.
#[doc(hidden)]
#[must_use]
pub fn build_body_dispatcher() -> AhoCorasick {
    AhoCorasickBuilder::new()
        .match_kind(MatchKind::LeftmostLongest)
        .start_kind(StartKind::Anchored)
        .build(BODY_PATTERNS.iter().map(|p| p.needle))
        .expect("BODY_PATTERNS is a static, non-empty, valid set")
}

/// One-time DFA build, amortised across the entire process lifetime.
/// Lookup cost is a few ns per call so the build pays back in under a
/// thousand annotations.
fn body_dispatcher() -> &'static AhoCorasick {
    static DFA: OnceLock<AhoCorasick> = OnceLock::new();
    DFA.get_or_init(build_body_dispatcher)
}

/// Force the one-time Aho-Corasick DFA build now.
///
/// This is the bulk of parser boot cost. Idempotent — the `OnceLock` is
/// set at most once per process. `body_dispatcher` stays private; this
/// only triggers its init.
pub(crate) fn prewarm() {
    let _ = body_dispatcher();
}

/// Single-pass classification of `body` (the trimmed bytes between
/// `［＃` and `］`) into an `EmitKind` for body-only annotation
/// families. Returns `None` if the body matches no body-only family;
/// the caller then falls through to forward classifiers and finally
/// the `Annotation{Unknown}` catch-all.
#[allow(
    clippy::too_many_lines,
    reason = "single match arm per BodyFamily — splitting would scatter \
              the dispatch logic and obscure the intentional 1:1 mapping"
)]
fn classify_annotation_body<'a>(
    body: &str,
    alloc: &mut BorrowedAllocator<'a>,
) -> Option<(EmitKind<'a>, Option<&'a borrowed::Annotation<'a>>)> {
    #[cfg(feature = "phase3-instrument")]
    let _phase3_guard = SubsystemGuard::new(Subsystem::BodyDispatcher);
    if body.is_empty() {
        return None;
    }
    // Paired / block headings route through the container machinery as
    // `ContainerKind::Heading`. Tried before the body dispatcher: their
    // keywords overlap the `ここから…` / `…終わり` shapes but always carry a
    // `見出し` keyword, so a non-heading `ここから…` body falls through.
    if let Some((container, is_open)) = parse_heading_directive(body) {
        let emit = if is_open {
            EmitKind::BlockOpen(container)
        } else {
            EmitKind::BlockClose(container)
        };
        return Some((emit, None));
    }
    let dfa = body_dispatcher();
    let mat = dfa.find(Input::new(body).anchored(Anchored::Yes))?;
    let pat = BODY_PATTERNS[mat.pattern().as_usize()];
    let match_end = mat.end();
    let exact = match_end == body.len();
    match pat.family {
        // ----- Exact-match families (must consume the entire body) -----
        BodyFamily::PageBreak if exact => Some((EmitKind::Aozora(alloc.page_break()), None)),
        BodyFamily::SectionKaicho if exact => Some((
            EmitKind::Aozora(alloc.section_break(SectionKind::Kaicho)),
            None,
        )),
        BodyFamily::SectionKaidan if exact => Some((
            EmitKind::Aozora(alloc.section_break(SectionKind::Kaidan)),
            None,
        )),
        BodyFamily::SectionKaimihiraki if exact => Some((
            EmitKind::Aozora(alloc.section_break(SectionKind::Kaimihiraki)),
            None,
        )),
        BodyFamily::AlignEnd0 if exact => Some((
            EmitKind::Aozora(alloc.align_end(AlignEnd { offset: 0 })),
            None,
        )),
        BodyFamily::CenterMarker if exact => {
            // ページの左右中央 (page centre) vs 中央揃え — a single-line
            // zero-width centring marker.
            let page = body == "ページの左右中央";
            Some((EmitKind::Aozora(alloc.center(Center { page })), None))
        }
        BodyFamily::KeigakomiOpen if exact => {
            Some((EmitKind::BlockOpen(ContainerKind::Keigakomi), None))
        }
        BodyFamily::KeigakomiClose if exact => {
            Some((EmitKind::BlockClose(ContainerKind::Keigakomi), None))
        }
        BodyFamily::IndentBlock1 if exact => Some((
            EmitKind::BlockOpen(ContainerKind::Indent {
                amount: 1,
                wrap: None,
                center: false,
            }),
            None,
        )),
        BodyFamily::AlignEndBlock0 if exact => Some((
            EmitKind::BlockOpen(ContainerKind::AlignEnd { offset: 0 }),
            None,
        )),
        BodyFamily::IndentBlockEnd if exact => Some((
            EmitKind::BlockClose(ContainerKind::Indent {
                amount: 0,
                wrap: None,
                center: false,
            }),
            None,
        )),
        BodyFamily::AlignEndBlockEnd if exact => Some((
            EmitKind::BlockClose(ContainerKind::AlignEnd { offset: 0 }),
            None,
        )),
        BodyFamily::LineWidthBlockEnd if exact => Some((
            // The close marker carries no width; the open-side payload is
            // authoritative when pairing (mirrors the generic 字下げ終わり).
            EmitKind::BlockClose(ContainerKind::LineWidth { width: 0 }),
            None,
        )),
        BodyFamily::TableBlockOpen if exact => {
            Some((EmitKind::BlockOpen(ContainerKind::Table), None))
        }
        BodyFamily::TableBlockEnd if exact => {
            Some((EmitKind::BlockClose(ContainerKind::Table), None))
        }
        BodyFamily::ColumnsBlockEnd if exact => Some((
            // Close marker carries no count; the open-side payload is authoritative.
            EmitKind::BlockClose(ContainerKind::Columns { count: 0 }),
            None,
        )),
        BodyFamily::WarichuOpen if exact => {
            let p = alloc.make_annotation("［＃割り注］", AnnotationKind::WarichuOpen);
            let node = alloc.annotation(p);
            // Re-build a payload for the segment-wrap case. The
            // borrowed allocator interns by string content, so the
            // second call hits the dedup table; the owned allocator
            // pays a single `Box<str>` clone, which is cheap relative
            // to the rare nested-Warichu shape this case targets.
            let p2 = alloc.make_annotation("［＃割り注］", AnnotationKind::WarichuOpen);
            Some((EmitKind::Aozora(node), Some(p2)))
        }
        BodyFamily::WarichuClose if exact => {
            let p = alloc.make_annotation("［＃割り注終わり］", AnnotationKind::WarichuClose);
            let node = alloc.annotation(p);
            let p2 = alloc.make_annotation("［＃割り注終わり］", AnnotationKind::WarichuClose);
            Some((EmitKind::Aozora(node), Some(p2)))
        }
        BodyFamily::KaeritenSingle | BodyFamily::KaeritenCompound if exact => {
            Some((EmitKind::Aozora(alloc.kaeriten(body)), None))
        }

        // ----- Prefix-with-parameter families -----
        BodyFamily::AlignEndParamPrefix => {
            // body == 地から{N}字上げ; remainder = body[match_end..]
            let rest = &body[match_end..];
            let (n, tail) = parse_decimal_u8_prefix(rest)?;
            (tail == "字上げ" && n >= 1).then(|| {
                (
                    EmitKind::Aozora(alloc.align_end(AlignEnd { offset: n })),
                    None,
                )
            })
        }
        BodyFamily::SashiePrefix => classify_sashie_body(body, alloc).map(|e| (e, None)),
        BodyFamily::IndentBlockParamPrefix => {
            // body == ここから{N}字下げ; remainder = body[match_end..]
            let rest = &body[match_end..];
            let (n, tail) = parse_decimal_u8_prefix(rest)?;
            if tail == "字下げ" {
                Some((
                    EmitKind::BlockOpen(ContainerKind::Indent {
                        amount: n,
                        wrap: None,
                        center: false,
                    }),
                    None,
                ))
            } else if let Some(after) = tail.strip_prefix("字下げ、折り返して") {
                // ここから{N}字下げ、折り返して{M}字下げ — hanging (wrap) indent:
                // the first line indents `n`, wrapped continuation lines `m`.
                let (m, tail2) = parse_decimal_u8_prefix(after)?;
                (tail2 == "字下げ").then_some((
                    EmitKind::BlockOpen(ContainerKind::Indent {
                        amount: n,
                        wrap: Some(m),
                        center: false,
                    }),
                    None,
                ))
            } else if matches!(
                tail,
                "字下げ、ページの左右中央" | "字下げ、ページの左右中央に" | "字下げ、左右中央"
            ) {
                // ここから{N}字下げ、ページの左右中央 — an indented block that is
                // also page-centred. The combined opener still closes with the
                // shared 字下げ終わり (pairing is by family).
                Some((
                    EmitKind::BlockOpen(ContainerKind::Indent {
                        amount: n,
                        wrap: None,
                        center: true,
                    }),
                    None,
                ))
            } else if tail == "字詰め" && n >= 1 {
                // ここから{N}字詰め — line-width container (字詰め): N
                // full-width characters per line. Shares the `ここから`
                // opener prefix with 字下げ; block-only, closes with
                // `ここで字詰め終わり`.
                Some((
                    EmitKind::BlockOpen(ContainerKind::LineWidth { width: n }),
                    None,
                ))
            } else if (tail == "段組" || tail == "段組み") && n >= 1 {
                // ここから{N}段組(み) — multi-column container (段組): N
                // columns. Shares the `ここから` prefix; closes with
                // `ここで段組(み)終わり`.
                Some((
                    EmitKind::BlockOpen(ContainerKind::Columns { count: n }),
                    None,
                ))
            } else {
                None
            }
        }
        BodyFamily::AlignEndBlockParamPrefix => {
            // body == ここから地から{N}字上げ; remainder = body[match_end..]
            let rest = &body[match_end..];
            let (n, tail) = parse_decimal_u8_prefix(rest)?;
            (tail == "字上げ").then_some((
                EmitKind::BlockOpen(ContainerKind::AlignEnd { offset: n }),
                None,
            ))
        }
        BodyFamily::OkuriganaPrefix => {
            // The DFA matched `（` at body[0..3]. Defer to the same
            // parens-recognising helper as the legacy code so the
            // length / character-class invariants stay in one place.
            is_okurigana_body(body).then(|| (EmitKind::Aozora(alloc.kaeriten(body)), None))
        }
        BodyFamily::IndentParamPrefix => {
            // The DFA matched a single digit. Re-parse from body[0]
            // for full multi-digit support.
            let (n, tail) = parse_decimal_u8_prefix(body)?;
            (tail == "字下げ" && n >= 1)
                .then(|| (EmitKind::Aozora(alloc.indent(Indent { amount: n })), None))
        }
        BodyFamily::BoutenRange => {
            // `傍点` / `白丸傍点` / `二重傍線` / `左に傍線` … with an optional
            // `終わり` close suffix. Re-parse the full body for the variant,
            // the `左に` position, and open vs close.
            let (kind, position, is_close) = parse_bouten_range_body(body)?;
            let container = ContainerKind::BoutenRange { kind, position };
            Some((
                if is_close {
                    EmitKind::BlockClose(container)
                } else {
                    EmitKind::BlockOpen(container)
                },
                None,
            ))
        }
        BodyFamily::Emphasis => {
            // `太字` / `斜体` / `ここから太字` / `ここで斜体終わり` … —
            // re-parse the full body for the kind, the block vs inline
            // form, and open vs close.
            let (kind, block, is_close) = parse_emphasis_body(body)?;
            let container = match kind {
                EmphasisKind::Italic => ContainerKind::Italic { block },
                // `EmphasisKind` is `#[non_exhaustive]`; 太字 and any
                // future weight pair as the bold container.
                _ => ContainerKind::Bold { block },
            };
            Some((
                if is_close {
                    EmitKind::BlockClose(container)
                } else {
                    EmitKind::BlockOpen(container)
                },
                None,
            ))
        }

        // Exact-only families that didn't fully consume the body (e.g.
        // `罫囲みfoo` matched `罫囲み` but body is longer): no claim.
        BodyFamily::PageBreak
        | BodyFamily::SectionKaicho
        | BodyFamily::SectionKaidan
        | BodyFamily::SectionKaimihiraki
        | BodyFamily::AlignEnd0
        | BodyFamily::CenterMarker
        | BodyFamily::KeigakomiOpen
        | BodyFamily::KeigakomiClose
        | BodyFamily::IndentBlock1
        | BodyFamily::AlignEndBlock0
        | BodyFamily::IndentBlockEnd
        | BodyFamily::AlignEndBlockEnd
        | BodyFamily::LineWidthBlockEnd
        | BodyFamily::TableBlockOpen
        | BodyFamily::TableBlockEnd
        | BodyFamily::ColumnsBlockEnd
        | BodyFamily::WarichuOpen
        | BodyFamily::WarichuClose
        | BodyFamily::KaeritenSingle
        | BodyFamily::KaeritenCompound => None,
    }
}

/// Streaming Phase 3 classifier.
///
/// Owns the upstream [`PairEvent`] iterator and consumes it lazily,
/// yielding one [`ClassifiedSpan`] per [`Iterator::next`] call. The
/// classifier maintains its own per-pair frame stack — when a top-level
/// `PairOpen` arrives, all subsequent events accumulate into a smallvec
/// body buffer until the matching `PairClose`; recognition then runs
/// against the buffer and yields a single span (or, in the rare
/// gaiji+ref-mark case, consumes a buffered `Solo(RefMark)` from the
/// previous emission and folds it into the bracket span).
///
/// State:
/// * `pending_outputs`: queue of complete `ClassifiedSpan`s waiting to
///   be returned by `next()`. A single input event can produce multiple
///   outputs (e.g. flush a pending Plain run + emit a recognised span);
///   draining this queue first keeps `next` simple.
/// * `frame`: current outermost open frame, if any. Inside a frame all
///   incoming events are appended to the body buffer; nested
///   `PairOpen`/`PairClose` adjust the buffer-local stack so `close_idx`
///   slots can be patched and the OUTER pair can be detected as
///   "matching close at depth 0".
/// * `pending_plain_start`: byte position where the current Plain run
///   began (top-level only).
/// * `pending_refmark`: a top-level `Solo(RefMark)` waiting to be
///   absorbed by the next `PairOpen(Bracket)` (gaiji shape). If the
///   following event is anything else the refmark is folded into the
///   pending Plain run.
/// * `diagnostics`: non-fatal observations accumulated during the pass.
#[allow(
    missing_debug_implementations,
    reason = "the &mut BorrowedAllocator field cannot derive Debug; the iterator is opaque to consumers"
)]
pub struct ClassifyStream<'src, 'al, 'a, I>
where
    I: Iterator<Item = PairEvent>,
{
    events: I,
    source: &'src str,
    source_len: u32,
    alloc: &'al mut BorrowedAllocator<'a>,
    /// Buffered ready-to-yield spans drained one-per-`next()` by the
    /// consumer. `VecDeque` (not `SmallVec`) because the consumer pulls
    /// from the front and `SmallVec::remove(0)` is `O(N)`: the
    /// `replay_unrecognised_body` path can push thousands of spans at
    /// once for top-level unrecognised paired containers (e.g. doc 49178
    /// in the corpus emits ~16k pending spans), and the per-yield
    /// front-pop turns into a quadratic memmove storm. `VecDeque` is a
    /// ring buffer with `O(1)` `push_back` / `pop_front`, eliminating
    /// the back-shift entirely.
    pending_outputs: VecDeque<ClassifiedSpan<'a>>,
    frame: Option<Frame>,
    /// Stream-through state for top-level pair kinds that have no
    /// recogniser (Quote, Tortoise). `None` in normal operation. When
    /// `Some((kind, depth))`, `process_event` bypasses frame buffering
    /// — events stream directly through `handle_stream_event` so we
    /// don't waste an O(N) `SmallVec` push per event followed by an
    /// O(N) replay walk. The depth counter tracks nested opens of the
    /// same kind so the outer close is unambiguous.
    ///
    /// Optimisation: doc 49178 (corpus outlier) wraps ~24k inner
    /// events in two top-level Quote pairs. With buffering the
    /// classify pass paid 24k × (push to body smallvec + read it back
    /// in replay); with stream-through that becomes a single forward
    /// walk.
    streaming: Option<StreamingFrame>,
    pending_plain_start: Option<u32>,
    pending_refmark: Option<Span>,
    diagnostics: Vec<Diagnostic>,
    /// Bracketed kaeriten observed in document order, drained by
    /// [`Self::finalize_kaeriten`] in [`Self::take_diagnostics`] for the
    /// document-wide base-presence pairing check and the outside-kanbun
    /// heuristic.
    kaeriten_obs: Vec<KaeritenObs>,
    finished: bool,
}

/// Family of a bracketed kaeriten ladder. Reading-order return marks
/// come in ordered families; a mark of rank `r` needs a same-family base
/// (`一` / `上` / `甲`) somewhere in the document (see
/// [`Diagnostic::bracketed_kaeriten_no_pair`]). `レ` (re-ten) and 送り仮名
/// `（X）` are standalone and never ladder.
#[derive(Clone, Copy, PartialEq, Eq)]
enum KaeritenFamily {
    /// `一` < `二` < `三` < `四` (and the `Xレ` compounds).
    Numeric,
    /// `上` < `中` < `下`.
    Jouge,
    /// `甲` < `乙` < `丙` < `丁`.
    Kouotsu,
    /// `レ`, 送り仮名, or any non-ladder mark.
    Other,
}

/// One bracketed kaeriten observed during classification, retained for
/// the end-of-document pairing / context checks run in
/// [`ClassifyStream::finalize_kaeriten`].
#[derive(Clone, Copy)]
struct KaeritenObs {
    family: KaeritenFamily,
    /// 1-based ladder rank; `0` for non-ladder marks.
    rank: u8,
    /// Whether this mark participates in ladder-base checks.
    is_ladder: bool,
    /// Byte-range of the `［＃…］` directive in the sanitized source.
    span: Span,
}

/// Classify a bracketed kaeriten body into `(family, rank, is_ladder)`.
/// `Xレ` compounds ladder by their base char `X`; `レ` alone and 送り仮名
/// `（X）` are non-ladder.
fn classify_kaeriten_mark(mark: &str) -> (KaeritenFamily, u8, bool) {
    // 送り仮名 ［＃（X）］ — a kaeriten node but not a ladder mark.
    if mark.starts_with('（') {
        return (KaeritenFamily::Other, 0, false);
    }
    // Compound `Xレ` ladders by its base char; `レ` alone has empty base.
    let base = mark
        .strip_suffix('レ')
        .filter(|b| !b.is_empty())
        .unwrap_or(mark);
    match base {
        "一" => (KaeritenFamily::Numeric, 1, true),
        "二" => (KaeritenFamily::Numeric, 2, true),
        "三" => (KaeritenFamily::Numeric, 3, true),
        "四" => (KaeritenFamily::Numeric, 4, true),
        "上" => (KaeritenFamily::Jouge, 1, true),
        "中" => (KaeritenFamily::Jouge, 2, true),
        "下" => (KaeritenFamily::Jouge, 3, true),
        "甲" => (KaeritenFamily::Kouotsu, 1, true),
        "乙" => (KaeritenFamily::Kouotsu, 2, true),
        "丙" => (KaeritenFamily::Kouotsu, 3, true),
        "丁" => (KaeritenFamily::Kouotsu, 4, true),
        _ => (KaeritenFamily::Other, 0, false),
    }
}

/// Dense index of a ladder [`KaeritenFamily`] for the base-presence
/// table. `Other` never ladders so it shares the `Numeric` slot (read
/// but never marked present for non-ladder marks).
fn family_index(fam: KaeritenFamily) -> usize {
    match fam {
        KaeritenFamily::Jouge => 1,
        KaeritenFamily::Kouotsu => 2,
        KaeritenFamily::Numeric | KaeritenFamily::Other => 0,
    }
}

/// Conservative "is this kana prose, not 漢文?" check for the lone-mark
/// outside-kanbun heuristic. Reads a small character window each side of
/// `span`: kana-dominant with almost no kanji ⇒ prose. Real 漢文 around a
/// genuine 返り点 is kanji-dense and never trips this.
fn looks_like_kana_prose(source: &str, span: Span) -> bool {
    const WIN: usize = 12;
    let before = source.get(..span.start as usize).unwrap_or("");
    let after = source.get(span.end as usize..).unwrap_or("");
    let (mut kana, mut kanji) = (0u32, 0u32);
    for c in before
        .chars()
        .rev()
        .take(WIN)
        .chain(after.chars().take(WIN))
    {
        if is_kana(c) {
            kana += 1;
        } else if is_kanji(c) {
            kanji += 1;
        }
    }
    kana >= 2 && kanji <= 1
}

/// Hiragana / katakana / half-width katakana.
fn is_kana(c: char) -> bool {
    matches!(c, '\u{3040}'..='\u{30FF}' | '\u{FF66}'..='\u{FF9D}')
}

/// CJK unified ideographs (incl. Ext-A and compatibility) — "kanji".
fn is_kanji(c: char) -> bool {
    matches!(c, '\u{3400}'..='\u{9FFF}' | '\u{F900}'..='\u{FAFF}')
}

/// Active stream-through frame for a top-level Quote / Tortoise pair.
/// Carries no event buffer — just the outer pair kind and a nested-
/// open depth counter. Each `PairOpen` of the same kind increments,
/// each `PairClose` of the same kind decrements; reaching zero ends
/// stream-through.
#[derive(Debug, Clone, Copy)]
struct StreamingFrame {
    kind: PairKind,
    depth: u32,
}

/// Body window passed to recogniser helpers.
///
/// `events` is a contiguous body slice (between matched
/// `PairOpen`/`PairClose`); `links[i]` gives the body-local index of
/// the matching `PairOpen`/`PairClose` for `events[i]` if it's a
/// paired event (`u32::MAX` otherwise). Both slices are the same
/// length and are constructed by [`ClassifyStream`]'s frame buffers.
///
/// The split keeps [`PairEvent`] free of cross-link fields (Phase 2
/// can stream events one-at-a-time without back-patching) while still
/// giving recogniser helpers O(1) "jump to my matching delimiter"
/// access via the parallel side-table.
#[derive(Clone, Copy, Debug)]
pub(crate) struct BodyView<'b> {
    pub events: &'b [PairEvent],
    pub links: &'b [u32],
}

/// Per-recognise-call shared context.
///
/// Bundles the (allocator, sanitized source) pair that every Phase 3
/// recogniser / classifier helper needs but doesn't vary per call
/// within a single recognise pass. Threading it as a single `&mut`
/// argument keeps each helper's signature at ≤4 args (project-rule
/// `clippy.toml::too-many-arguments-threshold = 4`) without losing
/// the per-call positional clarity of the body-window indices.
///
/// The three lifetimes deliberately stay distinct:
/// - `'al` — the borrow lifetime of the `&mut alloc` reference
/// - `'a`  — the arena lifetime that strings interned now will live in
/// - `'s`  — the sanitized source lifetime
///
/// In practice `'a` and `'s` collapse at the top-level driver
/// (the arena and the source both live as long as the `Document`),
/// but keeping them separate here avoids over-constraining helpers
/// that thread synthetic source slices through `Cow`.
pub(crate) struct RecogniseCtx<'al, 'a, 's> {
    pub alloc: &'al mut BorrowedAllocator<'a>,
    pub source: &'s str,
}

/// One outermost open-pair frame currently being buffered.
///
/// `body` holds every event seen between the open and the matching
/// close (inclusive of nested Pair events). The parallel `links`
/// smallvec records, for each entry of `body`, the body-local index
/// of the matching `PairOpen` / `PairClose` (or `u32::MAX` for
/// non-paired entries and unmatched delimiters). The recognise
/// helpers (`recognize_ruby` / `recognize_annotation` /
/// `recognize_gaiji` / `try_angle_quote`) consume the buffer as a
/// [`BodyView`] with `open_idx = 0` and `close_idx = body.len() - 1`.
///
/// `inner_stack` tracks the per-buffer mini-stack of nested opens —
/// `(kind, body_index)` — so that on each nested close we can locate
/// the matching open in the body buffer and patch the `links` table.
struct Frame {
    body: smallvec::SmallVec<[PairEvent; 16]>,
    links: smallvec::SmallVec<[u32; 16]>,
    inner_stack: smallvec::SmallVec<[(PairKind, usize); 8]>,
    /// `true` when the outer open follows a `Solo(RefMark)` and the
    /// frame should be recognised as gaiji rather than a generic
    /// bracket annotation.
    gaiji_refmark: Option<Span>,
}

/// Span of the first ruby (`《…》`) opening *inside* the body
/// event range `lo..hi`, if any. Used by [`ClassifyStream::try_ruby_emit`]
/// to flag `nested_ruby` — `build_content_from_body` folds only nested
/// gaiji / annotation, so an inner ruby open would otherwise survive raw.
fn first_nested_ruby_open(events: &[PairEvent], lo: usize, hi: usize) -> Option<Span> {
    events[lo..hi].iter().find_map(|e| match e {
        PairEvent::PairOpen {
            kind: PairKind::Ruby,
            span,
        } => Some(*span),
        _ => None,
    })
}

/// When an explicit-base ruby (`｜base《》`) has an empty reading, the span
/// of the whole `｜base《》` to flag as `empty_ruby_reading`. Returns `None`
/// when there is no explicit `｜` base (a bare `《》` is just literal text)
/// or when the `《…》` reading is non-empty.
fn empty_explicit_ruby_span(
    bar_byte_offset: Option<usize>,
    preceding_start: u32,
    open_span: Span,
    close_span: Span,
) -> Option<Span> {
    let bar_off = bar_byte_offset?;
    if open_span.end < close_span.start {
        return None; // reading carries bytes — not empty
    }
    let bar_pos = preceding_start + u32::try_from(bar_off).ok()?;
    Some(Span::new(bar_pos, close_span.end))
}

/// Build the synthetic event / link stream `recognize_ruby` expects in the
/// streaming model: `[optional Solo(Bar), Text(base), ...body events...]`,
/// with links shifted to account for the prepended prefix. Returns the
/// stream, its parallel link table, and the index of the synthetic `《`
/// open. `prev_text_range` is the preceding plain run (`start` = its byte
/// offset, `end` = the `《` open); `bar_byte_offset` is the position of a
/// `｜` inside it for the explicit form. Returns `None` when an explicit
/// `｜` base would leave the base text empty.
fn build_synth_ruby_view(
    body: BodyView<'_>,
    prev_text_range: Span,
    bar_byte_offset: Option<usize>,
) -> Option<(Vec<PairEvent>, Vec<u32>, usize)> {
    let mut synth: Vec<PairEvent> = Vec::with_capacity(body.events.len() + 2);
    let mut synth_links: Vec<u32> = Vec::with_capacity(body.events.len() + 2);
    let synth_open_idx = if let Some(bar_off) = bar_byte_offset {
        let bar_pos = prev_text_range.start + u32::try_from(bar_off).expect("bar offset fits");
        let bar_span = Span::new(bar_pos, bar_pos + u32::try_from('｜'.len_utf8()).unwrap());
        synth.push(PairEvent::Solo {
            kind: TriggerKind::Bar,
            span: bar_span,
        });
        synth_links.push(u32::MAX);
        if bar_span.end >= prev_text_range.end {
            return None; // explicit base would be empty
        }
        synth.push(PairEvent::Text {
            range: Span::new(bar_span.end, prev_text_range.end),
        });
        synth_links.push(u32::MAX);
        2
    } else {
        synth.push(PairEvent::Text {
            range: prev_text_range,
        });
        synth_links.push(u32::MAX);
        1
    };
    // Append the body events verbatim, shifting their links past the prefix.
    let shift = u32::try_from(synth.len()).expect("synth prefix fits u32");
    synth.extend(body.events.iter().cloned());
    synth_links.extend(
        body.links
            .iter()
            .map(|&l| if l == u32::MAX { u32::MAX } else { l + shift }),
    );
    Some((synth, synth_links, synth_open_idx))
}

impl<'src, 'al, 'a, I> ClassifyStream<'src, 'al, 'a, I>
where
    I: Iterator<Item = PairEvent>,
{
    fn new(events: I, source: &'src str, alloc: &'al mut BorrowedAllocator<'a>) -> Self {
        Self {
            events,
            source,
            source_len: u32::try_from(source.len()).expect("sanitize asserts fit in u32"),
            alloc,
            pending_outputs: VecDeque::new(),
            frame: None,
            streaming: None,
            pending_plain_start: None,
            pending_refmark: None,
            diagnostics: Vec::new(),
            kaeriten_obs: Vec::new(),
            finished: false,
        }
    }

    /// Drain accumulated diagnostics. Should be called after the
    /// iterator is exhausted (otherwise the trailing Plain flush has
    /// not yet recorded any final-span observations).
    pub fn take_diagnostics(&mut self) -> Vec<Diagnostic> {
        self.finalize_kaeriten();
        mem::take(&mut self.diagnostics)
    }

    /// Document-final kaeriten checks, run once the event stream is
    /// exhausted: the document-wide base-presence pairing check
    /// (`bracketed_kaeriten_no_pair`) and the conservative
    /// outside-kanbun heuristic (`kaeriten_outside_kanbun`). Both work
    /// off `kaeriten_obs`, accumulated during classification.
    ///
    /// The pairing rule is *document-wide base presence*: a ladder mark
    /// of rank ≥ 2 fires only when its family's base (`一` / `上` / `甲`)
    /// is absent from the entire document. This is calibrated against the
    /// real 青空文庫 corpus — kanbun return-mark groups routinely span
    /// `、` / `。` and line boundaries, and 上下点 skips `中`, so any
    /// narrower scope or stricter ladder misfires on valid kanbun (per-
    /// clause strict: 586 false positives across 337 corpus files; this
    /// rule: 2). It matches the catalogue's literal wording — "a `［＃二］`
    /// with no `［＃一］`".
    fn finalize_kaeriten(&mut self) {
        let obs = mem::take(&mut self.kaeriten_obs);
        if obs.is_empty() {
            return;
        }
        // Conservative outside-kanbun heuristic: the document holds a
        // single, isolated kaeriten and its surroundings read as kana
        // prose — most likely a stray annotation, not a genuine 返り点.
        if let [only] = obs.as_slice()
            && looks_like_kana_prose(self.source, only.span)
        {
            self.diagnostics
                .push(Diagnostic::kaeriten_outside_kanbun(only.span));
        }
        // Document-wide base presence per ladder family.
        let mut has_base = [false; 3];
        for o in obs.iter().filter(|o| o.is_ladder && o.rank == 1) {
            has_base[family_index(o.family)] = true;
        }
        for o in obs.iter().filter(|o| o.is_ladder && o.rank > 1) {
            if !has_base[family_index(o.family)] {
                self.diagnostics
                    .push(Diagnostic::bracketed_kaeriten_no_pair(o.span));
            }
        }
    }

    fn push_output(&mut self, span: ClassifiedSpan<'a>) {
        #[cfg(feature = "phase3-instrument")]
        record_yield(match &span.kind {
            SpanKind::Plain => YieldKind::Plain,
            SpanKind::Newline => YieldKind::Newline,
            SpanKind::Aozora(_) => YieldKind::Aozora,
            SpanKind::BlockOpen(_) => YieldKind::BlockOpen,
            SpanKind::BlockClose(_) => YieldKind::BlockClose,
        });
        self.pending_outputs.push_back(span);
    }

    /// Emit any pending top-level plain run whose end is `end`. The
    /// pending refmark, if any, is folded into the plain run's coverage
    /// (its span is contiguous with the surrounding text).
    fn flush_plain_up_to(&mut self, end: u32) {
        #[cfg(feature = "phase3-instrument")]
        let _phase3_guard = SubsystemGuard::new(Subsystem::FlushPlain);
        // A pending refmark contributes its bytes to the plain run.
        if let Some(rm) = self.pending_refmark.take()
            && self.pending_plain_start.is_none()
        {
            self.pending_plain_start = Some(rm.start);
        }
        if let Some(start) = self.pending_plain_start.take()
            && end > start
        {
            self.push_output(ClassifiedSpan {
                kind: SpanKind::Plain,
                source_span: Span::new(start, end),
            });
        }
    }

    /// Open a new top-level frame. `gaiji_refmark` is `Some(span)` when
    /// the outer open was preceded by a `Solo(RefMark)` waiting to be
    /// absorbed (the gaiji shape).
    fn open_frame(&mut self, open_event: PairEvent, gaiji_refmark: Option<Span>) {
        #[cfg(feature = "phase3-instrument")]
        let _phase3_guard = SubsystemGuard::new(Subsystem::OpenFrame);
        let mut body: smallvec::SmallVec<[PairEvent; 16]> = smallvec::SmallVec::new();
        let mut links: smallvec::SmallVec<[u32; 16]> = smallvec::SmallVec::new();
        // Inner stack tracks NESTED opens; the outer open lives at
        // body[0], so we record its position there.
        let mut inner_stack = smallvec::SmallVec::new();
        let &PairEvent::PairOpen { kind, .. } = &open_event else {
            unreachable!("open_frame called with non-PairOpen event");
        };
        inner_stack.push((kind, 0_usize));
        body.push(open_event);
        links.push(u32::MAX);
        self.frame = Some(Frame {
            body,
            links,
            inner_stack,
            gaiji_refmark,
        });
    }

    /// Append an event to the current frame's body, updating the
    /// inner-stack and patching the parallel `links` side-table as
    /// needed. Returns `true` if the appended event closed the
    /// OUTERMOST pair (i.e. `inner_stack` became empty), signalling
    /// that the caller should run recognition on the now-complete
    /// buffer.
    fn append_to_frame(&mut self, event: PairEvent) -> bool {
        #[cfg(feature = "phase3-instrument")]
        let _phase3_guard = SubsystemGuard::new(Subsystem::FrameAppend);
        let frame = self
            .frame
            .as_mut()
            .expect("append_to_frame requires an active frame");
        let body_idx = frame.body.len();

        match &event {
            PairEvent::PairOpen { kind, .. } => {
                frame.inner_stack.push((*kind, body_idx));
                frame.body.push(event);
                frame.links.push(u32::MAX);
            }
            PairEvent::PairClose { kind, .. } => {
                // Find the matching open via the inner stack. Phase 2
                // guarantees that a PairClose only arrives when the
                // top of the global stack matches its kind, but inside
                // the body buffer we may have nested opens of various
                // kinds — we patch the nearest matching open.
                if let Some(pos) = frame.inner_stack.iter().rposition(|&(k, _)| k == *kind) {
                    let (_, open_body_idx) = frame.inner_stack.remove(pos);
                    frame.body.push(event);
                    let body_idx_u32 = u32::try_from(body_idx)
                        .expect("body_idx fits u32 (corpus body lengths are bounded)");
                    let open_body_idx_u32 = u32::try_from(open_body_idx)
                        .expect("body_idx fits u32 (corpus body lengths are bounded)");
                    frame.links.push(open_body_idx_u32);
                    frame.links[open_body_idx] = body_idx_u32;
                } else {
                    // No matching open in this buffer — should not
                    // happen because Phase 2's stack-balance contract
                    // means a PairClose only arrives when the outer
                    // stack matches; but be defensive and append as-is.
                    frame.body.push(event);
                    frame.links.push(u32::MAX);
                }
            }
            _ => {
                frame.body.push(event);
                frame.links.push(u32::MAX);
            }
        }

        frame.inner_stack.is_empty()
    }

    /// Run recognition on the current frame's body buffer and emit the
    /// resulting span. Called when the OUTERMOST pair has just closed.
    fn recognize_and_emit(&mut self) {
        #[cfg(feature = "phase3-instrument")]
        let _phase3_guard = SubsystemGuard::new(Subsystem::RecognizeAndEmit);
        let frame = self
            .frame
            .take()
            .expect("recognize_and_emit requires an active frame");
        let body = frame.body;
        let links = frame.links;
        debug_assert!(body.len() >= 2, "frame body must contain open + close");
        debug_assert_eq!(body.len(), links.len(), "links must parallel body");

        // The frame's outer open lives at body[0], the matching close
        // at body[body.len() - 1].
        let open_idx = 0usize;
        let close_idx = body.len() - 1;

        // Pull open span / kind for emission and pending-plain truncation.
        let PairEvent::PairOpen {
            kind: open_kind, ..
        } = body[open_idx]
        else {
            unreachable!("frame body[0] must be PairOpen");
        };

        let view = BodyView {
            events: &body,
            links: &links,
        };

        match open_kind {
            PairKind::Ruby => {
                if let Some(span) = self.try_ruby_emit(view, open_idx, close_idx) {
                    self.push_output(span);
                    return;
                }
            }
            PairKind::AngleQuote => {
                if let Some(span) = self.try_angle_quote_emit(view, open_idx, close_idx) {
                    self.push_output(span);
                    return;
                }
                // Empty `≪≫` falls through to `replay_unrecognised_body`
                // so the bytes flow back into the pending plain run.
            }
            PairKind::Bracket => {
                let refmark = frame.gaiji_refmark;
                if let Some(rm_span) = refmark {
                    // B (zero-copy): pass the original body+links
                    // directly with `bracket_open_idx = 0`. The previous
                    // shape rebuilt body+links into a SmallVec just to
                    // prepend a synthetic Solo(RefMark) at index 0 so
                    // `recognize_gaiji` could read the bracket open at
                    // index 1. But `recognize_gaiji` parameterises the
                    // bracket entry point via `bracket_open_idx` and
                    // takes `refmark_span` as a separate argument — the
                    // synthetic prefix was never consumed. Eliminating
                    // the rebuild closes pathological doc 50685's
                    // memcpy_memmove 25.13 % bucket and doc 49178's
                    // 22.63 %, both attributed to this hot path.
                    if let Some(span) = self.try_gaiji_emit(view, open_idx, rm_span) {
                        self.push_output(span);
                        return;
                    }
                    // Gaiji recognition declined. Fold the refmark bytes
                    // into the pending plain run, then attempt a normal
                    // bracket annotation recognition on the original body.
                    if self.pending_plain_start.is_none() {
                        self.pending_plain_start = Some(rm_span.start);
                    }
                    if let Some(span) = self.try_bracket_emit(view, open_idx, close_idx) {
                        self.push_output(span);
                        return;
                    }
                    // Both gaiji and bracket annotation declined: replay
                    // the body and let the refmark span fall into plain.
                    self.replay_unrecognised_body(body, None);
                    return;
                }
                if let Some(span) = self.try_bracket_emit(view, open_idx, close_idx) {
                    self.push_output(span);
                    return;
                }
            }
            // Tortoise / Quote at top level have no built-in
            // recogniser; the bracket bytes flow through as plain.
            _ => {}
        }

        // Recognition declined — every event in the body becomes plain.
        // Replay the buffered events through the per-event acceptor so
        // that any Newlines inside fire as their own spans and the
        // surrounding bytes attach to a top-level Plain run. If the
        // frame was opened in gaiji-mode, the refmark span is also
        // folded back to plain.
        self.replay_unrecognised_body(body, frame.gaiji_refmark);
    }

    /// Replay the events from a frame whose recognition declined.
    /// Each event is treated as if it had been received at top level
    /// without a frame ever opening — text/solo/unmatched fold into
    /// the pending Plain run; newlines flush and fire as Newline
    /// spans.
    ///
    /// `refmark` is `Some(span)` when the frame was opened in
    /// gaiji-mode (Bracket preceded by `※`). The refmark bytes need
    /// to be re-folded into plain since gaiji recognition declined.
    ///
    /// `Unclosed` events are SKIPPED during replay: they are
    /// synthetic EOF markers carrying the same span as the original
    /// `PairOpen` (which is also in `body`), and re-adding their span
    /// to the pending plain run would double-count bytes already
    /// covered by the open's `body[0]` entry.
    fn replay_unrecognised_body(
        &mut self,
        body: smallvec::SmallVec<[PairEvent; 16]>,
        refmark: Option<Span>,
    ) {
        #[cfg(feature = "phase3-instrument")]
        let _phase3_guard = SubsystemGuard::new(Subsystem::ReplayBody);
        #[cfg(feature = "phase3-instrument")]
        record_replay_body_size(body.len() as u64);
        if let Some(rm) = refmark
            && self.pending_plain_start.is_none()
        {
            self.pending_plain_start = Some(rm.start);
        }
        for ev in body {
            if matches!(ev, PairEvent::Unclosed { .. }) {
                continue;
            }
            self.handle_top_level(ev, /*replay=*/ true);
        }
    }

    /// Handle a top-level event (no active frame) in either streaming
    /// mode (`replay = false`) or replay mode (`replay = true`, which
    /// suppresses the frame-open path so a residual nested `PairOpen` in
    /// a declined body doesn't try to re-open a sub-frame).
    fn handle_top_level(&mut self, event: PairEvent, replay: bool) {
        match event {
            PairEvent::Newline { pos } => {
                self.flush_plain_up_to(pos);
                self.push_output(ClassifiedSpan {
                    kind: SpanKind::Newline,
                    source_span: Span::new(pos, pos + 1),
                });
            }
            PairEvent::Solo {
                kind: TriggerKind::RefMark,
                span,
            } if !replay => {
                // Hold the refmark pending the next event. If a flush
                // is requested before the next event arrives the
                // refmark is folded into the plain run.
                self.pending_refmark = Some(span);
            }
            PairEvent::PairOpen { kind, span, .. } if !replay => {
                // Stream-through: Quote and Tortoise have no
                // top-level recogniser. Buffering their body events
                // for an inevitable replay would burn O(N) work for
                // nothing — instead enter `streaming` mode and let
                // body events flow straight through. The open's bytes
                // become the seed of a fresh pending plain run.
                if matches!(kind, PairKind::Quote | PairKind::Tortoise) {
                    // Fold any pending refmark into plain first, then
                    // start the new plain run at the open's first byte.
                    let pre_open = self
                        .pending_refmark
                        .take()
                        .map_or(span.start, |rm| rm.start);
                    self.flush_plain_up_to(pre_open);
                    if self.pending_plain_start.is_none() {
                        self.pending_plain_start = Some(span.start);
                    }
                    self.streaming = Some(StreamingFrame { kind, depth: 1 });
                    return;
                }
                // Opening a top-level pair MAY flush the pending plain
                // up to (but not including) the open's start. The
                // refmark, if any and only when this open is a
                // Bracket, is absorbed into the frame; for any other
                // pair kind the refmark is folded into plain first.
                //
                // Ruby and AngleQuote are special: they consume the
                // preceding text (explicit `｜base《reading》` or
                // implicit trailing-kanji). We DON'T flush
                // `pending_plain_start` here so `try_ruby_emit` can
                // walk the preceding source bytes and decide how much
                // of the plain run the ruby actually swallows.
                //
                // Bracket joins the same list: forward-reference
                // `［＃「X」に傍点］` / `…は縦中横］` classifiers need
                // the chance to pull `consume_start` back over the
                // preceding target literal. `try_bracket_emit` then
                // calls `flush_plain_up_to(consume_start)` itself,
                // with the same effect for recognised brackets and a
                // single fused Plain run (literal + raw bracket bytes)
                // for the unrecognised path's replay — visually
                // identical to the pre-defer two-span shape.
                let gaiji_refmark = if matches!(kind, PairKind::Bracket) {
                    self.pending_refmark.take()
                } else {
                    None
                };
                let preserve_pending_plain = matches!(
                    kind,
                    PairKind::Ruby | PairKind::AngleQuote | PairKind::Bracket
                );
                if !preserve_pending_plain {
                    let truncate_to = gaiji_refmark.map_or(span.start, |rm| rm.start);
                    self.flush_plain_up_to(truncate_to);
                }
                self.open_frame(PairEvent::PairOpen { kind, span }, gaiji_refmark);
            }
            other => {
                // Catch-all: every non-Newline event carries a span
                // and folds into the pending plain run.
                let Some(span) = other.span() else {
                    return;
                };
                if self.pending_plain_start.is_none() {
                    self.pending_plain_start = Some(span.start);
                }
            }
        }
    }

    /// Handle one event while in stream-through mode (top-level
    /// Quote / Tortoise pair, no recogniser candidate). Mirrors the
    /// `replay = true` behaviour of [`Self::handle_top_level`] but
    /// (a) reads from the live event stream rather than a buffered
    /// `SmallVec`, (b) tracks nested-open depth so the outer close
    /// unambiguously exits the mode, and (c) skips the inner-frame
    /// open path (a nested `Bracket` / `Ruby` / `AngleQuote` inside
    /// an unrecognised `Quote` folds into the surrounding plain run,
    /// same as the legacy buffered-replay behaviour).
    fn handle_stream_event(&mut self, event: PairEvent) {
        // Defensive — only called when streaming is Some.
        let stream = self
            .streaming
            .as_mut()
            .expect("handle_stream_event without streaming state");
        match event {
            PairEvent::Newline { pos } => {
                self.flush_plain_up_to(pos);
                self.push_output(ClassifiedSpan {
                    kind: SpanKind::Newline,
                    source_span: Span::new(pos, pos + 1),
                });
            }
            PairEvent::PairOpen { kind, span } if kind == stream.kind => {
                stream.depth = stream.depth.saturating_add(1);
                if self.pending_plain_start.is_none() {
                    self.pending_plain_start = Some(span.start);
                }
            }
            PairEvent::PairClose { kind, span } if kind == stream.kind => {
                stream.depth = stream.depth.saturating_sub(1);
                if self.pending_plain_start.is_none() {
                    self.pending_plain_start = Some(span.start);
                }
                if stream.depth == 0 {
                    self.streaming = None;
                }
            }
            PairEvent::Unclosed { kind, .. } => {
                // Phase 2 emits synthetic Unclosed events when EOF
                // arrives mid-frame, one per still-open pair. Each
                // one's span aliases its original PairOpen — those
                // bytes were already folded into the plain run when
                // the PairOpen arrived (or emitted by an intervening
                // Newline flush). Re-folding here would set
                // `pending_plain_start` to a position *behind* the
                // cursor and break the tiling invariant.
                //
                // For Unclosed of the streaming kind, decrement depth
                // so the count mirrors the matching PairOpens (one
                // increment per open, one Unclosed per still-open).
                // When depth reaches zero the outer pair is gone, so
                // exit streaming. Other-kind Unclosed events (a
                // nested Bracket / Ruby / AngleQuote that streaming
                // mode never opened a frame for) are simply ignored.
                if kind == stream.kind {
                    stream.depth = stream.depth.saturating_sub(1);
                    if stream.depth == 0 {
                        self.streaming = None;
                    }
                }
            }
            other => {
                let Some(span) = other.span() else {
                    return;
                };
                if self.pending_plain_start.is_none() {
                    self.pending_plain_start = Some(span.start);
                }
            }
        }
    }

    fn try_ruby_emit(
        &mut self,
        body: BodyView<'_>,
        open_idx: usize,
        close_idx: usize,
    ) -> Option<ClassifiedSpan<'a>> {
        #[cfg(feature = "phase3-instrument")]
        let _phase3_guard = SubsystemGuard::new(Subsystem::TryRubyEmit);
        // Ruby recognition uses the PRECEDING text (if any) as the
        // base — but in the streaming model we don't have that text in
        // the body buffer. We walk back through `pending_outputs` and
        // `pending_plain_start` to find it.
        //
        // The simplest correct approach: synthesise a body slice that
        // includes a single preceding Text event derived from the
        // current `pending_plain_start..open_span.start` range, plus
        // any `Solo(Bar)` if the explicit-ruby shape applies. This
        // mirrors what `recognize_ruby` expects:
        //   events[open_idx - 1] = Text { range: ... preceding ... }
        //   events[open_idx - 2] = optional Solo(Bar)
        let PairEvent::PairOpen {
            span: open_span, ..
        } = body.events[open_idx]
        else {
            return None;
        };

        // Nested ruby: a `《…》` opening *inside* the reading
        // body is an authoring error. Flag the first one (caret on the
        // inner `《`); the outer ruby still parses best-effort. Touched
        // before `ctx` reborrows `self.alloc`, so no borrow clash.
        if let Some(inner_open) = first_nested_ruby_open(body.events, open_idx + 1, close_idx) {
            self.diagnostics.push(Diagnostic::nested_ruby(inner_open));
        }

        // Determine the preceding plain run (the ruby base lives here) and
        // detect the explicit `｜` form by scanning it for a bar. The
        // streaming model has no preceding events, so we synthesise them.
        let preceding_start = self.pending_plain_start.unwrap_or(open_span.start);
        if preceding_start >= open_span.start {
            return None;
        }
        let prev_text_range = Span::new(preceding_start, open_span.start);
        let preceding_bytes = &self.source[preceding_start as usize..open_span.start as usize];
        let bar_byte_offset = preceding_bytes.rfind('｜');

        let (synth, synth_links, synth_open_idx) =
            build_synth_ruby_view(body, prev_text_range, bar_byte_offset)?;
        let synth_close_idx = synth_open_idx + (close_idx - open_idx);
        let synth_view = BodyView {
            events: &synth,
            links: &synth_links,
        };
        let mut ctx = RecogniseCtx {
            alloc: self.alloc,
            source: self.source,
        };
        let Some(m) = ctx.recognize_ruby(synth_view, synth_open_idx, synth_close_idx) else {
            // `recognize_ruby` rejects an empty `《》` reading; flag the
            // explicit-base `｜base《》` shape (a bare `《》` stays silent).
            // The bytes fall through to plain replay either way. `ctx`'s
            // reborrow of `self.alloc` ended at the call above, so pushing
            // onto the disjoint `self.diagnostics` is borrow-clear.
            if let PairEvent::PairClose {
                span: close_span, ..
            } = body.events[close_idx]
                && let Some(span) = empty_explicit_ruby_span(
                    bar_byte_offset,
                    preceding_start,
                    open_span,
                    close_span,
                )
            {
                self.diagnostics.push(Diagnostic::empty_ruby_reading(span));
            }
            return None;
        };
        // Truncate any in-progress plain run to end exactly where the ruby
        // takes over.
        self.flush_plain_up_to(m.consume_start);
        let base_content = self.alloc.content_plain(m.base);
        let node = self.alloc.ruby(base_content, m.reading, m.explicit);
        self.pending_plain_start = None;
        Some(ClassifiedSpan {
            kind: SpanKind::Aozora(node),
            source_span: Span::new(m.consume_start, m.consume_end),
        })
    }

    /// Attempt to classify the buffered body as a `AngleQuote` node.
    ///
    /// Returns `None` when the body content is empty (`≪≫` with
    /// no payload) — the caller falls through to plain replay so the
    /// bytes show up as literal source. Emitting a `AngleQuote` span
    /// here would violate the [`borrowed::NonEmpty`] invariant on the
    /// `Content` payload.
    fn try_angle_quote_emit(
        &mut self,
        body: BodyView<'_>,
        open_idx: usize,
        close_idx: usize,
    ) -> Option<ClassifiedSpan<'a>> {
        let PairEvent::PairOpen {
            span: open_span, ..
        } = body.events[open_idx]
        else {
            unreachable!("body[open_idx] must be PairOpen");
        };
        let PairEvent::PairClose {
            span: close_span, ..
        } = body.events[close_idx]
        else {
            unreachable!("body[close_idx] must be PairClose");
        };
        let mut ctx = RecogniseCtx {
            alloc: self.alloc,
            source: self.source,
        };
        let content = ctx.build_content_from_body(
            body,
            &BodyWindow {
                events: open_idx + 1..close_idx,
                bytes: open_span.end..close_span.start,
            },
        );
        // Empty `≪≫` is not a valid AngleQuote — let the bytes
        // flow through as plain text. The caller's fall-through path
        // (`replay_unrecognised_body`) handles the plain emission.
        if matches!(content, borrowed::Content::Plain(s) if s.is_empty())
            || matches!(content, borrowed::Content::Segments(segs) if segs.is_empty())
        {
            return None;
        }
        self.flush_plain_up_to(open_span.start);
        let node = self.alloc.angle_quote(content);
        self.pending_plain_start = None;
        Some(ClassifiedSpan {
            kind: SpanKind::Aozora(node),
            source_span: Span::new(open_span.start, close_span.end),
        })
    }

    fn try_bracket_emit(
        &mut self,
        body: BodyView<'_>,
        open_idx: usize,
        close_idx: usize,
    ) -> Option<ClassifiedSpan<'a>> {
        #[cfg(feature = "phase3-instrument")]
        let _phase3_guard = SubsystemGuard::new(Subsystem::TryBracketEmit);
        let mut ctx = RecogniseCtx {
            alloc: self.alloc,
            source: self.source,
        };
        let m = ctx.recognize_annotation(body, open_idx, close_idx)?;
        self.flush_plain_up_to(m.consume_start);
        let kind = match m.emit {
            EmitKind::Aozora(node) => SpanKind::Aozora(node),
            EmitKind::BlockOpen(container) => SpanKind::BlockOpen(container),
            EmitKind::BlockClose(container) => SpanKind::BlockClose(container),
        };
        self.pending_plain_start = None;
        // Surface any non-fatal warning the recogniser attached
        // (unrecognised container directive / 縦中横 target not found /
        // ambiguous bouten target). The emitted node is unaffected — for
        // the catch-all cases it is still `Annotation{Unknown}`, so the
        // Tier-A "no bare ［＃" canary holds. `ctx`'s reborrow of
        // `self.alloc` ended at `recognize_annotation`, so pushing onto the
        // disjoint `self.diagnostics` is borrow-clear.
        if let Some(diag) = m.pending_diagnostic {
            self.diagnostics.push(diag);
        }
        // Record bracketed kaeriten for the end-of-document pairing /
        // context checks (`finalize_kaeriten`). The directive span is the
        // whole `［＃…］`.
        if let SpanKind::Aozora(borrowed::AozoraNode::Kaeriten(k)) = kind {
            let span = Span::new(m.consume_start, m.consume_end);
            let (family, rank, is_ladder) = classify_kaeriten_mark(k.mark.as_str());
            self.kaeriten_obs.push(KaeritenObs {
                family,
                rank,
                is_ladder,
                span,
            });
        }
        Some(ClassifiedSpan {
            kind,
            source_span: Span::new(m.consume_start, m.consume_end),
        })
    }

    fn try_gaiji_emit(
        &mut self,
        body: BodyView<'_>,
        bracket_open_idx: usize,
        refmark_span: Span,
    ) -> Option<ClassifiedSpan<'a>> {
        let mut ctx = RecogniseCtx {
            alloc: self.alloc,
            source: self.source,
        };
        let m = ctx.recognize_gaiji(body, refmark_span, bracket_open_idx)?;
        self.flush_plain_up_to(m.consume_start);
        let node = self.alloc.gaiji(m.payload);
        self.pending_plain_start = None;
        // The gaiji still renders best-effort (as its description text)
        // when resolution misses; flag the miss so authors know the glyph
        // won't appear. `m.payload` is a `Copy` arena reference and the
        // `ctx` reborrow of `self.alloc` ended at the `gaiji()` call above,
        // so reading `ucs` and pushing onto `self.diagnostics` is clear of
        // the borrow. Scope: top-level `※［＃…］` only — gaiji nested inside
        // a ruby/bouten body is out of scope for now.
        if m.payload.ucs.is_none() {
            self.diagnostics
                .push(Diagnostic::unresolved_gaiji(Span::new(
                    m.consume_start,
                    m.consume_end,
                )));
        }
        Some(ClassifiedSpan {
            kind: SpanKind::Aozora(node),
            source_span: Span::new(m.consume_start, m.consume_end),
        })
    }

    /// Final flush: emit any trailing Plain run covering the source
    /// tail. Called once when the upstream iterator hits None.
    fn finalize(&mut self) {
        if let Some(rm) = self.pending_refmark.take()
            && self.pending_plain_start.is_none()
        {
            self.pending_plain_start = Some(rm.start);
        }
        let end = self.source_len;
        self.flush_plain_up_to(end);
    }
}

impl<'a, I> Iterator for ClassifyStream<'_, '_, 'a, I>
where
    I: Iterator<Item = PairEvent>,
{
    type Item = ClassifiedSpan<'a>;

    fn next(&mut self) -> Option<ClassifiedSpan<'a>> {
        #[cfg(feature = "phase3-instrument")]
        let _phase3_guard = SubsystemGuard::new(Subsystem::IterDispatch);
        loop {
            if let Some(span) = self.pending_outputs_pop_front() {
                return Some(span);
            }
            if self.finished {
                return None;
            }
            #[cfg(feature = "phase3-instrument")]
            let events_next_guard = SubsystemGuard::new(Subsystem::EventsNext);
            let next_event = self.events.next();
            #[cfg(feature = "phase3-instrument")]
            drop(events_next_guard);
            if let Some(event) = next_event {
                #[cfg(feature = "phase3-instrument")]
                let _phase3_loop_guard = SubsystemGuard::new(Subsystem::LoopBody);
                self.process_event(event);
            } else {
                // Upstream exhausted. Close any active frame as
                // unclosed (its body events fold back to plain;
                // a gaiji-mode refmark also falls into plain),
                // then run final flush.
                if let Some(frame) = self.frame.take() {
                    let refmark = frame.gaiji_refmark;
                    self.replay_unrecognised_body(frame.body, refmark);
                }
                self.finalize();
                self.finished = true;
            }
        }
    }
}

impl<'a, I> ClassifyStream<'_, '_, 'a, I>
where
    I: Iterator<Item = PairEvent>,
{
    fn pending_outputs_pop_front(&mut self) -> Option<ClassifiedSpan<'a>> {
        #[cfg(feature = "phase3-instrument")]
        {
            // Record pending_outputs.len() BEFORE the pop so the
            // distribution histogram tracks pre-pop sizes.
            let len = self.pending_outputs.len() as u64;
            if len > 0 {
                record_pending_size(len);
            }
        }
        self.pending_outputs.pop_front()
    }

    fn process_event(&mut self, event: PairEvent) {
        // Stream-through path for top-level Quote / Tortoise — see
        // `StreamingFrame` for the rationale. Bypasses both frame
        // buffering AND replay; events flow straight through.
        if self.streaming.is_some() {
            self.handle_stream_event(event);
            return;
        }
        if self.frame.is_some() {
            // Inside a frame: every event accumulates. A pending
            // refmark cannot exist while a frame is open (frames are
            // opened from top level and the refmark would have been
            // absorbed or flushed there).
            debug_assert!(
                self.pending_refmark.is_none(),
                "frames are opened from top level; any pending refmark should have been absorbed or flushed before frame entry"
            );
            let outer_closed = self.append_to_frame(event);
            if outer_closed {
                self.recognize_and_emit();
            }
            return;
        }

        // Top level. If a refmark is pending, decide based on the
        // current event:
        if self.pending_refmark.is_some() {
            if let PairEvent::PairOpen {
                kind: PairKind::Bracket,
                ..
            } = &event
            {
                // Will be absorbed by the next handle_top_level call.
            } else {
                // Refmark not followed by Bracket: fold into plain
                // up to the end of the refmark, then continue
                // processing the new event normally. The refmark's
                // span gets absorbed by `flush_plain_up_to` because
                // we set `pending_plain_start` to `rm.start` before
                // taking it.
                let rm = self.pending_refmark.take().expect("checked Some");
                if self.pending_plain_start.is_none() {
                    self.pending_plain_start = Some(rm.start);
                }
            }
        }

        self.handle_top_level(event, /*replay=*/ false);
    }
}

/// Intermediate result of `recognize_ruby`. `base` stays borrowed
/// (the two forms we handle — explicit `｜X《Y》` and implicit
/// trailing-kanji — both come from a single [`PairEvent::Text`] event
/// with no nested structure). `reading`, on the other hand, can carry
/// embedded gaiji (`※［＃…］`) or annotations (`［＃ママ］`), so it is
/// already resolved into a `Content` via `build_content_from_body`.
///
/// Collapsing inside the lexer (rather than leaving the splitting to
/// the renderer) keeps the [`borrowed::AozoraNode`] payload self-contained:
/// Phase 4 stamps one PUA sentinel over the whole `｜…《…》` source
/// span, and the inner gaiji/annotation never reach the top-level
/// `spans` list or downstream consumers.
struct RubyMatch<'s, 'a> {
    base: &'s str,
    reading: borrowed::Content<'a>,
    explicit: bool,
    consume_start: u32,
    consume_end: u32,
}

/// Try to recognize a Ruby span at `events[open_idx]`.
///
/// Two shapes per the Aozora annotation manual
/// (<https://www.aozora.gr.jp/annotation/ruby.html>):
///
/// * **Explicit** — `｜X《Y》`. A [`TriggerKind::Bar`] `Solo` two
///   events before the [`PairKind::Ruby`] open marks the full base.
///   Any Text, not just kanji, may be the base.
/// * **Implicit** — `…X《Y》` where the preceding Text event ends in
///   a run of ideographs. The base is the trailing kanji run of that
///   Text; any non-kanji prefix remains plain.
///
/// The `《…》` reading body is walked with `build_content_from_body`
/// so nested `※［＃…］` gaiji and `［＃…］` annotations fold into the
/// returned `Content` as `Segment::Gaiji` / `Segment::Annotation`.
/// Pure-text readings collapse back to `Content::Plain` via
/// `Content::from_segments`.
///
/// Returns `None` if neither shape applies (empty reading, no
/// preceding Text, no kanji for implicit).
impl<'a, 's> RecogniseCtx<'_, 'a, 's> {
    fn recognize_ruby(
        &mut self,
        view: BodyView<'_>,
        open_idx: usize,
        close_idx: usize,
    ) -> Option<RubyMatch<'s, 'a>> {
        #[cfg(feature = "phase3-instrument")]
        let _phase3_guard = SubsystemGuard::new(Subsystem::Ruby);
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
        if open_span.end >= close_span.start {
            // Empty reading — the `《…》` body has no bytes.
            return None;
        }
        if open_idx == 0 {
            return None;
        }
        let PairEvent::Text {
            range: prev_range, ..
        } = events[open_idx - 1]
        else {
            return None;
        };
        let prev_text = &self.source[prev_range.start as usize..prev_range.end as usize];

        let reading = self.build_content_from_body(
            view,
            &BodyWindow {
                events: open_idx + 1..close_idx,
                bytes: open_span.end..close_span.start,
            },
        );

        // Explicit form: Solo(Bar) two events before the open, with the
        // Text between them acting as the base.
        if open_idx >= 2
            && let PairEvent::Solo {
                kind: TriggerKind::Bar,
                span: bar_span,
            } = events[open_idx - 2]
        {
            if prev_text.is_empty() {
                return None;
            }
            return Some(RubyMatch {
                base: prev_text,
                reading,
                explicit: true,
                consume_start: bar_span.start,
                consume_end: close_span.end,
            });
        }

        // Implicit form: trailing-kanji run of the preceding Text.
        let kanji_offset = trailing_kanji_start(prev_text);
        if kanji_offset == prev_text.len() {
            return None;
        }
        let consume_start =
            prev_range.start + u32::try_from(kanji_offset).expect("kanji offset fits in u32");
        Some(RubyMatch {
            base: &prev_text[kanji_offset..],
            reading,
            explicit: false,
            consume_start,
            consume_end: close_span.end,
        })
    }
}

/// Half-open window into a [`PairEvent`] stream. Bundles the event-
/// index range with the matching byte-offset range so
/// `build_content_from_body` can flush text segments using source
/// byte slices without re-derefing event spans on every iteration.
///
/// The two ranges are redundant in principle — `bytes.start` always
/// equals `events[events.start]`'s leading edge — but caching them
/// avoids a branch when the range is empty and makes the helper
/// signature honest about what it needs.
struct BodyWindow {
    events: Range<usize>,
    bytes: Range<u32>,
}

/// Walk `window` over `events` and build the corresponding
/// `Content`.
///
/// Each nested `※［＃description、mencode］` reduces to a
/// `Segment::Gaiji` via `recognize_gaiji`; each standalone
/// `［＃…］` reduces to a `Segment::Annotation` via
/// `recognize_annotation`. Every other byte (plain text, stray
/// triggers, unmatched delimiters) is captured into adjacent
/// `Segment::Text` runs by tracking a single "outstanding text
/// start" byte offset and flushing only when a recognisable construct
/// consumes the intervening bytes.
///
/// Non-Annotation Aozora emits (a paired-container opener, a block
/// leaf, etc.) are *not* first-class segments and are folded back
/// into `Annotation{Unknown}` with the raw bracket bytes — this keeps
/// the Tier-A canary intact inside a ruby body regardless of how
/// unusual the inner annotation shape is.
///
/// ## Fast path
///
/// [`has_nested_candidate`] first short-circuits the body scan: when
/// no `Solo(RefMark)` and no `PairOpen(Bracket)` appear, the body is
/// guaranteed to be plain text (possibly peppered with unrelated
/// triggers like `｜` or mismatched quotes, which we treat as text).
/// Returning `Content::from(&str)` in that branch skips the `Vec`
/// allocation and the `from_segments` collapse pass — a win for the
/// 99%+ of ruby readings that carry no embedded structure.
///
/// ## Slow path
///
/// The fallback is a single `O(body_events)` sweep. `text_start`
/// tracks the earliest byte that has not yet been committed to a Text
/// segment; flushing is strictly triggered by a *recognised* nested
/// construct, so unrelated events cost a single index increment. Each
/// recognition jumps to `close_idx + 1` using Phase 2's pre-linked
/// pair indices, keeping the sweep strictly forward-only regardless
/// of nesting depth.
///
/// The returned value is always normalised via
/// `Content::from_segments`, so a slow-path body that turned out to
/// contain only text (for example because its brackets were malformed
/// and skipped) still collapses back to `Content::Plain`.
/// Immutable per-call body-walk context shared across the
/// `build_content_from_body` orchestrator and its per-shape helpers
/// (`try_emit_gaiji_at` / `try_emit_annotation_at`). Bundling `view`
/// and `window` together prevents the per-helper signatures from
/// exceeding the project's 4-arg threshold without losing positional
/// clarity at call sites.
#[derive(Clone, Copy)]
struct BodyWalkCtx<'b> {
    view: BodyView<'b>,
    window: &'b BodyWindow,
}

/// Mutable per-call build state for `build_content_from_body`.
/// Tracks the under-construction segment vector and the byte position
/// where the current pending Text run started. Threading these through
/// per-shape helpers as a single `&mut` field keeps the helper
/// signatures at ≤4 args.
struct ContentBuild<'a> {
    segments: Vec<borrowed::Segment<'a>>,
    /// Byte position of the earliest source byte not yet committed
    /// to a `Segment::Text`. Each successful gaiji / annotation emit
    /// advances this past the consumed bracket.
    text_start: u32,
}

impl<'a> RecogniseCtx<'_, 'a, '_> {
    /// Build a borrowed [`Content`](borrowed::Content) for the body
    /// window, recognising any nested gaiji / annotation constructs in
    /// a single forward sweep.
    ///
    /// Fast path returns when the body has no `※` and no `［` —
    /// emits the raw byte run as a single `Plain`. The slow path
    /// dispatches each event index through two per-shape recognise
    /// helpers and falls through (advancing `i`) when neither claims
    /// the slot.
    fn build_content_from_body(
        &mut self,
        view: BodyView<'_>,
        window: &BodyWindow,
    ) -> borrowed::Content<'a> {
        #[cfg(feature = "phase3-instrument")]
        let _phase3_guard = SubsystemGuard::new(Subsystem::BuildContent);
        debug_assert!(
            window.events.start <= window.events.end,
            "body window event range must be non-inverted",
        );
        debug_assert!(
            window.bytes.start <= window.bytes.end,
            "body window byte range must be non-inverted",
        );
        debug_assert_eq!(
            view.events.len(),
            view.links.len(),
            "BodyView events/links must be parallel",
        );

        let body_events = &view.events[window.events.start..window.events.end];
        if !has_nested_candidate(body_events) {
            // Fast path: no `※` and no `［` in the body; bytes pass
            // through verbatim. `content_plain("")` canonicalises to
            // empty `Segments(&[])` to match the legacy
            // `Content::from(&str)` shape exactly.
            let text = &self.source[window.bytes.start as usize..window.bytes.end as usize];
            return self.alloc.content_plain(text);
        }

        // Slow path: at least one potential nested construct exists.
        // Pre-size the segment vector: worst case is `ceil(n / 2)` runs
        // of `Text, Construct, Text, …` plus one trailing Text.
        // `body_events.len() + 1` is a safe upper bound that is small
        // in practice (ruby readings almost never reach double-digit
        // events).
        let body = BodyWalkCtx { view, window };
        let mut build = ContentBuild {
            segments: Vec::with_capacity(body_events.len() + 1),
            text_start: window.bytes.start,
        };
        let mut i = window.events.start;
        while i < window.events.end {
            if let Some(next_i) = self.try_emit_gaiji_at(body, &mut build, i) {
                i = next_i;
                continue;
            }
            if let Some(next_i) = self.try_emit_annotation_at(body, &mut build, i) {
                i = next_i;
                continue;
            }
            i += 1;
        }
        push_text_segment(
            &mut build.segments,
            self.source,
            build.text_start..window.bytes.end,
            self.alloc,
        );
        self.alloc.content_segments(&build.segments)
    }

    /// Shape 1: `※［＃…］` — `Solo(RefMark)` immediately followed by a
    /// matched `PairOpen(Bracket)`. On a successful gaiji recognise,
    /// flush the pending Text run, push a `Segment::Gaiji`, advance
    /// `text_start`, and return the index of the first event past the
    /// bracket close. Returns `None` if the shape doesn't match or if
    /// the inner [`Self::recognize_gaiji`] bails.
    fn try_emit_gaiji_at(
        &mut self,
        body: BodyWalkCtx<'_>,
        build: &mut ContentBuild<'a>,
        i: usize,
    ) -> Option<usize> {
        let PairEvent::Solo {
            kind: TriggerKind::RefMark,
            span: refmark_span,
        } = body.view.events[i]
        else {
            return None;
        };
        let bracket_idx = i + 1;
        if bracket_idx >= body.window.events.end {
            return None;
        }
        let PairEvent::PairOpen {
            kind: PairKind::Bracket,
            ..
        } = body.view.events[bracket_idx]
        else {
            return None;
        };
        let close_link = body.view.links[bracket_idx];
        if close_link == u32::MAX {
            return None;
        }
        let close_idx = close_link as usize;
        if close_idx >= body.window.events.end {
            return None;
        }
        let g = self.recognize_gaiji(body.view, refmark_span, bracket_idx)?;
        push_text_segment(
            &mut build.segments,
            self.source,
            build.text_start..g.consume_start,
            self.alloc,
        );
        build.segments.push(self.alloc.seg_gaiji(g.payload));
        build.text_start = g.consume_end;
        Some(close_idx + 1)
    }

    /// Shape 2: `［＃…］` — a standalone bracket annotation. Tried
    /// after [`Self::try_emit_gaiji_at`] so the `※`+bracket combo gets
    /// first claim on a leading bracket. `recognize_annotation` has an
    /// `Unknown` catch-all (only returns `None` for malformed brackets
    /// with no `＃` sentinel); on success the bracket folds into a
    /// `Segment::Annotation` with the recogniser's payload (or a
    /// synthetic `Unknown` payload built from the raw source bytes
    /// when the recogniser left `annotation_payload` unset). The
    /// fallback synthesis preserves the Tier-A canary: no bare `［＃`
    /// ever leaks outside an `aozora-annotation` wrapper.
    fn try_emit_annotation_at(
        &mut self,
        body: BodyWalkCtx<'_>,
        build: &mut ContentBuild<'a>,
        i: usize,
    ) -> Option<usize> {
        let PairEvent::PairOpen {
            kind: PairKind::Bracket,
            span: open_span,
        } = body.view.events[i]
        else {
            return None;
        };
        let close_link = body.view.links[i];
        if close_link == u32::MAX {
            return None;
        }
        let close_idx = close_link as usize;
        if close_idx >= body.window.events.end {
            return None;
        }
        let a = self.recognize_annotation(body.view, i, close_idx)?;
        let PairEvent::PairClose {
            span: close_span, ..
        } = body.view.events[close_idx]
        else {
            // Phase 2 invariant: PairOpen's link always targets a
            // PairClose of the matching kind.
            unreachable!("PairOpen link must target a PairClose");
        };
        let payload = if let Some(p) = a.annotation_payload {
            p
        } else {
            let raw = &self.source[open_span.start as usize..close_span.end as usize];
            self.alloc.make_annotation(raw, AnnotationKind::Unknown)
        };
        push_text_segment(
            &mut build.segments,
            self.source,
            build.text_start..a.consume_start,
            self.alloc,
        );
        build.segments.push(self.alloc.seg_annotation(payload));
        build.text_start = a.consume_end;
        Some(close_idx + 1)
    }
}

/// Whether `body` could host a nested gaiji / annotation. The Phase 2
/// event model guarantees that:
///
/// * `※［＃…］` always emits a `Solo(RefMark)` event at its `※`.
/// * `［＃…］` always emits a `PairOpen(Bracket)` event at its `［`.
///
/// So the absence of both event shapes in the body is sufficient proof
/// that no nested construct can be recognised, allowing
/// `build_content_from_body` to take the allocation-free fast path.
fn has_nested_candidate(body: &[PairEvent]) -> bool {
    body.iter().any(|e| {
        matches!(
            e,
            PairEvent::Solo {
                kind: TriggerKind::RefMark,
                ..
            } | PairEvent::PairOpen {
                kind: PairKind::Bracket,
                ..
            }
        )
    })
}

/// Append `source[start..end]` to `segments` as a `Segment::Text` if
/// the slice is non-empty. `start == end` occurs naturally when a
/// recognised construct sits at the very start of the body or
/// immediately follows a previous one; skipping those zero-length
/// flushes keeps the post-collapse invariant "no empty `Text` in a
/// `Segments` run" (see `Content::from_segments`) without a second
/// compaction pass.
#[inline]
fn push_text_segment<'a>(
    segments: &mut Vec<borrowed::Segment<'a>>,
    source: &str,
    bytes: Range<u32>,
    alloc: &mut BorrowedAllocator<'a>,
) {
    if !bytes.is_empty() {
        segments.push(alloc.seg_text(&source[bytes.start as usize..bytes.end as usize]));
    }
}

/// Intermediate result of `recognize_gaiji`.
///
/// Holds the payload (`&'a borrowed::Gaiji<'a>`) rather than a wrapped
/// node so the caller can route it to either `alloc.gaiji(p)`
/// (top-level span) or `alloc.seg_gaiji(p)` (nested inside a body
/// content) without re-paying the description / mencode intern cost.
struct GaijiMatch<'a> {
    payload: &'a borrowed::Gaiji<'a>,
    consume_start: u32,
    consume_end: u32,
}

/// Try to recognize a gaiji reference at `events[refmark_idx]`.
///
/// Shape: `※［＃<description>、<mencode>］` or `※［＃<description>］`.
/// The description may be wrapped in `「…」` (the common form) or
/// appear bare. `<mencode>` is the mencode reference (`第3水準1-85-54`,
/// `U+XXXX`, etc.) appearing after a `、` separator.
///
/// The UCS resolution column of `Gaiji` is populated by
/// `aozora_encoding::gaiji::lookup` before the recogniser returns, so
/// downstream consumers receive a resolved `Option<char>` without
/// having to re-probe the mencode table.
///
/// Event preconditions (checked):
/// * `events[refmark_idx]` is `Solo(RefMark)` [done by caller]
/// * `events[refmark_idx + 1]` is `PairOpen(Bracket)` [done by caller]
/// * `events[refmark_idx + 2]` is `Solo(Hash)` [checked here]
///
/// Consume range is from `refmark_span.start` to the bracket close's
/// end — i.e. the `※` and the entire following `［＃…］` fold into
/// one Aozora span.
impl<'a> RecogniseCtx<'_, 'a, '_> {
    fn recognize_gaiji(
        &mut self,
        view: BodyView<'_>,
        refmark_span: Span,
        bracket_open_idx: usize,
    ) -> Option<GaijiMatch<'a>> {
        #[cfg(feature = "phase3-instrument")]
        let _phase3_guard = SubsystemGuard::new(Subsystem::Gaiji);
        let events = view.events;
        let &PairEvent::PairOpen {
            kind: PairKind::Bracket,
            ..
        } = events.get(bracket_open_idx)?
        else {
            return None;
        };
        let bracket_close_link = *view.links.get(bracket_open_idx)?;
        if bracket_close_link == u32::MAX {
            return None;
        }
        let bracket_close_idx = bracket_close_link as usize;
        let hash_end = match events.get(bracket_open_idx + 1)? {
            PairEvent::Solo {
                kind: TriggerKind::Hash,
                span,
            } => span.end,
            _ => return None,
        };
        let &PairEvent::PairClose {
            span: bracket_close_span,
            ..
        } = events.get(bracket_close_idx)?
        else {
            return None;
        };

        // Try the quoted-description form first: `「DESC」、MENCODE`. Two
        // events after open: PairOpen(Quote).
        let quote_open_idx = bracket_open_idx + 2;
        let quoted = events.get(quote_open_idx).and_then(|ev| match *ev {
            PairEvent::PairOpen {
                kind: PairKind::Quote,
                span: qos,
            } => {
                let qci_link = *view.links.get(quote_open_idx)?;
                if qci_link == u32::MAX {
                    return None;
                }
                let qci = qci_link as usize;
                if qci >= bracket_close_idx {
                    return None;
                }
                let PairEvent::PairClose { span: qcs, .. } = *events.get(qci)? else {
                    return None;
                };
                let desc = &self.source[qos.end as usize..qcs.start as usize];
                if desc.is_empty() {
                    return None;
                }
                let tail = self.source[qcs.end as usize..bracket_close_span.start as usize].trim();
                let mencode = tail.strip_prefix('、').map(str::trim);
                Some((desc.to_owned(), mencode.map(str::to_owned)))
            }
            _ => None,
        });

        let (description, mencode) = if let Some(pair) = quoted {
            pair
        } else {
            self.extract_bare_gaiji_body(hash_end, bracket_close_span.start)?
        };

        if description.is_empty() {
            return None;
        }
        // Reject descriptions that carry structural quote characters.
        // The serializer wraps the description in `「…」` for round-trip,
        // so a stray `」` (e.g. from the malformed input `※［＃」］`) would
        // make `serialize ∘ parse` non-stable. Falling through to `None`
        // here lets the higher-level classifier wrap the raw bracket in
        // an `Annotation{Unknown}`, which round-trips byte-identical.
        if description.contains(['「', '」']) {
            return None;
        }
        // Reject descriptions that embed a nested annotation-opening
        // sequence `［＃`. Pathological shapes like `※［＃［＃改］］`
        // produce a description string that *contains* `［＃`, which the
        // gaiji renderer would emit verbatim inside `<span class=
        // "aozora-gaiji">…</span>` — leaking a bare `［＃` outside the
        // `aozora-annotation` wrapper and violating the Tier A canary.
        // Falling through here lets the outer bracket be claimed by
        // `Annotation{Unknown}`, which is rendered inside an
        // `aozora-annotation` wrapper as the canary requires.
        if description.contains("［＃") {
            return None;
        }

        // Resolve the Unicode scalar at lex time via the static table in
        // aozora-encoding so the downstream AST / renderer never has to
        // re-probe. `None` stays `None` when the mencode has no mapping
        // entry and no `U+XXXX` shape matches — the renderer falls back
        // to escaping the raw `description`.
        let ucs = gaiji_resolve::lookup(None, mencode.as_deref(), &description);

        let payload = self.alloc.make_gaiji(&description, ucs, mencode.as_deref());
        Some(GaijiMatch {
            payload,
            consume_start: refmark_span.start,
            consume_end: bracket_close_span.end,
        })
    }

    /// Bare-form `※［＃DESC、MENCODE］` body extraction. Kept to
    /// accommodate the historical form `※［＃二の字点、1-2-23］` (no
    /// `「…」` quotes) that some Aozora source uses, but tightened
    /// so the I3 idempotency property holds:
    ///
    ///   - body must split on `、` (description-only bodies like
    ///     `※［＃ 1234《...］` are ambiguous: the serializer would
    ///     wrap them in `「…」`, and the re-parse would lift them to
    ///     a quoted Gaiji, drifting the annotation count)
    ///   - the mencode portion must look like a real JIS X 0213
    ///     reference (`N-N-N` / `第N水準N-N-N`) or a `U+XXXX`
    ///     codepoint. Random tail bytes (`》` etc.) get rejected,
    ///     the whole bracket falls through to `Annotation::Unknown`,
    ///     and the raw bytes round-trip byte-identical.
    fn extract_bare_gaiji_body(
        &self,
        hash_end: u32,
        bracket_close_start: u32,
    ) -> Option<(String, Option<String>)> {
        let body = self.source[hash_end as usize..bracket_close_start as usize].trim();
        let (desc, men) = body.split_once('、')?;
        let men = men.trim();
        if !is_mencode_shaped(men) {
            return None;
        }
        Some((desc.trim().to_owned(), Some(men.to_owned())))
    }
}

/// Loose validator for a bare-form gaiji mencode field.
///
/// Accepts:
///   - `N-N-N` (plain JIS X 0213 row-cell, digits + ASCII hyphens)
///   - `第N水準N-N-N` (same with the explicit `水準` label)
///   - `U+XXXX` (1–6 ASCII hex digits)
///
/// Anything else — random kanji, half-/full-width punctuation, etc.
/// — is rejected so the bare-description bracket falls through to
/// `Annotation::Unknown` instead of getting serializer-promoted into
/// a quoted Gaiji. See `recognize_gaiji` for the I3-idempotency
/// rationale.
fn is_mencode_shaped(s: &str) -> bool {
    if let Some(hex) = s.strip_prefix("U+") {
        return !hex.is_empty() && hex.len() <= 6 && hex.chars().all(|c| c.is_ascii_hexdigit());
    }
    // Optional `第N水準` prefix: skip past the digits + `水準` token
    // if present, then validate the remainder as `N-N-N` (digits and
    // ASCII hyphens only, must contain at least one digit).
    let rest = s
        .strip_prefix('第')
        .and_then(|after_dai| {
            let nondigit = after_dai.find(|c: char| !c.is_ascii_digit())?;
            let (_digits, tail) = after_dai.split_at(nondigit);
            tail.strip_prefix("水準")
        })
        .unwrap_or(s);
    !rest.is_empty()
        && rest.chars().all(|c| c.is_ascii_digit() || c == '-')
        && rest.chars().any(|c| c.is_ascii_digit())
}

#[cfg(test)]
mod is_mencode_shaped_tests {
    use super::is_mencode_shaped;

    #[test]
    fn jis_x_0213_row_cell_passes() {
        assert!(is_mencode_shaped("1-2-23"));
        assert!(is_mencode_shaped("1-85-54"));
    }

    #[test]
    fn dai_n_suijun_prefix_passes() {
        assert!(is_mencode_shaped("第3水準1-85-54"));
        assert!(is_mencode_shaped("第4水準2-1-2"));
    }

    #[test]
    fn u_plus_codepoint_passes() {
        assert!(is_mencode_shaped("U+304B"));
        assert!(is_mencode_shaped("U+1F94A"));
    }

    #[test]
    fn random_garbage_rejected() {
        assert!(!is_mencode_shaped("》"));
        assert!(!is_mencode_shaped("abc"));
        assert!(!is_mencode_shaped("漢字"));
        assert!(!is_mencode_shaped(""));
        assert!(!is_mencode_shaped("U+"));
        assert!(!is_mencode_shaped("U+ZZZZ"));
        assert!(!is_mencode_shaped("第3水準"));
    }
}

/// Byte offset where the trailing kanji run in `text` begins.
///
/// Walks chars right-to-left, keeping track of the earliest byte
/// offset reached while every char is a ruby-base char. Returns
/// `text.len()` if the final char is not a ruby-base char (→ no
/// implicit base available).
fn trailing_kanji_start(text: &str) -> usize {
    let mut start = text.len();
    for (idx, ch) in text.char_indices().rev() {
        if is_ruby_base_char(ch) {
            start = idx;
        } else {
            break;
        }
    }
    start
}

/// Intermediate result of `recognize_annotation`.
///
/// `emit` decides which [`SpanKind`] the driver pushes for the
/// top-level case. `annotation_payload` is `Some` exactly when the
/// recogniser produced an `Annotation{…}` payload — the
/// `build_content_from_body` caller uses it to wrap the same payload
/// as a `Segment::Annotation` without reconstructing it. The emit
/// variants `BlockOpen` / `BlockClose` and non-`Annotation` `Aozora`
/// nodes leave `annotation_payload` as `None`, so the body-builder
/// falls back to its `Annotation{Unknown}` synthesis path.
struct AnnotationMatch<'a> {
    emit: EmitKind<'a>,
    annotation_payload: Option<&'a borrowed::Annotation<'a>>,
    consume_start: u32,
    consume_end: u32,
    /// A non-fatal warning to surface for this bracket, if any —
    /// `unrecognised_container_directive`, `tcy_target_not_found`, or
    /// `bouten_target_ambiguous`. The caller drains it into the diagnostic
    /// stream; the emitted node (often the `Annotation{Unknown}` catch-all,
    /// canary intact) is unaffected.
    pending_diagnostic: Option<Diagnostic>,
}

/// What to emit for a matched annotation.
enum EmitKind<'a> {
    /// Inline or block-leaf — becomes [`SpanKind::Aozora`].
    Aozora(borrowed::AozoraNode<'a>),
    /// Paired-container opener — becomes [`SpanKind::BlockOpen`].
    BlockOpen(ContainerKind),
    /// Paired-container closer — becomes [`SpanKind::BlockClose`].
    BlockClose(ContainerKind),
}

/// Try to recognize a `［＃keyword…］` annotation at
/// `events[open_idx]`.
///
/// Requires the immediately-next event to be a [`TriggerKind::Hash`]
/// [`PairEvent::Solo`] — the shape `［` `＃` `body` `］`. Bodies
/// without a hash (plain `［…］`) are not annotations; bodies with a
/// hash whose keyword no specialised recogniser matches fall through
/// to the `Annotation { Unknown }` catch-all so the bracket is
/// always consumed into some `AozoraNode`.
impl<'a> RecogniseCtx<'_, 'a, '_> {
    #[allow(
        clippy::too_many_lines,
        reason = "a flat dispatch chain over the forward-reference recognisers \
                  (body / bouten / 縦中横 / heading / emphasis) — each block is \
                  the same shape and splitting them would scatter the ordered \
                  fall-through to the Annotation{Unknown} catch-all"
    )]
    fn recognize_annotation(
        &mut self,
        view: BodyView<'_>,
        open_idx: usize,
        close_idx: usize,
    ) -> Option<AnnotationMatch<'a>> {
        #[cfg(feature = "phase3-instrument")]
        let _phase3_guard = SubsystemGuard::new(Subsystem::Annotation);
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
                // wrap as a `Segment::Annotation` with the correct
                // `WarichuOpen` / `WarichuClose` kind instead of the
                // catch-all `Unknown` downgrade. Other body-keyword
                // families (PageBreak, Indent, …) leave the payload as
                // `None`, matching the legacy behaviour where the body-
                // builder fell through to its `Annotation{Unknown}`
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
        // `「X」の左に「Y」の注記` — left-side annotation (注記). The `の注記`
        // keyword is disjoint from `のルビ` and every bouten kind, so the
        // bouten and left-ruby classifiers above have already declined.
        if let Some((node, consume_start)) =
            self.classify_forward_left_annotation(view, open_idx, close_idx)
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
        // fall-through path while still degrading to `Annotation{Unknown}`.
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

        // No specialised recogniser claimed the bracket — fall back to the
        // `Annotation{Unknown}` catch-all.
        Some(self.unknown_annotation_match(directive_span, body, tcy_pending))
    }

    /// Catch-all for any well-formed `［＃…］` whose body no specialised
    /// recogniser claimed — including empty bodies (`［＃］`), which real
    /// Aozora corpora occasionally use as illustrative glyphs. Emitting
    /// `Annotation{Unknown}` with the raw source slice keeps the Tier-A
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
        let payload = self.alloc.make_annotation(raw, AnnotationKind::Unknown);
        let node = self.alloc.annotation(payload);
        let payload_for_seg = self.alloc.make_annotation(raw, AnnotationKind::Unknown);
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
}

/// Classify a `［＃「target」に<bouten-kind>］` forward-reference
/// bouten annotation.
///
/// Uses the event-stream layout to find the target quote pair,
/// avoiding the string-find-first-`」` pitfall when the target text
/// itself contains nested `「…」`. Phase 2 has already balanced the
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
    fn classify_forward_bouten(
        &mut self,
        view: BodyView<'_>,
        open_idx: usize,
        close_idx: usize,
    ) -> Option<(borrowed::AozoraNode<'a>, u32, bool)> {
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
        // has no referent and we fall through to the Annotation{Unknown}
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
/// bracket degrades to `Annotation{Unknown}`), and from a bracket that is
/// not a 縦中横 directive at all (silent fall-through).
enum ForwardTcy<'a> {
    /// A 縦中横 with a located target — the node plus its consume start.
    Recognised(borrowed::AozoraNode<'a>, u32),
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
/// `classify_forward_tcy` instead of leaking to `Annotation{Unknown}`.
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
        if extracted.suffix != "は縦中横" {
            return ForwardTcy::NotTcy;
        }
        let Some(first) = extracted.targets.first() else {
            return ForwardTcy::NotTcy;
        };
        // The shape is a 縦中横 directive. If its target has no referent in
        // the preceding text the styling is meaningless — flag it (the
        // caller turns this into `tcy_target_not_found`) and let the bracket
        // fall through to `Annotation{Unknown}`.
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
    #[cfg(feature = "phase3-instrument")]
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

/// Whether `body` is the okurigana shape `（X）` where X is a short
/// run of Japanese characters.
///
/// The length bound guards against accidentally claiming long
/// parenthesised glosses (which belong to the generic annotation
/// catch-all). 6 characters is the ~99th-percentile okurigana length
/// in Aozora corpora; anything longer is practically always editorial
/// prose rather than an inflection marker.
fn is_okurigana_body(body: &str) -> bool {
    let Some(inner) = body.strip_prefix('（').and_then(|s| s.strip_suffix('）')) else {
        return false;
    };
    // Byte-length prefilter: every accepted okurigana char is a CJK
    // glyph in {hiragana, katakana, half-width katakana, CJK unified}.
    // Hiragana/katakana/CJK are 3 bytes UTF-8; half-width katakana
    // is also 3 bytes (U+FF61..U+FF9F). So a 1..=6 char inner has
    // byte length in `3..=18`. Any inner outside that range cannot
    // satisfy `is_okurigana_char.all` and we skip the char decode.
    if !(3..=18).contains(&inner.len()) {
        return false;
    }
    // Single-pass fusion of `chars().count()` + `chars().all()`:
    // count and class-check in one walk, with early-out at >6 chars
    // or first non-conforming char. Replaces two iterations over
    // the same byte stream.
    let mut count = 0usize;
    for c in inner.chars() {
        count += 1;
        if count > 6 || !is_okurigana_char(c) {
            return false;
        }
    }
    count >= 1
}

/// Character class accepted inside okurigana parens: hiragana,
/// katakana (incl. half-width), CJK unified ideographs. Deliberately
/// narrower than "any non-whitespace" so editorial `（注）` or
/// punctuation-rich glosses fall through to the annotation path.
const fn is_okurigana_char(ch: char) -> bool {
    matches!(
        ch,
        '\u{3041}'..='\u{309F}'      // hiragana
        | '\u{30A0}'..='\u{30FF}'    // katakana
        | '\u{FF66}'..='\u{FF9F}'    // half-width katakana
        | '\u{4E00}'..='\u{9FFF}'    // CJK unified
        | '\u{3400}'..='\u{4DBF}'    // CJK ext A
        | '\u{F900}'..='\u{FAFF}'    // CJK compat
    )
}

/// Classify a `［＃挿絵（file）入る］` sashie (illustration insert),
/// optionally bundling a caption: `［＃挿絵（file）「caption」入る］`.
///
/// Called from [`classify_annotation_body`]'s `SashiePrefix` arm —
/// the AC has already verified the `挿絵（` prefix at body[0..9]; this
/// function captures the filename between `（` and `）`, an optional
/// `「caption」` (per <https://www.aozora.gr.jp/annotation/graphics.html>),
/// and confirms the trailing `入る` keyword. The caption is plain content,
/// rendered into `<figcaption>` (§8).
fn classify_sashie_body<'a>(body: &str, alloc: &mut BorrowedAllocator<'a>) -> Option<EmitKind<'a>> {
    let rest = body.strip_prefix("挿絵（")?;
    // `）` is a full-width right parenthesis (U+FF09). Find its first
    // occurrence — corpus rarely nests `（）` inside a filename.
    let close_off = rest.find('）')?;
    let file = &rest[..close_off];
    if file.is_empty() {
        return None;
    }
    let tail = &rest[close_off + '）'.len_utf8()..];
    // After `）` the tail is either the bare `入る` keyword or a bundled
    // `「caption」入る`. Any other shape declines (→ `Annotation{Unknown}`).
    let caption = if tail == "入る" {
        None
    } else if let Some(inner) = tail
        .strip_prefix('「')
        .and_then(|t| t.strip_suffix("」入る"))
    {
        if inner.is_empty() {
            return None;
        }
        Some(alloc.content_plain(inner))
    } else {
        return None;
    };
    Some(EmitKind::Aozora(alloc.sashie(file, caption)))
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
/// `borrowed::AozoraHeading` (大→`<h1>` / 中→`<h2>` / 小→`<h3>`): the
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
/// the catch-all emit `Annotation { Unknown }` so the reader at least
/// sees the raw bracket text in diagnostics.
impl<'a> RecogniseCtx<'_, 'a, '_> {
    fn classify_forward_heading(
        &mut self,
        view: BodyView<'_>,
        open_idx: usize,
        close_idx: usize,
    ) -> Option<(borrowed::AozoraNode<'a>, u32)> {
        let extracted = extract_forward_quote_targets(view, self.source, open_idx, close_idx)?;
        let rest = extracted.suffix.strip_prefix("は")?;
        let (style, kind) = parse_heading_keyword(rest)?;

        // Reject hints whose targets are not preceded by matching text.
        // See `classify_forward_bouten` for the same rationale.
        for target in &extracted.targets {
            if target.is_empty() {
                continue;
            }
            if !forward_target_is_preceded(view.events, self.source, open_idx, target) {
                return None;
            }
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
            self.alloc
                .heading_hint(heading_level_u8(kind), style, &combined),
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
    ) -> Option<(borrowed::AozoraNode<'a>, u32)> {
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

/// Classify a `「X」の左に「Y」の注記` forward-reference **left-side
/// annotation** (注記). The structural twin of
/// [`Self::classify_forward_left_ruby`] — same single-target pull-back — but
/// the `の注記` keyword selects a distinct [`borrowed::AozoraNode::SideNote`]
/// node, so it round-trips to `の注記`, not `のルビ`. The `の左に「…」の注記`
/// suffix is unique, so the left-side bouten / left-ruby classifiers above
/// have already declined.
impl<'a> RecogniseCtx<'_, 'a, '_> {
    fn classify_forward_left_annotation(
        &mut self,
        view: BodyView<'_>,
        open_idx: usize,
        close_idx: usize,
    ) -> Option<(borrowed::AozoraNode<'a>, u32)> {
        let extracted = extract_forward_quote_targets(view, self.source, open_idx, close_idx)?;
        let [target] = extracted.targets.as_slice() else {
            return None;
        };
        // suffix == の左に「<note>」の注記
        let note_text = extracted
            .suffix
            .strip_prefix("の左に「")?
            .strip_suffix("」の注記")?;
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
        Some((self.alloc.side_note(base, note), consume_start))
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
/// `Annotation{Unknown}`. The `forward_target_is_preceded` gate rejects a
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
    ) -> Option<(borrowed::AozoraNode<'a>, u32)> {
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

/// Map an [`AozoraHeadingKind`] (大/中/小) to the numeric outline level
/// (1/2/3) the inline `heading-hint` carries in `data-level`.
const fn heading_level_u8(kind: AozoraHeadingKind) -> u8 {
    match kind {
        AozoraHeadingKind::Medium => 2,
        AozoraHeadingKind::Small => 3,
        // 大見出し and any future level default to the top level.
        _ => 1,
    }
}

/// Byte position where `target` begins, **only if** it is the bare line
/// immediately preceding the `［` at `open_idx` — i.e. `target` followed
/// by a single `\n`, and itself starting at a line boundary (BOF or after
/// a `\n`). The promoted heading's consume span is pulled back to this
/// position so `序章\n［＃「序章」は…見出し］` collapses into one
/// `AozoraHeading`; the mandatory `\n` keeps the serializer's round-trip
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

/// Parse a heading keyword into `(style, kind)`. An optional `同行`
/// (same-line) / `窓` (window) prefix selects the style; the remaining
/// `大 / 中 / 小見出し` selects the level. Shared by the forward-reference
/// hint (`「X」はSTYLEレベル見出し`) and the paired / block container forms
/// ([`parse_heading_directive`]).
///
/// `副見出し` is not a real annotation — it never occurs in the corpus — so
/// it matches nothing and the directive falls through to `Annotation{Unknown}`.
/// The 同行 / 窓 styles cross with every level (`同行中見出し`, `窓小見出し`, …).
fn parse_heading_keyword(s: &str) -> Option<(AozoraHeadingStyle, AozoraHeadingKind)> {
    // An optional 同行 / 窓 style prefix, else the standard style; `rest` is
    // the remaining 大/中/小見出し keyword.
    let (style, rest) = [
        ("同行", AozoraHeadingStyle::SameLine),
        ("窓", AozoraHeadingStyle::Window),
    ]
    .into_iter()
    .find_map(|(prefix, style)| s.strip_prefix(prefix).map(|rest| (style, rest)))
    .unwrap_or((AozoraHeadingStyle::Standard, s));
    let kind = match rest {
        "大見出し" => AozoraHeadingKind::Large,
        "中見出し" => AozoraHeadingKind::Medium,
        "小見出し" => AozoraHeadingKind::Small,
        _ => return None,
    };
    Some((style, kind))
}

/// Recognise a **paired** (`STYLEレベル見出し` / `…見出し終わり`) or **block**
/// (`ここからSTYLEレベル見出し` / `ここでSTYLEレベル見出し終わり`) heading
/// directive body, returning `(container, is_open)`. These delimit their
/// content and route through the container pairing machinery as
/// [`ContainerKind::Heading`] (the counterpart of the `は`-form leaf heading).
///
/// The forward-reference `「X」は…見出し` hint starts with `「`, so it never
/// matches here; a `ここから…` / `…終わり` body that is not a heading keyword
/// (e.g. `ここから2字下げ`) fails `parse_heading_keyword` and falls through to
/// the body dispatcher.
fn parse_heading_directive(body: &str) -> Option<(ContainerKind, bool)> {
    if let Some(rest) = body.strip_prefix("ここから") {
        let (style, kind) = parse_heading_keyword(rest)?;
        return Some((
            ContainerKind::Heading {
                kind,
                style,
                block: true,
            },
            true,
        ));
    }
    if let Some(rest) = body.strip_prefix("ここで") {
        let (style, kind) = parse_heading_keyword(rest.strip_suffix("終わり")?)?;
        return Some((
            ContainerKind::Heading {
                kind,
                style,
                block: true,
            },
            false,
        ));
    }
    if let Some(inner) = body.strip_suffix("終わり") {
        let (style, kind) = parse_heading_keyword(inner)?;
        return Some((
            ContainerKind::Heading {
                kind,
                style,
                block: false,
            },
            false,
        ));
    }
    let (style, kind) = parse_heading_keyword(body)?;
    Some((
        ContainerKind::Heading {
            kind,
            style,
            block: false,
        },
        true,
    ))
}

/// Map the keyword after `は` to an [`EmphasisKind`].
///
/// 太字 → Bold, 斜体 → Italic (per
/// <https://www.aozora.gr.jp/annotation/emphasis.html>); 上付き小文字 →
/// `SuperScript`, 下付き小文字 → `SubScript`, 行右小書き → `SmallRight`,
/// 行左小書き → `SmallLeft` (per
/// <https://www.aozora.gr.jp/annotation/etc.html>). Unknown suffixes
/// return `None` (→ `Annotation{Unknown}`).
fn emphasis_kind_from_suffix(s: &str) -> Option<EmphasisKind> {
    Some(match s {
        "太字" => EmphasisKind::Bold,
        "斜体" => EmphasisKind::Italic,
        "上付き小文字" => EmphasisKind::SuperScript,
        "下付き小文字" => EmphasisKind::SubScript,
        "行右小書き" => EmphasisKind::SmallRight,
        "行左小書き" => EmphasisKind::SmallLeft,
        _ => return None,
    })
}

/// Map the trailing keyword (after `に`) to a [`BoutenKind`].
///
/// Covers the eleven bouten kinds catalogued at
/// <https://www.aozora.gr.jp/annotation/bouten.html> plus the common
/// emphasis-page variants (`白ゴマ` / `ばつ` / `白三角` / `二重傍線`).
/// Unknown suffixes return `None`, letting the annotation fall through
/// to the `Annotation{Unknown}` catch-all.
///
/// The dispatch is a straight `match` rather than a PHF table: 11
/// entries, each a short literal, lookup cost is dominated by hash
/// overhead either way. The exhaustive test in
/// `bouten_kind_from_suffix_recognises_all_spec_keywords` catches
/// typos before they silence recognition.
fn bouten_kind_from_suffix(s: &str) -> Option<BoutenKind> {
    Some(match s {
        "傍点" => BoutenKind::Goma,
        "白ゴマ傍点" => BoutenKind::WhiteSesame,
        "丸傍点" => BoutenKind::Circle,
        "白丸傍点" => BoutenKind::WhiteCircle,
        "二重丸傍点" => BoutenKind::DoubleCircle,
        "蛇の目傍点" => BoutenKind::Janome,
        "ばつ傍点" => BoutenKind::Cross,
        "白三角傍点" => BoutenKind::WhiteTriangle,
        "波線" => BoutenKind::WavyLine,
        "傍線" => BoutenKind::UnderLine,
        "二重傍線" => BoutenKind::DoubleUnderLine,
        "鎖線" => BoutenKind::ChainLine,
        "破線" => BoutenKind::DashedLine,
        "黒三角傍点" => BoutenKind::BlackTriangle,
        _ => return None,
    })
}

/// Parse a 傍点/傍線 range-form body into `(kind, position, is_close)`.
/// Strips an optional `左に` left-side prefix and an optional `終わり`
/// close suffix; the remainder must be a [`bouten_kind_from_suffix`]
/// keyword. Returns `None` (→ `Annotation{Unknown}`) otherwise — e.g.
/// for `鎖線` / `破線`, which the kind table does not yet cover.
fn parse_bouten_range_body(body: &str) -> Option<(BoutenKind, BoutenPosition, bool)> {
    let (position, rest) = body
        .strip_prefix("左に")
        .map_or((BoutenPosition::Right, body), |r| (BoutenPosition::Left, r));
    let (is_close, kind_str) = rest
        .strip_suffix("終わり")
        .map_or((false, rest), |k| (true, k));
    let kind = bouten_kind_from_suffix(kind_str)?;
    Some((kind, position, is_close))
}

/// Parse a 太字 / 斜体 range / block body into `(kind, block, is_close)`.
/// `block` is `true` for the `ここから…` / `ここで…終わり` block form,
/// `false` for the bare inline range `［＃太字］…［＃太字終わり］`. Returns
/// `None` (→ `Annotation{Unknown}`) for any non-emphasis body.
fn parse_emphasis_body(body: &str) -> Option<(EmphasisKind, bool, bool)> {
    Some(match body {
        "太字" => (EmphasisKind::Bold, false, false),
        "太字終わり" => (EmphasisKind::Bold, false, true),
        "ここから太字" => (EmphasisKind::Bold, true, false),
        "ここで太字終わり" => (EmphasisKind::Bold, true, true),
        "斜体" => (EmphasisKind::Italic, false, false),
        "斜体終わり" => (EmphasisKind::Italic, false, true),
        "ここから斜体" => (EmphasisKind::Italic, true, false),
        "ここで斜体終わり" => (EmphasisKind::Italic, true, true),
        _ => return None,
    })
}

/// Parse a leading run of ASCII / full-width decimal digits into a
/// [`u8`] and return the remainder slice.
///
/// Returns `None` if the leading char is not a digit, or if the value
/// overflows `u8` (> 255). `saturating_mul` / `saturating_add` during
/// accumulation keep the `u32` intermediate bounded, but the final
/// `try_from` enforces the `u8` range — a body like `300字下げ` fails
/// cleanly rather than wrapping to 44.
fn parse_decimal_u8_prefix(s: &str) -> Option<(u8, &str)> {
    let mut value: u32 = 0;
    let mut consumed = 0;
    for (idx, ch) in s.char_indices() {
        let digit = match ch {
            '0'..='9' => Some(u32::from(ch) - u32::from('0')),
            '０'..='９' => Some(u32::from(ch) - u32::from('０')),
            _ => None,
        };
        let Some(d) = digit else { break };
        value = value.saturating_mul(10).saturating_add(d);
        consumed = idx + ch.len_utf8();
    }
    if consumed == 0 {
        return None;
    }
    let value_u8 = u8::try_from(value).ok()?;
    Some((value_u8, &s[consumed..]))
}

/// Characters eligible as an implicit-ruby base. Covers:
///
/// * CJK Unified Ideographs (main block + Extension A)
/// * CJK Compatibility Ideographs
/// * CJK Unified Ideographs Extension B..F (supplementary plane)
/// * `々` (U+3005) ideographic iteration mark — usually kanji-like
/// * `〆` (U+3006) ideographic closing mark — sometimes used as kanji
const fn is_ruby_base_char(ch: char) -> bool {
    matches!(
        ch,
        '\u{3400}'..='\u{4DBF}'
        | '\u{4E00}'..='\u{9FFF}'
        | '\u{F900}'..='\u{FAFF}'
        | '\u{20000}'..='\u{2FFFF}'
        | '々'
        | '〆'
    )
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;
    // Borrowed-AST types pattern-matched throughout. `AozoraNode<'a>`
    // is `Copy` and holds payloads via `&'a Ruby<'a>` etc., so tests
    // pattern-match `AozoraNode::Ruby(r)` where `r` is already a
    // reference — no `Box` deref needed.
    #[allow(
        unused_imports,
        reason = "individual tests pattern-match on subsets; bringing them all in keeps the import block stable"
    )]
    use aozora_syntax::borrowed::{
        AngleQuote, Annotation, AozoraNode, Arena, Bouten, Content, Gaiji, HeadingHint, Kaeriten,
        Ruby, Sashie, Segment, TateChuYoko,
    };

    use crate::lexer::phase1_events::tokenize;
    use crate::lexer::phase2_pair::pair;

    /// Test-only materialised classify output: collects `spans` from
    /// the streaming iterator and merges its post-exhaustion
    /// diagnostics with the upstream pair stream's diagnostics.
    /// Per-test convenience shape so tests can assert on the full
    /// pipeline result without rebuilding the collection inline at
    /// every site.
    #[derive(Debug)]
    struct TestClassifyOutput<'a> {
        spans: Vec<ClassifiedSpan<'a>>,
        diagnostics: Vec<Diagnostic>,
    }

    /// Test-only `run` macro. Materialises a fresh
    /// [`Arena`] / [`BorrowedAllocator`] pair in the calling scope and
    /// binds `out` (or the explicitly-named identifier) to a
    /// [`TestClassifyOutput`]. Replaces the legacy
    /// `let out = run(src)` shape so each test's borrow chain is
    /// arena-rooted in the test's own stack frame, with no per-test
    /// allocator boilerplate.
    macro_rules! run {
        ($name:ident, $src:expr) => {
            let arena = Arena::new();
            let mut alloc = BorrowedAllocator::new(&arena);
            let mut pair_stream = pair(tokenize($src));
            let mut spans: Vec<ClassifiedSpan<'_>> = Vec::new();
            let classify_diagnostics: Vec<Diagnostic> = {
                let mut stream = classify(&mut pair_stream, $src, &mut alloc);
                for span in &mut stream {
                    spans.push(span);
                }
                stream.take_diagnostics()
            };
            let mut diagnostics = pair_stream.take_diagnostics();
            diagnostics.extend(classify_diagnostics);
            let $name = TestClassifyOutput { spans, diagnostics };
        };
    }

    /// Test-only helper: extract the `Aozora` variant's borrowed
    /// `AozoraNode<'a>` (which is `Copy`) so tests can pattern-match
    /// on it without spelling out the variant boilerplate at every
    /// call site.
    fn aozora_node<'a>(span: &ClassifiedSpan<'a>) -> Option<AozoraNode<'a>> {
        match span.kind {
            SpanKind::Aozora(node) => Some(node),
            _ => None,
        }
    }

    #[test]
    fn empty_input_produces_empty_span_vector() {
        run!(out, "");
        assert!(out.spans.is_empty());
        assert!(out.diagnostics.is_empty());
    }

    #[test]
    fn plain_ascii_becomes_single_plain_span() {
        run!(out, "hello");
        assert_eq!(out.spans.len(), 1);
        assert_eq!(out.spans[0].kind, SpanKind::Plain);
        assert_eq!(out.spans[0].source_span, Span::new(0, 5));
    }

    #[test]
    fn plain_multibyte_becomes_single_plain_span() {
        let src = "こんにちは";
        run!(out, src);
        assert_eq!(out.spans.len(), 1);
        assert_eq!(out.spans[0].kind, SpanKind::Plain);
        assert_eq!(
            out.spans[0].source_span,
            Span::new(0, u32::try_from(src.len()).expect("fits"))
        );
    }

    #[test]
    fn newline_in_middle_splits_into_three_spans() {
        run!(out, "line1\nline2");
        assert_eq!(out.spans.len(), 3);
        assert_eq!(out.spans[0].kind, SpanKind::Plain);
        assert_eq!(out.spans[0].source_span, Span::new(0, 5));
        assert_eq!(out.spans[1].kind, SpanKind::Newline);
        assert_eq!(out.spans[1].source_span, Span::new(5, 6));
        assert_eq!(out.spans[2].kind, SpanKind::Plain);
        assert_eq!(out.spans[2].source_span, Span::new(6, 11));
    }

    #[test]
    fn leading_and_trailing_newlines_do_not_emit_empty_plain_spans() {
        run!(out, "\nbody\n");
        // Expected: Newline, Plain("body"), Newline. No empty Plain at the edges.
        assert_eq!(out.spans.len(), 3);
        assert_eq!(out.spans[0].kind, SpanKind::Newline);
        assert_eq!(out.spans[1].kind, SpanKind::Plain);
        assert_eq!(out.spans[2].kind, SpanKind::Newline);
    }

    #[test]
    fn explicit_ruby_produces_single_aozora_span() {
        let src = "｜青梅《おうめ》";
        run!(out, src);
        assert_eq!(out.spans.len(), 1);
        let SpanKind::Aozora(node) = out.spans[0].kind else {
            panic!("expected Aozora span, got {:?}", out.spans[0].kind);
        };
        let AozoraNode::Ruby(ruby) = node else {
            panic!("expected Ruby variant, got {node:?}");
        };
        assert_eq!(ruby.base.as_plain(), Some("青梅"));
        assert_eq!(ruby.reading.as_plain(), Some("おうめ"));
        assert!(ruby.delim_explicit);
        assert_eq!(out.spans[0].source_span.end as usize, src.len());
    }

    #[test]
    fn implicit_ruby_consumes_trailing_kanji_only() {
        // "あいう" (kana) + "漢字" (kanji) + ruby → base is "漢字",
        // leading kana stays Plain.
        let src = "あいう漢字《かんじ》";
        run!(out, src);
        assert_eq!(out.spans.len(), 2);
        assert_eq!(out.spans[0].kind, SpanKind::Plain);
        let SpanKind::Aozora(node) = out.spans[1].kind else {
            panic!("expected Aozora span, got {:?}", out.spans[1].kind);
        };
        let AozoraNode::Ruby(ruby) = node else {
            panic!("expected Ruby variant, got {node:?}");
        };
        assert_eq!(ruby.base.as_plain(), Some("漢字"));
        assert_eq!(ruby.reading.as_plain(), Some("かんじ"));
        assert!(!ruby.delim_explicit);
        // Plain covers "あいう"; ruby covers "漢字《かんじ》".
        assert_eq!(out.spans[0].source_span.slice(src), "あいう");
    }

    #[test]
    fn implicit_ruby_without_leading_kanji_leaves_ruby_unrecognized() {
        // No kanji before 《 → ruby can't bind. Ruby remains plain.
        let src = "あいう《かんじ》";
        run!(out, src);
        assert!(
            !out.spans
                .iter()
                .any(|s| matches!(s.kind, SpanKind::Aozora(_))),
            "expected no Aozora spans, got {:?}",
            out.spans
        );
    }

    #[test]
    fn explicit_ruby_with_empty_reading_is_not_recognized() {
        let src = "｜漢字《》";
        run!(out, src);
        // Empty reading fails recognition; whole source stays plain.
        assert!(
            !out.spans
                .iter()
                .any(|s| matches!(s.kind, SpanKind::Aozora(_))),
            "expected no Aozora spans, got {:?}",
            out.spans
        );
    }

    #[test]
    fn ruby_after_newline_keeps_newline_as_its_own_span() {
        let src = "line1\n｜漢《かん》";
        run!(out, src);
        // Plain("line1"), Newline, Aozora(Ruby)
        assert_eq!(out.spans.len(), 3);
        assert_eq!(out.spans[0].kind, SpanKind::Plain);
        assert_eq!(out.spans[1].kind, SpanKind::Newline);
        let is_ruby = matches!(out.spans[2].kind, SpanKind::Aozora(AozoraNode::Ruby(_)));
        assert!(
            is_ruby,
            "expected Aozora(Ruby), got {:?}",
            out.spans[2].kind
        );
    }

    #[test]
    fn implicit_ruby_after_non_text_event_is_not_recognized() {
        // A close-bracket between `」` and `《` means the preceding
        // event is PairClose, not Text. Implicit ruby can't bind.
        let src = "「台詞」《かんじ》";
        run!(out, src);
        assert!(
            !out.spans
                .iter()
                .any(|s| matches!(s.kind, SpanKind::Aozora(_))),
            "expected no Aozora spans, got {:?}",
            out.spans
        );
    }

    // ---------------------------------------------------------------
    // Ruby reading Content::Segments — nested gaiji / annotation
    // inside the `《reading》` body.
    // ---------------------------------------------------------------

    /// Pull the sole `SpanKind::Aozora(Ruby(...))` out of a
    /// [`ClassifyOutput`] so tests can assert on the Ruby payload
    /// without repeating the shape-match boilerplate.
    fn only_ruby<'a>(out: &TestClassifyOutput<'a>) -> &'a Ruby<'a> {
        let mut found = None;
        for span in &out.spans {
            if let SpanKind::Aozora(AozoraNode::Ruby(r)) = span.kind {
                assert!(found.is_none(), "more than one Ruby span: {:?}", out.spans);
                found = Some(r);
            }
        }
        found.unwrap_or_else(|| panic!("no Ruby span in {:?}", out.spans))
    }

    #[test]
    fn ruby_plain_reading_still_collapses_to_plain_content() {
        // The Segments lift must not regress the plain-text ruby case:
        // when the body holds only text, `Content::from_segments` is
        // obliged to collapse back to `Content::Plain` so `.as_plain()`
        // returns `Some(&str)` for downstream consumers (renderer fast
        // path, property tests that assert the textual shape).
        run!(out, "｜青梅《おうめ》");
        let r = only_ruby(&out);
        assert_eq!(r.base.as_plain(), Some("青梅"));
        assert_eq!(r.reading.as_plain(), Some("おうめ"));
    }

    #[test]
    fn ruby_reading_with_embedded_gaiji_produces_segments() {
        // `※［＃「ほ」、第3水準1-85-54］` inside the reading must fold
        // into a `Segment::Gaiji` between Text segments so the renderer
        // can wrap it in `<span class="aozora-gaiji">` without leaking the
        // bare `［＃` marker (Tier A).
        run!(out, "｜日本《に※［＃「ほ」、第3水準1-85-54］ん》");
        let r = only_ruby(&out);
        assert_eq!(r.base.as_plain(), Some("日本"));
        let Content::Segments(segs) = r.reading.get() else {
            panic!("expected Segments, got {:?}", r.reading);
        };
        assert_eq!(segs.len(), 3);
        assert!(
            matches!(&segs[0], Segment::Text(t) if &**t == "に"),
            "segment 0: {:?}",
            segs[0]
        );
        let Segment::Gaiji(g) = segs[1] else {
            panic!("segment 1 should be Gaiji, got {:?}", segs[1]);
        };
        assert_eq!(g.description, "ほ");
        assert_eq!(g.mencode, Some("第3水準1-85-54"));
        assert!(
            matches!(&segs[2], Segment::Text(t) if &**t == "ん"),
            "segment 2: {:?}",
            segs[2]
        );
    }

    #[test]
    fn ruby_reading_wholly_gaiji_produces_single_gaiji_segment() {
        // No surrounding text; the reading is exactly one gaiji
        // marker. The Segments run must be a single Gaiji (not a
        // trailing empty Text on either side).
        run!(out, "｜日本《※［＃「にほん」、第3水準1-85-54］》");
        let r = only_ruby(&out);
        let Content::Segments(segs) = r.reading.get() else {
            panic!("expected Segments, got {:?}", r.reading);
        };
        assert_eq!(segs.len(), 1);
        let Segment::Gaiji(g) = segs[0] else {
            panic!("expected Gaiji, got {:?}", segs[0]);
        };
        assert_eq!(g.description, "にほん");
    }

    #[test]
    fn ruby_reading_with_trailing_annotation_produces_annotation_segment() {
        // `［＃ママ］` inside a reading indicates editorial "sic" —
        // must fold as `Segment::Annotation` so the renderer wraps it
        // in the hidden `aozora-annotation` span (Tier A compliance).
        run!(out, "｜日本《にほん［＃ママ］》");
        let r = only_ruby(&out);
        let Content::Segments(segs) = r.reading.get() else {
            panic!("expected Segments, got {:?}", r.reading);
        };
        assert_eq!(segs.len(), 2);
        assert!(
            matches!(&segs[0], Segment::Text(t) if &**t == "にほん"),
            "segment 0: {:?}",
            segs[0]
        );
        let Segment::Annotation(a) = segs[1] else {
            panic!("segment 1 should be Annotation, got {:?}", segs[1]);
        };
        assert_eq!(a.raw.as_str(), "［＃ママ］");
    }

    #[test]
    fn ruby_reading_with_gaiji_and_annotation_interleaved() {
        // Exercises the general Segments shape: Text, Gaiji, Text,
        // Annotation. Proves the flusher preserves ordering and the
        // `text_start` advancement correctly spans each gap.
        run!(out, "｜日本《に※［＃「ほ」、第3水準1-85-54］ん［＃ママ］》");
        let r = only_ruby(&out);
        let Content::Segments(segs) = r.reading.get() else {
            panic!("expected Segments, got {:?}", r.reading);
        };
        assert_eq!(segs.len(), 4);
        assert!(matches!(&segs[0], Segment::Text(t) if &**t == "に"));
        assert!(matches!(&segs[1], Segment::Gaiji(_)));
        assert!(matches!(&segs[2], Segment::Text(t) if &**t == "ん"));
        assert!(matches!(&segs[3], Segment::Annotation(_)));
    }

    #[test]
    fn implicit_ruby_reading_with_embedded_gaiji_also_produces_segments() {
        // Implicit form must use the same body walker; only the base
        // extraction differs (trailing-kanji run instead of explicit
        // `｜`-delimited Text event).
        run!(out, "日本《に※［＃「ほ」、第3水準1-85-54］ん》");
        let r = only_ruby(&out);
        assert_eq!(r.base.as_plain(), Some("日本"));
        assert!(!r.delim_explicit);
        let Content::Segments(segs) = r.reading.get() else {
            panic!("expected Segments, got {:?}", r.reading);
        };
        assert_eq!(segs.len(), 3);
        assert!(matches!(&segs[0], Segment::Text(t) if &**t == "に"));
        assert!(matches!(&segs[1], Segment::Gaiji(_)));
        assert!(matches!(&segs[2], Segment::Text(t) if &**t == "ん"));
    }

    #[test]
    fn ruby_reading_consume_span_still_covers_outer_source_bytes() {
        // The Segments lift must not disturb the outer `source_span`
        // of the classified span: Phase 4 still needs to replace the
        // full `｜…《…》` bytes with a single PUA sentinel, and the
        // inner gaiji/annotation source bytes are folded into the
        // Ruby payload — not re-exposed to the outer classifier.
        let src = "｜日本《に※［＃「ほ」、第3水準1-85-54］ん》";
        run!(out, src);
        let aozora_spans: Vec<_> = out
            .spans
            .iter()
            .filter(|s| matches!(s.kind, SpanKind::Aozora(_)))
            .collect();
        assert_eq!(
            aozora_spans.len(),
            1,
            "nested gaiji must stay inside the Ruby payload, not leak into a \
             sibling span at the top level: {:?}",
            out.spans
        );
        assert_eq!(
            aozora_spans[0].source_span.end as usize,
            src.len(),
            "ruby span must cover through the final `》`"
        );
        assert_eq!(aozora_spans[0].source_span.start, 0);
    }

    #[test]
    fn ruby_reading_preserves_tier_a_even_for_nested_block_leaf() {
        // `［＃改ページ］` inside a ruby reading is nonsensical, but
        // real corpora have been known to carry freak shapes. The
        // non-Annotation emit path in `build_content_from_body` must
        // downgrade such shapes into `Annotation{Unknown}` so the
        // bare `［＃` never reaches the rendered HTML through a
        // `Segment::Text` channel (Tier A canary).
        run!(out, "｜日本《にほん［＃改ページ］》");
        let r = only_ruby(&out);
        let Content::Segments(segs) = r.reading.get() else {
            panic!("expected Segments, got {:?}", r.reading);
        };
        // Last segment must be an Annotation carrying the raw bytes.
        let last = segs.last().expect("non-empty segments");
        let Segment::Annotation(a) = last else {
            panic!("final segment should be Annotation, got {last:?}");
        };
        assert_eq!(a.raw.as_str(), "［＃改ページ］");
        assert_eq!(a.kind, AnnotationKind::Unknown);
    }

    #[test]
    fn page_break_annotation_becomes_single_page_break_span() {
        let src = "前\n［＃改ページ］\n後";
        run!(out, src);
        // Plain("前"), Newline, Aozora(PageBreak), Newline, Plain("後")
        assert_eq!(out.spans.len(), 5);
        assert_eq!(out.spans[0].kind, SpanKind::Plain);
        assert_eq!(out.spans[1].kind, SpanKind::Newline);
        assert!(matches!(
            aozora_node(&out.spans[2]),
            Some(AozoraNode::PageBreak)
        ));
        assert_eq!(out.spans[2].source_span.slice(src), "［＃改ページ］");
        assert_eq!(out.spans[3].kind, SpanKind::Newline);
        assert_eq!(out.spans[4].kind, SpanKind::Plain);
    }

    #[test]
    fn section_break_kaicho_recognized() {
        run!(out, "［＃改丁］");
        assert_eq!(out.spans.len(), 1);
        assert!(matches!(
            aozora_node(&out.spans[0]),
            Some(AozoraNode::SectionBreak(SectionKind::Kaicho))
        ));
    }

    #[test]
    fn section_break_dan_recognized() {
        run!(out, "［＃改段］");
        assert_eq!(out.spans.len(), 1);
        assert!(matches!(
            aozora_node(&out.spans[0]),
            Some(AozoraNode::SectionBreak(SectionKind::Kaidan))
        ));
    }

    #[test]
    fn section_break_spread_recognized() {
        run!(out, "［＃改見開き］");
        assert_eq!(out.spans.len(), 1);
        assert!(matches!(
            aozora_node(&out.spans[0]),
            Some(AozoraNode::SectionBreak(SectionKind::Kaimihiraki))
        ));
    }

    #[test]
    fn bracket_without_hash_is_not_an_annotation() {
        // `［普通］` (no `＃`) is plain literal text, not an annotation.
        run!(out, "［普通］");
        assert!(
            !out.spans
                .iter()
                .any(|s| matches!(s.kind, SpanKind::Aozora(_))),
            "expected no Aozora spans, got {:?}",
            out.spans
        );
    }

    #[test]
    fn unknown_annotation_keyword_is_promoted_to_annotation_unknown() {
        // The lexer claims every well-formed `［＃…］`: if no specialised
        // recogniser matches, the `Annotation{Unknown}` fallback wraps
        // the raw source so the renderer can emit an `aozora-annotation`
        // hidden span instead of leaking the brackets as plain text.
        run!(out, "［＃未知のキーワード］");
        let ann = out
            .spans
            .iter()
            .find_map(|s| match aozora_node(s) {
                Some(AozoraNode::Annotation(a)) => Some(a),
                _ => None,
            })
            .expect("unknown keyword must promote to Annotation{Unknown}");
        assert_eq!(ann.kind, AnnotationKind::Unknown);
        assert_eq!(ann.raw.as_str(), "［＃未知のキーワード］");
    }

    #[test]
    fn annotation_with_whitespace_padding_still_matches() {
        // Corpus occasionally has `［＃ 改ページ ］` with spaces. We
        // trim the body to be lenient.
        run!(out, "［＃ 改ページ ］");
        assert_eq!(out.spans.len(), 1);
        assert!(matches!(
            aozora_node(&out.spans[0]),
            Some(AozoraNode::PageBreak)
        ));
    }

    #[test]
    fn empty_bracket_with_hash_is_wrapped_as_annotation_unknown() {
        // Real Aozora corpora use `［＃］` as an illustrative glyph
        // inside explanatory prose (e.g. "［＃］：入力者注…"). The
        // Tier-A canary (no bare `［＃` in HTML output) requires that
        // the bracket not leak even for empty-body forms, so the
        // catch-all fallback wraps it as Annotation{Unknown} with the
        // raw `［＃］` bytes preserved for round-trip.
        run!(out, "［＃］");
        let ann = out
            .spans
            .iter()
            .find_map(|s| match aozora_node(s) {
                Some(AozoraNode::Annotation(a)) => Some(a),
                _ => None,
            })
            .expect("empty body must still wrap as Annotation{Unknown}");
        assert_eq!(ann.kind, AnnotationKind::Unknown);
        assert_eq!(ann.raw.as_str(), "［＃］");
    }

    #[test]
    fn indent_with_full_width_digit() {
        run!(out, "［＃２字下げ］");
        assert_eq!(out.spans.len(), 1);
        assert!(matches!(
            aozora_node(&out.spans[0]),
            Some(AozoraNode::Indent(Indent { amount: 2 }))
        ));
    }

    #[test]
    fn indent_with_ascii_digit() {
        run!(out, "［＃10字下げ］");
        assert_eq!(out.spans.len(), 1);
        assert!(matches!(
            aozora_node(&out.spans[0]),
            Some(AozoraNode::Indent(Indent { amount: 10 }))
        ));
    }

    #[test]
    fn indent_overflow_falls_back_to_annotation_unknown() {
        // 300 > 255, doesn't fit in u8 — the `N字下げ` recogniser
        // declines. The `Annotation { Unknown }` catch-all then
        // claims the bracket so the renderer wraps the body in an
        // aozora-annotation span instead of leaking raw brackets.
        run!(out, "［＃300字下げ］");
        let ann = out
            .spans
            .iter()
            .find_map(|s| match aozora_node(s) {
                Some(AozoraNode::Annotation(a)) => Some(a),
                _ => None,
            })
            .expect("overflow should fall back to Annotation{Unknown}");
        assert_eq!(ann.kind, AnnotationKind::Unknown);
        assert_eq!(ann.raw.as_str(), "［＃300字下げ］");
        // The specialised Indent recogniser MUST NOT claim it.
        assert!(
            !out.spans
                .iter()
                .any(|s| matches!(aozora_node(s), Some(AozoraNode::Indent(_)))),
        );
    }

    #[test]
    fn indent_zero_digit_falls_through() {
        // N=0 is meaningless for 字下げ (a zero-width indent is not
        // a thing). Fullwidth-digit variant.
        run!(out, "［＃０字下げ］");
        assert!(
            !out.spans
                .iter()
                .any(|s| matches!(aozora_node(s), Some(AozoraNode::Indent(_)))),
        );
    }

    #[test]
    fn indent_zero_ascii_digit_falls_through() {
        // ASCII-digit variant of the N=0 reject.
        run!(out, "［＃0字下げ］");
        assert!(
            !out.spans
                .iter()
                .any(|s| matches!(aozora_node(s), Some(AozoraNode::Indent(_)))),
        );
    }

    #[test]
    fn align_end_zero_digit_falls_through() {
        // 地から0字上げ is redundant with 地付き and not spec-sanctioned —
        // reject so the text falls through to a generic Annotation.
        run!(out, "［＃地から0字上げ］");
        assert!(
            !out.spans
                .iter()
                .any(|s| matches!(aozora_node(s), Some(AozoraNode::AlignEnd(_)))),
        );
    }

    #[test]
    fn chitsuki_zero_offset_recognized() {
        run!(out, "［＃地付き］");
        assert_eq!(out.spans.len(), 1);
        assert!(matches!(
            aozora_node(&out.spans[0]),
            Some(AozoraNode::AlignEnd(AlignEnd { offset: 0 }))
        ));
    }

    #[test]
    fn chi_kara_n_ji_age_recognized() {
        run!(out, "［＃地から３字上げ］");
        assert_eq!(out.spans.len(), 1);
        assert!(matches!(
            aozora_node(&out.spans[0]),
            Some(AozoraNode::AlignEnd(AlignEnd { offset: 3 }))
        ));
    }

    #[test]
    fn indent_without_digits_falls_through() {
        // "ここから字下げ" is a paired-container opener, not a leaf
        // indent — the leaf classifier must not grab it, and the
        // paired-container recogniser claims it instead.
        run!(out, "［＃ここから字下げ］");
        assert!(
            !out.spans
                .iter()
                .any(|s| matches!(aozora_node(s), Some(AozoraNode::Indent(_)))),
        );
    }

    #[test]
    fn forward_bouten_goma_recognized() {
        // Preceding text "前置き" plus "青空" before the bracket — the
        // target literal must appear in the preceding source for the
        // forward-reference classifier to promote.
        run!(out, "前置きの青空［＃「青空」に傍点］後ろ");
        let bouten = out
            .spans
            .iter()
            .find_map(|s| match aozora_node(s) {
                Some(AozoraNode::Bouten(b)) => Some(b),
                _ => None,
            })
            .expect("expected a Bouten span");
        assert_eq!(bouten.kind, BoutenKind::Goma);
        assert_eq!(bouten.target.as_plain(), Some("青空"));
    }

    #[test]
    fn forward_emphasis_script_and_koshogaki_recognized() {
        // 上付き/下付き小文字 (super/subscript) and 行右/行左小書き — the
        // four emphasis-page forward-reference families beyond 太字/斜体
        // (per <https://www.aozora.gr.jp/annotation/etc.html>). Each is a
        // first-class `Emphasis` leaf, NOT an `Annotation{Unknown}`.
        for (src, want) in [
            ("x２［＃「２」は上付き小文字］", EmphasisKind::SuperScript),
            ("H２［＃「２」は下付き小文字］", EmphasisKind::SubScript),
            ("あ［＃「あ」は行右小書き］", EmphasisKind::SmallRight),
            ("い［＃「い」は行左小書き］", EmphasisKind::SmallLeft),
        ] {
            run!(out, src);
            let emphasis = out
                .spans
                .iter()
                .find_map(|s| match aozora_node(s) {
                    Some(AozoraNode::Emphasis(e)) => Some(e),
                    _ => None,
                })
                .unwrap_or_else(|| panic!("expected an Emphasis span for {src:?}"));
            assert_eq!(emphasis.kind, want, "src = {src:?}");
        }
    }

    #[test]
    fn forward_bouten_consumes_immediate_predecessor_literal() {
        // The fix for forward-ref text duplication: when the target
        // literal sits *immediately* before the `［`, the classifier
        // hands back a `consume_start` that pulls the bouten span back
        // to swallow it, so the preceding plain run flushes only up to
        // the literal's start. Without this, the renderer would emit
        // both the raw literal and the bouten's own content, producing
        // `<text>青空<em>...青空</em>` — the playground welcome-page bug.
        //
        // Concretely we expect two spans:
        //   [0] Plain("前置きの")     — the prefix up to (but not
        //                               including) the consumed literal
        //   [1] Aozora(Bouten { … })  — covers "青空［＃「青空」に傍点］"
        //   [2] Plain("後ろ")         — the trailing prefix
        run!(out, "前置きの青空［＃「青空」に傍点］後ろ");
        let classified: Vec<_> = out
            .spans
            .iter()
            .map(|s| {
                let kind_str = match s.kind {
                    SpanKind::Plain => "Plain",
                    SpanKind::Aozora(AozoraNode::Bouten(_)) => "Bouten",
                    _ => "Other",
                };
                let slice = &"前置きの青空［＃「青空」に傍点］後ろ"
                    [s.source_span.start as usize..s.source_span.end as usize];
                (kind_str, slice)
            })
            .collect();
        // The legacy bug-state would have produced ["Plain('前置きの青空')",
        // "Bouten('［＃「青空」に傍点］')", "Plain('後ろ')"]. The fix
        // shrinks the Plain prefix and grows the Bouten span backwards.
        assert_eq!(
            classified.as_slice(),
            &[
                ("Plain", "前置きの"),
                ("Bouten", "青空［＃「青空」に傍点］"),
                ("Plain", "後ろ"),
            ],
            "spans drift from the consume_start=literal_start contract",
        );
        // The classifier must also flip the per-node consumed flag so the
        // serializer round-trips the literal back into place.
        let bouten_flag = out.spans.iter().find_map(|s| match s.kind {
            SpanKind::Aozora(AozoraNode::Bouten(b)) => Some(b.consumed_predecessor),
            _ => None,
        });
        assert_eq!(
            bouten_flag,
            Some(true),
            "consume_start shrunk → consumed_predecessor must be true",
        );
    }

    #[test]
    fn forward_bouten_with_intervening_text_keeps_legacy_consume() {
        // Edge case: target appears earlier in the paragraph but NOT
        // immediately before the bracket. We deliberately leave the
        // legacy duplicating behaviour in that case rather than
        // splice a hole into the middle of the pending plain run
        // (the `flush_plain_up_to` API is truncate-only). The Bouten
        // is still emitted with its own target content.
        run!(out, "青空の下を歩く［＃「青空」に傍点］");
        let mut saw_bouten = false;
        let mut saw_plain_with_aozora = false;
        for s in &out.spans {
            match &s.kind {
                SpanKind::Aozora(AozoraNode::Bouten(_)) => saw_bouten = true,
                SpanKind::Plain => {
                    let slice = &"青空の下を歩く［＃「青空」に傍点］"
                        [s.source_span.start as usize..s.source_span.end as usize];
                    if slice.contains("青空") {
                        saw_plain_with_aozora = true;
                    }
                }
                _ => {}
            }
        }
        assert!(
            saw_bouten,
            "Bouten must still promote with non-adjacent target"
        );
        assert!(
            saw_plain_with_aozora,
            "non-adjacent target stays in preceding Plain (legacy behaviour preserved)"
        );
    }

    #[test]
    fn forward_bouten_circle_recognized() {
        run!(out, "X［＃「X」に丸傍点］");
        let bouten = out
            .spans
            .iter()
            .find_map(|s| match aozora_node(s) {
                Some(AozoraNode::Bouten(b)) => Some(b),
                _ => None,
            })
            .expect("expected a Bouten span");
        assert_eq!(bouten.kind, BoutenKind::Circle);
        assert_eq!(bouten.target.as_plain(), Some("X"));
    }

    #[test]
    fn forward_bouten_all_eleven_kinds() {
        // All eleven bouten kinds — the seven core shapes plus
        // 白ゴマ / ばつ / 白三角 / 二重傍線. Each suffix must promote
        // the bracket into a `Bouten` node rather than fall through
        // to `Annotation{Unknown}`, lowering the sweep leak rate.
        let cases = [
            ("傍点", BoutenKind::Goma),
            ("白ゴマ傍点", BoutenKind::WhiteSesame),
            ("丸傍点", BoutenKind::Circle),
            ("白丸傍点", BoutenKind::WhiteCircle),
            ("二重丸傍点", BoutenKind::DoubleCircle),
            ("蛇の目傍点", BoutenKind::Janome),
            ("ばつ傍点", BoutenKind::Cross),
            ("白三角傍点", BoutenKind::WhiteTriangle),
            ("波線", BoutenKind::WavyLine),
            ("傍線", BoutenKind::UnderLine),
            ("二重傍線", BoutenKind::DoubleUnderLine),
        ];
        for (suffix, expected_kind) in cases {
            let src = format!("t［＃「t」に{suffix}］");
            run!(out, &src);
            let Some(b) = out.spans.iter().find_map(|s| match aozora_node(s) {
                Some(AozoraNode::Bouten(b)) => Some(b),
                _ => None,
            }) else {
                panic!("no Bouten span for suffix {suffix:?}");
            };
            assert_eq!(b.kind, expected_kind, "suffix {suffix:?}");
            // All default `に` shapes produce right-side position.
            assert_eq!(b.position, BoutenPosition::Right, "suffix {suffix:?}");
        }
    }

    #[test]
    fn forward_bouten_left_side_flips_position() {
        // `の左に傍点` sets BoutenPosition::Left. The same forward-
        // reference validation (target appears in preceding text) still
        // applies so we prepend a matching target.
        run!(out, "X［＃「X」の左に傍点］");
        let b = out
            .spans
            .iter()
            .find_map(|s| match aozora_node(s) {
                Some(AozoraNode::Bouten(b)) => Some(b),
                _ => None,
            })
            .expect("Bouten expected");
        assert_eq!(b.kind, BoutenKind::Goma);
        assert_eq!(b.position, BoutenPosition::Left);
        assert_eq!(b.target.as_plain(), Some("X"));
    }

    #[test]
    fn forward_bouten_left_side_pairs_with_every_kind() {
        // 左 + every kind must work (same suffix grammar).
        let cases = [
            ("傍点", BoutenKind::Goma),
            ("白ゴマ傍点", BoutenKind::WhiteSesame),
            ("丸傍点", BoutenKind::Circle),
            ("二重傍線", BoutenKind::DoubleUnderLine),
            ("傍線", BoutenKind::UnderLine),
        ];
        for (suffix, expected_kind) in cases {
            let src = format!("t［＃「t」の左に{suffix}］");
            run!(out, &src);
            let Some(b) = out.spans.iter().find_map(|s| match aozora_node(s) {
                Some(AozoraNode::Bouten(b)) => Some(b),
                _ => None,
            }) else {
                panic!("no Bouten span for left-side suffix {suffix:?}");
            };
            assert_eq!(b.kind, expected_kind);
            assert_eq!(b.position, BoutenPosition::Left);
        }
    }

    #[test]
    fn forward_bouten_multi_quote_concatenates_targets() {
        // `［＃「A」「B」に傍点］` walks consecutive PairOpen(Quote)
        // events after the `＃` and folds their bodies into a single
        // Bouten target joined with `、`. Both A and B must appear in
        // the preceding text for the classifier to promote — this
        // keeps the forward-reference semantic intact.
        run!(out, "AとB［＃「A」「B」に傍点］");
        let b = out
            .spans
            .iter()
            .find_map(|s| match aozora_node(s) {
                Some(AozoraNode::Bouten(b)) => Some(b),
                _ => None,
            })
            .expect("multi-quote Bouten expected");
        assert_eq!(b.kind, BoutenKind::Goma);
        // Targets collapse to `A、B` through `Content::from_segments`
        // (all-Text segments → `Plain`).
        assert_eq!(b.target.as_plain(), Some("A、B"));
    }

    #[test]
    fn forward_bouten_multi_quote_without_all_targets_preceded_falls_through() {
        // Only "A" appears before the bracket; "B" does not. The
        // classifier refuses to promote — the bracket is consumed as
        // `Annotation{Unknown}` by the catch-all instead, preserving
        // Tier-A without inventing a bouten target.
        run!(out, "A［＃「A」「B」に傍点］");
        assert!(
            !out.spans
                .iter()
                .any(|s| matches!(aozora_node(s), Some(AozoraNode::Bouten(_)))),
            "Bouten must not promote when any target is unreferenced"
        );
    }

    #[test]
    fn forward_bouten_empty_inner_quotes_are_skipped() {
        // `「」` placeholders in the middle of a multi-quote body do
        // not contribute to the target list. This guards against
        // corpus stragglers like `［＃「A」「」「B」に傍点］`.
        run!(out, "AB［＃「A」「」「B」に傍点］");
        let b = out
            .spans
            .iter()
            .find_map(|s| match aozora_node(s) {
                Some(AozoraNode::Bouten(b)) => Some(b),
                _ => None,
            })
            .expect("Bouten expected");
        assert_eq!(b.target.as_plain(), Some("A、B"));
    }

    #[test]
    fn forward_bouten_position_slug_and_segments_render_together() {
        // Regression: the position modifier must be propagated even
        // when the target is a Segments (multi-quote) value.
        run!(out, "AB［＃「A」「B」の左に傍点］");
        let b = out
            .spans
            .iter()
            .find_map(|s| match aozora_node(s) {
                Some(AozoraNode::Bouten(b)) => Some(b),
                _ => None,
            })
            .expect("Bouten expected");
        assert_eq!(b.position, BoutenPosition::Left);
        assert_eq!(b.target.as_plain(), Some("A、B"));
    }

    #[test]
    fn forward_bouten_empty_target_falls_through() {
        run!(out, "［＃「」に傍点］");
        assert!(
            !out.spans
                .iter()
                .any(|s| matches!(aozora_node(s), Some(AozoraNode::Bouten(_)))),
        );
    }

    #[test]
    fn forward_bouten_unknown_suffix_falls_through() {
        run!(out, "［＃「X」に未知］");
        assert!(
            !out.spans
                .iter()
                .any(|s| matches!(aozora_node(s), Some(AozoraNode::Bouten(_)))),
        );
    }

    #[test]
    fn forward_bouten_missing_ni_particle_falls_through() {
        run!(out, "［＃「X」傍点］");
        assert!(
            !out.spans
                .iter()
                .any(|s| matches!(aozora_node(s), Some(AozoraNode::Bouten(_)))),
        );
    }

    #[test]
    fn forward_bouten_without_preceding_target_falls_through() {
        // Target 可哀想 never appears before the bracket — refusing to
        // promote to Bouten lets the generic Annotation classifier
        // wrap the raw `［＃…］` in an aozora-annotation span instead of
        // styling a non-existent referent.
        run!(out, "［＃「可哀想」に傍点］後");
        assert!(
            !out.spans
                .iter()
                .any(|s| matches!(aozora_node(s), Some(AozoraNode::Bouten(_)))),
        );
    }

    #[test]
    fn forward_bouten_target_in_preceding_paragraph_still_promotes() {
        // The classifier currently scans the entire preceding source
        // (not just the current paragraph). Preserving that lenient
        // behaviour keeps real Aozora corpora working — authors
        // sometimes refer backwards across paragraph boundaries.
        run!(out, "青空\n\n改行後［＃「青空」に傍点］");
        assert!(
            out.spans
                .iter()
                .any(|s| matches!(aozora_node(s), Some(AozoraNode::Bouten(_)))),
        );
    }

    #[test]
    fn forward_tcy_without_preceding_target_falls_through() {
        run!(out, "［＃「29」は縦中横］後");
        assert!(
            !out.spans
                .iter()
                .any(|s| matches!(aozora_node(s), Some(AozoraNode::TateChuYoko(_)))),
        );
    }

    #[test]
    fn forward_bouten_with_nested_quote_in_target_uses_outer_quote() {
        // Phase 2 balances 「「」」 correctly. The target is the full
        // outer-quote contents including the inner 「inner」 — not
        // truncated at the first 」. The preceding copy of the target
        // is required so the classifier's target-exists check passes.
        run!(out, "A「inner」B［＃「A「inner」B」に傍点］");
        let bouten = out
            .spans
            .iter()
            .find_map(|s| match aozora_node(s) {
                Some(AozoraNode::Bouten(b)) => Some(b),
                _ => None,
            })
            .expect("expected a Bouten span");
        assert_eq!(bouten.target.as_plain(), Some("A「inner」B"));
    }

    #[test]
    fn forward_tcy_single_recognized() {
        run!(out, "20［＃「20」は縦中横］");
        let tcy = out
            .spans
            .iter()
            .find_map(|s| match aozora_node(s) {
                Some(AozoraNode::TateChuYoko(t)) => Some(t),
                _ => None,
            })
            .expect("expected a TateChuYoko span");
        assert_eq!(tcy.text.as_plain(), Some("20"));
    }

    #[test]
    fn forward_tcy_wrong_particle_falls_through() {
        // Using に instead of は — not a TCY shape.
        run!(out, "［＃「20」に縦中横］");
        assert!(
            !out.spans
                .iter()
                .any(|s| matches!(aozora_node(s), Some(AozoraNode::TateChuYoko(_)))),
        );
    }

    #[test]
    fn forward_tcy_empty_target_falls_through() {
        run!(out, "［＃「」は縦中横］");
        assert!(
            !out.spans
                .iter()
                .any(|s| matches!(aozora_node(s), Some(AozoraNode::TateChuYoko(_)))),
        );
    }

    // ---------------------------------------------------------------
    // Forward-reference heading hints — `［＃「X」は(大|中|小)見出し］`.
    // These tests pin the lexer contract that drives post-process
    // paragraph promotion (docs/plan.md §M2): the classifier emits a
    // `HeadingHint { level: 1..=3 }` when the target is preceded by a
    // matching run in the source, otherwise falls through so the
    // catch-all emits `Annotation { Unknown }` and the Tier-A canary
    // ([# never leaks) still holds.
    // ---------------------------------------------------------------

    fn find_heading_hint<'a>(out: &TestClassifyOutput<'a>) -> Option<&'a HeadingHint<'a>> {
        out.spans.iter().find_map(|s| match aozora_node(s) {
            Some(AozoraNode::HeadingHint(h)) => Some(h),
            _ => None,
        })
    }

    #[test]
    fn forward_heading_large_recognized() {
        // Spec: 大見出し → Markdown H1 (level 1). The preceding
        // occurrence of the target literal is required — same gate as
        // forward-bouten.
        run!(out, "第一篇［＃「第一篇」は大見出し］");
        let h = find_heading_hint(&out).expect("expected HeadingHint");
        assert_eq!(h.level, 1);
        assert_eq!(h.target.as_str(), "第一篇");
    }

    #[test]
    fn forward_heading_medium_recognized() {
        // 中見出し → H2.
        run!(out, "一［＃「一」は中見出し］");
        let h = find_heading_hint(&out).expect("expected HeadingHint");
        assert_eq!(h.level, 2);
        assert_eq!(h.target.as_str(), "一");
    }

    #[test]
    fn forward_heading_small_recognized() {
        // 小見出し → H3.
        run!(out, "小題［＃「小題」は小見出し］");
        let h = find_heading_hint(&out).expect("expected HeadingHint");
        assert_eq!(h.level, 3);
        assert_eq!(h.target.as_str(), "小題");
    }

    #[test]
    fn forward_heading_without_preceding_target_falls_through() {
        // No 「第一篇」 run in the preceding source — hint has no
        // referent; classifier must reject so the paragraph isn't
        // promoted to an empty heading. The catch-all then emits
        // `Annotation { Unknown }` to preserve Tier-A.
        run!(out, "［＃「第一篇」は大見出し］後");
        assert!(find_heading_hint(&out).is_none());
    }

    #[test]
    fn forward_heading_unknown_keyword_falls_through() {
        // `大見出し` and friends are the only supported heading
        // keywords; anything else (包括的, 飾り見出し, …) should not
        // promote.
        run!(out, "X［＃「X」は飾り見出し］");
        assert!(find_heading_hint(&out).is_none());
    }

    #[test]
    fn forward_heading_wrong_particle_falls_through() {
        // The Aozora annotation spec's heading shape uses `は` as the
        // particle. Using `に` (the bouten particle) must not promote
        // to HeadingHint — otherwise we'd clobber the bouten path.
        run!(out, "X［＃「X」に大見出し］");
        assert!(find_heading_hint(&out).is_none());
    }

    #[test]
    fn forward_heading_empty_target_falls_through() {
        run!(out, "［＃「」は大見出し］");
        assert!(find_heading_hint(&out).is_none());
    }

    #[test]
    fn forward_heading_all_three_levels_exercised_in_one_paragraph() {
        // A single paragraph could conceivably carry multiple heading
        // hints — the lexer emits one HeadingHint per bracket and
        // post-process handles the first. This test locks the per-
        // bracket classification rather than the post_process policy.
        run!(
            out,
            "A［＃「A」は大見出し］B［＃「B」は中見出し］C［＃「C」は小見出し］"
        );
        let levels: Vec<u8> = out
            .spans
            .iter()
            .filter_map(|s| match aozora_node(s) {
                Some(AozoraNode::HeadingHint(h)) => Some(h.level),
                _ => None,
            })
            .collect();
        assert_eq!(levels, vec![1, 2, 3]);
    }

    #[test]
    fn sashie_without_caption_recognized() {
        run!(out, "［＃挿絵（fig01.png）入る］");
        let sashie = out
            .spans
            .iter()
            .find_map(|s| match aozora_node(s) {
                Some(AozoraNode::Sashie(s)) => Some(s),
                _ => None,
            })
            .expect("expected a Sashie span");
        assert_eq!(sashie.file.as_str(), "fig01.png");
        assert!(sashie.caption.is_none());
    }

    #[test]
    fn sashie_with_caption_recognized() {
        // Bundled-caption form `挿絵（file）「caption」入る` — the caption is
        // captured as plain content (rendered into <figcaption>, §8).
        run!(out, "［＃挿絵（fig01.png）「キャプション」入る］");
        let found = out.spans.iter().find_map(|s| match aozora_node(s) {
            Some(AozoraNode::Sashie(s)) => Some(s),
            _ => None,
        });
        let Some(sashie) = found else {
            panic!("expected a Sashie span");
        };
        assert_eq!(sashie.file.as_str(), "fig01.png");
        let Some(caption) = sashie.caption else {
            panic!("expected a bundled caption");
        };
        assert_eq!(caption.as_plain(), Some("キャプション"));
    }

    #[test]
    fn sashie_empty_caption_falls_through() {
        // `「」入る` is a degenerate empty caption — decline cleanly.
        run!(out, "［＃挿絵（fig01.png）「」入る］");
        assert!(
            !out.spans
                .iter()
                .any(|s| matches!(aozora_node(s), Some(AozoraNode::Sashie(_)))),
        );
    }

    #[test]
    fn sashie_empty_filename_falls_through() {
        run!(out, "［＃挿絵（）入る］");
        assert!(
            !out.spans
                .iter()
                .any(|s| matches!(aozora_node(s), Some(AozoraNode::Sashie(_)))),
        );
    }

    #[test]
    fn sashie_missing_iru_suffix_falls_through() {
        run!(out, "［＃挿絵（fig01.png）］");
        assert!(
            !out.spans
                .iter()
                .any(|s| matches!(aozora_node(s), Some(AozoraNode::Sashie(_)))),
        );
    }

    #[test]
    fn gaiji_quoted_description_with_mencode() {
        use aozora_encoding::gaiji::Resolved;
        run!(out, "※［＃「木＋吶のつくり」、第3水準1-85-54］");
        let gaiji = out
            .spans
            .iter()
            .find_map(|s| match aozora_node(s) {
                Some(AozoraNode::Gaiji(g)) => Some(g),
                _ => None,
            })
            .expect("expected a Gaiji span");
        assert_eq!(gaiji.description, "木＋吶のつくり");
        assert_eq!(gaiji.mencode, Some("第3水準1-85-54"));
        // JIS X 0213:2004 plane 1 row 85 cell 54 = 枘 (U+6798).
        // (Pre-regen seed had U+6903 (椃) — that was a different
        // character, U+6903 = 木+室. The corrected mapping is sourced
        // from glibc's EUC-JISX0213 charmap = the spec.)
        assert_eq!(gaiji.ucs, Some(Resolved::Char('\u{6798}')));
    }

    #[test]
    fn gaiji_quoted_description_without_mencode() {
        run!(out, "※［＃「試」］");
        let gaiji = out
            .spans
            .iter()
            .find_map(|s| match aozora_node(s) {
                Some(AozoraNode::Gaiji(g)) => Some(g),
                _ => None,
            })
            .expect("expected a Gaiji span");
        assert_eq!(gaiji.description, "試");
        assert!(gaiji.mencode.is_none());
    }

    #[test]
    fn gaiji_bare_description_with_mencode() {
        run!(out, "※［＃二の字点、1-2-23］");
        let gaiji = out
            .spans
            .iter()
            .find_map(|s| match aozora_node(s) {
                Some(AozoraNode::Gaiji(g)) => Some(g),
                _ => None,
            })
            .expect("expected a Gaiji span");
        assert_eq!(gaiji.description, "二の字点");
        assert_eq!(gaiji.mencode, Some("1-2-23"));
    }

    #[test]
    fn gaiji_consumes_refmark_and_bracket_as_one_span() {
        let src = "a※［＃「X」、m］b";
        run!(out, src);
        let gaiji_span = out
            .spans
            .iter()
            .find(|s| matches!(aozora_node(s), Some(AozoraNode::Gaiji(_))))
            .expect("expected a Gaiji span");
        // span must start at the ※ (after "a"), not at ［.
        assert_eq!(gaiji_span.source_span.slice(src), "※［＃「X」、m］");
    }

    #[test]
    fn refmark_without_following_bracket_stays_plain() {
        // Bare ※ without ［＃...］ — not a gaiji, emit as Plain.
        run!(out, "a※b");
        assert!(
            !out.spans
                .iter()
                .any(|s| matches!(aozora_node(s), Some(AozoraNode::Gaiji(_)))),
        );
    }

    #[test]
    fn gaiji_without_hash_is_not_recognized() {
        // ※ followed by ［ but no ＃ inside — not a gaiji shape.
        run!(out, "※［普通］");
        assert!(
            !out.spans
                .iter()
                .any(|s| matches!(aozora_node(s), Some(AozoraNode::Gaiji(_)))),
        );
    }

    #[test]
    fn kaeriten_ichi_recognized() {
        run!(out, "之［＃一］");
        let kaeriten = out
            .spans
            .iter()
            .find_map(|s| match aozora_node(s) {
                Some(AozoraNode::Kaeriten(k)) => Some(k),
                _ => None,
            })
            .expect("expected a Kaeriten span");
        assert_eq!(kaeriten.mark.as_str(), "一");
    }

    #[test]
    fn kaeriten_all_twelve_marks_recognized() {
        for mark in [
            "一", "二", "三", "四", "上", "中", "下", "レ", "甲", "乙", "丙", "丁",
        ] {
            let src = format!("［＃{mark}］");
            run!(out, &src);
            let Some(k) = out.spans.iter().find_map(|s| match aozora_node(s) {
                Some(AozoraNode::Kaeriten(k)) => Some(k),
                _ => None,
            }) else {
                panic!("no Kaeriten span for mark {mark:?}");
            };
            assert_eq!(k.mark.as_str(), mark);
        }
    }

    #[test]
    fn kaeriten_unknown_mark_falls_through() {
        run!(out, "［＃甬］");
        assert!(
            !out.spans
                .iter()
                .any(|s| matches!(aozora_node(s), Some(AozoraNode::Kaeriten(_)))),
        );
    }

    #[test]
    fn kaeriten_compound_marks_recognized() {
        // Compound kaeriten pair an order mark with the reversal mark
        // (`レ`). Six combinations are canonical per the Aozora
        // kunten spec. Each must produce a Kaeriten with the combo
        // string preserved verbatim.
        let cases = ["一レ", "二レ", "三レ", "上レ", "中レ", "下レ"];
        for mark in cases {
            let src = format!("［＃{mark}］");
            run!(out, &src);
            let k = out
                .spans
                .iter()
                .find_map(|s| match aozora_node(s) {
                    Some(AozoraNode::Kaeriten(k)) => Some(k),
                    _ => None,
                })
                .unwrap_or_else(|| panic!("no Kaeriten span for mark {mark:?}"));
            assert_eq!(k.mark.as_str(), mark, "mark={mark:?}");
        }
    }

    #[test]
    fn kaeriten_okurigana_shape_recognized() {
        // `［＃（X）］` where X is 1–6 Japanese chars is treated as an
        // okurigana marker — same AozoraNode::Kaeriten with the
        // parenthesised payload kept verbatim for the renderer.
        let cases = [
            "（カ）",
            "（ダ）",
            "（シクシテ）",
            "（弖）",       // kanji payload
            "（テニヲハ）", // 4-char katakana
        ];
        for mark in cases {
            let src = format!("［＃{mark}］");
            run!(out, &src);
            let k = out
                .spans
                .iter()
                .find_map(|s| match aozora_node(s) {
                    Some(AozoraNode::Kaeriten(k)) => Some(k),
                    _ => None,
                })
                .unwrap_or_else(|| panic!("no Kaeriten for okurigana {mark:?}"));
            assert_eq!(k.mark.as_str(), mark, "mark={mark:?}");
        }
    }

    #[test]
    fn kaeriten_okurigana_with_long_body_falls_through() {
        // 7+ character parenthesised content is almost always an
        // editorial gloss, not okurigana. Must fall through to
        // Annotation{Unknown} so we don't mislabel it as kaeriten.
        run!(out, "［＃（これはおくりがなではない）］");
        assert!(
            !out.spans
                .iter()
                .any(|s| matches!(aozora_node(s), Some(AozoraNode::Kaeriten(_)))),
            "long parenthesised bodies must not be Kaeriten: {:?}",
            out.spans
        );
    }

    #[test]
    fn kaeriten_okurigana_with_latin_body_falls_through() {
        // Okurigana payload must be hiragana/katakana/kanji. ASCII
        // inside parens is probably an editorial note, not kaeriten.
        run!(out, "［＃（abc）］");
        assert!(
            !out.spans
                .iter()
                .any(|s| matches!(aozora_node(s), Some(AozoraNode::Kaeriten(_)))),
        );
    }

    #[test]
    fn kaeriten_okurigana_empty_parens_fall_through() {
        run!(out, "［＃（）］");
        assert!(
            !out.spans
                .iter()
                .any(|s| matches!(aozora_node(s), Some(AozoraNode::Kaeriten(_)))),
        );
    }

    // ---------------------------------------------------------------
    // Double-angle quotation `≪X≫`.
    // ---------------------------------------------------------------

    #[test]
    fn angle_quote_plain_body_produces_angle_quote_span() {
        run!(out, "前≪強調≫後");
        let aozora = out
            .spans
            .iter()
            .find_map(aozora_node)
            .expect("AngleQuote expected");
        let AozoraNode::AngleQuote(d) = aozora else {
            panic!("expected AngleQuote, got {aozora:?}");
        };
        assert_eq!(d.content.as_plain(), Some("強調"));
    }

    #[test]
    fn angle_quote_consumes_entire_source_span() {
        // Source `≪X≫` must fold into ONE Aozora span that covers
        // the angle brackets AND the body. No `≪` characters may
        // leak to the outer `spans` list.
        let src = "≪ABC≫";
        run!(out, src);
        let aozora_count = out
            .spans
            .iter()
            .filter(|s| matches!(s.kind, SpanKind::Aozora(_)))
            .count();
        assert_eq!(
            aozora_count, 1,
            "one AngleQuote span expected: {:?}",
            out.spans
        );
        let aozora = out
            .spans
            .iter()
            .find(|s| matches!(s.kind, SpanKind::Aozora(_)))
            .expect("Aozora span");
        assert_eq!(aozora.source_span.start, 0);
        assert_eq!(aozora.source_span.end as usize, src.len());
    }

    #[test]
    fn angle_quote_with_nested_gaiji_folds_into_segments() {
        // The helper reuses `build_content_from_body`, so a `※［＃…］`
        // inside the angle brackets must surface as `Segment::Gaiji`
        // in the content — same invariant as nested gaiji in ruby.
        run!(out, "≪※［＃「ほ」、第3水準1-85-54］≫");
        let aozora = out
            .spans
            .iter()
            .find_map(aozora_node)
            .expect("Aozora expected");
        let AozoraNode::AngleQuote(d) = aozora else {
            panic!("expected AngleQuote, got {aozora:?}");
        };
        let Content::Segments(segs) = &d.content.get() else {
            panic!("expected Segments, got {:?}", d.content.get());
        };
        assert_eq!(segs.len(), 1);
        assert!(matches!(&segs[0], Segment::Gaiji(_)));
    }

    #[test]
    fn angle_quote_empty_body_falls_through_to_plain() {
        // `≪≫` with no body is not classified as AngleQuote.
        // The empty payload would violate the
        // `borrowed::NonEmpty<Content>` invariant; instead the bytes
        // flow through as plain text (the catch-all `replay_unrecognised_body`
        // fold). No Aozora span is emitted for the empty case.
        run!(out, "A≪≫B");
        let aozora_count = out
            .spans
            .iter()
            .filter(|s| matches!(s.kind, SpanKind::Aozora(_)))
            .count();
        assert_eq!(
            aozora_count, 0,
            "empty angle-quote must not emit a AngleQuote span — \
             empty content violates the NonEmpty<Content> invariant"
        );
    }

    #[test]
    fn container_open_indent_default_amount_one() {
        run!(out, "［＃ここから字下げ］");
        assert_eq!(out.spans.len(), 1);
        assert!(matches!(
            out.spans[0].kind,
            SpanKind::BlockOpen(ContainerKind::Indent {
                amount: 1,
                wrap: None,
                center: false
            })
        ));
    }

    #[test]
    fn container_open_indent_with_amount() {
        run!(out, "［＃ここから３字下げ］");
        assert!(matches!(
            out.spans[0].kind,
            SpanKind::BlockOpen(ContainerKind::Indent {
                amount: 3,
                wrap: None,
                center: false
            })
        ));
    }

    #[test]
    fn container_open_wrap_indent_parses_both_amounts() {
        run!(out, "［＃ここから２字下げ、折り返して４字下げ］");
        assert!(matches!(
            out.spans[0].kind,
            SpanKind::BlockOpen(ContainerKind::Indent {
                amount: 2,
                wrap: Some(4),
                center: false
            })
        ));
    }

    #[test]
    fn container_close_indent_matches_open_by_variant() {
        run!(out, "［＃ここから字下げ］本文［＃ここで字下げ終わり］");
        // Spans: BlockOpen(Indent{1}), Plain("本文"), BlockClose(Indent{0})
        assert_eq!(out.spans.len(), 3);
        assert!(matches!(
            out.spans[0].kind,
            SpanKind::BlockOpen(ContainerKind::Indent { .. })
        ));
        assert_eq!(out.spans[1].kind, SpanKind::Plain);
        assert!(matches!(
            out.spans[2].kind,
            SpanKind::BlockClose(ContainerKind::Indent { .. })
        ));
    }

    #[test]
    fn container_open_chitsuki_and_chi_kara_n() {
        run!(out, "［＃ここから地付き］");
        assert!(matches!(
            out.spans[0].kind,
            SpanKind::BlockOpen(ContainerKind::AlignEnd { offset: 0 })
        ));
        run!(out2, "［＃ここから地から2字上げ］");
        assert!(matches!(
            out2.spans[0].kind,
            SpanKind::BlockOpen(ContainerKind::AlignEnd { offset: 2 })
        ));
    }

    #[test]
    fn container_open_close_keigakomi() {
        run!(out, "［＃罫囲み］内部［＃罫囲み終わり］");
        assert!(matches!(
            out.spans[0].kind,
            SpanKind::BlockOpen(ContainerKind::Keigakomi)
        ));
        assert!(matches!(
            out.spans[2].kind,
            SpanKind::BlockClose(ContainerKind::Keigakomi)
        ));
    }

    #[test]
    fn warichu_open_close_are_inline_annotations() {
        // Aozora spec: `［＃割り注］…［＃割り注終わり］` is inline
        // (`<span class="aozora-warichu">…</span>`). The legacy block
        // form (`ここから割り注` / `ここで割り注終わり`) is deprecated
        // and not classified here.
        use aozora_syntax::AnnotationKind;
        run!(out, "［＃割り注］内部［＃割り注終わり］");
        let Some(AozoraNode::Annotation(open)) = aozora_node(&out.spans[0]) else {
            panic!(
                "expected Aozora(Annotation) for ［＃割り注］, got {:?}",
                out.spans[0].kind,
            );
        };
        assert_eq!(open.kind, AnnotationKind::WarichuOpen);
        assert_eq!(open.raw.as_str(), "［＃割り注］");

        let Some(AozoraNode::Annotation(close)) = aozora_node(&out.spans[2]) else {
            panic!(
                "expected Aozora(Annotation) for ［＃割り注終わり］, got {:?}",
                out.spans[2].kind,
            );
        };
        assert_eq!(close.kind, AnnotationKind::WarichuClose);
        assert_eq!(close.raw.as_str(), "［＃割り注終わり］");
    }

    #[test]
    fn container_close_without_matching_open_still_emits_close() {
        // Phase 3 does not pair opens with closes — that's `post_process`.
        // A bare `［＃罫囲み終わり］` is still classified.
        run!(out, "［＃罫囲み終わり］");
        assert!(matches!(
            out.spans[0].kind,
            SpanKind::BlockClose(ContainerKind::Keigakomi)
        ));
    }

    #[test]
    fn container_unknown_here_from_keyword_falls_through() {
        run!(out, "［＃ここから未知］");
        assert!(
            !out.spans
                .iter()
                .any(|s| matches!(s.kind, SpanKind::BlockOpen(_) | SpanKind::BlockClose(_))),
            "expected no block container spans, got {:?}",
            out.spans
        );
    }

    #[test]
    fn only_newline_source_emits_only_newline_span() {
        run!(out, "\n");
        assert_eq!(out.spans.len(), 1);
        assert_eq!(out.spans[0].kind, SpanKind::Newline);
        assert_eq!(out.spans[0].source_span, Span::new(0, 1));
    }

    #[test]
    fn diagnostics_from_phase2_are_forwarded() {
        run!(out, "stray］");
        // Phase 2 emits an UnmatchedClose diagnostic for `］`. The
        // classifier must propagate it (and not swallow it silently).
        assert!(
            out.diagnostics.iter().any(|d| matches!(
                d,
                Diagnostic::UnmatchedClose {
                    kind: PairKind::Bracket,
                    ..
                }
            )),
            "expected UnmatchedClose to be forwarded, got {:?}",
            out.diagnostics
        );
    }

    proptest! {
        /// Spans must tile the source contiguously, starting at 0 and
        /// ending at `source.len()` with no gaps or overlaps.
        #[test]
        fn proptest_spans_tile_source_contiguously(src in source_strategy()) {
            run!(out, &src);
            if src.is_empty() {
                prop_assert!(out.spans.is_empty());
                return Ok(());
            }
            prop_assert!(!out.spans.is_empty());
            prop_assert_eq!(out.spans[0].source_span.start, 0);
            for window in out.spans.windows(2) {
                prop_assert_eq!(
                    window[0].source_span.end,
                    window[1].source_span.start
                );
            }
            prop_assert_eq!(
                out.spans.last().unwrap().source_span.end as usize,
                src.len()
            );
        }

        /// No empty-range spans leak into the output. An empty span
        /// would usually indicate a double-flush bug and breaks the
        /// "each span represents at least one source byte" expectation
        /// Phase 4 holds.
        #[test]
        fn proptest_no_empty_spans(src in source_strategy()) {
            run!(out, &src);
            for span in &out.spans {
                prop_assert!(span.source_span.end > span.source_span.start);
            }
        }

        /// Every Newline span covers exactly one byte at a `\n`
        /// position.
        #[test]
        fn proptest_newline_spans_are_single_byte(src in source_strategy()) {
            run!(out, &src);
            for span in &out.spans {
                if span.kind == SpanKind::Newline {
                    prop_assert_eq!(span.source_span.len(), 1);
                    prop_assert_eq!(
                        &src[span.source_span.start as usize..span.source_span.end as usize],
                        "\n"
                    );
                }
            }
        }

        /// Classification is a pure function of the input.
        ///
        /// Determinism is asserted span-by-span; we cannot direct-`==`
        /// the two `ClassifyOutput`s across separate arenas because
        /// `borrowed::AozoraNode<'a>` `PartialEq` recurses through the
        /// arena-allocated payload pointers, which differ across runs
        /// even when the logical AST is identical. The pointer-aware
        /// equality is the right semantics — it lets the byte-identical
        /// proptest in `aozora-lex` pin pointer dedup. Here we want
        /// logical equality, so we compare via the `Debug` shape, which
        /// formats payload values rather than addresses.
        #[test]
        fn proptest_classify_is_deterministic(src in source_strategy()) {
            run!(a, &src);
            run!(b, &src);
            prop_assert_eq!(a.spans.len(), b.spans.len());
            for (l, r) in a.spans.iter().zip(b.spans.iter()) {
                prop_assert_eq!(l.source_span, r.source_span);
                prop_assert_eq!(format!("{:?}", l.kind), format!("{:?}", r.kind));
            }
        }
    }

    fn source_strategy() -> impl Strategy<Value = String> {
        prop::collection::vec(
            prop_oneof![
                Just('a'),
                Just('あ'),
                Just('漢'),
                Just('｜'),
                Just('《'),
                Just('》'),
                Just('［'),
                Just('］'),
                Just('＃'),
                Just('※'),
                Just('〔'),
                Just('〕'),
                Just('「'),
                Just('」'),
                Just('\n'),
            ],
            0..40,
        )
        .prop_map(|chars| chars.into_iter().collect())
    }

    // -----------------------------------------------------------------
    // Forward-target index threshold smoke tests (G.4 / phase3 mod).
    //
    // The forward-reference target index is only built when a source
    // contains at least `FORWARD_QUOTE_BODY_THRESHOLD` (= 64) distinct
    // `「…」` quote bodies. Below the threshold we want to confirm the
    // pipeline still works and the result is identical to a re-run
    // (stability across the threshold gate).
    // -----------------------------------------------------------------

    /// Inputs *below* `FORWARD_QUOTE_BODY_THRESHOLD` skip the index
    /// build altogether. Drive a small input through the full lex
    /// pipeline twice and pin determinism — proves the gate decision
    /// (skip the AC index) doesn't itself perturb output.
    #[test]
    fn forward_target_index_handles_short_corpus() {
        // 5 distinct quote bodies — well below the 64-body threshold.
        let src = "「a」「b」「c」「d」「e」";
        run!(a, src);
        run!(b, src);
        assert_eq!(a.spans.len(), b.spans.len());
        for (l, r) in a.spans.iter().zip(b.spans.iter()) {
            assert_eq!(l.source_span, r.source_span);
            assert_eq!(format!("{:?}", l.kind), format!("{:?}", r.kind));
        }
    }

    /// Forward-reference behaviour DEPENDS on whether the cited target
    /// (`「青空」`) appears earlier in source.
    ///
    /// * With a preceding `「青空」`: the bouten classifier sees the
    ///   prior occurrence and recognises `［＃「青空」に傍点］` as
    ///   a Bouten span.
    /// * Without a preceding occurrence: `forward_target_is_preceded`
    ///   returns `false` and the recogniser falls through to
    ///   `Annotation { kind: Unknown }` so the renderer doesn't apply
    ///   styling to a non-existent referent.
    ///
    /// The two outcomes must differ observably — this is the public
    /// behaviour gated on the forward-target lookup. We keep the
    /// assertion shape behavioural rather than poking at the
    /// thread-local index (which is non-public).
    #[test]
    fn forward_target_lookup_changes_output_for_preceded_vs_absent() {
        use aozora_syntax::borrowed::AozoraNode;

        // Case A: target exists earlier in source.
        let with_prior = "「青空」が見える。［＃「青空」に傍点］";
        run!(a, with_prior);
        let bouten_in_a = a
            .spans
            .iter()
            .any(|s| matches!(aozora_node(s), Some(AozoraNode::Bouten(_))));
        let unknown_in_a = a.spans.iter().any(|s| {
            matches!(
                aozora_node(s),
                Some(AozoraNode::Annotation(ann)) if ann.kind == AnnotationKind::Unknown
            )
        });

        // Case B: no prior `「青空」` occurrence.
        let without_prior = "ただの本文。［＃「青空」に傍点］";
        run!(b, without_prior);
        let bouten_in_b = b
            .spans
            .iter()
            .any(|s| matches!(aozora_node(s), Some(AozoraNode::Bouten(_))));
        let unknown_in_b = b.spans.iter().any(|s| {
            matches!(
                aozora_node(s),
                Some(AozoraNode::Annotation(ann)) if ann.kind == AnnotationKind::Unknown
            )
        });

        assert!(
            bouten_in_a && !unknown_in_a,
            "with prior `「青空」`, expected a Bouten span and no Unknown annotation, \
             got spans={:?}",
            a.spans
        );
        assert!(
            unknown_in_b && !bouten_in_b,
            "without prior `「青空」`, expected fallback Annotation{{Unknown}} and no Bouten, \
             got spans={:?}",
            b.spans
        );
    }

    /// Empty input is the "smallest possible corpus"; the pipeline
    /// must short-circuit cleanly without installing any thread-local
    /// state and produce no spans / no diagnostics.
    #[test]
    fn forward_target_index_handles_empty_corpus() {
        run!(out, "");
        assert!(out.spans.is_empty());
        assert!(out.diagnostics.is_empty());
    }
}
