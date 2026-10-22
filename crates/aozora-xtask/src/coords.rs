//! Rejects source coordinates in line comments through `lint coordinates`.

use std::fs;

use regex::Regex;

use crate::scan::{tracked_rs_files, workspace_root};

const FILE_LINE: &str =
    r"(?:[A-Za-z0-9_-]+\.(?:rs|md|toml|js|jsx|ts|tsx|json|yml|yaml|sh|go|py|c|h)|Justfile):[0-9]+";

const BARE_LINE: &str = r"\(lines?\s+[0-9][^)]*\)";

#[expect(
    clippy::print_stderr,
    reason = "the command reports results and diagnostics to developers"
)]
pub(crate) fn check() -> Result<(), String> {
    let root = workspace_root()?;
    let file_line = Regex::new(FILE_LINE).map_err(|e| format!("compile pattern: {e}"))?;
    let bare_line = Regex::new(BARE_LINE).map_err(|e| format!("compile pattern: {e}"))?;

    let mut hits = Vec::new();
    let mut scanned = 0usize;
    for rel in tracked_rs_files(&root)? {
        let text = fs::read_to_string(root.join(&rel))
            .map_err(|e| format!("read {}: {e}", rel.display()))?;
        scanned += 1;
        for (i, line) in text.lines().enumerate() {
            let Some(comment) = line_comment(line) else {
                continue;
            };
            if let Some(coord) = coordinate(comment, &file_line, &bare_line) {
                hits.push(format!("{}:{}: {}", rel.display(), i + 1, coord));
            }
        }
    }

    if !hits.is_empty() {
        for h in &hits {
            eprintln!("    {h}");
        }
        return Err(format!(
            "{} comment coordinate(s) — a source line in a comment rots on the \
             next edit above it; name a symbol instead",
            hits.len()
        ));
    }
    eprintln!(
        "xtask lint coordinates: clean — {scanned} files scanned, no comment cites a source coordinate"
    );
    Ok(())
}

fn coordinate(comment: &str, file_line: &Regex, bare_line: &Regex) -> Option<String> {
    if let Some(m) = file_line.find(comment) {
        return Some(m.as_str().to_owned());
    }
    bare_line
        .find_iter(comment)
        .find(|m| !m.as_str().contains(", col"))
        .map(|m| m.as_str().to_owned())
}

fn line_comment(line: &str) -> Option<&str> {
    let b = line.as_bytes();
    let mut i = 0;
    while i < b.len() {
        let c = b[i];
        if c == b'/' && b.get(i + 1) == Some(&b'/') {
            return Some(&line[i + 2..]);
        } else if c == b'"' {
            i = skip_string(b, i);
        } else if c == b'\'' {
            i = skip_char_or_lifetime(b, i);
        } else if c == b'r'
            && let Some(hashes) = raw_string_start(b, i)
        {
            i = skip_raw_string(b, i, hashes);
        } else {
            i += 1;
        }
    }
    None
}

fn skip_string(b: &[u8], start: usize) -> usize {
    let mut i = start + 1;
    while i < b.len() {
        match b[i] {
            b'\\' => i += 2,
            b'"' => return i + 1,
            _ => i += 1,
        }
    }
    i
}

fn raw_string_start(b: &[u8], start: usize) -> Option<usize> {
    let mut j = start + 1;
    while b.get(j) == Some(&b'#') {
        j += 1;
    }
    (b.get(j) == Some(&b'"')).then_some(j - start - 1)
}

fn skip_raw_string(b: &[u8], start: usize, hashes: usize) -> usize {
    let mut i = start + 1 + hashes + 1;
    while i < b.len() {
        if b[i] == b'"' {
            let mut k = i + 1;
            let mut seen = 0;
            while seen < hashes && b.get(k) == Some(&b'#') {
                seen += 1;
                k += 1;
            }
            if seen == hashes {
                return k;
            }
        }
        i += 1;
    }
    i
}

fn skip_char_or_lifetime(b: &[u8], start: usize) -> usize {
    if b.get(start + 1) == Some(&b'\\') {
        let mut i = start + 2;
        while i < b.len() && b[i] != b'\'' {
            i += 1;
        }
        return i + 1;
    }
    if b.get(start + 2) == Some(&b'\'') {
        const CHAR_LITERAL_BYTES: usize = 3;
        return start + CHAR_LITERAL_BYTES;
    }
    start + 1
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hits(line: &str) -> bool {
        let file_line = Regex::new(FILE_LINE).expect("pattern compiles");
        let bare_line = Regex::new(BARE_LINE).expect("pattern compiles");
        line_comment(line)
            .and_then(|c| coordinate(c, &file_line, &bare_line))
            .is_some()
    }

    #[test]
    fn flags_a_coordinate_in_a_line_comment() {
        assert!(hits("    // classify_err lives at main.rs:697 now"));
    }

    #[test]
    fn flags_a_coordinate_in_an_inner_doc_comment() {
        assert!(hits("//! next to Justfile:718, which has since moved"));
    }

    #[test]
    fn ignores_prose_without_a_coordinate() {
        assert!(!hits("// classify_err is the final error disposition"));
    }

    #[test]
    fn ignores_times_versions_and_codepoints() {
        assert!(!hits("// pandoc 1.23 as of 12:30, gaiji U+3099"));
    }

    #[test]
    fn ignores_a_coordinate_inside_a_string_literal() {
        assert!(!hits(r#"    let label = "main.rs:697";"#));
    }

    #[test]
    fn a_url_in_a_string_is_not_a_comment() {
        assert!(!hits(
            r#"    let u = "http://example.com/a.rs:1"; let n = 5;"#
        ));
    }

    #[test]
    fn a_quote_char_literal_does_not_swallow_a_trailing_comment() {
        assert!(hits(r#"    let q = '"'; // really main.rs:42"#));
    }

    #[test]
    fn a_lifetime_does_not_swallow_a_trailing_comment() {
        assert!(hits("fn f<'a>(x: &'a str) {} // see backend.rs:9"));
    }

    #[test]
    fn a_raw_string_hides_its_contents_but_not_the_comment_after() {
        assert!(!hits(r##"    let r = r#"a.rs:1"#;"##));
        assert!(hits(r##"    let r = r#"x"#; // b.rs:2"##));
    }

    #[test]
    fn flags_a_bare_parenthesised_line_cite() {
        assert!(hits("    // kills the `||`->`&&` guard (line 114)"));
        assert!(hits(
            "    // ---- Piece::narrow partitions (lines 509, 512) ----"
        ));
    }

    #[test]
    fn ignores_an_lsp_line_col_position() {
        assert!(!hits(
            "    /// `offset = 10` gives start = (line 10, col 0) and"
        ));
    }

    #[test]
    fn ignores_prose_line_without_a_number() {
        assert!(!hits("    // line up the columns before emitting the row"));
    }

    #[test]
    fn the_repo_cites_no_source_coordinates() {
        check().expect("no comment in the tree cites a source coordinate");
    }
}
