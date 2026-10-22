#![expect(
    clippy::expect_used,
    reason = "diagnostic hints write into an infallible String buffer"
)]

use std::error::Error;
use std::fmt;
use std::io::{self, IsTerminal, Write};

use crate::i18n::{self as i18n, FluentArgs, LanguageIdentifier};
use aozora::json;
use clap::ValueEnum;
use miette::{NamedSource, Report};

/// How document subcommands render diagnostics.
#[derive(Debug, Clone, Copy, Default, ValueEnum, serde::Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "lowercase")]
pub(crate) enum DiagFormat {
    /// Graphical (`human`) on a terminal, machine (`json`) when piped.
    #[default]
    Auto,
    /// miette graphical report — source snippet + caret + label + help.
    Human,
    /// The `aozora::json` JSON envelope — the cross-binding machine view.
    Json,
    /// One grep-able line per diagnostic: `path:offset: severity[code]: msg`.
    Short,
}

impl DiagFormat {
    fn resolved(self) -> Self {
        self.resolve(io::stderr().is_terminal())
    }

    fn resolve(self, is_terminal: bool) -> Self {
        match self {
            Self::Auto if is_terminal => Self::Human,
            Self::Auto => Self::Json,
            other => other,
        }
    }
}

#[expect(
    clippy::too_many_arguments,
    reason = "the five render inputs (format / path / doc / diagnostics / lang) are each distinct; bundling them behind a struct would move the arity without clarifying it"
)]
pub(crate) fn render(
    format: DiagFormat,
    path: &str,
    source: &str,
    diagnostics: &[aozora::Diagnostic],
    lang: &LanguageIdentifier,
) -> io::Result<()> {
    match format.resolved() {
        DiagFormat::Human | DiagFormat::Auto => render_human(path, source, diagnostics, lang),
        DiagFormat::Json => render_json(diagnostics),
        DiagFormat::Short => render_short(path, diagnostics),
    }
}

fn render_human(
    path: &str,
    source: &str,
    diagnostics: &[aozora::Diagnostic],
    lang: &LanguageIdentifier,
) -> io::Result<()> {
    let localize_headline = !i18n::is_english(lang);
    let mut stderr = io::stderr().lock();
    for diag in diagnostics {
        let source = NamedSource::new(path, source.to_owned());
        if localize_headline {
            let headline = LocalizedHeadline {
                title: i18n::diag_title(lang, diag.code()),
                inner: diag.clone(),
            };
            let report = Report::new(headline).with_source_code(source);
            writeln!(stderr, "{report:?}")?;
        } else {
            let report = Report::new(diag.clone()).with_source_code(source);
            writeln!(stderr, "{report:?}")?;
        }
    }
    // ADR-0033 reserves explain hints for human output to keep machine views byte-stable.
    write_explain_hint(&mut stderr, diagnostics, lang)
}

struct LocalizedHeadline {
    inner: aozora::Diagnostic,
    title: String,
}

impl fmt::Display for LocalizedHeadline {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.title)
    }
}

impl fmt::Debug for LocalizedHeadline {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(&self.inner, f)
    }
}

impl Error for LocalizedHeadline {}

impl miette::Diagnostic for LocalizedHeadline {
    fn code(&self) -> Option<Box<dyn fmt::Display + '_>> {
        miette::Diagnostic::code(&self.inner)
    }

    fn severity(&self) -> Option<miette::Severity> {
        miette::Diagnostic::severity(&self.inner)
    }

    fn help(&self) -> Option<Box<dyn fmt::Display + '_>> {
        miette::Diagnostic::help(&self.inner)
    }

    fn url(&self) -> Option<Box<dyn fmt::Display + '_>> {
        miette::Diagnostic::url(&self.inner)
    }

    fn labels(&self) -> Option<Box<dyn Iterator<Item = miette::LabeledSpan> + '_>> {
        miette::Diagnostic::labels(&self.inner)
    }
}

fn write_explain_hint(
    w: &mut impl Write,
    diagnostics: &[aozora::Diagnostic],
    lang: &LanguageIdentifier,
) -> io::Result<()> {
    if let Some(hint) = explain_hint(diagnostics.iter().map(aozora::Diagnostic::code), lang) {
        w.write_all(hint.as_bytes())?;
    }
    Ok(())
}

