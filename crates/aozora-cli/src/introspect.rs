//! Prints parser contracts and explanations through [`run_kinds`], [`run_schema`], [`run_slugs`], and [`run_explain`].

use std::io::{self, IsTerminal, Write};
use std::mem;
use std::process::ExitCode;

use crate::i18n::{self as i18n, FluentArgs, LanguageIdentifier};
use anyhow::{Context, Result, bail};
use clap::{Args, ValueEnum};
use comfy_table::{ContentArrangement, Table, presets::UTF8_FULL};

use crate::config::ConfigFile;
use crate::output;
use crate::wire::Envelope;
use aozora::InternalCheckCode;
use aozora::{
    Diagnostic, DiagnosticSource, NodeKind, PairKind, Severity,
    json::{self, schema_container_pairs, schema_diagnostics, schema_nodes, schema_pairs},
};

#[derive(Debug, Clone, Copy, ValueEnum)]
pub(crate) enum SchemaKind {
    /// .aozora.toml configuration shape.
    Config,
    /// Output shape for `diagnostics`: `JsonEnvelope<Diagnostic>`.
    Diagnostics,
    /// Output shape for `nodes`: `JsonEnvelope<Node>`.
    Nodes,
    /// Output shape for `pairs`: `JsonEnvelope<Pair>`.
    Pairs,
    /// Output shape for `container_pairs`: `JsonEnvelope<ContainerPair>`.
    ContainerPairs,
}

#[derive(Debug, Clone, Copy, Default, ValueEnum)]
pub(crate) enum OutputFormat {
    /// Use human tables on a terminal or JSON when piped.
    /// This is the default.
    #[default]
    Auto,
    /// Show one `comfy-table` table per enum.
    Human,
    /// Emit the versioned `{"schemaVersion":…,"data":{nodeKinds,pairKinds,…}}` envelope for scripts and agents.
    Json,
}

impl OutputFormat {
    fn resolved(self) -> Self {
        self.resolve(io::stdout().is_terminal())
    }

    fn resolve(self, is_terminal: bool) -> Self {
        match self {
            Self::Auto if is_terminal => Self::Human,
            Self::Auto => Self::Json,
            other => other,
        }
    }
}

/// List parser kinds as human tables or JSON.
#[derive(Debug, Args)]
pub(crate) struct KindsArgs {
    /// Choose `auto`, `human` tables, or `json`.
    /// Defaults to `auto`, which uses tables on a terminal and JSON when piped.
    #[arg(long, value_enum, default_value_t = OutputFormat::Auto)]
    format: OutputFormat,
}

/// `aozora explain <target>` arguments.
#[derive(Debug, Args)]
#[command(after_long_help = "Examples:
  aozora explain ruby                          # NodeKind handbook chapter
  aozora explain tcy                           # notation concept (縦中横)
  aozora explain aozora::lex::unclosed_bracket # diagnostic code -> help + URL
  aozora explain unresolved_gaiji              # short form of the code

An unrecognised target suggests the nearest known one (\"did you mean …?\").")]
pub(crate) struct ExplainArgs {
    /// Choose a `NodeKind` tag, notation concept, or diagnostic code.
    /// Use `camelCase` tags such as `ruby` or `angleQuote`, listed by `aozora spec kinds`.
    /// Concept examples include `tcy` and `傍点`.
    /// Diagnostic codes accept full names such as `aozora::lex::unclosed_bracket` or short names such as `unclosed_bracket`.
    #[arg(value_name = "TARGET")]
    pub(crate) kind: String,
}

/// `aozora spec schema <which>` arguments.
#[derive(Debug, Args)]
pub(crate) struct SchemaArgs {
    /// Which configuration or JSON-envelope schema to dump.
    #[arg(value_enum)]
    pub(crate) which: SchemaKind,
}

struct KindTable {
    title: &'static str,
    json_key: &'static str,
    blurb: &'static str,
    rows: Vec<(&'static str, &'static str)>,
}

