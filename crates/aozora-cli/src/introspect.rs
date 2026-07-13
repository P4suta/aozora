//! `aozora kinds` / `aozora schema` / `aozora explain` — shell-level
//! introspection of the parser's typed contracts.
//!
//! No parsing happens here — the goal is to make "what tags can the
//! JSON format produce?" / "what is the JSON envelope shape?" /
//! "what does `bouten` mean?" answerable without reading source.
//!
//! - `aozora kinds` walks every `pub const ALL: [Self; N]` on the
//!   spec / syntax enums and tabulates them.
//! - `aozora schema` pretty-prints the generated JSON Schema for
//!   one of the four JSON envelopes (delegated to
//!   `aozora::json::schema_*` behind the `schema` Cargo feature).
//! - `aozora explain <kind>` prints the embedded handbook chapter
//!   for that `NodeKind` — the same `nodes/<kind>.md` rendered by
//!   mdbook, surfaced in the terminal via `include_str!`.
//!
//! Output goes to stdout; non-zero exit only on argument errors.

use std::io::{self, Write};
use std::process::ExitCode;

use anyhow::{Context, Result, bail};
use clap::{Args, ValueEnum};
use comfy_table::{ContentArrangement, Table, presets::UTF8_FULL};

use aozora::{
    Diagnostic, DiagnosticSource, InternalCheckCode, NodeKind, PairKind, Sentinel, Severity,
    json::{schema_container_pairs, schema_diagnostics, schema_nodes, schema_pairs},
};

/// `aozora schema <which>` subcommand argument.
#[derive(Debug, Clone, Copy, ValueEnum)]
pub(crate) enum SchemaKind {
    /// `JsonEnvelope<Diagnostic>` — `diagnostics` output shape.
    Diagnostics,
    /// `JsonEnvelope<Node>` — `nodes` output shape.
    Nodes,
    /// `JsonEnvelope<Pair>` — `pairs` output shape.
    Pairs,
    /// `JsonEnvelope<ContainerPair>` — `container_pairs` output shape.
    ContainerPairs,
}

/// Output format for `aozora kinds`: the human tables (default) or the
/// machine `{"schemaVersion":2,"data":{…}}` envelope. Mirrors
/// [`crate::timing::TimingFormat`]'s two-value shape; `check`'s richer
/// `DiagFormat` (with `auto` / `short`) is diagnostic-specific and does not
/// apply here.
#[derive(Debug, Clone, Copy, Default, ValueEnum)]
pub(crate) enum OutputFormat {
    /// `comfy-table` tables, one per enum. The default.
    #[default]
    Human,
    /// The `{"schemaVersion":2,"data":{nodeKinds,pairKinds,…}}` envelope —
    /// the agent / scripting view.
    Json,
}

/// `aozora kinds` arguments. The table set is one fixed shape; `--format`
/// selects the human tables or the JSON envelope.
#[derive(Debug, Args)]
pub(crate) struct KindsArgs {
    /// Output format: `human` (tables, the default) or `json`.
    #[arg(long, value_enum, default_value_t = OutputFormat::Human)]
    format: OutputFormat,
}

/// `aozora explain <target>` arguments.
#[derive(Debug, Args)]
#[command(after_long_help = "Examples:
  aozora explain ruby                          # NodeKind handbook chapter
  aozora explain aozora::lex::unclosed_bracket # diagnostic code -> help + URL
  aozora explain unresolved_gaiji              # short form of the code")]
pub(crate) struct ExplainArgs {
    /// A `NodeKind` camelCase tag (e.g. `ruby`, `angleQuote`; run
    /// `aozora kinds` for the list) or a diagnostic code (e.g.
    /// `aozora::lex::unclosed_bracket`, or the short `unclosed_bracket`).
    #[arg(value_name = "TARGET")]
    pub(crate) kind: String,
}

/// `aozora schema <which>` arguments.
#[derive(Debug, Args)]
pub(crate) struct SchemaArgs {
    /// Which JSON envelope schema to dump.
    #[arg(value_enum)]
    pub(crate) which: SchemaKind,
}

/// One introspection table: a human title + blurb, a camelCase JSON key,
/// and the `(wire tag, summary)` rows. Both the `human` and `json` renderers
/// consume the same set so the two never drift.
struct KindTable {
    title: &'static str,
    json_key: &'static str,
    blurb: &'static str,
    rows: Vec<(&'static str, &'static str)>,
}

