//! `aozora kinds` / `aozora schema` / `aozora explain` — shell-level
//! introspection of the parser's typed contracts.
//!
//! No parsing happens here — the goal is to make "what tags can the
//! wire format produce?" / "what is the JSON envelope shape?" /
//! "what does `bouten` mean?" answerable without reading source.
//!
//! - `aozora kinds` walks every `pub const ALL: [Self; N]` on the
//!   spec / syntax enums and tabulates them.
//! - `aozora schema` pretty-prints the generated JSON Schema for
//!   one of the four wire envelopes (delegated to
//!   `aozora::wire::schema_*` behind the `schema` Cargo feature).
//! - `aozora explain <kind>` prints the embedded handbook chapter
//!   for that `NodeKind` — the same `nodes/<kind>.md` rendered by
//!   mdbook, surfaced in the terminal via `include_str!`.
//!
//! Output goes to stdout; non-zero exit only on argument errors.

use std::borrow::Cow;
use std::io::{self, Write};
use std::process::ExitCode;

use anyhow::{Context, Result, bail};
use clap::{Args, ValueEnum};
use comfy_table::{ContentArrangement, Table, presets::UTF8_FULL};

use aozora::{
    Diagnostic, DiagnosticSource, InternalCheckCode, NodeKind, PairKind, Sentinel, Severity,
    wire::{schema_container_pairs, schema_diagnostics, schema_nodes, schema_pairs},
};

/// `aozora schema <which>` subcommand argument.
#[derive(Debug, Clone, Copy, ValueEnum)]
pub(crate) enum SchemaKind {
    /// `WireEnvelope<DiagnosticWire>` — `serialize_diagnostics` output shape.
    Diagnostics,
    /// `WireEnvelope<NodeWire>` — `serialize_nodes` output shape.
    Nodes,
    /// `WireEnvelope<PairWire>` — `serialize_pairs` output shape.
    Pairs,
    /// `WireEnvelope<ContainerPairWire>` — `serialize_container_pairs` output shape.
    ContainerPairs,
}

/// `aozora kinds` arguments. No flags today — the table is one
/// fixed shape. Kept as a struct so future filters (`--enum NodeKind`,
/// `--format json`) compose without breaking the subcommand surface.
#[derive(Debug, Args)]
pub(crate) struct KindsArgs;

/// `aozora explain <target>` arguments.
#[derive(Debug, Args)]
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
    /// Which wire envelope schema to dump.
    #[arg(value_enum)]
    pub(crate) which: SchemaKind,
}

/// Render the unified introspection tables to stdout.
pub(crate) fn run_kinds(_args: &KindsArgs) -> Result<ExitCode> {
    let mut stdout = io::stdout().lock();

    write_table(
        &mut stdout,
        "NodeKind",
        "AST node / NodeRef projection tag",
        NodeKind::ALL
            .iter()
            .map(|k| (k.as_camel_case(), describe_node(*k))),
    )?;
    write_table(
        &mut stdout,
        "PairKind",
        "Balanced delimiter pair tag (PairWire)",
        PairKind::ALL
            .iter()
            .map(|k| (k.as_camel_case(), describe_pair(*k))),
    )?;
    write_table(
        &mut stdout,
        "Severity",
        "Diagnostic severity tier (DiagnosticWire.severity)",
        Severity::ALL
            .iter()
            .map(|s| (s.as_wire_str(), describe_severity(*s))),
    )?;
    write_table(
        &mut stdout,
        "DiagnosticSource",
        "Diagnostic origin (DiagnosticWire.source)",
        DiagnosticSource::ALL
            .iter()
            .map(|s| (s.as_wire_str(), describe_source(*s))),
    )?;
    write_table(
        &mut stdout,
        "Sentinel",
        "PUA sentinel kind (U+E001..U+E004 markers)",
        Sentinel::ALL
            .iter()
            .map(|s| (sentinel_label(*s), describe_sentinel(*s))),
    )?;
    write_table(
        &mut stdout,
        "InternalCheckCode",
        "Library-internal sanity-check identifier",
        InternalCheckCode::ALL
            .iter()
            .map(|c| (c.as_code(), describe_internal(*c))),
    )?;
    Ok(ExitCode::SUCCESS)
}

/// Pretty-print the requested wire envelope schema as JSON.
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
// multi-paragraph prose for each `NodeKind` lives in
// `crates/aozora-book/src/nodes/<kind>.md` and is surfaced verbatim
// by `aozora explain <kind>` via `include_str!`.

fn describe_node(k: NodeKind) -> &'static str {
    match k {
        NodeKind::Ruby => "Ruby annotation (｜base《reading》).",
        NodeKind::Bouten => "Bouten (傍点) — emphasis dots over a span.",
        NodeKind::TateChuYoko => "縦中横 — horizontal text inside a vertical run.",
        NodeKind::Gaiji => "外字 — non-Unicode character reference.",
        NodeKind::Indent => "Inline indent (字下げ) marker.",
        NodeKind::AlignEnd => "Right-edge alignment (字上げ) marker.",
        NodeKind::Center => "Centring (中央) marker — ページの左右中央 / 中央揃え.",
        NodeKind::Warichu => "割注 — split-line annotation.",
        NodeKind::Keigakomi => "罫囲み — ruled box.",
        NodeKind::PageBreak => "改ページ.",
        NodeKind::SectionBreak => "Section break.",
        NodeKind::AozoraHeading => "Aozora heading (見出し).",
        NodeKind::HeadingHint => "Heading hint informing downstream rendering.",
        NodeKind::Sashie => "挿絵 — illustration reference.",
        NodeKind::Kaeriten => "返り点 — kanbun reading marker.",
        NodeKind::Annotation => "Generic annotation no specific recogniser claimed.",
        NodeKind::AngleQuote => "Double-angle quotation (≪…≫, displays as 《…》).",
        NodeKind::SideNote => "Side annotation (注記) — 「X」の左に「Y」の注記.",
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

/// Embedded handbook pages for `aozora explain <tag>`. Index keyed
/// by camelCase wire tag → file slug; the markdown body is loaded
/// at compile time via `include_str!` from the handbook chapters
/// under `crates/aozora-book/src/nodes/`.
const NODE_PAGES: &[(&str, &str)] = &[
    ("ruby", include_str!("node-docs/ruby.md")),
    ("bouten", include_str!("node-docs/bouten.md")),
    ("tateChuYoko", include_str!("node-docs/tate-chu-yoko.md")),
    ("gaiji", include_str!("node-docs/gaiji.md")),
    ("indent", include_str!("node-docs/indent.md")),
    ("alignEnd", include_str!("node-docs/align-end.md")),
    ("warichu", include_str!("node-docs/warichu.md")),
    ("keigakomi", include_str!("node-docs/keigakomi.md")),
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
    // Accept the full `aozora::lex::<token>` code or the bare trailing
    // token; expand the short form to the canonical code.
    let code: Cow<'_, str> = if arg.contains("::") {
        Cow::Borrowed(arg)
    } else {
        Cow::Owned(format!("aozora::lex::{arg}"))
    };
    let info = Diagnostic::explain(&code)?;
    let mut out = format!(
        "{}\n{} · {}",
        info.code,
        info.severity.as_wire_str(),
        info.source.as_wire_str()
    );
    if !info.help.is_empty() {
        out.push_str("\n\n");
        out.push_str(&info.help);
    }
    if let Some(url) = &info.url {
        out.push_str("\n\nsee: ");
        out.push_str(url);
    }
    Some(out)
}
