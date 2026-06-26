//! Load-bearing gate for incremental re-parse (#237, Stage A).
//!
//! Proves two things over every document in `AOZORA_CORPUS_ROOT`:
//!
//! 1. **Reassembly equivalence.** [`aozora::SegmentedParse`]'s per-segment
//!    locals (rebased) plus its whole-document-scoped diagnostics equal a
//!    whole-document parse as a positional multiset. Because the merge is
//!    `max(local, whole-scoped)` per diagnostic, an exact match means the
//!    segments never invent or misplace a diagnostic (no phantoms).
//! 2. **Bounded non-locality.** The only diagnostics a segment cannot
//!    reproduce locally — and so must be carried from the whole-document
//!    parse — are the documented whole-document-scoped class
//!    (forward-reference bouten ambiguity). A new non-local diagnostic
//!    surfacing here fails the gate so it gets a deliberate review.
//!
//! Skipped silently when `AOZORA_CORPUS_ROOT` is unset; never hard-fails on
//! a missing corpus (mirrors `corpus_sweep`).

use aozora::render::{render_html_owned, serialize_owned};
use aozora::syntax::owned::{ContentOwned, NodeOwned, NodeRefOwned, NodeStore, SegmentOwned};
use aozora::syntax::owned::{ContentRange, GaijiCanonicalOwned, GaijiOwned, SegRange};
use aozora::{Diagnostic, Document, SegmentedParse, reparse_incremental_owned};
use aozora_encoding::decode_auto;

/// Diagnostic variants whose computation depends on the whole document
/// (forward-reference resolution + end-of-document kaeriten pairing) and so
/// cannot be reproduced by an isolated segment. Keep in sync with
/// `aozora::segmented::is_whole_document_scoped`.
const WHOLE_DOCUMENT_SCOPED: &[&str] = &[
    "BoutenTargetAmbiguous",
    "TcyTargetNotFound",
    "UnresolvedGaiji",
    "UnrecognisedContainerDirective",
    "BracketedKaeritenNoPair",
    "KaeritenOutsideKanbun",
    "MismatchedContainerClose",
    "MismatchedBoutenContainer",
];

#[test]
fn segmented_merge_equals_whole_doc_parse() {
    let Some(source) = aozora_corpus::from_env() else {
        eprintln!("AOZORA_CORPUS_ROOT not set; skipping incremental-merge gate");
        return;
    };

    let mut count: usize = 0;
    let mut segmented: usize = 0;
    let mut with_scoped: usize = 0;
    // Collect every problem rather than failing on the first.
    let mut diverged: Vec<String> = Vec::new();
    let mut unexpected_scoped: Vec<String> = Vec::new();

    for item in source.iter() {
        let item = item.expect("corpus iteration must not error");

        let Ok(text) = decode_auto(&item.bytes) else {
            eprintln!("skip (neither UTF-8 nor Shift_JIS): {}", item.label);
            continue;
        };

        let whole = Document::new(text.as_ref());
        let whole_diags = sorted_debug(whole.parse().diagnostics().to_vec());

        let seg = SegmentedParse::of(text.as_ref());
        if seg.is_segmented() {
            segmented += 1;
        }
        if !seg.whole_document_scoped().is_empty() {
            with_scoped += 1;
        }

        // (1) reassembly equivalence
        let merged_diags = sorted_debug(seg.merged_diagnostics());
        if whole_diags != merged_diags {
            diverged.push(format!(
                "{} (segments={}): whole={:?} merged={:?}",
                item.label,
                seg.segment_count(),
                whole_diags,
                merged_diags,
            ));
        }

        // (2) every carried diagnostic is of a documented whole-doc-scoped
        // variant
        for d in seg.whole_document_scoped() {
            let variant = variant_name(d);
            if !WHOLE_DOCUMENT_SCOPED.contains(&variant) {
                unexpected_scoped.push(format!("{}: {variant}", item.label));
            }
        }

        count += 1;
    }

    eprintln!(
        "incremental-merge gate: {count} docs walked, {segmented} multi-segment, \
         {with_scoped} with whole-document-scoped diagnostics"
    );

    let mut problems = Vec::new();
    if !diverged.is_empty() {
        problems.push(format!(
            "{} document(s) where reassembled merge != whole-doc parse:\n  {}",
            diverged.len(),
            diverged.join("\n  "),
        ));
    }
    if !unexpected_scoped.is_empty() {
        problems.push(format!(
            "{} undocumented whole-document-scoped diagnostic(s) — add to WHOLE_DOCUMENT_SCOPED \
             and the `aozora::segmented` module docs after review:\n  {}",
            unexpected_scoped.len(),
            unexpected_scoped.join("\n  "),
        ));
    }
    assert!(problems.is_empty(), "\n{}", problems.join("\n\n"));
}

