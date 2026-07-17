//! `textDocument/documentSymbol` — outline view from heading
//! markers in aozora notation.
//!
//! Aozora has three heading levels declared via slugs:
//! `大見出し` / `中見出し` / `小見出し`. The slug body either
//! references a forward quote (`「序章」は大見出し`) or precedes the
//! line text directly (`［＃大見出し］序章`). We accept both shapes.
//!
//! ## Output shape
//!
//! Returns a flat list of `DocumentSymbol`s in source order. Editors
//! that present a tree view will show 大 → 中 → 小 nesting because we
//! place each child under the most recent open heading of strictly
//! lower level.

use crate::i18n::{self as i18n, LanguageIdentifier};
use tower_lsp::lsp_types::{DocumentSymbol, Position, Range, SymbolKind};

/// Compute every heading symbol in `source`, nested by level, with the
/// untitled-heading placeholder rendered in `lang`.
#[must_use]
pub(super) fn document_symbols(source: &str, lang: &LanguageIdentifier) -> Vec<DocumentSymbol> {
    // Resolve the placeholder once for the whole outline. The heading-level
    // names shown as each symbol's `detail` (大見出し / 中見出し / 小見出し) are
    // the aozora directive vocabulary itself, so they stay verbatim.
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

/// Detect whether `line` declares a heading and at what level.
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
        // `見出し` without a 大/中/小 prefix — accept as Middle.
        // Real aozora docs almost always specify a level, so this is
        // a defensive default.
        Some(HeadingLevel::Middle)
    }
}

/// Placeholder used when no usable title can be extracted from a heading line,
/// rendered in `lang`. Surfaces in the outline picker so the user can still
/// navigate to the heading even before they've typed its title.
///
/// IMPORTANT: every catalog value for this key MUST be non-empty (and
/// non-whitespace) — the LSP spec for `DocumentSymbol.name` explicitly forbids
/// empty strings, and VS Code's client-side `DocumentSymbol` constructor throws
/// "name must not be falsy" the moment we hand it one.
fn untitled_placeholder(lang: &LanguageIdentifier) -> String {
    i18n::t(lang, "lsp-outline-untitled")
}

