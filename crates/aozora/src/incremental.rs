use core::ops::Range;
use std::sync::Arc;

#[cfg(test)]
use crate::pipeline::lex;
use crate::pipeline::{LexOutput, RegionOutput, SanitizedText, SourceNode, lex_region};
use crate::spec::{Diagnostic, NormalizedOffset, PairLink};
use crate::syntax::ast::{ContainerPair, Node, NodeRef, NodeStore, Registry};
use crate::syntax::{DirectiveKind, ForwardOrigin};

pub(crate) struct IncrementalOutput {
    pub(crate) output: LexOutput,
    #[cfg(test)]
    pub(crate) reused_nodes: usize,
    #[cfg(test)]
    pub(crate) reparsed_nodes: usize,
}

pub(crate) fn reparse(
    cached: &LexOutput,
    source: impl Into<SanitizedText>,
    edit: Range<usize>,
    source_unchanged: bool,
) -> Option<IncrementalOutput> {
    let source = source.into();
    if cached.sanitized == source {
        return None;
    }
    if has_ruby_format_dependency(cached, edit.start) {
        return None;
    }

    let region = balanced_region(cached, edit)?;
    let delta = i64::try_from(source.len()).ok()? - i64::try_from(cached.sanitized.len()).ok()?;
    let new_region_end = shift_usize(region.end, delta)?;
    let reparsed = lex_region(&source, region.start..new_region_end)?;
    if !balanced_nodes(&reparsed.source_nodes) {
        return None;
    }
    if introduces_ruby_format_dependency(cached, &reparsed, region.start, region.end) {
        return None;
    }

    merge(MergeInput {
        cached,
        reparsed: &reparsed,
        source,
        region,
        source_delta: delta,
        source_unchanged,
    })
}

fn has_ruby_format_dependency(cached: &LexOutput, edit_start: usize) -> bool {
    let mut rubies = Vec::new();
    for entry in &cached.source_nodes {
        match entry.node {
            NodeRef::Inline(Node::Ruby(ruby)) => {
                if let Some(base) = cached.store.content_range_as_plain(ruby.base) {
                    rubies.push(base);
                }
            }
            NodeRef::Inline(Node::Format(format))
                if format.origin == ForwardOrigin::Referenced
                    && entry.source_span.end as usize > edit_start =>
            {
                let Some(target) = cached.store.content_range_as_plain(format.target) else {
                    continue;
                };
                if rubies.contains(&target) {
                    return true;
                }
            }
            _ => {}
        }
    }
    false
}

fn introduces_ruby_format_dependency(
    cached: &LexOutput,
    reparsed: &RegionOutput,
    region_start: usize,
    region_end: usize,
) -> bool {
    let prefix_rubies = cached
        .source_nodes
        .iter()
        .take_while(|entry| entry.source_span.end as usize <= region_start)
        .filter_map(|entry| {
            let NodeRef::Inline(Node::Ruby(ruby)) = entry.node else {
                return None;
            };
            cached.store.content_range_as_plain(ruby.base)
        })
        .collect::<Vec<_>>();
    let region_has_ruby = reparsed
        .source_nodes
        .iter()
        .any(|entry| matches!(entry.node, NodeRef::Inline(Node::Ruby(_))));
    let suffix_has_referenced_format = cached
        .source_nodes
        .iter()
        .skip_while(|entry| (entry.source_span.start as usize) < region_end)
        .any(|entry| {
            matches!(
                entry.node,
                NodeRef::Inline(Node::Format(format))
                    if format.origin == ForwardOrigin::Referenced
            )
        });
    if region_has_ruby && suffix_has_referenced_format {
        return true;
    }
    reparsed.source_nodes.iter().any(|entry| {
        let NodeRef::Inline(Node::Format(format)) = entry.node else {
            return false;
        };
        if format.origin != ForwardOrigin::Referenced {
            return false;
        }
        reparsed
            .store
            .content_range_as_plain(format.target)
            .is_some_and(|target| prefix_rubies.contains(&target))
    })
}