/// Stage A2: `SegmentedParse::reparse_incremental` must produce the same
/// diagnostics as a from-scratch parse of the edited text, for every corpus
/// document. A single deterministic plain-character insertion near the
/// document's midpoint exercises the incremental fast path on global-free
/// documents and the full-parse fallback elsewhere. Also reports how often the
/// fast path applied, so the reuse rate is visible.
#[test]
fn reparse_incremental_equals_full_parse() {
    let Some(source) = aozora_corpus::from_env() else {
        eprintln!("AOZORA_CORPUS_ROOT not set; skipping incremental-reparse gate");
        return;
    };

    let mut count: usize = 0;
    let mut fast_path: usize = 0;
    let mut diverged: Vec<String> = Vec::new();

    for item in source.iter() {
        let item = item.expect("corpus iteration must not error");
        let Ok(text) = decode_auto(&item.bytes) else {
            continue;
        };
        let text = text.as_ref();
        if text.is_empty() {
            continue;
        }

        // Insert a plain ASCII character at a char boundary near the midpoint.
        let mut at = text.len() / 2;
        while at < text.len() && !text.is_char_boundary(at) {
            at += 1;
        }
        let mut edited = String::with_capacity(text.len() + 1);
        edited.push_str(&text[..at]);
        edited.push('x');
        edited.push_str(&text[at..]);

        let cached = SegmentedParse::of(text);
        let (incremental, outcome) = cached.reparse_incremental(&edited, at..at);
        if outcome.reused {
            fast_path += 1;
        }

        let got = sorted_debug(incremental.merged_diagnostics());
        let want = sorted_debug(SegmentedParse::of(&edited).merged_diagnostics());
        if got != want {
            diverged.push(format!(
                "{} (fast_path={}): incremental != full",
                item.label, outcome.reused
            ));
        }
        count += 1;
    }

    eprintln!(
        "incremental-reparse gate: {count} docs edited, {fast_path} took the incremental fast path"
    );
    assert!(
        diverged.is_empty(),
        "{} document(s) where reparse_incremental != full parse:\n  {}",
        diverged.len(),
        diverged.join("\n  "),
    );
}

