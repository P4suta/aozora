//! `textDocument/semanticTokens/full` from the core semantic snapshot.

use aozora::{GaijiResolution, PairKind, PairLink, Snapshot, Span};
use tower_lsp::lsp_types::{SemanticToken, SemanticTokenType, SemanticTokens};

use crate::lsp::line_index::LineIndex;

/// LSP semantic-token-type legend. Index = `tokenType` field in the
/// emitted tuples; the values must match the order published in
/// `ServerCapabilities::semantic_tokens_provider`.
#[must_use]
pub(super) fn legend() -> Vec<SemanticTokenType> {
    vec![
        SemanticTokenType::MACRO,  // 0 → gaiji
        SemanticTokenType::ENUM,   // 1 → ruby base
        SemanticTokenType::STRING, // 2 → ruby reading
    ]
}

const TT_GAIJI: u32 = 0;
const TT_RUBY_BASE: u32 = 1;
const TT_RUBY_READING: u32 = 2;

#[must_use]
pub(super) fn semantic_tokens_full(snapshot: &Snapshot) -> SemanticTokens {
    let source = snapshot.source();
    let line_index = LineIndex::new(source);
    let mut tokens: Vec<RawToken> = Vec::new();
    let gaiji_spans = snapshot
        .gaiji_resolutions()
        .iter()
        .map(GaijiResolution::span)
        .collect::<Vec<_>>();
    let mut content_exclusions = gaiji_spans.clone();
    content_exclusions.extend(
        snapshot
            .pairs()
            .iter()
            .filter(|pair| pair.kind == PairKind::Bracket)
            .map(|pair| Span::new(pair.open.start, pair.close.end)),
    );
    content_exclusions.sort_unstable_by_key(|span| (span.start, span.end));
    let pair_index = PairIndex::new(snapshot.pairs());
    {
        let mut sink = TokenSink {
            tokens: &mut tokens,
            source,
            line_index: &line_index,
            exclusions: &content_exclusions,
        };
        for &span in &gaiji_spans {
            sink.push_lines(span, TT_GAIJI);
        }
        for ruby in snapshot.rubies() {
            if let Some((base_span, reading_span)) = pair_index.ruby_spans(ruby.span(), source) {
                sink.push_fragments(base_span, TT_RUBY_BASE);
                sink.push_fragments(reading_span, TT_RUBY_READING);
                continue;
            }
            let Some(full) = snapshot.slice(ruby.span()) else {
                continue;
            };
            let Some(base) = ruby.base() else {
                continue;
            };
            let Some(reading) = ruby.reading() else {
                continue;
            };
            let Some(base_start) = full.find(base) else {
                continue;
            };
            let after_base = base_start + base.len();
            let Some(reading_start) = full[after_base..]
                .find(reading)
                .map(|offset| after_base + offset)
            else {
                continue;
            };
            let ruby_start = ruby.span().start;
            let base_span = Span::new(
                ruby_start.saturating_add(u32::try_from(base_start).unwrap_or(u32::MAX)),
                ruby_start.saturating_add(u32::try_from(after_base).unwrap_or(u32::MAX)),
            );
            let reading_span = Span::new(
                ruby_start.saturating_add(u32::try_from(reading_start).unwrap_or(u32::MAX)),
                ruby_start.saturating_add(
                    u32::try_from(reading_start + reading.len()).unwrap_or(u32::MAX),
                ),
            );
            sink.push_fragments(base_span, TT_RUBY_BASE);
            sink.push_fragments(reading_span, TT_RUBY_READING);
        }
    }
    tokens.sort_unstable_by_key(|token| (token.start_byte, token.token_type));
    SemanticTokens {
        result_id: None,
        data: encode_delta(&tokens),
    }
}

struct PairIndex {
    ruby: Vec<PairLink>,
    bracket: Vec<PairLink>,
    quote: Vec<PairLink>,
}

impl PairIndex {
    fn new(pairs: &[PairLink]) -> Self {
        let mut ruby = pairs
            .iter()
            .filter(|pair| pair.kind == PairKind::Ruby)
            .copied()
            .collect::<Vec<_>>();
        let mut bracket = pairs
            .iter()
            .filter(|pair| pair.kind == PairKind::Bracket)
            .copied()
            .collect::<Vec<_>>();
        let mut quote = pairs
            .iter()
            .filter(|pair| pair.kind == PairKind::Quote)
            .copied()
            .collect::<Vec<_>>();
        ruby.sort_unstable_by_key(|pair| pair.close.end);
        bracket.sort_unstable_by_key(|pair| pair.close.end);
        quote.sort_unstable_by_key(|pair| pair.open.start);
        Self {
            ruby,
            bracket,
            quote,
        }
    }

