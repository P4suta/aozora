//! Coordinate-literal gate: a comment may not cite source by `file:line`.
//!
//! A line number in a comment is a fact with a shelf life — the next edit
//! above it makes it wrong, and nothing re-reads it. #553 shipped a table
//! whose comment named a wire key the code did not use; the coordinate form
//! of the same rot is a test header pointing at a function ~300 lines from
//! where it actually sits. Reference a symbol instead: the compiler moves it,
//! and a grep still finds it.
//!
//! Sound over complete. It scans the `//` comment text of every git-tracked
//! `.rs` line, stepping over string, raw-string and char literals so a
//! coordinate inside `"…"` — a fixture, a format string — is never read as
//! a comment. A `file:line` hidden in a block comment is out of scope; the
//! forms that rot in practice are `//` and `//!`.

use std::fs;

use regex::Regex;

use crate::scan::{tracked_rs_files, workspace_root};

/// A source coordinate: a filename with a code/config extension, or a bare
/// `Justfile`, joined by `:` to a line number. Shapes without the `.<ext>:` /
/// `Justfile:` prefix — times (`12:30`), versions (`1.23`), codepoints
/// (`U+3099`) — cannot match.
const PATTERN: &str =
    r"(?:[A-Za-z0-9_-]+\.(?:rs|md|toml|js|jsx|ts|tsx|json|yml|yaml|sh|go|py|c|h)|Justfile):[0-9]+";

/// `xtask lint coordinates` — no comment may cite a `file:line`.
pub(crate) fn check() -> Result<(), String> {
    let root = workspace_root()?;
    let re = Regex::new(PATTERN).map_err(|e| format!("compile pattern: {e}"))?;

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
            if let Some(m) = re.find(comment) {
                hits.push(format!("{}:{}: {}", rel.display(), i + 1, m.as_str()));
            }
        }
    }

    if !hits.is_empty() {
        for h in &hits {
            eprintln!("    {h}");
        }
        return Err(format!(
            "{} comment coordinate(s) — a `file:line` in a comment rots on the \
             next edit above it; name a symbol instead",
            hits.len()
        ));
    }
    eprintln!(
        "xtask lint coordinates: clean — {scanned} files scanned, no comment cites a source coordinate"
    );
    Ok(())
}

/// The `//` line-comment text of a source line, or `None` when it has none.
/// String, raw-string and char literals are walked over, so neither a `//`
/// inside `"http://…"` nor a coordinate inside a string is read as a comment.
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

/// Advance past a `"…"` string literal (`b[start]` is the opening quote),
/// honouring `\` escapes. An unterminated literal runs to end-of-line.
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

/// The `#` count of a raw-string opener at `b[start]` (`r"…"`, `r#"…"#`, and
/// the byte-string `br…` forms), or `None` when `r` is an ordinary
/// identifier or a raw identifier (`r#type`).
fn raw_string_start(b: &[u8], start: usize) -> Option<usize> {
    let mut j = start + 1;
    while b.get(j) == Some(&b'#') {
        j += 1;
    }
    (b.get(j) == Some(&b'"')).then_some(j - start - 1)
}

/// Advance past a raw string opened at `b[start]` with `hashes` hashes,
/// whose terminator is `"` followed by that many `#`.
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

/// Advance past a char literal (`'x'`, `'\n'`, `'\''`) at `b[start]`, or past
/// just the tick of a lifetime (`'a`, `'static`). A char literal is the only
/// form that can hide a `"`, so misreading a lifetime as one would only cost
/// a skipped tick — never a false string.
fn skip_char_or_lifetime(b: &[u8], start: usize) -> usize {
    if b.get(start + 1) == Some(&b'\\') {
        let mut i = start + 2;
        while i < b.len() && b[i] != b'\'' {
            i += 1;
        }
        return i + 1;
    }
    if b.get(start + 2) == Some(&b'\'') {
        return start + 3;
    }
    start + 1
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hits(line: &str) -> bool {
        let re = Regex::new(PATTERN).expect("pattern compiles");
        line_comment(line).is_some_and(|c| re.is_match(c))
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

    /// Self-check mirroring `docs.rs` / `lint.rs`: the live tree cites no
    /// `file:line` in any comment.
    #[test]
    fn the_repo_cites_no_source_coordinates() {
        check().expect("no comment in the tree cites a source coordinate");
    }
}