/// PR3b-2 Stage B'2: `reparse_incremental_owned` (the owned-table splice)
/// must produce an `OwnedLexOutput` byte-for-byte equal — on every
/// resolved/rendered surface — to a from-scratch parse of the edited text, for
/// every corpus document whose midpoint insertion is a sanitize fixed point.
///
/// A single deterministic plain-character insertion near the document midpoint
/// exercises the splice fast path on global-free documents and the full-parse
/// fallback elsewhere. The fast path is asserted byte-identical; the fallback
/// (`None`) is trivially correct (production full-parses). Divergences are
/// collected and reported together rather than failing fast, and the fast-path
/// count is asserted non-zero so the gate actually exercises the splice.
#[test]
#[allow(
    clippy::too_many_lines,
    clippy::if_not_else,
    reason = "a linear differential harness: each surface is compared inline and divergences collected; the length-mismatch-first branches read most directly as written"
)]
fn reparse_owned_incremental_equals_full_parse() {
    let Some(source) = aozora_corpus::from_env() else {
        eprintln!("AOZORA_CORPUS_ROOT not set; skipping owned incremental-splice gate");
        return;
    };

    let mut count: usize = 0;
    let mut fast_path: usize = 0;
    let mut fallback: usize = 0;
    let mut diverged: Vec<String> = Vec::new();

    for item in source.iter() {
        let item = item.expect("corpus iteration must not error");
        let Ok(text) = decode_auto(&item.bytes) else {
            continue;
        };
        let text = text.as_ref();
        if text.is_empty() {
            continue;
        }

        // The incremental engine operates entirely in **sanitized** space (the
        // raw↔sanitized bridge is a later wiring PR): `cached` must therefore be
        // the parse of already-sanitized text, not the raw corpus bytes.
        // Parsing the raw text would leave sanitize-stage diagnostics
        // (`AccentDecompositionApplied` / `SourceContainsPua`) in `cached` that a
        // re-parse of the sanitized buffer never reproduces — a harness
        // asymmetry, not a splice property. So sanitize once, then take the parse
        // of the sanitized buffer as the cached baseline.
        let san = Document::new(text).parse_owned().sanitized;
        if san.is_empty() {
            continue;
        }
        let cached = Document::new(san.as_str()).parse_owned();
        // Skip documents whose sanitized buffer is not itself a sanitize fixed
        // point (sanitize is non-idempotent for them): the incremental contract
        // assumes a stable sanitized baseline, and production would full-parse.
        if cached.sanitized != san {
            continue;
        }

        // A plain ASCII insertion at a char boundary near the sanitized midpoint.
        let mut mid = san.len() / 2;
        while mid < san.len() && !san.is_char_boundary(mid) {
            mid += 1;
        }
        let new_san = format!("{}x{}", &san[..mid], &san[mid..]);

        // Idempotence precheck: only edits that are sanitize fixed points are
        // representative — production would full-parse a non-fixed-point edit.
        let full = Document::new(new_san.as_str()).parse_owned();
        if full.sanitized != new_san {
            continue;
        }

        let Some(splice) = reparse_incremental_owned(&cached, &new_san, mid..mid) else {
            fallback += 1;
            count += 1;
            continue;
        };
        let spliced = splice.output;
        fast_path += 1;
        count += 1;

        let mut problems: Vec<String> = Vec::new();

        if render_html_owned(&spliced) != render_html_owned(&full) {
            problems.push("rendered HTML differs".to_owned());
        }
        if serialize_owned(&spliced) != serialize_owned(&full) {
            problems.push("serialized source differs".to_owned());
        }
        if spliced.normalized != full.normalized {
            problems.push("normalized differs".to_owned());
        }
        if spliced.sanitized != full.sanitized {
            problems.push("sanitized differs".to_owned());
        }
        if spliced.sanitized_len != full.sanitized_len {
            problems.push(format!(
                "sanitized_len differs ({} vs {})",
                spliced.sanitized_len, full.sanitized_len
            ));
        }
        if sorted_debug(spliced.diagnostics.clone()) != sorted_debug(full.diagnostics.clone()) {
            problems.push("diagnostics multiset differs".to_owned());
        }
        if sorted_strings(spliced.container_pairs.iter().map(|c| format!("{c:?}")))
            != sorted_strings(full.container_pairs.iter().map(|c| format!("{c:?}")))
        {
            problems.push("container_pairs multiset differs".to_owned());
        }
        if sorted_strings(spliced.pairs.iter().map(|p| format!("{p:?}")))
            != sorted_strings(full.pairs.iter().map(|p| format!("{p:?}")))
        {
            problems.push("pairs multiset differs".to_owned());
        }

        // Registry: same length, same positions, and per-position store-
        // independent node projection.
        if spliced.registry.len() != full.registry.len() {
            problems.push(format!(
                "registry length differs ({} vs {})",
                spliced.registry.len(),
                full.registry.len()
            ));
        }
        let spliced_reg: Vec<(u32, NodeRefOwned)> = spliced.registry.iter_sorted().collect();
        let full_reg: Vec<(u32, NodeRefOwned)> = full.registry.iter_sorted().collect();
        let spliced_positions: Vec<u32> = spliced_reg.iter().map(|(p, _)| *p).collect();
        let full_positions: Vec<u32> = full_reg.iter().map(|(p, _)| *p).collect();
        if spliced_positions != full_positions {
            problems.push("registry positions differ".to_owned());
        } else {
            for ((_, sn), (_, fln)) in spliced_reg.iter().zip(full_reg.iter()) {
                if project_noderef(*sn, &spliced.store) != project_noderef(*fln, &full.store) {
                    problems.push("registry node projection differs".to_owned());
                    break;
                }
            }
            // node_at lookup parity for every full position.
            for &p in &full_positions {
                let s = spliced.registry.node_at(p.into());
                let f = full.registry.node_at(p.into());
                let s_proj = s.map(|n| project_noderef(n, &spliced.store));
                let f_proj = f.map(|n| project_noderef(n, &full.store));
                if s_proj != f_proj {
                    problems.push(format!("registry node_at({p}) projection differs"));
                    break;
                }
            }
        }

        // source_nodes: same length, equal spans, equal node projections.
        if spliced.source_nodes.len() != full.source_nodes.len() {
            problems.push(format!(
                "source_nodes length differs ({} vs {})",
                spliced.source_nodes.len(),
                full.source_nodes.len()
            ));
        } else {
            for (s, f) in spliced.source_nodes.iter().zip(full.source_nodes.iter()) {
                if s.source_span != f.source_span {
                    problems.push(format!(
                        "source_node span differs ({:?} vs {:?})",
                        s.source_span, f.source_span
                    ));
                    break;
                }
                if project_noderef(s.node, &spliced.store) != project_noderef(f.node, &full.store) {
                    problems.push("source_node projection differs".to_owned());
                    break;
                }
            }
        }

        if !problems.is_empty() {
            diverged.push(format!("{}: {}", item.label, problems.join("; ")));
        }
    }

    eprintln!(
        "owned incremental-splice gate: {count} docs edited, {fast_path} fast-path, \
         {fallback} fallback, {} diverged",
        diverged.len(),
    );
    assert!(
        diverged.is_empty(),
        "{} document(s) where the owned splice != full parse:\n  {}",
        diverged.len(),
        diverged.join("\n  "),
    );
    assert!(
        fast_path > 0,
        "the owned-splice gate must exercise the fast path at least once \
         (got {fast_path}); a perpetual fallback proves nothing",
    );
}

