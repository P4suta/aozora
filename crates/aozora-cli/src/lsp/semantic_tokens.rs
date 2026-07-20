//! `textDocument/semanticTokens/full` from the core semantic snapshot.

use aozora::{NodeKind, Snapshot, Span};
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
    for node in snapshot.nodes() {
        if node.kind() == NodeKind::Gaiji {
            tokens.push(token_at(node.span(), source, &line_index, TT_GAIJI));
        }
    }
    for ruby in snapshot.rubies() {
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
            ruby_start
                .saturating_add(u32::try_from(reading_start + reading.len()).unwrap_or(u32::MAX)),
        );
        tokens.push(token_at(base_span, source, &line_index, TT_RUBY_BASE));
        tokens.push(token_at(reading_span, source, &line_index, TT_RUBY_READING));
    }
    tokens.sort_unstable_by_key(|token| (token.start_byte, token.token_type));
    SemanticTokens {
        result_id: None,
        data: encode_delta(&tokens),
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
}