fn explain_hint(
    codes: impl Iterator<Item = &'static str>,
    lang: &LanguageIdentifier,
) -> Option<String> {
    use std::fmt::Write as _;

    const MAX_HINTS: usize = 3;
    let mut seen: Vec<&'static str> = Vec::new();
    for code in codes {
        let short = short_code(code);
        if !seen.contains(&short) {
            seen.push(short);
        }
    }
    if seen.is_empty() {
        return None;
    }
    let mut out = String::new();
    writeln!(out, "{}", i18n::t(lang, "explain-hint-header"))
        .expect("writing to a String is infallible");
    for code in seen.iter().take(MAX_HINTS) {
        writeln!(out, "      aozora explain {code}").expect("writing to a String is infallible");
    }
    if seen.len() > MAX_HINTS {
        let mut args = FluentArgs::new();
        args.set("count", (seen.len() - MAX_HINTS).to_string());
        writeln!(out, "      {}", i18n::tf(lang, "explain-hint-more", &args))
            .expect("writing to a String is infallible");
    }
    Some(out)
}

fn short_code(code: &'static str) -> &'static str {
    code.rsplit_once("::").map_or(code, |(_, tail)| tail)
}

fn render_json(diagnostics: &[aozora::Diagnostic]) -> io::Result<()> {
    writeln!(io::stderr().lock(), "{}", json::diagnostics(diagnostics))
}

fn render_short(path: &str, diagnostics: &[aozora::Diagnostic]) -> io::Result<()> {
    let mut stderr = io::stderr().lock();
    for diag in diagnostics {
        let span = diag.span();
        writeln!(
            stderr,
            "{path}:{}: {}[{}]: {diag}",
            span.start,
            diag.severity().as_json_str(),
            diag.code(),
        )?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_auto_on_terminal_is_human() {
        assert!(matches!(DiagFormat::Auto.resolve(true), DiagFormat::Human));
    }

    #[test]
    fn resolve_auto_when_piped_is_json() {
        assert!(matches!(DiagFormat::Auto.resolve(false), DiagFormat::Json));
    }

    #[test]
    fn resolve_concrete_formats_pass_through() {
        assert!(matches!(DiagFormat::Human.resolve(true), DiagFormat::Human));
        assert!(matches!(
            DiagFormat::Human.resolve(false),
            DiagFormat::Human
        ));
        assert!(matches!(DiagFormat::Json.resolve(true), DiagFormat::Json));
        assert!(matches!(DiagFormat::Json.resolve(false), DiagFormat::Json));
        assert!(matches!(DiagFormat::Short.resolve(true), DiagFormat::Short));
        assert!(matches!(
            DiagFormat::Short.resolve(false),
            DiagFormat::Short
        ));
    }

    fn lang(tag: &str) -> LanguageIdentifier {
        tag.parse().expect("test locale tag parses")
    }

    fn pua_diagnostic() -> aozora::Diagnostic {
        aozora::parse("a\u{E001}b")
            .expect("source fits parser span limit")
            .snapshot()
            .diagnostics()
            .first()
            .cloned()
            .expect("a PUA sentinel yields one diagnostic")
    }

    fn headline_over(inner: aozora::Diagnostic) -> LocalizedHeadline {
        LocalizedHeadline {
            title: "ローカライズ見出し".to_owned(),
            inner,
        }
    }

    #[test]
    fn headline_display_is_the_localized_title() {
        let h = headline_over(pua_diagnostic());
        assert_eq!(h.to_string(), "ローカライズ見出し");
    }

    #[test]
    fn headline_debug_delegates_to_inner() {
        let inner = pua_diagnostic();
        let shown = format!("{:?}", headline_over(inner.clone()));
        assert!(!shown.is_empty(), "Debug must not render empty: {shown:?}");
        assert_eq!(shown, format!("{inner:?}"), "Debug delegates to the inner");
    }

    #[test]
    fn headline_severity_delegates_to_inner() {
        let inner = pua_diagnostic();
        let h = headline_over(inner.clone());
        assert_eq!(
            miette::Diagnostic::severity(&h),
            Some(miette::Severity::Warning),
            "the PUA diagnostic renders as a warning",
        );
        assert_eq!(
            miette::Diagnostic::severity(&h),
            miette::Diagnostic::severity(&inner),
            "severity delegates to the inner",
        );
    }

    #[test]
    fn headline_help_url_code_delegate_to_inner() {
        let inner = pua_diagnostic();
        let h = headline_over(inner.clone());

        let h_help = miette::Diagnostic::help(&h).map(|d| d.to_string());
        assert!(h_help.is_some(), "PUA diagnostic carries help text");
        assert_eq!(
            h_help,
            miette::Diagnostic::help(&inner).map(|d| d.to_string()),
            "help delegates to the inner",
        );

        let h_url = miette::Diagnostic::url(&h).map(|d| d.to_string());
        assert!(h_url.is_some(), "PUA diagnostic carries a docs url");
        assert_eq!(
            h_url,
            miette::Diagnostic::url(&inner).map(|d| d.to_string()),
            "url delegates to the inner",
        );

        assert_eq!(
            miette::Diagnostic::code(&h).map(|d| d.to_string()),
            miette::Diagnostic::code(&inner).map(|d| d.to_string()),
            "code delegates to the inner",
        );
    }

    #[test]
    fn headline_labels_delegate_to_inner() {
        let h = headline_over(pua_diagnostic());
        let label_count = miette::Diagnostic::labels(&h)
            .expect("the headline forwards the inner caret label")
            .count();
        assert_eq!(label_count, 1, "the PUA diagnostic has one caret label");
    }

    #[test]
    fn explain_hint_empty_is_none() {
        use std::iter::empty;
        assert_eq!(explain_hint(empty(), &lang("en")), None);
    }

    #[test]
    fn explain_hint_dedups_by_short_code() {
        let codes = ["aozora::lex::foo", "aozora::parse::foo", "aozora::lex::bar"];
        let hint = explain_hint(codes.into_iter(), &lang("en")).expect("non-empty");
        assert_eq!(
            hint,
            "help: run `aozora explain <code>` for details, e.g.\n\
             \u{20}     aozora explain foo\n\
             \u{20}     aozora explain bar\n",
        );
    }

    #[test]
    fn explain_hint_exactly_max_has_no_more_tail() {
        let codes = ["aozora::a::x", "aozora::b::y", "aozora::c::z"];
        let hint = explain_hint(codes.into_iter(), &lang("en")).expect("non-empty");
        assert_eq!(
            hint,
            "help: run `aozora explain <code>` for details, e.g.\n\
             \u{20}     aozora explain x\n\
             \u{20}     aozora explain y\n\
             \u{20}     aozora explain z\n",
        );
        assert!(!hint.contains("more"));
    }

    #[test]
    fn explain_hint_over_max_shows_more_tail() {
        let codes = [
            "aozora::a::x",
            "aozora::b::y",
            "aozora::c::z",
            "aozora::d::w",
        ];
        let hint = explain_hint(codes.into_iter(), &lang("en")).expect("non-empty");
        assert_eq!(
            hint,
            "help: run `aozora explain <code>` for details, e.g.\n\
             \u{20}     aozora explain x\n\
             \u{20}     aozora explain y\n\
             \u{20}     aozora explain z\n\
             \u{20}     … and 1 more\n",
        );
    }

    #[test]
    fn explain_hint_more_tail_counts_the_overflow_not_a_quotient() {
        let codes = [
            "aozora::a::x",
            "aozora::b::y",
            "aozora::c::z",
            "aozora::d::w",
            "aozora::e::v",
        ];
        let hint = explain_hint(codes.into_iter(), &lang("en")).expect("non-empty");
        assert_eq!(
            hint,
            "help: run `aozora explain <code>` for details, e.g.\n\
             \u{20}     aozora explain x\n\
             \u{20}     aozora explain y\n\
             \u{20}     aozora explain z\n\
             \u{20}     … and 2 more\n",
        );
    }

    #[test]
    fn explain_hint_localizes_header_and_tail() {
        let codes = [
            "aozora::a::x",
            "aozora::b::y",
            "aozora::c::z",
            "aozora::d::w",
        ];
        let ja = explain_hint(codes.into_iter(), &lang("ja")).expect("non-empty");
        assert_eq!(
            ja,
            "ヒント: 詳細は `aozora explain <code>` を実行。例:\n\
             \u{20}     aozora explain x\n\
             \u{20}     aozora explain y\n\
             \u{20}     aozora explain z\n\
             \u{20}     … 他 1 件\n",
        );
        let zh = explain_hint(codes.into_iter(), &lang("zh")).expect("non-empty");
        assert_eq!(
            zh,
            "提示: 运行 `aozora explain <code>` 查看详情，例如:\n\
             \u{20}     aozora explain x\n\
             \u{20}     aozora explain y\n\
             \u{20}     aozora explain z\n\
             \u{20}     … 还有 1 项\n",
        );
    }
}