/// A canonical multiset ordering of an iterator of debug strings.
fn sorted_strings(items: impl Iterator<Item = String>) -> Vec<String> {
    let mut v: Vec<String> = items.collect();
    v.sort();
    v
}

/// Fully project a [`NodeRefOwned`] to a store-independent `String`: a variant
/// tag plus every resolved string / content / segment, recursively. Equal
/// projections from two distinct stores prove the splice preserved every
/// resolvable byte and the structure. `BlockOpen`/`BlockClose` carry only
/// `Copy` scalar discriminants, so their `Debug` is already store-independent.
fn project_noderef(nr: NodeRefOwned, store: &NodeStore) -> String {
    match nr {
        NodeRefOwned::Inline(n) => format!("Inline[{}]", project_node(store, n)),
        NodeRefOwned::BlockLeaf(n) => format!("BlockLeaf[{}]", project_node(store, n)),
        NodeRefOwned::BlockOpen(rf) => format!("BlockOpen[{rf:?}]"),
        NodeRefOwned::BlockClose(rc) => format!("BlockClose[{rc:?}]"),
        // `NodeRefOwned` is `#[non_exhaustive]`; an unknown future variant is
        // projected via Debug so the gate still compares it store-independently.
        other => format!("Other[{other:?}]"),
    }
}