fn kind_tables() -> Vec<KindTable> {
    vec![
        KindTable {
            title: "NodeKind",
            json_key: "nodeKinds",
            blurb: "AST node / NodeRef projection tag",
            rows: NodeKind::ALL
                .iter()
                .map(|k| (k.as_json_tag(), describe_node(*k)))
                .collect(),
        },
        KindTable {
            title: "PairKind",
            json_key: "pairKinds",
            blurb: "Balanced delimiter pair tag (Pair)",
            rows: PairKind::ALL
                .iter()
                .map(|k| (k.as_json_tag(), describe_pair(*k)))
                .collect(),
        },
        KindTable {
            title: "Severity",
            json_key: "severities",
            blurb: "Diagnostic severity tier (Diagnostic.severity)",
            rows: Severity::ALL
                .iter()
                .map(|s| (s.as_json_str(), describe_severity(*s)))
                .collect(),
        },
        KindTable {
            title: "DiagnosticSource",
            json_key: "diagnosticSources",
            blurb: "Diagnostic origin (Diagnostic.source)",
            rows: DiagnosticSource::ALL
                .iter()
                .map(|s| (s.as_json_str(), describe_source(*s)))
                .collect(),
        },
        KindTable {
            title: "InternalCheckCode",
            json_key: "internalCheckCodes",
            blurb: "Library-internal sanity-check identifier",
            rows: InternalCheckCode::ALL
                .iter()
                .map(|c| (c.as_code(), describe_internal(*c)))
                .collect(),
        },
    ]
}

pub(crate) fn run_kinds(args: &KindsArgs) -> Result<ExitCode> {
    let tables = kind_tables();
    let mut stdout = output::stdout();
    match args.format.resolved() {
        OutputFormat::Human | OutputFormat::Auto => {
            for t in &tables {
                write_table(&mut stdout, t.title, t.blurb, t.rows.iter().copied())?;
            }
        }
        OutputFormat::Json => write_kinds_json(&mut stdout, &tables)?,
    }
    Ok(ExitCode::SUCCESS)
}

fn write_kinds_json(out: &mut dyn Write, tables: &[KindTable]) -> Result<()> {
    let mut data = serde_json::Map::new();
    for t in tables {
        let rows: Vec<serde_json::Value> = t
            .rows
            .iter()
            .map(|(tag, summary)| serde_json::json!({ "tag": tag, "summary": summary }))
            .collect();
        data.insert(t.json_key.to_owned(), serde_json::Value::Array(rows));
    }
    let envelope = Envelope::new(data);
    let line = serde_json::to_string(&envelope).context("serialize kinds envelope as JSON")?;
    writeln!(out, "{line}").context("write kinds JSON to stdout")
}

pub(crate) fn run_schema(args: &SchemaArgs) -> Result<ExitCode> {
    let value = match args.which {
        SchemaKind::Config => serde_json::to_value(schemars::schema_for!(ConfigFile))
            .context("failed to serialize config schema")?,
        SchemaKind::Diagnostics => schema_diagnostics(),
        SchemaKind::Nodes => schema_nodes(),
        SchemaKind::Pairs => schema_pairs(),
        SchemaKind::ContainerPairs => schema_container_pairs(),
    };
    let pretty =
        serde_json::to_string_pretty(&value).context("failed to serialize schema as JSON")?;
    let mut stdout = output::stdout();
    writeln!(stdout, "{pretty}").context("write schema to stdout")?;
    Ok(ExitCode::SUCCESS)
}

pub(crate) fn run_slugs() -> Result<ExitCode> {
    let mut stdout = output::stdout();
    writeln!(stdout, "{}", json::slugs()).context("write slugs to stdout")?;
    Ok(ExitCode::SUCCESS)
}

/// Prints an explanation or reports an unknown target with a localized suggestion.
pub(crate) fn run_explain(args: &ExplainArgs, lang: &LanguageIdentifier) -> Result<ExitCode> {
    match resolve_explain(&args.kind, lang) {
        Some(text) => {
            let mut stdout = output::stdout();
            writeln!(stdout, "{text}").context("write explain to stdout")?;
            Ok(ExitCode::SUCCESS)
        }
        None => bail!("{}", unknown_target_message(&args.kind, lang)),
    }
}