/// Best-effort title extraction from a heading line.
///
/// Three shapes:
/// 1. `「序章」は大見出し` — title is the quoted text.
/// 2. `［＃大見出し］序章［＃大見出し終わり］` — title is the text
///    after the opening slug.
/// 3. `［＃大見出し］序章` — title is whatever follows the slug on
///    the same line, until end-of-line or next slug.
///
/// Each shape's result is checked for emptiness; the first shape
/// that yields a non-empty trimmed string wins. An empty match (e.g.
/// the user typed `「」は大見出し` with no body yet) falls through to
/// the next shape rather than returning `""`, which would violate
/// the LSP `DocumentSymbol.name` non-empty contract.
fn extract_title(line: &str, untitled: &str) -> String {
    // Shape 1: quoted title
    if let Some(quote_start) = line.find('「') {
        let after = &line[quote_start + '「'.len_utf8()..];
        if let Some(quote_end) = after.find('」') {
            let body = after[..quote_end].trim();
            if !body.is_empty() {
                return body.to_owned();
            }
        }
    }
    // Shape 2/3: text after closing ］
    if let Some(close_idx) = line.find('］') {
        let after = &line[close_idx + '］'.len_utf8()..];
        // Trim any trailing closing slug like `［＃大見出し終わり］`.
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
    // LSP `Position.character` is the UTF-16 code-unit offset under
    // the default `PositionEncodingKind::UTF16`. Emit the *actual*
    // end-of-line column rather than `u32::MAX`: the spec is permissive
    // on out-of-range characters ("defaults back to the line length")
    // but vscode-languageclient's hierarchical → flat symbol adapter
    // crashes on the sentinel because it tries to slice the line text
    // by the column and dereferences an undefined.
    let utf16_len = u32::try_from(line_text.encode_utf16().count()).unwrap_or(u32::MAX);
    let line_range = Range::new(
        Position::new(line_idx, 0),
        Position::new(line_idx, utf16_len),
    );
    // Defensive: the LSP spec forbids `DocumentSymbol.name` from
    // being empty or whitespace-only, and VS Code throws "name
    // must not be falsy" on the very next line. `extract_title`
    // already enforces this, but layering the guard here means a
    // future call site that constructs DocumentSymbols some other
    // way can't reintroduce the bug.
    let name = if title.trim().is_empty() {
        untitled.to_owned()
    } else {
        title
    };
    // `lsp_types::DocumentSymbol` keeps a deprecated `deprecated` field
    // for LSP <3.15 compat; we pass `None` and use `tags` instead.
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

/// Re-fold the flat list into a tree by `HeadingLevel` strict
/// containment: a Minor under the most recent Middle, a Middle
/// under the most recent Major, a Major at the top level.
fn nest_by_level(flat: Vec<(HeadingLevel, DocumentSymbol)>) -> Vec<DocumentSymbol> {
    // Build via index manipulation on a single growing root vec.
    // For each new symbol, walk down the rightmost path until we
    // find a parent of strictly lower level (smaller order). Push
    // there; otherwise push at root.
    let mut root: Vec<DocumentSymbol> = Vec::new();
    let mut stack: Vec<(HeadingLevel, Vec<usize>)> = Vec::new();
    for (level, sym) in flat {
        // Pop the stack until the top is strictly higher than `level`.
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
    for (i, &idx) in path.iter().enumerate() {
        let node = &mut cur[idx];
        if i + 1 == path.len() {
            return node.children.get_or_insert_with(Vec::new);
        }
        cur = node.children.get_or_insert_with(Vec::new);
    }
    cur
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
enum HeadingLevel {
    Major = 0,  // 大見出し — outermost
    Middle = 1, // 中見出し
    Minor = 2,  // 小見出し
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

    /// The canonical English untitled placeholder — the outline `name` a
    /// heading with no typed title falls back to under the default locale.
    const UNTITLED_EN: &str = "(untitled)";

    fn syms(src: &str) -> Vec<DocumentSymbol> {
        document_symbols(src, &en())
    }

    #[test]
    fn empty_source_yields_no_symbols() {
        assert!(syms("").is_empty());
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
        // Regression: the user types `「」は大見出し` (empty between
        // 「 and 」, body still being typed), and `extract_title`
        // returned `""` directly. The LSP DocumentSymbol spec
        // forbids empty `name`, and VS Code's client-side validator
        // throws "name must not be falsy" — the entire
        // documentSymbol response gets dropped, which silently
        // breaks the outline picker.
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
        // Same contract as above but for `「   」` (whitespace-only).
        let src = "「   」は大見出し\n";
        let s = syms(src);
        assert_eq!(s.len(), 1);
        assert!(!s[0].name.trim().is_empty());
        assert_eq!(s[0].name, UNTITLED_EN);
    }

    #[test]
    fn range_character_is_actual_line_length_not_u32_max() {
        // Regression: an earlier version emitted `u32::MAX` as
        // `range.end.character`, which crashes vscode-languageclient
        // 9.x's hierarchical → flat symbol adapter on every
        // documentSymbol response — the client tries to slice the
        // line text by the column and dereferences an undefined.
        // The proper value is the line's UTF-16 length.
        let src = "［＃大見出し］序章\n本文\n";
        let s = syms(src);
        assert_eq!(s.len(), 1);
        let sym = &s[0];
        // The heading line is `［＃大見出し］序章` — count its UTF-16
        // code units. Each char in this BMP-only string is exactly
        // 1 utf-16 code unit, so the count equals `chars().count()`.
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
        // Each major has exactly its own middle as a child.
        assert_eq!(tree[0].children.as_ref().unwrap().len(), 1);
        assert_eq!(tree[0].children.as_ref().unwrap()[0].name, "節一");
        assert_eq!(tree[1].children.as_ref().unwrap().len(), 1);
        assert_eq!(tree[1].children.as_ref().unwrap()[0].name, "節二");
    }
}