struct MergeWindow {
    source_start: u32,
    source_end: u32,
    normalized_start: u32,
    normalized_end: u32,
    source_delta: i64,
    normalized_delta: i64,
}

struct MergeInput<'a> {
    cached: &'a LexOutput,
    reparsed: &'a RegionOutput,
    source: SanitizedText,
    region: Range<usize>,
    source_delta: i64,
    source_unchanged: bool,
}

fn merge(input: MergeInput<'_>) -> Option<IncrementalOutput> {
    let MergeInput {
        cached,
        reparsed,
        source,
        region,
        source_delta,
        source_unchanged,
    } = input;
    let start = u32::try_from(region.start).ok()?;
    let end = u32::try_from(region.end).ok()?;
    let normalized_start = normalized_offset(cached, start)?;
    let normalized_end = normalized_offset(cached, end)?;
    let normalized_delta = i64::try_from(reparsed.normalized.len()).ok()?
        - i64::from(normalized_end - normalized_start);
    let window = MergeWindow {
        source_start: start,
        source_end: end,
        normalized_start,
        normalized_end,
        source_delta,
        normalized_delta,
    };

    let (store, grafted) = if reparsed.source_nodes.is_empty() {
        (Arc::clone(&cached.store), Vec::new())
    } else {
        let mut store = NodeStore::layered(Arc::clone(&cached.store));
        let grafted = reparsed
            .source_nodes
            .iter()
            .map(|entry| store.graft_node_ref(&reparsed.store, entry.node))
            .collect::<Vec<_>>();
        (Arc::new(store), grafted)
    };
    let normalized = merge_normalized(cached, reparsed, &window)?;
    let source_nodes = merge_source_nodes(cached, reparsed, grafted, &window)?;
    let registry_entries = source_nodes
        .iter()
        .map(|entry| (entry.normalized_offset.get(), entry.node))
        .collect::<Vec<_>>();
    let registry = Registry::from_sorted_slice(&registry_entries);
    let pairs = merge_pairs(&cached.pairs, &reparsed.pairs, &window);
    let container_pairs =
        merge_container_pairs(&cached.container_pairs, &reparsed.container_pairs, &window)?;
    let diagnostics = merge_diagnostics(&cached.diagnostics, &reparsed.diagnostics, &window);
    #[cfg(test)]
    let reused_nodes = cached
        .source_nodes
        .iter()
        .filter(|entry| {
            entry.source_span.end <= window.source_start
                || entry.source_span.start >= window.source_end
        })
        .count();
    #[cfg(test)]
    let reparsed_nodes = reparsed.source_nodes.len();

    Some(IncrementalOutput {
        output: LexOutput::new(
            normalized,
            source,
            source_unchanged,
            registry,
            diagnostics,
            pairs,
            source_nodes,
            container_pairs,
            store,
        ),
        #[cfg(test)]
        reused_nodes,
        #[cfg(test)]
        reparsed_nodes,
    })
}

fn merge_normalized(
    cached: &LexOutput,
    reparsed: &RegionOutput,
    window: &MergeWindow,
) -> Option<String> {
    let mut normalized = String::with_capacity(
        cached
            .normalized
            .len()
            .saturating_add(reparsed.normalized.len()),
    );
    normalized.push_str(cached.normalized.get(..window.normalized_start as usize)?);
    normalized.push_str(&reparsed.normalized);
    normalized.push_str(cached.normalized.get(window.normalized_end as usize..)?);
    Some(normalized)
}