/// Resolves node tags before concept aliases and diagnostic codes.
fn resolve_explain(target: &str, lang: &LanguageIdentifier) -> Option<String> {
    explain_kind(target)
        .or_else(|| explain_concept(target, lang))
        .or_else(|| explain_diagnostic(target, lang))
}

fn unknown_target_message(target: &str, lang: &LanguageIdentifier) -> String {
    let mut args = FluentArgs::new();
    args.set("target", target.to_owned());
    let mut msg = i18n::tf(lang, "explain-unknown", &args);
    if let Some(suggestion) = nearest_target(target) {
        let mut hint = FluentArgs::new();
        hint.set("suggestion", suggestion.to_owned());
        msg.push(' ');
        msg.push_str(&i18n::tf(lang, "explain-did-you-mean", &hint));
    }
    msg.push('\n');
    msg.push_str(&i18n::t(lang, "explain-unknown-hint"));
    msg
}

fn write_table<I>(out: &mut dyn Write, title: &str, blurb: &str, rows: I) -> Result<()>
where
    I: IntoIterator<Item = (&'static str, &'static str)>,
{
    writeln!(out, "{title} — {blurb}").context("write section header")?;
    let mut table = Table::new();
    table
        .load_preset(UTF8_FULL)
        .set_content_arrangement(ContentArrangement::Dynamic)
        .set_header(vec!["wire tag", "summary"]);
    for (tag, summary) in rows {
        table.add_row(vec![tag, summary]);
    }
    writeln!(out, "{table}\n").context("write table")?;
    Ok(())
}

fn describe_node(k: NodeKind) -> &'static str {
    match k {
        NodeKind::Ruby => "Ruby annotation (｜base《reading》).",
        NodeKind::Bouten => "Bouten (傍点) — emphasis dots over a span.",
        NodeKind::CombineUpright => "縦中横 — horizontal text inside a vertical run.",
        NodeKind::Gaiji => "外字 — non-Unicode character reference.",
        NodeKind::Indent => "Inline indent (字下げ) marker.",
        NodeKind::AlignEnd => "Right-edge alignment (字上げ) marker.",
        NodeKind::Center => "Centring (中央) marker — ページの左右中央 / 中央揃え.",
        NodeKind::LineGothic => "ゴシック体 line marker — この行はゴシック体.",
        NodeKind::LineFontSize => "絶対サイズ line marker — ［＃大文字］ ほか.",
        NodeKind::PageBreak => "改ページ.",
        NodeKind::SectionBreak => "Section break.",
        NodeKind::BodyEnd => "本文終わり — the main body ends; a colophon follows.",
        NodeKind::ForcedBreak => "改行 — a forced line break inside a paragraph.",
        NodeKind::Emphasis => "太字 / 斜体 — bold or italic emphasis.",
        NodeKind::Heading => "Aozora heading (見出し).",
        NodeKind::HeadingHint => "Heading hint informing downstream rendering.",
        NodeKind::Illustration => "挿絵 — illustration reference.",
        NodeKind::Kaeriten => "返り点 — kanbun reading marker.",
        NodeKind::Directive => "Generic annotation no specific recogniser claimed.",
        NodeKind::AngleQuote => "Double-angle quotation (≪…≫, displays as 《…》).",
        NodeKind::MarginNote => "Side annotation (注記) — 「X」の左に「Y」の注記.",
        NodeKind::ContainerOpen => "NodeRef::BlockOpen — paired-container open sentinel.",
        NodeKind::ContainerClose => "NodeRef::BlockClose — paired-container close sentinel.",
        _ => "(unrecognised NodeKind variant — this build is missing a summary; please report it).",
    }
}

fn describe_pair(k: PairKind) -> &'static str {
    match k {
        PairKind::Bracket => "［ … ］ — annotation body container.",
        PairKind::Ruby => "《 … 》 — ruby reading.",
        PairKind::AngleQuote => "≪ … ≫ — double-angle quotation (displays as 《…》).",
        PairKind::Tortoise => "〔 … 〕 — accent-decomposition segment.",
        PairKind::Quote => "「 … 」 — quoted literal inside annotation bodies.",
        _ => "(unrecognised PairKind variant — this build is missing a summary; please report it).",
    }
}

