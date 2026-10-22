use crate::i18n::{self as i18n, LanguageIdentifier};
use tower_lsp::lsp_types::{DocumentSymbol, Position, Range, SymbolKind};

#[must_use]
pub(super) fn document_symbols(source: &str, lang: &LanguageIdentifier) -> Vec<DocumentSymbol> {
    let untitled = untitled_placeholder(lang);
    let mut flat: Vec<(HeadingLevel, DocumentSymbol)> = Vec::new();
    for (line_idx, line) in source.lines().enumerate() {
        let Some(level) = heading_level_in(line) else {
            continue;
        };
        let line_idx = u32::try_from(line_idx).unwrap_or(u32::MAX);
        let title = extract_title(line, &untitled);
        let symbol = build_symbol(line_idx, line, level, title, &untitled);
        flat.push((level, symbol));
    }
    nest_by_level(flat)
}

fn heading_level_in(line: &str) -> Option<HeadingLevel> {
    if !line.contains("見出し") {
        return None;
    }
    if line.contains("大見出し") {
        Some(HeadingLevel::Major)
    } else if line.contains("中見出し") {
        Some(HeadingLevel::Middle)
    } else if line.contains("小見出し") {
        Some(HeadingLevel::Minor)
    } else {
        Some(HeadingLevel::Middle)
    }
}

fn untitled_placeholder(lang: &LanguageIdentifier) -> String {
    i18n::t(lang, "lsp-outline-untitled")
}

fn extract_title(line: &str, untitled: &str) -> String {
    if let Some(quote_start) = line.find('「') {
        let after = &line[quote_start + '「'.len_utf8()..];
        if let Some(quote_end) = after.find('」') {
            let body = after[..quote_end].trim();
            if !body.is_empty() {
                return body.to_owned();
            }
        }
    }
    if let Some(close_idx) = line.find('］') {
        let after = &line[close_idx + '］'.len_utf8()..];
        let trimmed = after.find('［').map_or(after, |stop| &after[..stop]).trim();
        if !trimmed.is_empty() {
            return trimmed.to_owned();
        }
    }
    untitled.to_owned()
}

#[expect(
    clippy::too_many_arguments,
    reason = "the symbol's line/level/title data plus the localized untitled \
              fallback for an empty title; untitled is cross-cutting, not a \
              data param to bundle"
)]
fn build_symbol(
    line_idx: u32,
    line_text: &str,
    level: HeadingLevel,
    title: String,
    untitled: &str,
) -> DocumentSymbol {
    let utf16_len = u32::try_from(line_text.encode_utf16().count()).unwrap_or(u32::MAX);
    let line_range = Range::new(
        Position::new(line_idx, 0),
        Position::new(line_idx, utf16_len),
    );
    let name = if title.trim().is_empty() {
        untitled.to_owned()
    } else {
        title
    };
    #[expect(
        deprecated,
        reason = "upstream lsp_types field retained for LSP <3.15 compat"
    )]
    DocumentSymbol {
        name,
        detail: Some(level.as_str().to_owned()),
        kind: level.symbol_kind(),
        tags: None,
        deprecated: None,
        range: line_range,
        selection_range: line_range,
        children: Some(Vec::new()),
    }
}

fn nest_by_level(flat: Vec<(HeadingLevel, DocumentSymbol)>) -> Vec<DocumentSymbol> {
    let mut root: Vec<DocumentSymbol> = Vec::new();
    let mut stack: Vec<(HeadingLevel, Vec<usize>)> = Vec::new();
    for (level, sym) in flat {
        while stack.last().is_some_and(|(top_lv, _)| *top_lv >= level) {
            stack.pop();
        }
        let path = stack.last().map(|(_, p)| p.clone()).unwrap_or_default();
        let parent_children = navigate_mut(&mut root, &path);
        let new_idx = parent_children.len();
        parent_children.push(sym);
        let mut new_path = path;
        new_path.push(new_idx);
        stack.push((level, new_path));
    }
    root
}

fn navigate_mut<'a>(
    root: &'a mut Vec<DocumentSymbol>,
    path: &[usize],
) -> &'a mut Vec<DocumentSymbol> {
    if path.is_empty() {
        return root;
    }
    let mut cur: &mut Vec<DocumentSymbol> = root;
    for &idx in path {
        let node = &mut cur[idx];
        cur = node.children.get_or_insert_with(Vec::new);
    }
    cur
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
enum HeadingLevel {
    Major = 0,
    Middle = 1,
    Minor = 2,
}

