//! Canonical Aozora source formatting.

use crate::render::SerializeOptions;
use crate::{Document, Severity};

/// Canonicalise Aozora source to a serialization fixed point.
///
/// ```
/// let canonical = aozora::fmt::format_source("｜日本《にほん》\n");
/// assert_eq!(canonical, "日本《にほん》\n");
/// ```
#[must_use]
pub fn format_source(source: &str) -> String {
    format_source_with(source, SerializeOptions::default())
}

/// Canonicalise Aozora source with the given serialization options.
///
/// Canonical directive normalization also corrects verified spelling variants.
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
        assert!(
            format_source(near_miss).contains("［＃字下げ終わり］"),
            "default fmt must not rewrite notation"
        );
        let fixed = format_source_with(near_miss, fix);
        assert!(
            fixed.contains("［＃ここで字下げ終わり］"),
            "fix should canonicalise the directive; got {fixed:?}"
        );
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