fn merge_source_nodes(
    cached: &LexOutput,
    reparsed: &RegionOutput,
    grafted: Vec<NodeRef>,
    window: &MergeWindow,
) -> Option<Vec<SourceNode>> {
    let mut nodes = Vec::with_capacity(cached.source_nodes.len() + reparsed.source_nodes.len());
    nodes.extend(
        cached
            .source_nodes
            .iter()
            .copied()
            .filter(|entry| entry.source_span.end <= window.source_start),
    );
    nodes.extend(
        reparsed
            .source_nodes
            .iter()
            .zip(grafted)
            .map(|(entry, node)| SourceNode {
                source_span: entry.source_span.shifted(i64::from(window.source_start)),
                normalized_offset: NormalizedOffset::new(
                    entry.normalized_offset.get() + window.normalized_start,
                ),
                node,
            }),
    );
    for entry in cached
        .source_nodes
        .iter()
        .filter(|entry| entry.source_span.start >= window.source_end)
    {
        nodes.push(SourceNode {
            source_span: entry.source_span.shifted(window.source_delta),
            normalized_offset: NormalizedOffset::new(shift_u32(
                entry.normalized_offset.get(),
                window.normalized_delta,
            )?),
            node: entry.node,
        });
    }
    Some(nodes)
}

fn balanced_region(cached: &LexOutput, edit: Range<usize>) -> Option<Range<usize>> {
    if edit.start > edit.end || edit.end > cached.sanitized.len() {
        return None;
    }
    let start = cached.sanitized[..edit.start]
        .match_indices('\n')
        .map(|(offset, _)| offset + 1)
        .rev()
        .find(|&offset| safe_cut(cached, offset))
        .unwrap_or(0);
    let end = cached.sanitized[edit.end..]
        .match_indices('\n')
        .filter_map(|(offset, _)| edit.end.checked_add(offset)?.checked_add(1))
        .find(|&offset| safe_cut(cached, offset))
        .unwrap_or(cached.sanitized.len());
    (start != 0 || end != cached.sanitized.len()).then_some(start..end)
}

fn safe_cut(cached: &LexOutput, offset: usize) -> bool {
    if offset == 0 || offset == cached.sanitized.len() {
        return true;
    }
    if offset < 2 || &cached.sanitized.as_bytes()[offset - 2..offset] != b"\n\n" {
        return false;
    }
    let offset = u32::try_from(offset).expect("source fits parser spans");
    let mut depth = 0_i32;
    let mut warichu_depth = 0_i32;
    for entry in &cached.source_nodes {
        if entry.source_span.start >= offset {
            break;
        }
        if entry.source_span.end > offset {
            return false;
        }
        match entry.node {
            NodeRef::BlockOpen(_) => depth += 1,
            NodeRef::BlockClose(_) => depth = (depth - 1).max(0),
            NodeRef::Inline(Node::Directive(directive))
            | NodeRef::BlockLeaf(Node::Directive(directive)) => match directive.kind {
                DirectiveKind::WarichuOpen => warichu_depth += 1,
                DirectiveKind::WarichuClose => warichu_depth = (warichu_depth - 1).max(0),
                _ => {}
            },
            NodeRef::Inline(_) | NodeRef::BlockLeaf(_) => {}
        }
    }
    depth == 0
        && warichu_depth == 0
        && !cached
            .pairs
            .iter()
            .any(|pair| pair.open.start < offset && pair.close.end > offset)
}

fn balanced_nodes(nodes: &[SourceNode]) -> bool {
    let mut depth = 0_i32;
    let mut warichu_depth = 0_i32;
    for entry in nodes {
        match entry.node {
            NodeRef::BlockOpen(_) => depth += 1,
            NodeRef::BlockClose(_) => {
                depth -= 1;
                if depth < 0 {
                    return false;
                }
            }
            NodeRef::Inline(Node::Directive(directive))
            | NodeRef::BlockLeaf(Node::Directive(directive)) => match directive.kind {
                DirectiveKind::WarichuOpen => warichu_depth += 1,
                DirectiveKind::WarichuClose => {
                    warichu_depth -= 1;
                    if warichu_depth < 0 {
                        return false;
                    }
                }
                _ => {}
            },
            NodeRef::Inline(_) | NodeRef::BlockLeaf(_) => {}
        }
    }
    depth == 0 && warichu_depth == 0
}