impl HeadingLevel {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Major => "大見出し",
            Self::Middle => "中見出し",
            Self::Minor => "小見出し",
        }
    }

    const fn symbol_kind(self) -> SymbolKind {
        match self {
            Self::Major => SymbolKind::CLASS,
            Self::Middle => SymbolKind::NAMESPACE,
            Self::Minor => SymbolKind::FUNCTION,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn en() -> LanguageIdentifier {
        "en".parse().expect("en parses")
    }

    const UNTITLED_EN: &str = "(untitled)";

    fn syms(src: &str) -> Vec<DocumentSymbol> {
        document_symbols(src, &en())
    }

    #[test]
    fn empty_source_yields_no_symbols() {
        assert_eq!(syms(""), Vec::<DocumentSymbol>::new());
    }

    #[test]
    fn single_major_heading_quoted() {
        let src = "［＃「序章」は大見出し］\n本文\n";
        let s = syms(src);
        assert_eq!(s.len(), 1);
        assert_eq!(s[0].name, "序章");
        assert_eq!(s[0].kind, SymbolKind::CLASS);
        assert_eq!(s[0].detail.as_deref(), Some("大見出し"));
    }

    #[test]
    fn single_middle_heading_inline_text() {
        let src = "［＃中見出し］第一節［＃中見出し終わり］\n";
        let s = syms(src);
        assert_eq!(s.len(), 1);
        assert_eq!(s[0].name, "第一節");
        assert_eq!(s[0].kind, SymbolKind::NAMESPACE);
    }

    #[test]
    fn nested_middle_under_major() {
        let src = "［＃「序章」は大見出し］\n本文\n［＃「第一節」は中見出し］\n本文2\n";
        let tree = syms(src);
        assert_eq!(tree.len(), 1);
        let major = &tree[0];
        assert_eq!(major.name, "序章");
        let children = major.children.as_ref().expect("children");
        assert_eq!(children.len(), 1);
        assert_eq!(children[0].name, "第一節");
        assert_eq!(children[0].kind, SymbolKind::NAMESPACE);
    }

    #[test]
    fn nested_minor_under_middle_under_major() {
        let src = "\
［＃「巻」は大見出し］\n\
［＃「章」は中見出し］\n\
［＃「節」は小見出し］\n\
本文\n";
        let tree = syms(src);
        assert_eq!(tree.len(), 1);
        let major = &tree[0];
        let middle = &major.children.as_ref().unwrap()[0];
        let minor = &middle.children.as_ref().unwrap()[0];
        assert_eq!(major.name, "巻");
        assert_eq!(middle.name, "章");
        assert_eq!(minor.name, "節");
        assert_eq!(minor.kind, SymbolKind::FUNCTION);
    }

    #[test]
    fn sibling_majors_at_root() {
        let src = "［＃「一」は大見出し］\n本\n［＃「二」は大見出し］\n本2\n";
        let tree = syms(src);
        assert_eq!(tree.len(), 2);
        assert_eq!(tree[0].name, "一");
        assert_eq!(tree[1].name, "二");
    }

    #[test]
    fn empty_quote_body_falls_back_to_untitled_placeholder() {
        let src = "「」は大見出し\n本文\n";
        let s = syms(src);
        assert_eq!(s.len(), 1);
        assert!(
            !s[0].name.is_empty(),
            "name must never be empty; LSP spec forbids it",
        );
        assert!(
            !s[0].name.trim().is_empty(),
            "name must not be whitespace-only either",
        );
        assert_eq!(s[0].name, UNTITLED_EN);
    }

    #[test]
    fn whitespace_only_quote_body_also_falls_back() {
        let src = "「   」は大見出し\n";
        let s = syms(src);
        assert_eq!(s.len(), 1);
        assert_ne!(s[0].name.trim(), "");
        assert_eq!(s[0].name, UNTITLED_EN);
    }

    #[test]
    fn range_character_is_actual_line_length_not_u32_max() {
        let src = "［＃大見出し］序章\n本文\n";
        let s = syms(src);
        assert_eq!(s.len(), 1);
        let sym = &s[0];
        let expected_utf16 =
            u32::try_from("［＃大見出し］序章".encode_utf16().count()).expect("fits in u32");
        assert_eq!(
            sym.range.end.character, expected_utf16,
            "range.end.character must equal the actual line UTF-16 length",
        );
        assert!(
            sym.range.end.character < u32::MAX / 2,
            "range.end.character must not be a sentinel like u32::MAX"
        );
        assert_eq!(
            sym.selection_range.end.character, expected_utf16,
            "selection_range must mirror range",
        );
    }

    #[test]
    fn deeper_then_shallower_pops_stack() {
        let src = "\
［＃「巻一」は大見出し］\n\
［＃「節一」は中見出し］\n\
［＃「巻二」は大見出し］\n\
［＃「節二」は中見出し］\n";
        let tree = syms(src);
        assert_eq!(tree.len(), 2);
        assert_eq!(tree[0].children.as_ref().unwrap().len(), 1);
        assert_eq!(tree[0].children.as_ref().unwrap()[0].name, "節一");
        assert_eq!(tree[1].children.as_ref().unwrap().len(), 1);
        assert_eq!(tree[1].children.as_ref().unwrap()[0].name, "節二");
    }
}