fn project_node(store: &NodeStore, node: NodeOwned) -> String {
    match node {
        NodeOwned::Ruby(r) => format!(
            "Ruby[base={};reading={};side={:?}]",
            project_content_range(store, r.base),
            project_content_range(store, r.reading),
            r.side,
        ),
        NodeOwned::Format(f) => format!(
            "Format[attr={:?};target={};origin={:?}]",
            f.attr,
            project_content_range(store, f.target),
            f.origin,
        ),
        NodeOwned::Gaiji(g) => format!("Gaiji[{}]", project_gaiji(store, g)),
        NodeOwned::Line(l) => format!("Line[{l:?}]"),
        NodeOwned::Warichu(w) => format!(
            "Warichu[upper={};lower={}]",
            project_content(store, w.upper),
            project_content(store, w.lower),
        ),
        NodeOwned::PageBreak => "PageBreak".to_owned(),
        NodeOwned::SectionBreak(k) => format!("SectionBreak[{k:?}]"),
        NodeOwned::BodyEnd => "BodyEnd".to_owned(),
        NodeOwned::ForcedBreak => "ForcedBreak".to_owned(),
        NodeOwned::Heading(h) => format!(
            "Heading[kind={:?};style={:?};text={}]",
            h.kind,
            h.style,
            project_content_range(store, h.text),
        ),
        NodeOwned::HeadingHint(h) => format!(
            "HeadingHint[level={:?};style={:?};target={:?}]",
            h.level,
            h.style,
            store.resolve_str(h.target),
        ),
        NodeOwned::Illustration(i) => format!(
            "Illustration[file={:?};number={:?};dimensions={:?};caption={:?};description={:?}]",
            store.resolve_str(i.file),
            i.number.map(|id| store.resolve_str(id).to_owned()),
            i.dimensions.map(|id| store.resolve_str(id).to_owned()),
            i.caption.map(|c| project_content(store, c)),
            i.description.map(|id| store.resolve_str(id).to_owned()),
        ),
        NodeOwned::Kaeriten(k) => format!("Kaeriten[{:?}]", store.resolve_str(k.mark)),
        NodeOwned::Directive(d) => {
            format!(
                "Directive[raw={:?};kind={:?}]",
                store.resolve_str(d.raw),
                d.kind
            )
        }
        NodeOwned::AngleQuote(a) => {
            format!("AngleQuote[{}]", project_content_range(store, a.content))
        }
        NodeOwned::MarginNote(m) => format!(
            "MarginNote[kind={:?};base={};note={}]",
            m.kind,
            project_content_range(store, m.base),
            project_content_range(store, m.note),
        ),
        NodeOwned::Container(c) => format!("Container[{c:?}]"),
        // `NodeOwned` is `#[non_exhaustive]`; project an unknown variant via
        // Debug (its handles, if any, would compare as raw indices — acceptable
        // since the gate would already diverge on the resolved surfaces).
        other => format!("Other[{other:?}]"),
    }
}

fn project_content_range(store: &NodeStore, r: ContentRange) -> String {
    let parts: Vec<String> = store
        .resolve_content_range(r)
        .iter()
        .map(|c| project_content(store, *c))
        .collect();
    format!("<{}>", parts.join(","))
}

fn project_content(store: &NodeStore, c: ContentOwned) -> String {
    match c {
        ContentOwned::Plain(id) => format!("P:{:?}", store.resolve_str(id)),
        ContentOwned::Segments(sr) => format!("S[{}]", project_seg_range(store, sr)),
        // `ContentOwned` is `#[non_exhaustive]` to this external test crate.
        other => format!("Other[{other:?}]"),
    }
}

