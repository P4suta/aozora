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

use std::io::{self, Write};
use std::process::ExitCode;

use anyhow::{Context, Result, bail};
use clap::{Args, ValueEnum};
use comfy_table::{ContentArrangement, Table, presets::UTF8_FULL};

use aozora::{
    DiagnosticSource, InternalCheckCode, NodeKind, PairKind, Sentinel, Severity,
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

/// `aozora explain <kind>` arguments.
#[derive(Debug, Args)]
pub(crate) struct ExplainArgs {
    /// camelCase tag from `aozora kinds` (e.g. `ruby`, `doubleRuby`,
    /// `containerOpen`). Run `aozora kinds` for the canonical list.
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
    let prose = explain_kind(&args.kind);
    let mut stdout = io::stdout().lock();
    match prose {
        Some(text) => {
            writeln!(stdout, "{text}").context("write explain to stdout")?;
            Ok(ExitCode::SUCCESS)
        }
        None => {
            bail!(
                "unknown kind {:?}; run `aozora kinds` for the canonical list",
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
        NodeKind::Warichu => "割注 — split-line annotation.",
        NodeKind::Keigakomi => "罫囲み — ruled box.",
        NodeKind::PageBreak => "改ページ.",
        NodeKind::SectionBreak => "Section break.",
        NodeKind::AozoraHeading => "Aozora heading (見出し).",
        NodeKind::HeadingHint => "Heading hint informing downstream rendering.",
        NodeKind::Sashie => "挿絵 — illustration reference.",
        NodeKind::Kaeriten => "返り点 — kanbun reading marker.",
        NodeKind::Annotation => "Generic annotation no specific recogniser claimed.",
        NodeKind::DoubleRuby => "Double ruby (《《…》》).",
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
        PairKind::DoubleRuby => "《《 … 》》 — double-bracket bouten.",
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
    ("ruby", include_str!("../../aozora-book/src/nodes/ruby.md")),
    (
        "bouten",
        include_str!("../../aozora-book/src/nodes/bouten.md"),
    ),
    (
        "tateChuYoko",
        include_str!("../../aozora-book/src/nodes/tate-chu-yoko.md"),
    ),
    (
        "gaiji",
        include_str!("../../aozora-book/src/nodes/gaiji.md"),
    ),
    (
        "indent",
        include_str!("../../aozora-book/src/nodes/indent.md"),
    ),
    (
        "alignEnd",
        include_str!("../../aozora-book/src/nodes/align-end.md"),
    ),
    (
        "warichu",
        include_str!("../../aozora-book/src/nodes/warichu.md"),
    ),
    (
        "keigakomi",
        include_str!("../../aozora-book/src/nodes/keigakomi.md"),
    ),
    (
        "pageBreak",
        include_str!("../../aozora-book/src/nodes/page-break.md"),
    ),
    (
        "sectionBreak",
        include_str!("../../aozora-book/src/nodes/section-break.md"),
    ),
    (
        "heading",
        include_str!("../../aozora-book/src/nodes/aozora-heading.md"),
    ),
    (
        "headingHint",
        include_str!("../../aozora-book/src/nodes/heading-hint.md"),
    ),
    (
        "sashie",
        include_str!("../../aozora-book/src/nodes/sashie.md"),
    ),
    (
        "kaeriten",
        include_str!("../../aozora-book/src/nodes/kaeriten.md"),
    ),
    (
        "annotation",
        include_str!("../../aozora-book/src/nodes/annotation.md"),
    ),
    (
        "doubleRuby",
        include_str!("../../aozora-book/src/nodes/double-ruby.md"),
    ),
    (
        "container",
        include_str!("../../aozora-book/src/nodes/container.md"),
    ),
    (
        "containerOpen",
        include_str!("../../aozora-book/src/nodes/container-open.md"),
    ),
    (
        "containerClose",
        include_str!("../../aozora-book/src/nodes/container-close.md"),
    ),
];

fn explain_kind(tag: &str) -> Option<String> {
    NODE_PAGES
        .iter()
        .find(|(t, _)| *t == tag)
        .map(|(_, body)| (*body).to_owned())
}