/// The six introspection tables, in display order. Single source of truth
/// for `aozora kinds` (both `--format human` and `--format json`).
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
            title: "Sentinel",
            json_key: "sentinels",
            blurb: "PUA sentinel kind (U+E001..U+E004 markers)",
            rows: Sentinel::ALL
                .iter()
                .map(|s| (sentinel_label(*s), describe_sentinel(*s)))
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

/// Render the unified introspection tables to stdout, as human tables or a
/// JSON envelope per `--format`.
pub(crate) fn run_kinds(args: &KindsArgs) -> Result<ExitCode> {
    let tables = kind_tables();
    let mut stdout = io::stdout().lock();
    match args.format {
        OutputFormat::Human => {
            for t in &tables {
                write_table(&mut stdout, t.title, t.blurb, t.rows.iter().copied())?;
            }
        }
        OutputFormat::Json => write_kinds_json(&mut stdout, &tables)?,
    }
    Ok(ExitCode::SUCCESS)
}

/// Emit the `{"schemaVersion":2,"data":{<jsonKey>:[{tag,summary}]}}` envelope.
/// Single-line / compact, matching the `inspect` JSON envelopes (the
/// `aozora::json::*` outputs) rather than the pretty-printed `schema` dump.
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
    let envelope = serde_json::json!({ "schemaVersion": 2, "data": data });
    let line = serde_json::to_string(&envelope).context("serialize kinds envelope as JSON")?;
    writeln!(out, "{line}").context("write kinds JSON to stdout")
}

/// Pretty-print the requested JSON envelope schema as JSON.
pub(crate) fn run_schema(args: &SchemaArgs) -> Result<ExitCode> {
    let value = match args.which {
        SchemaKind::Diagnostics => schema_diagnostics(),
        SchemaKind::Nodes => schema_nodes(),
        SchemaKind::Pairs => schema_pairs(),
        SchemaKind::ContainerPairs => schema_container_pairs(),
    };
    let pretty =
        serde_json::to_string_pretty(&value).context("failed to serialize schema as JSON")?;
    let mut stdout = io::stdout().lock();
    writeln!(stdout, "{pretty}").context("write schema to stdout")?;
    Ok(ExitCode::SUCCESS)
}

/// Print the explainer for `args.kind`. Recognises every camelCase
/// tag exposed by `aozora kinds`. Returns a non-zero exit code when
/// the tag is unknown, with a hint pointing back at `aozora kinds`.
pub(crate) fn run_explain(args: &ExplainArgs) -> Result<ExitCode> {
    // NodeKind tags (camelCase, no `_`/`::`) and diagnostic codes (which
    // always carry `_` and/or `::`) never collide, so try the node page
    // first and fall back to a diagnostic-code lookup.
    let prose = explain_kind(&args.kind).or_else(|| explain_diagnostic(&args.kind));
    let mut stdout = io::stdout().lock();
    match prose {
        Some(text) => {
            writeln!(stdout, "{text}").context("write explain to stdout")?;
            Ok(ExitCode::SUCCESS)
        }
        None => {
            bail!(
                "unknown explain target {:?}; expected a NodeKind tag (run \
                 `aozora kinds`) or a diagnostic code such as \
                 `aozora::lex::unclosed_bracket`",
                args.kind
            );
        }
    }
}

// ---- table layout ---------------------------------------------------

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

// ---- per-variant prose ---------------------------------------------
//
// Short, single-line summaries used by `aozora kinds` rows. The full
// multi-paragraph prose for each `NodeKind` lives in this crate under
// `src/node-docs/<kind>.md` and is surfaced verbatim by
// `aozora explain <kind>` via `include_str!`; the handbook borrows the
// same bytes with `{{#include}}` from `aozora-book/src/nodes/<kind>.md`.

