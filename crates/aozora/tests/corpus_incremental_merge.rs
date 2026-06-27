//! Load-bearing gate for incremental re-parse (#237).
//!
//! Proves, over every document in `AOZORA_CORPUS_ROOT`, that the owned-table
//! splice [`aozora::reparse_incremental_owned`] is a true incremental engine:
//! its spliced `OwnedLexOutput` is **byte-for-byte equal** — on every
//! resolved/rendered surface (HTML, source, normalized/sanitized text, the
//! diagnostics multiset, container pairs, delimiter pairs, the registry, and
//! the source-node table) — to a from-scratch parse of the edited text. A
//! single deterministic plain-character insertion near each document's midpoint
//! exercises the splice fast path on global-free documents and the full-parse
//! fallback elsewhere; the fast-path count is asserted non-zero so the gate
//! actually drives the splice. Any edit the splice cannot prove byte-identical
//! returns `None` and falls back to a full parse (trivially correct).
//!
//! Skipped silently when `AOZORA_CORPUS_ROOT` is unset; never hard-fails on
//! a missing corpus (mirrors `corpus_sweep`).

use aozora::render::{render_html_owned, serialize_owned};
use aozora::syntax::owned::{ContentOwned, NodeOwned, NodeRefOwned, NodeStore, SegmentOwned};
use aozora::syntax::owned::{ContentRange, GaijiCanonicalOwned, GaijiOwned, SegRange};
use aozora::{
    DiagBaseRef, Diagnostic, Document, reparse_incremental_diagnostics_only,
    reparse_incremental_owned,
};
use aozora_encoding::decode_auto;

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

/// #237 Tier 1: the **diagnostics-only** incremental engine
/// ([`aozora::reparse_incremental_diagnostics_only`]) — the LSP's per-keystroke
/// hot path — must produce diagnostics byte-identical to a from-scratch parse of
/// the edited text, for every corpus document whose midpoint insertion is a
/// sanitize fixed point. It must additionally be **pinned** to the owned splice
/// ([`aozora::reparse_incremental_owned`]): wherever the diagnostics-only engine
/// takes the fast path, the owned engine must too, and their diagnostics must be
/// identical (the two share the prologue, so any drift is a bug).
///
/// Same harness as [`reparse_owned_incremental_equals_full_parse`] (sanitized
/// fixed-point baseline, midpoint plain insertion). Fast-path count asserted
/// non-zero so the gate actually drives the engine.
#[test]
fn reparse_diagnostics_only_equals_full_parse() {
    let Some(source) = aozora_corpus::from_env() else {
        eprintln!("AOZORA_CORPUS_ROOT not set; skipping diagnostics-only incremental gate");
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

        // Sanitized fixed-point baseline (see the owned gate for the rationale).
        let san = Document::new(text).parse_owned().sanitized;
        if san.is_empty() {
            continue;
        }
        let cached = Document::new(san.as_str()).parse_owned();
        if cached.sanitized != san {
            continue;
        }

        let mut mid = san.len() / 2;
        while mid < san.len() && !san.is_char_boundary(mid) {
            mid += 1;
        }
        let new_san = format!("{}x{}", &san[..mid], &san[mid..]);

        let full = Document::new(new_san.as_str()).parse_owned();
        if full.sanitized != new_san {
            continue;
        }

        let Some(diag) =
            reparse_incremental_diagnostics_only(DiagBaseRef::of(&cached), &new_san, mid..mid)
        else {
            fallback += 1;
            count += 1;
            continue;
        };
        fast_path += 1;
        count += 1;

        let mut problems: Vec<String> = Vec::new();

        // 1. Diagnostics byte-identical to a full parse (the production contract).
        if sorted_debug(diag.diagnostics.clone()) != sorted_debug(full.diagnostics.clone()) {
            problems.push("diagnostics-only multiset != full parse".to_owned());
        }

        // 2. Pinned to the owned splice: the owned engine must also fast-path
        //    here (shared prologue), with identical diagnostics.
        match reparse_incremental_owned(&cached, &new_san, mid..mid) {
            Some(owned) => {
                if sorted_debug(diag.diagnostics.clone())
                    != sorted_debug(owned.output.diagnostics.clone())
                {
                    problems.push("diagnostics-only != owned splice diagnostics".to_owned());
                }
            }
            None => problems.push(
                "diagnostics-only fast-pathed but owned splice declined (prologue drift)"
                    .to_owned(),
            ),
        }

        if !problems.is_empty() {
            diverged.push(format!("{}: {}", item.label, problems.join("; ")));
        }
    }

    eprintln!(
        "diagnostics-only incremental gate: {count} docs edited, {fast_path} fast-path, \
         {fallback} fallback, {} diverged",
        diverged.len(),
    );
    assert!(
        diverged.is_empty(),
        "{} document(s) where the diagnostics-only splice diverged:\n  {}",
        diverged.len(),
        diverged.join("\n  "),
    );
    assert!(
        fast_path > 0,
        "the diagnostics-only gate must exercise the fast path at least once \
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
