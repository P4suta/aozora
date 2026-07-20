//! Source-canonicalising formatter algorithm.
//!
//! [`format_source`](crate::fmt::format_source) runs the `parse ∘ to_source` round-trip that produces an
//! idempotent, canonicalised aozora document: every consumer — the `aozora`
//! CLI's `fmt` subcommand and the CI/test gates that cross-check against it —
//! reaches the same canonical form, and the round-trip is a fixed point on the
//! second pass.
//!
//! This module is the pure algorithm only. Its output byte-identity is
//! inherited from [`Document::snapshot`](crate::Document::snapshot) +
//! [`Snapshot::to_source_with`](crate::Snapshot::to_source_with); it adds no new
//! dependency. The batch-driver CLI plumbing (file discovery, diff/check
//! reporting, progress UI, encoding selection) lives in `aozora-cli`.

use crate::render::SerializeOptions;
use crate::{Document, Severity};

/// Canonicalise an aozora source string.
///
/// Runs the aozora lex pipeline and then the inverse serializer. The returned
/// `String` is byte-identical on the second pass.
///
/// ```
/// let canonical = aozora::fmt::format_source("｜日本《にほん》\n");
/// // The redundant `｜` before an all-kanji base is dropped (ADR 0002/0003).
/// assert_eq!(canonical, "日本《にほん》\n");
/// ```
#[must_use]
pub fn format_source(source: &str) -> String {
    format_source_with(source, SerializeOptions::default())
}

/// Canonicalise an aozora source string under explicit [`SerializeOptions`].
///
/// With the default options this equals [`format_source`]. With
/// `DirectiveNormalization::Canonical` it additionally rewrites non-canonical
/// directive near-misses to their canonical spelling — the `--fix` autofix —
/// which stays a second-pass fixed point (the canonical form parses to a
/// recognized node and is not rewritten again).
#[must_use]
pub fn format_source_with(source: &str, opts: SerializeOptions) -> String {
    let snapshot = Document::new(source).snapshot();
    if snapshot
        .diagnostics()
        .iter()
        .any(|diagnostic| diagnostic.severity() == Severity::Error)
    {
        source.to_owned()
    } else {
        snapshot.to_source_with(opts)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::DirectiveNormalization;

    #[test]
    fn empty_input_formats_to_empty() {
        assert_eq!(format_source(""), "");
    }

    #[test]
    fn plain_text_passes_through_unchanged() {
        let input = "hello world\n";
        assert_eq!(format_source(input), input);
    }

    #[test]
    fn format_is_idempotent_on_ruby() {
        let input = "｜青梅《おうめ》へ";
        let once = format_source(input);
        let twice = format_source(&once);
        assert_eq!(once, twice, "second pass must be byte-identical");
    }

    #[test]
    fn format_is_idempotent_on_bouten() {
        let input = "彼は可哀想［＃「可哀想」に傍点］と言った";
        let once = format_source(input);
        let twice = format_source(&once);
        assert_eq!(once, twice);
    }

    #[test]
    fn format_is_idempotent_on_page_break() {
        let input = "前\n［＃改ページ］\n後\n";
        let once = format_source(input);
        let twice = format_source(&once);
        assert_eq!(once, twice);
    }

    #[test]
    fn malformed_source_is_preserved() {
        let source = "《《※［＃「あ」、U+3042］［＃";
        assert_eq!(format_source(source), source);
    }

    #[test]
    fn fix_rewrites_flagged_near_miss_only_when_opted_in() {
        let fix = SerializeOptions {
            directives: DirectiveNormalization::Canonical,
        };
        let near_miss = "あ［＃字下げ終わり］";
        // Default fmt keeps the flagged near-miss verbatim.
        assert!(
            format_source(near_miss).contains("［＃字下げ終わり］"),
            "default fmt must not rewrite notation"
        );
        // Opt-in rewrites it to the canonical spelling.
        let fixed = format_source_with(near_miss, fix);
        assert!(
            fixed.contains("［＃ここで字下げ終わり］"),
            "fix should canonicalise the directive; got {fixed:?}"
        );
        // A genuine editorial Unknown is left untouched even with the flag.
        let editorial = "あ［＃底本では「蒼空」］";
        assert!(
            format_source_with(editorial, fix).contains("［＃底本では「蒼空」］"),
            "fix must not touch genuine editorial Unknowns"
        );
    }

    #[test]
    fn fix_reflows_a_canonicalized_block_directive_in_one_pass() {
        let fix = SerializeOptions::default().directives(DirectiveNormalization::Canonical);
        let input = "［＃中中見出し］\0\0";
        let once = format_source_with(input, fix);
        let twice = format_source_with(&once, fix);
        assert_eq!(once, "\n\n［＃中見出し］\n\n\0\0");
        assert_eq!(once, twice);
    }
}