fn normalized_offset(cached: &LexOutput, source_offset: u32) -> Option<u32> {
    let mut drift = 0_i64;
    for entry in cached
        .source_nodes
        .iter()
        .take_while(|entry| entry.source_span.end <= source_offset)
    {
        drift +=
            i64::from(standalone_padding(entry.node)) * 2 + 3 - i64::from(entry.source_span.len());
    }
    let offset = shift_u32(source_offset, drift)?;
    cached
        .normalized
        .is_char_boundary(offset as usize)
        .then_some(offset)
}

fn standalone_padding(node: NodeRef) -> u32 {
    match node {
        NodeRef::BlockLeaf(_) => 2,
        NodeRef::BlockOpen(format) => u32::from(!format.is_inline()) * 2,
        NodeRef::BlockClose(close) => u32::from(!close.is_inline()) * 2,
        NodeRef::Inline(_) => 0,
    }
}

fn merge_pairs(cached: &[PairLink], reparsed: &[PairLink], window: &MergeWindow) -> Vec<PairLink> {
    cached
        .iter()
        .copied()
        .filter(|pair| pair.close.end <= window.source_start)
        .chain(reparsed.iter().map(|pair| {
            PairLink::new(
                pair.kind,
                pair.open.shifted(i64::from(window.source_start)),
                pair.close.shifted(i64::from(window.source_start)),
            )
        }))
        .chain(
            cached
                .iter()
                .filter(|pair| pair.open.start >= window.source_end)
                .map(|pair| {
                    PairLink::new(
                        pair.kind,
                        pair.open.shifted(window.source_delta),
                        pair.close.shifted(window.source_delta),
                    )
                }),
        )
        .collect()
}

fn merge_container_pairs(
    cached: &[ContainerPair],
    reparsed: &[ContainerPair],
    window: &MergeWindow,
) -> Option<Vec<ContainerPair>> {
    cached
        .iter()
        .copied()
        .filter(|pair| pair.close.get() < window.normalized_start)
        .map(Some)
        .chain(reparsed.iter().map(|pair| {
            Some(ContainerPair {
                kind: pair.kind,
                open: NormalizedOffset::new(pair.open.get() + window.normalized_start),
                close: NormalizedOffset::new(pair.close.get() + window.normalized_start),
            })
        }))
        .chain(
            cached
                .iter()
                .filter(|pair| pair.open.get() >= window.normalized_end)
                .map(|pair| {
                    Some(ContainerPair {
                        kind: pair.kind,
                        open: NormalizedOffset::new(shift_u32(
                            pair.open.get(),
                            window.normalized_delta,
                        )?),
                        close: NormalizedOffset::new(shift_u32(
                            pair.close.get(),
                            window.normalized_delta,
                        )?),
                    })
                }),
        )
        .collect()
}

fn merge_diagnostics(
    cached: &[Diagnostic],
    reparsed: &[Diagnostic],
    window: &MergeWindow,
) -> Vec<Diagnostic> {
    cached
        .iter()
        .filter(|diagnostic| diagnostic.span().end <= window.source_start)
        .cloned()
        .chain(
            reparsed
                .iter()
                .cloned()
                .map(|diagnostic| diagnostic.shifted(i64::from(window.source_start))),
        )
        .chain(
            cached
                .iter()
                .filter(|diagnostic| diagnostic.span().start >= window.source_end)
                .cloned()
                .map(|diagnostic| diagnostic.shifted(window.source_delta)),
        )
        .collect()
}

fn shift_u32(value: u32, delta: i64) -> Option<u32> {
    u32::try_from(i64::from(value) + delta).ok()
}