    fn ruby_spans(&self, span: Span, source: &str) -> Option<(Span, Span)> {
        if let Some(pair) = pair_ending_at(&self.ruby, span.end)
            && span.start <= pair.open.start
        {
            let base_start = strip_base_markers(span.start, pair.open.start, source);
            return Some((
                Span::new(base_start, pair.open.start),
                Span::new(pair.open.end, pair.close.start),
            ));
        }
        let outer = pair_ending_at(&self.bracket, span.end)?;
        if outer.open.start < span.start {
            return None;
        }
        let quote_start = self
            .quote
            .partition_point(|pair| pair.open.start < outer.open.end);
        let reading = self.quote[quote_start..]
            .iter()
            .take_while(|pair| pair.open.start < outer.close.start)
            .find(|pair| {
                pair.close.end <= outer.close.start
                    && source.get(pair.close.end as usize..outer.close.start as usize)
                        == Some("のルビ")
            })?;
        Some((
            Span::new(span.start, outer.open.start),
            Span::new(reading.open.end, reading.close.start),
        ))
    }
}

fn pair_ending_at(pairs: &[PairLink], end: u32) -> Option<PairLink> {
    pairs
        .binary_search_by_key(&end, |pair| pair.close.end)
        .ok()
        .map(|index| pairs[index])
}

fn strip_base_markers(start: u32, end: u32, source: &str) -> u32 {
    let text = &source[start as usize..end as usize];
    let stripped = text.trim_start_matches('｜');
    start.saturating_add(u32::try_from(text.len() - stripped.len()).unwrap_or(u32::MAX))
}

struct TokenSink<'a> {
    tokens: &'a mut Vec<RawToken>,
    source: &'a str,
    line_index: &'a LineIndex,
    exclusions: &'a [Span],
}