fn describe_severity(s: Severity) -> &'static str {
    match s {
        Severity::Error => "Hard failure; downstream cannot proceed.",
        Severity::Warning => "Recoverable; output is still produced.",
        Severity::Note => "Informational hint; never blocks compilation.",
        _ => "(unrecognised Severity variant — this build is missing a summary; please report it).",
    }
}

fn describe_source(s: DiagnosticSource) -> &'static str {
    match s {
        DiagnosticSource::Source => "Issue rooted in user input.",
        DiagnosticSource::Internal => "Library-internal sanity-check failure (bug).",
        _ => {
            "(unrecognised DiagnosticSource variant — this build is missing a summary; please report it)."
        }
    }
}

fn describe_internal(c: InternalCheckCode) -> &'static str {
    match c {
        InternalCheckCode::ResidualAnnotationMarker => "［＃ digraph survived classification",
        InternalCheckCode::UnregisteredSentinel => "PUA sentinel without registry entry",
        InternalCheckCode::RegistryOutOfOrder => "registry vector not strictly position-sorted",
        InternalCheckCode::RegistryPositionMismatch => {
            "registry entry position disagrees with sentinel kind"
        }
        _ => "(unrecognised InternalCheckCode — this build is missing a summary; please report it)",
    }
}

const NODE_PAGES: &[(&str, &str)] = &[
    ("ruby", include_str!("node-docs/ruby.md")),
    ("bouten", include_str!("node-docs/bouten.md")),
    ("combineUpright", include_str!("node-docs/tate-chu-yoko.md")),
    ("gaiji", include_str!("node-docs/gaiji.md")),
    ("indent", include_str!("node-docs/indent.md")),
    ("alignEnd", include_str!("node-docs/align-end.md")),
    ("pageBreak", include_str!("node-docs/page-break.md")),
    ("sectionBreak", include_str!("node-docs/section-break.md")),
    ("heading", include_str!("node-docs/aozora-heading.md")),
    ("headingHint", include_str!("node-docs/heading-hint.md")),
    ("illustration", include_str!("node-docs/sashie.md")),
    ("kaeriten", include_str!("node-docs/kaeriten.md")),
    ("directive", include_str!("node-docs/annotation.md")),
    ("angleQuote", include_str!("node-docs/angle-quote.md")),
    ("containerOpen", include_str!("node-docs/container-open.md")),
    (
        "containerClose",
        include_str!("node-docs/container-close.md"),
    ),
];

/// Falls back to the kind summary when no embedded page exists.
fn explain_kind(tag: &str) -> Option<String> {
    const NOTATION_SPEC_URL: &str = "https://p4suta.github.io/aozora-notation-spec/";
    if let Some((_, body)) = NODE_PAGES.iter().find(|(t, _)| *t == tag) {
        return Some((*body).to_owned());
    }
    let kind = NodeKind::ALL.iter().find(|k| k.as_json_tag() == tag)?;
    Some(format!(
        "# {tag}\n\n{}\n\nNo detailed page for this kind yet. \
         `aozora spec kinds` lists every tag, and the notation itself is at \
         {NOTATION_SPEC_URL}\n",
        describe_node(*kind)
    ))
}

fn explain_diagnostic(arg: &str, lang: &LanguageIdentifier) -> Option<String> {
    let info = if arg.contains("::") {
        Diagnostic::explain(arg)?
    } else {
        Diagnostic::explain(&format!("aozora::lex::{arg}"))
            .or_else(|| Diagnostic::explain(&format!("aozora::lint::{arg}")))?
    };
    let title = i18n::diag_title(lang, info.code);
    let mut body_args = FluentArgs::new();
    for (name, value) in &info.body_args {
        body_args.set(*name, value.clone());
    }
    let body = i18n::diag_body(lang, info.code, &body_args);
    let mut out = format!(
        "{}  —  {}\n{} · {}\n\n{}",
        info.code,
        title,
        info.severity.as_json_str(),
        info.source.as_json_str(),
        body,
    );
    out.push_str("\n\n");
    out.push_str(&i18n::t(lang, "explain-repro-label"));
    out.push('\n');
    out.push_str(info.repro);
    out.push_str("\n\n");
    out.push_str(&i18n::t(lang, "explain-fixed-label"));
    out.push('\n');
    out.push_str(info.fixed);
    if let Some(url) = &info.url {
        out.push_str("\n\n");
        out.push_str(&i18n::t(lang, "explain-see-label"));
        out.push(' ');
        out.push_str(url);
    }
    Some(out)
}