fn shift_usize(value: usize, delta: i64) -> Option<usize> {
    usize::try_from(i64::try_from(value).ok()? + delta).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::{render_html, serialize};
    use crate::spec::{PairKind, Span};
    use crate::syntax::{RegionClose, RegionFormat};

    fn region(source: &str) -> RegionOutput {
        lex_region(source, 0..source.len()).expect("valid region")
    }

    fn assert_incremental_matches_full(source: &str, range: Range<usize>, replacement: &str) {
        let cached = lex(source);
        let mut edited = source.to_owned();
        edited.replace_range(range.clone(), replacement);
        let incremental =
            reparse(&cached, edited.as_str(), range, true).expect("incremental parse");
        let full = lex(&edited);
        assert_eq!(incremental.output.sanitized, full.sanitized);
        assert_eq!(incremental.output.normalized, full.normalized);
        assert_eq!(
            incremental
                .output
                .source_nodes
                .iter()
                .map(|entry| (
                    entry.source_span,
                    entry.normalized_offset,
                    entry.node.kind()
                ))
                .collect::<Vec<_>>(),
            full.source_nodes
                .iter()
                .map(|entry| (
                    entry.source_span,
                    entry.normalized_offset,
                    entry.node.kind()
                ))
                .collect::<Vec<_>>()
        );
        assert_eq!(incremental.output.pairs, full.pairs);
        assert_eq!(incremental.output.container_pairs, full.container_pairs);
        assert_eq!(serialize(&incremental.output), serialize(&full));
        assert_eq!(render_html(&incremental.output), render_html(&full));
        assert_eq!(
            incremental
                .output
                .diagnostics
                .iter()
                .map(|diagnostic| (diagnostic.code(), diagnostic.span()))
                .collect::<Vec<_>>(),
            full.diagnostics
                .iter()
                .map(|diagnostic| (diagnostic.code(), diagnostic.span()))
                .collect::<Vec<_>>()
        );
        assert!(incremental.reused_nodes > 0);
        assert!(incremental.reparsed_nodes < full.source_nodes.len());
        assert_eq!(
            incremental.reused_nodes + incremental.reparsed_nodes,
            full.source_nodes.len()
        );
    }

    #[test]
    fn reparses_only_the_edited_paragraph() {
        let source = "｜前《まえ》\n\n｜中《なか》\n\n｜後《あと》";
        let cached = lex(source);
        let start = source.find("なか").expect("reading");
        assert_incremental_matches_full(source, start..start + "なか".len(), "ちゅう");
        let edited = source.replacen("なか", "ちゅう", 1);
        let incremental = reparse(&cached, edited.as_str(), start..start + "なか".len(), true)
            .expect("incremental parse");
        assert!(incremental.output.store.inherits_from(&cached.store));
    }

    #[test]
    fn shifts_suffix_offsets_after_growth() {
        let source = "｜前《まえ》\n\n中央\n\n｜後《あと》";
        let start = source.find("中央").expect("middle");
        assert_incremental_matches_full(source, start..start + "中央".len(), "中央の段落");
    }

    #[test]
    fn plain_paragraph_edit_shares_the_node_store() {
        let source = "｜前《まえ》\n\n中央\n\n｜後《あと》";
        let cached = lex(source);
        let start = source.find("中央").expect("middle");
        let mut edited = source.to_owned();
        edited.insert(start, 'x');
        let incremental =
            reparse(&cached, edited.as_str(), start..start, true).expect("incremental parse");
        assert!(Arc::ptr_eq(&cached.store, &incremental.output.store));
    }

    #[test]
    fn forward_references_see_the_unchanged_prefix() {
        let source = "里見｜前《まえ》\n\n里見の［＃「里見」に丸傍点］\n\n｜後《あと》";
        let start = source.find('の').expect("particle");
        assert_incremental_matches_full(source, start..start + 'の'.len_utf8(), "という");

        let source = "語句｜前《まえ》\n\n説明［＃「語句」は太字］\n\n｜後《あと》";
        let start = source.find("説明").expect("description");
        assert_incremental_matches_full(source, start..start + "説明".len(), "詳しい説明");
    }

    #[test]
    fn ruby_format_dependency_uses_full_parse() {
        let source = "｜語句《ごく》\n\n説明［＃「語句」は太字］\n\n｜後《あと》";
        let cached = lex(source);
        let start = source.find("説明").expect("description");
        let mut edited = source.to_owned();
        edited.replace_range(start..start + "説明".len(), "詳しい説明");
        assert!(reparse(&cached, edited.as_str(), start..start + "説明".len(), true).is_none());
    }

    #[test]
    fn ruby_format_dependency_requires_a_later_referenced_format() {
        let referenced = lex("｜語句《ごく》\n\n説明［＃「語句」は太字］");
        let span = referenced
            .source_nodes
            .iter()
            .find_map(|entry| {
                matches!(
                    entry.node,
                    NodeRef::Inline(Node::Format(format))
                        if format.origin == ForwardOrigin::Referenced
                )
                .then_some(entry.source_span)
            })
            .expect("referenced format");
        assert!(has_ruby_format_dependency(
            &referenced,
            span.end as usize - 1
        ));
        assert!(!has_ruby_format_dependency(&referenced, span.end as usize));

        let reclaimed = lex("｜語句《ごく》\n語句［＃「語句」は太字］");
        assert!(!has_ruby_format_dependency(&reclaimed, 0));
    }

    #[test]
    fn introduced_ruby_dependencies_respect_region_boundaries_and_origin() {
        let source = "｜語句《ごく》\n\n説明［＃「語句」は太字］";
        let referenced = lex(source);
        let referenced_region = region(source);
        assert!(introduces_ruby_format_dependency(
            &referenced,
            &referenced_region,
            usize::MAX,
            usize::MAX,
        ));

        let reclaimed = region("｜語句《ごく》\n語句［＃「語句」は太字］");
        assert!(!introduces_ruby_format_dependency(
            &referenced,
            &reclaimed,
            usize::MAX,
            usize::MAX,
        ));

        let suffix = lex("語句\n\n説明［＃「語句」は太字］");
        let format_start = suffix
            .source_nodes
            .iter()
            .find_map(|entry| {
                matches!(
                    entry.node,
                    NodeRef::Inline(Node::Format(format))
                        if format.origin == ForwardOrigin::Referenced
                )
                .then_some(entry.source_span.start as usize)
            })
            .expect("referenced suffix format");
        let ruby_region = region("｜新《しん》");
        assert!(introduces_ruby_format_dependency(
            &suffix,
            &ruby_region,
            0,
            format_start,
        ));
        assert!(!introduces_ruby_format_dependency(
            &suffix,
            &ruby_region,
            0,
            format_start + 1,
        ));
    }

    #[test]
    fn balanced_region_validates_edits_and_selects_exact_windows() {
        let source = "first\n\nmiddle\n\nlast";
        let cached = lex(source);
        let middle = source.find("middle").expect("middle paragraph");
        assert_eq!(
            balanced_region(&cached, middle + 1..middle + 1),
            Some(7..15)
        );
        assert_eq!(balanced_region(&cached, 1..1), Some(0..7));
        assert_eq!(
            balanced_region(&cached, source.len() - 1..source.len()),
            Some(15..source.len())
        );
        let inverted = Range { start: 2, end: 1 };
        assert_eq!(balanced_region(&cached, inverted), None);
        assert_eq!(balanced_region(&cached, 0..source.len() + 1), None);
    }

    #[test]
    fn safe_cut_accepts_boundaries_and_rejects_non_paragraph_offsets() {
        let source = "alpha\n\nbeta";
        let cached = lex(source);
        assert!(safe_cut(&cached, 0));
        assert!(safe_cut(&cached, source.len()));
        assert!(safe_cut(&cached, "alpha\n\n".len()));
        assert!(!safe_cut(&cached, "alpha\n".len()));
        assert!(!safe_cut(&cached, 1));

        let leading_empty = lex("\n\nrest");
        assert!(safe_cut(&leading_empty, 2));

        let node_source = "｜字《じ》\n\nrest";
        let cut = node_source.find("rest").expect("line boundary");
        let mut crossing = lex(node_source);
        crossing.source_nodes[0].source_span =
            Span::new(0, u32::try_from(node_source.len()).expect("source length"));
        assert!(!safe_cut(&crossing, cut));

        let mut ending = lex(node_source);
        ending.source_nodes[0].source_span = Span::new(0, u32::try_from(cut).expect("cut"));
        assert!(safe_cut(&ending, cut));
    }

    #[test]
    fn safe_cut_preserves_nested_block_depth() {
        let source = "［＃ここから2字下げ］\n\
                      ［＃ここから3字下げ］\n\
                      body\n\
                      ［＃ここで字下げ終わり］\n\
                      \n\
                      rest\n\
                      ［＃ここで字下げ終わり］";
        let cached = lex(source);
        let cut = source.find("rest").expect("remaining block content");
        let cut_u32 = u32::try_from(cut).expect("test source offset fits u32");
        assert_eq!(
            cached.source_nodes[..]
                .iter()
                .take_while(|entry| entry.source_span.start < cut_u32)
                .filter(|entry| matches!(entry.node, NodeRef::BlockOpen(_)))
                .count(),
            2
        );
        assert!(!safe_cut(&cached, cut));

        let closed = "［＃ここから2字下げ］\nbody\n［＃ここで字下げ終わり］\n\nrest";
        let cached = lex(closed);
        assert!(safe_cut(
            &cached,
            closed.find("rest").expect("closed block boundary")
        ));
    }

    #[test]
    fn safe_cut_tracks_warichu_depth() {
        let closed = "［＃割り注］\n\nbody\n［＃割り注終わり］\n\nrest";
        let cached = lex(closed);
        assert!(!safe_cut(
            &cached,
            closed.find("body").expect("open warichu boundary")
        ));
        assert!(safe_cut(
            &cached,
            closed.find("rest").expect("closed warichu boundary")
        ));

        let nested = "［＃割り注］\n\
                      ［＃割り注］\n\
                      body\n\
                      ［＃割り注終わり］\n\
                      \n\
                      rest\n\
                      ［＃割り注終わり］";
        let cached = lex(nested);
        let cut = nested.find("rest").expect("remaining warichu content");
        let cut_u32 = u32::try_from(cut).expect("test source offset fits u32");
        assert_eq!(
            cached.source_nodes[..]
                .iter()
                .take_while(|entry| entry.source_span.start < cut_u32)
                .filter(|entry| {
                    matches!(
                        entry.node,
                        NodeRef::Inline(Node::Directive(directive))
                            if directive.kind == DirectiveKind::WarichuOpen
                    )
                })
                .count(),
            2
        );
        assert!(!safe_cut(&cached, cut));
    }

    #[test]
    fn safe_cut_rejects_only_pairs_crossing_the_boundary() {
        let source = "a\n\nb";
        let cut = "a\n\n".len();
        let cut_u32 = u32::try_from(cut).expect("test source offset fits u32");
        let mut cached = lex(source);
        cached.pairs = vec![PairLink::new(
            PairKind::Bracket,
            Span::new(0, 1),
            Span::new(cut_u32, cut_u32 + 1),
        )];
        assert!(!safe_cut(&cached, cut));

        cached.pairs[0].open = Span::new(
            cut_u32,
            cut_u32.checked_add(1).expect("test source offset fits u32"),
        );
        assert!(safe_cut(&cached, cut));

        cached.pairs[0].open = Span::new(0, 1);
        cached.pairs[0].close = Span::new(1, cut_u32);
        assert!(safe_cut(&cached, cut));
    }

    #[test]
    fn balanced_nodes_requires_closed_block_regions() {
        let closed = lex("［＃ここから2字下げ］\nbody\n［＃ここで字下げ終わり］");
        assert!(balanced_nodes(&closed.source_nodes));

        let nested = lex("［＃ここから2字下げ］\n\
             ［＃ここから3字下げ］\n\
             body\n\
             ［＃ここで字下げ終わり］\n\
             ［＃ここで字下げ終わり］");
        assert!(balanced_nodes(&nested.source_nodes));

        let open = lex("［＃ここから2字下げ］\nbody");
        assert!(!balanced_nodes(&open.source_nodes));

        let close = lex("body\n［＃ここで字下げ終わり］");
        assert!(!balanced_nodes(&close.source_nodes));

        let nested_warichu = lex("［＃割り注］\n\
             ［＃割り注］\n\
             body\n\
             ［＃割り注終わり］\n\
             ［＃割り注終わり］");
        assert!(balanced_nodes(&nested_warichu.source_nodes));
    }

    #[test]
    fn standalone_padding_distinguishes_inline_and_block_nodes() {
        let block = lex("［＃ここから2字下げ］\nbody\n［＃ここで字下げ終わり］");
        let block_open = block
            .source_nodes
            .iter()
            .find(|entry| matches!(entry.node, NodeRef::BlockOpen(_)))
            .expect("block open");
        let block_close = block
            .source_nodes
            .iter()
            .find(|entry| matches!(entry.node, NodeRef::BlockClose(_)))
            .expect("block close");
        assert_eq!(standalone_padding(block_open.node), 2);
        assert_eq!(standalone_padding(block_close.node), 2);

        let inline_open = NodeRef::BlockOpen(RegionFormat::Bold { padded: false });
        let inline_close = NodeRef::BlockClose(RegionClose::Bold { padded: false });
        assert_eq!(standalone_padding(inline_open), 0);
        assert_eq!(standalone_padding(inline_close), 0);

        let leaf = lex("［＃改ページ］");
        assert_eq!(
            standalone_padding(
                leaf.source_nodes
                    .iter()
                    .find(|entry| matches!(entry.node, NodeRef::BlockLeaf(_)))
                    .expect("block leaf")
                    .node
            ),
            2
        );
    }

    #[test]
    fn merge_container_pairs_replaces_window_and_shifts_suffix() {
        let pair = |open, close| ContainerPair {
            kind: RegionFormat::Bold { padded: true },
            open: NormalizedOffset::new(open),
            close: NormalizedOffset::new(close),
        };
        let cached = vec![pair(1, 9), pair(2, 10), pair(5, 25), pair(20, 22)];
        let reparsed = vec![pair(2, 4)];
        let window = MergeWindow {
            source_start: 0,
            source_end: 0,
            normalized_start: 10,
            normalized_end: 20,
            source_delta: 0,
            normalized_delta: 5,
        };
        assert_eq!(
            merge_container_pairs(&cached, &reparsed, &window),
            Some(vec![pair(1, 9), pair(12, 14), pair(25, 27)])
        );
    }

    #[test]
    fn region_covers_open_warichu_state() {
        let source = "｜前《まえ》\n\n［＃割り注］\n中\n［＃改ページ］\n後\n［＃割り注終わり］\n\n｜末《すえ》";
        let start = source.find('中').expect("middle");
        assert_incremental_matches_full(source, start..start + '中'.len_utf8(), "中央");
    }

    #[test]
    fn unbalanced_warichu_edit_uses_full_parse() {
        let source =
            "｜前《まえ》\n\n［＃割り注］本文［＃割り注終わり］\n\n［＃改ページ］\n\n｜末《すえ》";
        let cached = lex(source);
        let start = source.find("終わり").expect("close");
        let mut edited = source.to_owned();
        edited.insert(start, 'x');
        assert!(reparse(&cached, edited.as_str(), start..start, true).is_none());
    }
}