impl TokenSink<'_> {
    fn push_fragments(&mut self, span: Span, token_type: u32) {
        let mut cursor = span.start;
        let first = self
            .exclusions
            .partition_point(|exclusion| exclusion.start < span.start);
        for &exclusion in &self.exclusions[first..] {
            if exclusion.start >= span.end {
                break;
            }
            if exclusion.end > span.end {
                continue;
            }
            if exclusion.end <= cursor {
                continue;
            }
            let exclusion_start = exclusion.start.max(span.start);
            if cursor < exclusion_start {
                self.push_lines(Span::new(cursor, exclusion_start), token_type);
            }
            cursor = cursor.max(exclusion.end.min(span.end));
        }
        if cursor < span.end {
            self.push_lines(Span::new(cursor, span.end), token_type);
        }
    }

    fn push_lines(&mut self, span: Span, token_type: u32) {
        let text = &self.source[span.start as usize..span.end as usize];
        let mut line_start = span.start;
        for (offset, ch) in text.char_indices() {
            if ch != '\n' {
                continue;
            }
            let line_end = span
                .start
                .saturating_add(u32::try_from(offset).unwrap_or(u32::MAX));
            if line_start < line_end {
                self.tokens.push(token_at(
                    Span::new(line_start, line_end),
                    self.source,
                    self.line_index,
                    token_type,
                ));
            }
            line_start = line_end.saturating_add(1);
        }
        if line_start < span.end {
            self.tokens.push(token_at(
                Span::new(line_start, span.end),
                self.source,
                self.line_index,
                token_type,
            ));
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct RawToken {
    start_byte: u32,
    line: u32,
    start_char: u32,
    length: u32,
    token_type: u32,
}

fn token_at(span: Span, source: &str, line_index: &LineIndex, token_type: u32) -> RawToken {
    let start = line_index.position(source, span.start as usize);
    let end = line_index.position(source, span.end as usize);
    debug_assert_eq!(start.line, end.line, "semantic tokens must be single-line");
    RawToken {
        start_byte: span.start,
        line: start.line,
        start_char: start.character,
        length: end.character.saturating_sub(start.character),
        token_type,
    }
}

fn encode_delta(raw: &[RawToken]) -> Vec<SemanticToken> {
    let mut out: Vec<SemanticToken> = Vec::with_capacity(raw.len());
    let mut prev_line: u32 = 0;
    let mut prev_start: u32 = 0;
    for tok in raw {
        let delta_line = tok.line.saturating_sub(prev_line);
        let delta_start = if delta_line == 0 {
            tok.start_char.saturating_sub(prev_start)
        } else {
            tok.start_char
        };
        out.push(SemanticToken {
            delta_line,
            delta_start,
            length: tok.length,
            token_type: tok.token_type,
            token_modifiers_bitset: 0,
        });
        prev_line = tok.line;
        prev_start = tok.start_char;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tokens_for(src: &str) -> Vec<SemanticToken> {
        let document = aozora::parse(src).expect("test source fits parser limits");
        semantic_tokens_full(&document.snapshot()).data
    }

    fn assert_non_overlapping(tokens: &[SemanticToken]) {
        let mut line = 0;
        let mut start = 0;
        let mut previous_line = 0;
        let mut previous_end = 0;
        for token in tokens {
            line += token.delta_line;
            start = if token.delta_line == 0 {
                start + token.delta_start
            } else {
                token.delta_start
            };
            if line == previous_line {
                assert!(
                    start >= previous_end,
                    "overlapping semantic tokens: {tokens:?}"
                );
            }
            previous_line = line;
            previous_end = start + token.length;
        }
    }

    #[test]
    fn legend_is_stable_index_order() {
        let l = legend();
        assert_eq!(l[TT_GAIJI as usize], SemanticTokenType::MACRO);
        assert_eq!(l[TT_RUBY_BASE as usize], SemanticTokenType::ENUM);
        assert_eq!(l[TT_RUBY_READING as usize], SemanticTokenType::STRING);
    }

    #[test]
    fn empty_doc_yields_no_tokens() {
        let tokens = tokens_for("");
        assert!(tokens.is_empty());
    }

    #[test]
    fn plain_text_yields_no_tokens() {
        let tokens = tokens_for("ただの文章\n二行目\n");
        assert!(tokens.is_empty());
    }

    #[test]
    fn gaiji_emits_one_macro_token() {
        let src = "※［＃「desc」、第3水準1-85-54］";
        let tokens = tokens_for(src);
        assert_eq!(tokens.len(), 1);
        assert_eq!(tokens[0].token_type, TT_GAIJI);
        assert_eq!(tokens[0].delta_line, 0);
        assert_eq!(tokens[0].delta_start, 0);
    }

    #[test]
    fn explicit_ruby_emits_base_then_reading() {
        let src = "｜青空《あおぞら》";
        let tokens = tokens_for(src);
        assert_eq!(tokens.len(), 2, "{tokens:?}");
        assert_eq!(tokens[0].token_type, TT_RUBY_BASE);
        assert_eq!(tokens[1].token_type, TT_RUBY_READING);
        // Reading comes after base on the same line.
        assert_eq!(tokens[1].delta_line, 0);
        assert!(tokens[1].delta_start > 0);
    }

    #[test]
    fn implicit_ruby_emits_base_then_reading() {
        let src = "青空《あおぞら》";
        let tokens = tokens_for(src);
        assert_eq!(tokens.len(), 2, "{tokens:?}");
        assert_eq!(tokens[0].token_type, TT_RUBY_BASE);
        assert_eq!(tokens[1].token_type, TT_RUBY_READING);
    }

    #[test]
    fn delta_encoding_resets_on_new_line() {
        let src = "｜青空《あおぞら》\n｜白雲《はくうん》";
        let tokens = tokens_for(src);
        // Tokens: base1, reading1, base2, reading2
        assert_eq!(tokens.len(), 4);
        // 3rd token (base2) is on the next line, delta_line == 1.
        // delta_start is absolute when delta_line > 0; tree-sitter
        // places `ruby_base_explicit`'s start *after* the `｜` so
        // the absolute char position is 1 (｜ is one UTF-16 unit).
        assert_eq!(tokens[2].delta_line, 1);
        assert_eq!(tokens[2].delta_start, 1);
    }

    #[test]
    fn multiple_gaiji_in_source_order() {
        let src = "※［＃「a」、X］\n※［＃「b」、Y］";
        let tokens = tokens_for(src);
        assert_eq!(tokens.len(), 2);
        assert!(tokens.iter().all(|t| t.token_type == TT_GAIJI));
        assert_eq!(tokens[1].delta_line, 1);
    }

    #[test]
    fn gaiji_only_ruby_keeps_gaiji_and_reading_tokens() {
        let src = "※［＃「特のへん＋廴＋聿」、第3水準1-87-71］《かん》";
        let tokens = tokens_for(src);
        assert_eq!(
            tokens
                .iter()
                .map(|token| token.token_type)
                .collect::<Vec<_>>(),
            vec![TT_GAIJI, TT_RUBY_READING]
        );
    }

    #[test]
    fn mixed_gaiji_ruby_partitions_base_without_losing_reading() {
        let src = "※［＃「特のへん＋廴＋聿」、第3水準1-87-71］陀多《かんだた》";
        let tokens = tokens_for(src);
        assert_eq!(
            tokens
                .iter()
                .map(|token| token.token_type)
                .collect::<Vec<_>>(),
            vec![TT_GAIJI, TT_RUBY_BASE, TT_RUBY_READING]
        );
    }

    #[test]
    fn gaiji_in_reading_partitions_the_reading_tokens() {
        let src = "｜日本《に※［＃「特のへん＋廴＋聿」、第3水準1-87-71］ん》";
        let tokens = tokens_for(src);
        assert_eq!(
            tokens
                .iter()
                .map(|token| token.token_type)
                .collect::<Vec<_>>(),
            vec![TT_RUBY_BASE, TT_RUBY_READING, TT_GAIJI, TT_RUBY_READING]
        );
        assert_non_overlapping(&tokens);
    }

    #[test]
    fn directive_in_reading_keeps_surrounding_reading_tokens() {
        let src = "日本《に［＃ママ］ほん》";
        let tokens = tokens_for(src);
        assert_eq!(
            tokens
                .iter()
                .map(|token| token.token_type)
                .collect::<Vec<_>>(),
            vec![TT_RUBY_BASE, TT_RUBY_READING, TT_RUBY_READING]
        );
        assert_non_overlapping(&tokens);
    }

    #[test]
    fn directives_in_explicit_base_do_not_overlap_base_tokens() {
        let cases: [(&str, &[u32]); 3] = [
            (
                "｜瑞岩東畔命［＃二］軽舟［＃一］《ずいがんとうはんめいずけいしうを》",
                &[TT_RUBY_BASE, TT_RUBY_BASE, TT_RUBY_READING],
            ),
            (
                "｜磐田［＃底本では「盤田」と誤記］《いわた》",
                &[TT_RUBY_BASE, TT_RUBY_READING],
            ),
            (
                "｜瀕［＃「瀕」は太字］《ほとり》",
                &[TT_RUBY_BASE, TT_RUBY_READING],
            ),
        ];
        for (source, expected) in cases {
            let tokens = tokens_for(source);
            assert_eq!(
                tokens
                    .iter()
                    .map(|token| token.token_type)
                    .collect::<Vec<_>>(),
                expected
            );
            assert_non_overlapping(&tokens);
        }
    }

    #[test]
    fn left_ruby_with_gaiji_base_uses_source_pairs() {
        let gaiji = "※［＃「特のへん＋廴＋聿」、第3水準1-87-71］";
        let src = format!("{gaiji}陀［＃「{gaiji}陀」の左に「さい」のルビ］");
        let tokens = tokens_for(&src);
        assert_eq!(
            tokens
                .iter()
                .map(|token| token.token_type)
                .collect::<Vec<_>>(),
            vec![TT_GAIJI, TT_RUBY_BASE, TT_GAIJI, TT_RUBY_READING]
        );
        assert_non_overlapping(&tokens);
    }

    #[test]
    fn left_ruby_with_gaiji_reading_uses_source_pairs() {
        let src = "未［＃「未」の左に「さ※［＃「特のへん＋廴＋聿」、第3水準1-87-71］い」のルビ］";
        let tokens = tokens_for(src);
        assert_eq!(
            tokens
                .iter()
                .map(|token| token.token_type)
                .collect::<Vec<_>>(),
            vec![TT_RUBY_BASE, TT_RUBY_READING, TT_GAIJI, TT_RUBY_READING]
        );
        assert_non_overlapping(&tokens);
    }
}