const CONCEPTS: &[(&str, &str)] = &[
    ("tcy", "tcy"),
    ("縦中横", "tcy"),
    ("ルビ", "ruby"),
    ("外字", "gaiji"),
    ("傍点", "bouten"),
    ("warichu", "warichu"),
    ("割注", "warichu"),
    ("返り点", "kaeriten"),
    ("kanbun", "kaeriten"),
];

fn explain_concept(key: &str, lang: &LanguageIdentifier) -> Option<String> {
    let slug = CONCEPTS.iter().find(|(k, _)| *k == key).map(|(_, s)| *s)?;
    let title = i18n::t(lang, &format!("concept-{slug}-title"));
    let body = i18n::t(lang, &format!("concept-{slug}-body"));
    Some(format!("{title}\n\n{body}"))
}

fn known_targets() -> Vec<&'static str> {
    let mut targets: Vec<&'static str> = Vec::new();
    targets.extend(NODE_PAGES.iter().map(|(tag, _)| *tag));
    targets.extend(CONCEPTS.iter().map(|(key, _)| *key));
    for &code in &Diagnostic::ALL_CODES {
        targets.push(code);
        if let Some((_, short)) = code.rsplit_once("::") {
            targets.push(short);
        }
    }
    targets
}

/// Returns the nearest plausible target, preferring earlier entries on ties.
fn nearest_target(target: &str) -> Option<&'static str> {
    const TYPO_DISTANCE_DIVISOR: usize = 3;
    let threshold = target.chars().count().max(TYPO_DISTANCE_DIVISOR) / TYPO_DISTANCE_DIVISOR;
    let mut best: Option<(usize, &'static str)> = None;
    for candidate in known_targets() {
        let distance = levenshtein(target, candidate);
        if best.is_none_or(|(current, _)| distance < current) {
            best = Some((distance, candidate));
        }
    }
    best.filter(|(distance, _)| *distance <= threshold)
        .map(|(_, candidate)| candidate)
}