fn describe_node(k: NodeKind) -> &'static str {
    match k {
        NodeKind::Ruby => "Ruby annotation (｜base《reading》).",
        NodeKind::Bouten => "Bouten (傍点) — emphasis dots over a span.",
        NodeKind::CombineUpright => "縦中横 — horizontal text inside a vertical run.",
        NodeKind::Gaiji => "外字 — non-Unicode character reference.",
        NodeKind::Indent => "Inline indent (字下げ) marker.",
        NodeKind::AlignEnd => "Right-edge alignment (字上げ) marker.",
        NodeKind::Center => "Centring (中央) marker — ページの左右中央 / 中央揃え.",
        NodeKind::Warichu => "割注 — split-line annotation.",
        NodeKind::LineGothic => "ゴシック体 line marker — この行はゴシック体.",
        NodeKind::LineFontSize => "絶対サイズ line marker — ［＃大文字］ ほか.",
        NodeKind::PageBreak => "改ページ.",
        NodeKind::SectionBreak => "Section break.",
        NodeKind::Heading => "Aozora heading (見出し).",
        NodeKind::HeadingHint => "Heading hint informing downstream rendering.",
        NodeKind::Illustration => "挿絵 — illustration reference.",
        NodeKind::Kaeriten => "返り点 — kanbun reading marker.",
        NodeKind::Directive => "Generic annotation no specific recogniser claimed.",
        NodeKind::AngleQuote => "Double-angle quotation (≪…≫, displays as 《…》).",
        NodeKind::MarginNote => "Side annotation (注記) — 「X」の左に「Y」の注記.",
        NodeKind::Container => "Inline-attached container (字下げ系の wrap).",
        NodeKind::ContainerOpen => "NodeRef::BlockOpen — paired-container open sentinel.",
        NodeKind::ContainerClose => "NodeRef::BlockClose — paired-container close sentinel.",
        _ => "(unrecognised NodeKind variant — handbook out of date).",
    }
}

fn describe_pair(k: PairKind) -> &'static str {
    match k {
        PairKind::Bracket => "［ … ］ — annotation body container.",
        PairKind::Ruby => "《 … 》 — ruby reading.",
        PairKind::AngleQuote => "≪ … ≫ — double-angle quotation (displays as 《…》).",
        PairKind::Tortoise => "〔 … 〕 — accent-decomposition segment.",
        PairKind::Quote => "「 … 」 — quoted literal inside annotation bodies.",
        _ => "(unrecognised PairKind variant — handbook out of date).",
    }
}

fn describe_severity(s: Severity) -> &'static str {
    match s {
        Severity::Error => "Hard failure; downstream cannot proceed.",
        Severity::Warning => "Recoverable; output is still produced.",
        Severity::Note => "Informational hint; never blocks compilation.",
        _ => "(unrecognised Severity variant — handbook out of date).",
    }
}

fn describe_source(s: DiagnosticSource) -> &'static str {
    match s {
        DiagnosticSource::Source => "Issue rooted in user input.",
        DiagnosticSource::Internal => "Library-internal sanity-check failure (bug).",
        _ => "(unrecognised DiagnosticSource variant — handbook out of date).",
    }
}

fn describe_sentinel(s: Sentinel) -> &'static str {
    match s {
        Sentinel::Inline => "U+E001 — inline registry entry.",
        Sentinel::BlockLeaf => "U+E002 — single-line block leaf.",
        Sentinel::BlockOpen => "U+E003 — paired container open boundary.",
        Sentinel::BlockClose => "U+E004 — paired container close boundary.",
    }
}

fn sentinel_label(s: Sentinel) -> &'static str {
    match s {
        Sentinel::Inline => "inline",
        Sentinel::BlockLeaf => "blockLeaf",
        Sentinel::BlockOpen => "blockOpen",
        Sentinel::BlockClose => "blockClose",
    }
}

fn describe_internal(c: InternalCheckCode) -> &'static str {
    // Stable namespaced codes — keep prose terse. The handbook
    // chapter `arch/error-recovery.md` carries the full reasoning.
    match c {
        InternalCheckCode::ResidualAnnotationMarker => "［＃ digraph survived classification",
        InternalCheckCode::UnregisteredSentinel => "PUA sentinel without registry entry",
        InternalCheckCode::RegistryOutOfOrder => "registry vector not strictly position-sorted",
        InternalCheckCode::RegistryPositionMismatch => {
            "registry entry position disagrees with sentinel kind"
        }
        _ => "(unrecognised InternalCheckCode — handbook out of date)",
    }
}

/// Embedded prose pages for `aozora explain <tag>`. Index keyed by
/// camelCase wire tag → file slug; the markdown body is loaded at
/// compile time via `include_str!` from this crate's `src/node-docs/`
/// — the single authority, which the handbook borrows via `{{#include}}`.
const NODE_PAGES: &[(&str, &str)] = &[
    ("ruby", include_str!("node-docs/ruby.md")),
    ("bouten", include_str!("node-docs/bouten.md")),
    ("tateChuYoko", include_str!("node-docs/tate-chu-yoko.md")),
    ("gaiji", include_str!("node-docs/gaiji.md")),
    ("indent", include_str!("node-docs/indent.md")),
    ("alignEnd", include_str!("node-docs/align-end.md")),
    ("warichu", include_str!("node-docs/warichu.md")),
    ("pageBreak", include_str!("node-docs/page-break.md")),
    ("sectionBreak", include_str!("node-docs/section-break.md")),
    ("heading", include_str!("node-docs/aozora-heading.md")),
    ("headingHint", include_str!("node-docs/heading-hint.md")),
    ("sashie", include_str!("node-docs/sashie.md")),
    ("kaeriten", include_str!("node-docs/kaeriten.md")),
    ("annotation", include_str!("node-docs/annotation.md")),
    ("angleQuote", include_str!("node-docs/angle-quote.md")),
    ("container", include_str!("node-docs/container.md")),
    ("containerOpen", include_str!("node-docs/container-open.md")),
    (
        "containerClose",
        include_str!("node-docs/container-close.md"),
    ),
];