fn project_seg_range(store: &NodeStore, sr: SegRange) -> String {
    let parts: Vec<String> = store
        .resolve_seg_range(sr)
        .iter()
        .map(|s| project_seg(store, *s))
        .collect();
    parts.join(",")
}

fn project_seg(store: &NodeStore, s: SegmentOwned) -> String {
    match s {
        SegmentOwned::Text(id) => format!("T:{:?}", store.resolve_str(id)),
        SegmentOwned::Gaiji(g) => format!("G[{}]", project_gaiji(store, g)),
        SegmentOwned::Directive(d) => {
            format!("D[raw={:?};kind={:?}]", store.resolve_str(d.raw), d.kind)
        }
        // `SegmentOwned` is `#[non_exhaustive]` to this external test crate.
        other => format!("Other[{other:?}]"),
    }
}

fn project_gaiji(store: &NodeStore, g: GaijiOwned) -> String {
    let canonical = match g.canonical {
        GaijiCanonicalOwned::MenKuTen(m) => format!("MenKuTen{m:?}"),
        GaijiCanonicalOwned::Unicode(c) => format!("Unicode{c:?}"),
        GaijiCanonicalOwned::Unresolved { mencode } => {
            format!(
                "Unresolved{:?}",
                mencode.map(|id| store.resolve_str(id).to_owned())
            )
        }
    };
    format!(
        "hint={:?};canonical={canonical};standalone={}",
        store.resolve_str(g.hint),
        g.standalone,
    )
}

/// Diagnostics sorted by position then debug string — a canonical positional
/// multiset ordering for comparison.
fn sorted_debug(mut diags: Vec<Diagnostic>) -> Vec<String> {
    diags.sort_by(|a, b| {
        let (sa, sb) = (a.span(), b.span());
        (sa.start, sa.end)
            .cmp(&(sb.start, sb.end))
            .then_with(|| format!("{a:?}").cmp(&format!("{b:?}")))
    });
    diags.iter().map(|d| format!("{d:?}")).collect()
}

/// The variant name of a diagnostic (the leading identifier of its debug
/// representation), e.g. `"BoutenTargetAmbiguous"`.
fn variant_name(d: &Diagnostic) -> &'static str {
    // Match the variants directly so the name is a compile-time constant.
    match d {
        Diagnostic::SourceContainsPua { .. } => "SourceContainsPua",
        Diagnostic::UnclosedBracket { .. } => "UnclosedBracket",
        Diagnostic::UnmatchedClose { .. } => "UnmatchedClose",
        Diagnostic::AccentDecompositionApplied { .. } => "AccentDecompositionApplied",
        Diagnostic::UnresolvedGaiji { .. } => "UnresolvedGaiji",
        Diagnostic::MismatchedContainerClose { .. } => "MismatchedContainerClose",
        Diagnostic::EmptyRubyReading { .. } => "EmptyRubyReading",
        Diagnostic::NestedRuby { .. } => "NestedRuby",
        Diagnostic::UnrecognisedContainerDirective { .. } => "UnrecognisedContainerDirective",
        Diagnostic::TcyTargetNotFound { .. } => "TcyTargetNotFound",
        Diagnostic::BoutenTargetAmbiguous { .. } => "BoutenTargetAmbiguous",
        Diagnostic::BreakInSingleLineContainer { .. } => "BreakInSingleLineContainer",
        Diagnostic::BracketedKaeritenNoPair { .. } => "BracketedKaeritenNoPair",
        Diagnostic::KaeritenOutsideKanbun { .. } => "KaeritenOutsideKanbun",
        Diagnostic::MismatchedBoutenContainer { .. } => "MismatchedBoutenContainer",
        Diagnostic::Internal { .. } => "Internal",
        _ => "Unknown",
    }
}