/// Measures edit distance in Unicode scalar values.
fn levenshtein(a: &str, b: &str) -> usize {
    let b: Vec<char> = b.chars().collect();
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    let mut curr: Vec<usize> = vec![0; b.len() + 1];
    for (i, ca) in a.chars().enumerate() {
        curr[0] = i + 1;
        for (j, &cb) in b.iter().enumerate() {
            let substitute = prev[j] + usize::from(ca != cb);
            curr[j + 1] = substitute.min(prev[j + 1] + 1).min(curr[j] + 1);
        }
        mem::swap(&mut prev, &mut curr);
    }
    prev[b.len()]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn output_format_resolve_auto_on_terminal_is_human() {
        assert!(matches!(
            OutputFormat::Auto.resolve(true),
            OutputFormat::Human
        ));
    }

    #[test]
    fn output_format_resolve_auto_when_piped_is_json() {
        assert!(matches!(
            OutputFormat::Auto.resolve(false),
            OutputFormat::Json
        ));
    }

    #[test]
    fn output_format_resolve_concrete_passes_through() {
        assert!(matches!(
            OutputFormat::Human.resolve(false),
            OutputFormat::Human
        ));
        assert!(matches!(
            OutputFormat::Json.resolve(true),
            OutputFormat::Json
        ));
    }

    #[test]
    fn output_format_default_is_auto() {
        assert!(matches!(OutputFormat::default(), OutputFormat::Auto));
    }

    #[test]
    fn describe_node_labels_are_exact() {
        assert_eq!(
            describe_node(NodeKind::Ruby),
            "Ruby annotation (｜base《reading》)."
        );
        assert_eq!(
            describe_node(NodeKind::Bouten),
            "Bouten (傍点) — emphasis dots over a span."
        );
        assert_eq!(
            describe_node(NodeKind::CombineUpright),
            "縦中横 — horizontal text inside a vertical run."
        );
        assert_eq!(
            describe_node(NodeKind::Gaiji),
            "外字 — non-Unicode character reference."
        );
        assert_eq!(
            describe_node(NodeKind::Indent),
            "Inline indent (字下げ) marker."
        );
        assert_eq!(
            describe_node(NodeKind::AlignEnd),
            "Right-edge alignment (字上げ) marker."
        );
        assert_eq!(
            describe_node(NodeKind::Center),
            "Centring (中央) marker — ページの左右中央 / 中央揃え."
        );
        assert_eq!(
            describe_node(NodeKind::LineGothic),
            "ゴシック体 line marker — この行はゴシック体."
        );
        assert_eq!(
            describe_node(NodeKind::LineFontSize),
            "絶対サイズ line marker — ［＃大文字］ ほか."
        );
        assert_eq!(describe_node(NodeKind::PageBreak), "改ページ.");
        assert_eq!(describe_node(NodeKind::SectionBreak), "Section break.");
        assert_eq!(describe_node(NodeKind::Heading), "Aozora heading (見出し).");
        assert_eq!(
            describe_node(NodeKind::HeadingHint),
            "Heading hint informing downstream rendering."
        );
        assert_eq!(
            describe_node(NodeKind::Illustration),
            "挿絵 — illustration reference."
        );
        assert_eq!(
            describe_node(NodeKind::Kaeriten),
            "返り点 — kanbun reading marker."
        );
        assert_eq!(
            describe_node(NodeKind::Directive),
            "Generic annotation no specific recogniser claimed."
        );
        assert_eq!(
            describe_node(NodeKind::AngleQuote),
            "Double-angle quotation (≪…≫, displays as 《…》)."
        );
        assert_eq!(
            describe_node(NodeKind::MarginNote),
            "Side annotation (注記) — 「X」の左に「Y」の注記."
        );
        assert_eq!(
            describe_node(NodeKind::ContainerOpen),
            "NodeRef::BlockOpen — paired-container open sentinel."
        );
        assert_eq!(
            describe_node(NodeKind::ContainerClose),
            "NodeRef::BlockClose — paired-container close sentinel."
        );
    }

    #[test]
    fn describe_pair_labels_are_exact() {
        assert_eq!(
            describe_pair(PairKind::Bracket),
            "［ … ］ — annotation body container."
        );
        assert_eq!(describe_pair(PairKind::Ruby), "《 … 》 — ruby reading.");
        assert_eq!(
            describe_pair(PairKind::AngleQuote),
            "≪ … ≫ — double-angle quotation (displays as 《…》)."
        );
        assert_eq!(
            describe_pair(PairKind::Tortoise),
            "〔 … 〕 — accent-decomposition segment."
        );
        assert_eq!(
            describe_pair(PairKind::Quote),
            "「 … 」 — quoted literal inside annotation bodies."
        );
    }

    #[test]
    fn describe_pair_covers_every_variant_without_fallback() {
        let fallback =
            "(unrecognised PairKind variant — this build is missing a summary; please report it).";
        for &k in PairKind::ALL {
            assert_ne!(describe_pair(k), fallback, "{k:?} hit the fallback arm");
        }
    }

    #[test]
    fn describe_node_covers_every_variant_without_fallback() {
        let fallback =
            "(unrecognised NodeKind variant — this build is missing a summary; please report it).";
        for k in NodeKind::ALL {
            assert_ne!(describe_node(k), fallback, "{k:?} hit the fallback arm");
        }
    }

    #[test]
    fn node_pages_are_keyed_by_wire_tag() {
        let tags: Vec<&str> = NodeKind::ALL.iter().map(|k| k.as_json_tag()).collect();
        for (key, _) in NODE_PAGES {
            assert!(
                tags.contains(key),
                "NODE_PAGES key `{key}` is not a NodeKind wire tag, so \
                 `aozora spec kinds` never prints it and nothing points at \
                 this page"
            );
        }
    }

    #[test]
    fn each_page_states_the_tag_that_reaches_it() {
        for (key, body) in NODE_PAGES {
            let line = body
                .lines()
                .find(|l| l.starts_with("Inspect tag:"))
                .unwrap_or_else(|| panic!("`{key}`'s page has no `Inspect tag:` line"));
            assert!(
                line.contains(&format!("`{key}`")),
                "`{key}`'s page says `{line}` — it names a tag that does not \
                 reach it"
            );
        }
    }

    #[test]
    fn every_advertised_tag_explains() {
        for k in NodeKind::ALL {
            let tag = k.as_json_tag();
            assert!(
                explain_kind(tag).is_some(),
                "`aozora spec kinds` advertises `{tag}` but \
                 `aozora explain {tag}` has nothing to say"
            );
        }
    }

    #[test]
    fn concepts_never_shadow_a_node_page() {
        let tags: Vec<&str> = NodeKind::ALL.iter().map(|k| k.as_json_tag()).collect();
        for (key, slug) in CONCEPTS {
            assert!(
                !tags.contains(key),
                "concept `{key}` -> `{slug}` is a NodeKind wire tag; node \
                 pages resolve first, so this entry is unreachable"
            );
        }
    }

    #[test]
    fn describe_severity_labels_are_exact() {
        assert_eq!(
            describe_severity(Severity::Error),
            "Hard failure; downstream cannot proceed."
        );
        assert_eq!(
            describe_severity(Severity::Warning),
            "Recoverable; output is still produced."
        );
        assert_eq!(
            describe_severity(Severity::Note),
            "Informational hint; never blocks compilation."
        );
    }

    #[test]
    fn describe_severity_covers_every_variant_without_fallback() {
        let fallback =
            "(unrecognised Severity variant — this build is missing a summary; please report it).";
        for s in Severity::ALL {
            assert_ne!(describe_severity(s), fallback, "{s:?} hit the fallback arm");
        }
    }

    #[test]
    fn describe_source_labels_are_exact() {
        assert_eq!(
            describe_source(DiagnosticSource::Source),
            "Issue rooted in user input."
        );
        assert_eq!(
            describe_source(DiagnosticSource::Internal),
            "Library-internal sanity-check failure (bug)."
        );
    }

    #[test]
    fn describe_source_covers_every_variant_without_fallback() {
        let fallback = "(unrecognised DiagnosticSource variant — this build is missing a summary; please report it).";
        for s in DiagnosticSource::ALL {
            assert_ne!(describe_source(s), fallback, "{s:?} hit the fallback arm");
        }
    }

    #[test]
    fn describe_internal_labels_are_exact() {
        assert_eq!(
            describe_internal(InternalCheckCode::ResidualAnnotationMarker),
            "［＃ digraph survived classification"
        );
        assert_eq!(
            describe_internal(InternalCheckCode::UnregisteredSentinel),
            "PUA sentinel without registry entry"
        );
        assert_eq!(
            describe_internal(InternalCheckCode::RegistryOutOfOrder),
            "registry vector not strictly position-sorted"
        );
        assert_eq!(
            describe_internal(InternalCheckCode::RegistryPositionMismatch),
            "registry entry position disagrees with sentinel kind"
        );
    }

    #[test]
    fn describe_internal_covers_every_variant_without_fallback() {
        let fallback =
            "(unrecognised InternalCheckCode — this build is missing a summary; please report it)";
        for c in InternalCheckCode::ALL {
            assert_ne!(describe_internal(c), fallback, "{c:?} hit the fallback arm");
        }
    }

    #[test]
    fn levenshtein_base_cases() {
        assert_eq!(levenshtein("", ""), 0);
        assert_eq!(levenshtein("ruby", "ruby"), 0);
        assert_eq!(levenshtein("ruby", "rubyx"), 1);
        assert_eq!(levenshtein("rubyx", "ruby"), 1);
        assert_eq!(levenshtein("ruby", "rubi"), 1);
        assert_eq!(levenshtein("", "gaiji"), 5);
        assert_eq!(levenshtein("gaiji", ""), 5);
    }

    #[test]
    fn levenshtein_transposition_costs_two_edits() {
        assert_eq!(levenshtein("unclsoed_bracket", "unclosed_bracket"), 2);
    }

    #[test]
    fn levenshtein_counts_unicode_scalars_not_bytes() {
        assert_eq!(levenshtein("傍点", "傍点"), 0);
        assert_eq!(levenshtein("傍点", "傍線"), 1);
        assert_eq!(levenshtein("縦中横", "縦横"), 1);
    }

    #[test]
    fn nearest_target_suggests_close_typos() {
        assert_eq!(nearest_target("rubi"), Some("ruby"));
        assert_eq!(nearest_target("unclsoed_bracket"), Some("unclosed_bracket"));
        assert_eq!(nearest_target("tcyy"), Some("tcy"));
        assert_eq!(
            nearest_target("aozora::lex::unclosed_bracet"),
            Some("aozora::lex::unclosed_bracket"),
        );
    }

    #[test]
    fn nearest_target_tie_resolves_to_earliest_pool_entry() {
        assert_eq!(nearest_target("ル字"), Some("ルビ"));
    }

    #[test]
    fn nearest_target_declines_when_nothing_is_close() {
        assert_eq!(nearest_target("bogus"), None);
        assert_eq!(nearest_target("zzzz"), None);
    }

    #[test]
    fn known_targets_are_all_actually_resolvable() {
        let en = lang_en();
        for target in known_targets() {
            assert!(
                resolve_explain(target, &en).is_some(),
                "suggestion pool entry `{target}` does not resolve",
            );
        }
    }

    fn lang_en() -> LanguageIdentifier {
        "en".parse().expect("`en` parses")
    }

    #[test]
    fn resolve_explain_prefers_node_page_over_concept() {
        let text = resolve_explain("ruby", &lang_en()).expect("ruby resolves");
        assert!(text.contains("NodeKind::Ruby"), "node page: {text:?}");
    }

    #[test]
    fn resolve_explain_serves_concepts_that_lack_a_node_page() {
        let en = lang_en();
        let ruby_concept = resolve_explain("ルビ", &en).expect("ルビ resolves");
        assert!(
            !ruby_concept.contains("NodeKind::Ruby"),
            "concept, not page"
        );
        assert!(
            ruby_concept.contains("Ruby"),
            "concept title: {ruby_concept:?}"
        );

        let tcy = resolve_explain("tcy", &en).expect("tcy resolves");
        assert!(tcy.to_lowercase().contains("tate"), "tcy prose: {tcy:?}");
    }

    #[test]
    fn resolve_explain_falls_through_to_diagnostic_codes() {
        let en = lang_en();
        for target in ["aozora::lex::unclosed_bracket", "unclosed_bracket"] {
            let text = resolve_explain(target, &en).expect("code resolves");
            assert!(
                text.contains("aozora::lex::unclosed_bracket"),
                "diagnostic prose for {target}: {text:?}",
            );
        }
    }

    #[test]
    fn resolve_explain_returns_none_for_unknown_target() {
        assert!(resolve_explain("bogus", &lang_en()).is_none());
    }

    #[test]
    fn unknown_target_message_offers_suggestion_and_keeps_kinds_hint() {
        let msg = unknown_target_message("rubi", &lang_en());
        assert!(msg.contains("rubi"), "echoes the bad target: {msg:?}");
        assert!(msg.contains("ruby"), "suggests the near neighbour: {msg:?}");
        assert!(msg.contains("aozora spec kinds"), "keeps the hint: {msg:?}");
    }

    #[test]
    fn unknown_target_message_omits_suggestion_when_far() {
        let msg = unknown_target_message("bogus", &lang_en());
        assert!(msg.contains("bogus"), "echoes the bad target: {msg:?}");
        assert!(msg.contains("aozora spec kinds"), "still hints: {msg:?}");
    }
}