fn explain_kind(tag: &str) -> Option<String> {
    NODE_PAGES
        .iter()
        .find(|(t, _)| *t == tag)
        .map(|(_, body)| (*body).to_owned())
}

/// Explain a diagnostic code: `aozora explain aozora::lex::unclosed_bracket`
/// (or the short `unclosed_bracket`). Prints the same code / severity /
/// help / URL that `aozora check` attaches to the diagnostic, sourced
/// from [`aozora::Diagnostic::explain`] so the two never diverge.
fn explain_diagnostic(arg: &str) -> Option<String> {
    // Accept the full `aozora::lex::<token>` / `aozora::lint::<token>` code or
    // the bare trailing token; expand the short form against both namespaces.
    let info = if arg.contains("::") {
        Diagnostic::explain(arg)?
    } else {
        Diagnostic::explain(&format!("aozora::lex::{arg}"))
            .or_else(|| Diagnostic::explain(&format!("aozora::lint::{arg}")))?
    };
    let mut out = format!(
        "{}  —  {}\n{} · {}\n\n{}",
        info.code,
        info.title,
        info.severity.as_json_str(),
        info.source.as_json_str(),
        info.body,
    );
    out.push_str("\n\n再現例:\n");
    out.push_str(info.repro);
    out.push_str("\n\n修正後:\n");
    out.push_str(info.fixed);
    if let Some(url) = &info.url {
        out.push_str("\n\nsee: ");
        out.push_str(url);
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[allow(
        clippy::too_many_lines,
        reason = "one exact-label assertion per NodeKind variant — the exhaustiveness is the test"
    )]
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
            describe_node(NodeKind::Warichu),
            "割注 — split-line annotation."
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
            describe_node(NodeKind::Container),
            "Inline-attached container (字下げ系の wrap)."
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

    // NOTE: describe_node intentionally routes structural NodeKind variants
    // (e.g. BodyEnd) to the wildcard `_` fallback — they are not user-facing
    // `inspect` labels — so a "no variant hits the fallback" guard would be a
    // false invariant here. The exact-label test above already kills every
    // arm-deletion and whole-body survivor; that is the real coverage.

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
        let fallback = "(unrecognised PairKind variant — handbook out of date).";
        for k in PairKind::ALL {
            assert_ne!(describe_pair(k), fallback, "{k:?} hit the fallback arm");
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
        let fallback = "(unrecognised Severity variant — handbook out of date).";
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
        let fallback = "(unrecognised DiagnosticSource variant — handbook out of date).";
        for s in DiagnosticSource::ALL {
            assert_ne!(describe_source(s), fallback, "{s:?} hit the fallback arm");
        }
    }

    #[test]
    fn describe_sentinel_labels_are_exact() {
        assert_eq!(
            describe_sentinel(Sentinel::Inline),
            "U+E001 — inline registry entry."
        );
        assert_eq!(
            describe_sentinel(Sentinel::BlockLeaf),
            "U+E002 — single-line block leaf."
        );
        assert_eq!(
            describe_sentinel(Sentinel::BlockOpen),
            "U+E003 — paired container open boundary."
        );
        assert_eq!(
            describe_sentinel(Sentinel::BlockClose),
            "U+E004 — paired container close boundary."
        );
    }

    #[test]
    fn sentinel_label_values_are_exact() {
        assert_eq!(sentinel_label(Sentinel::Inline), "inline");
        assert_eq!(sentinel_label(Sentinel::BlockLeaf), "blockLeaf");
        assert_eq!(sentinel_label(Sentinel::BlockOpen), "blockOpen");
        assert_eq!(sentinel_label(Sentinel::BlockClose), "blockClose");
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
        let fallback = "(unrecognised InternalCheckCode — handbook out of date)";
        for c in InternalCheckCode::ALL {
            assert_ne!(describe_internal(c), fallback, "{c:?} hit the fallback arm");
        }
    }
}
