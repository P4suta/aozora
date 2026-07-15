//! `aozora spec {kinds,schema,slugs}` / `aozora explain` — shell-level
//! introspection of the parser's typed contracts.
//!
//! No parsing happens here — the goal is to make "what tags can the
//! JSON format produce?" / "what is the JSON envelope shape?" /
//! "what does `bouten` mean?" answerable without reading source.
//!
//! - `aozora spec kinds` walks every `pub const ALL: [Self; N]` on the
//!   spec / syntax enums and tabulates them.
//! - `aozora spec schema` pretty-prints the generated JSON Schema for
//!   one of the four JSON envelopes (delegated to
//!   `aozora::json::schema_*` behind the `schema` Cargo feature).
//! - `aozora spec slugs` prints the static ［＃…］ slug catalogue as the
//!   shared `aozora::json` envelope (delegated to `aozora::json::slugs`).
//! - `aozora explain <kind>` prints the embedded prose page for that
//!   `NodeKind` — this crate's `src/node-docs/<kind>.md`, embedded via
//!   `include_str!`.
//!
//! Output goes to stdout; non-zero exit only on argument errors.

use std::io::{self, IsTerminal, Write};
use std::mem;
use std::process::ExitCode;

use anyhow::{Context, Result, bail};
use aozora_i18n::{self as i18n, FluentArgs, LanguageIdentifier};
use clap::{Args, ValueEnum};
use comfy_table::{ContentArrangement, Table, presets::UTF8_FULL};

use aozora::{
    Diagnostic, DiagnosticSource, InternalCheckCode, NodeKind, PairKind, Sentinel, Severity,
    json::{self, schema_container_pairs, schema_diagnostics, schema_nodes, schema_pairs},
};

/// `aozora spec schema <which>` subcommand argument.
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

/// Output format for `aozora spec kinds`: the human tables or the machine
/// `{"schemaVersion":1,"data":{…}}` envelope, auto-selected by default on the
/// same rule as `check`'s diagnostics — tables when stdout is a terminal, the
/// JSON envelope when it is piped. `check`'s richer `DiagFormat` (with `short`)
/// is diagnostic-specific and does not apply here.
#[derive(Debug, Clone, Copy, Default, ValueEnum)]
pub(crate) enum OutputFormat {
    /// Human tables on a terminal, machine (`json`) when piped. The default.
    #[default]
    Auto,
    /// `comfy-table` tables, one per enum.
    Human,
    /// The `{"schemaVersion":1,"data":{nodeKinds,pairKinds,…}}` envelope —
    /// the agent / scripting view.
    Json,
}

impl OutputFormat {
    /// Collapse `Auto` to a concrete view based on whether stdout is a TTY.
    fn resolved(self) -> Self {
        self.resolve(io::stdout().is_terminal())
    }

    /// Pure decision seam for [`resolved`](Self::resolved): `Auto` becomes
    /// `Human` on a terminal and `Json` otherwise; concrete formats pass
    /// through unchanged. Mirrors [`crate::diagnostics_render::DiagFormat`]'s
    /// auto rule (that one keys on stderr; `kinds` writes to stdout).
    fn resolve(self, is_terminal: bool) -> Self {
        match self {
            Self::Auto if is_terminal => Self::Human,
            Self::Auto => Self::Json,
            other => other,
        }
    }
}

/// `aozora spec kinds` arguments. The table set is one fixed shape; `--format`
/// selects the human tables or the JSON envelope (`auto` by default).
#[derive(Debug, Args)]
pub(crate) struct KindsArgs {
    /// Output format: `auto` (the default — tables on a terminal, `json` when
    /// piped), `human` (tables), or `json`.
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
    /// A `NodeKind` camelCase tag (e.g. `ruby`, `angleQuote`; run
    /// `aozora spec kinds` for the list), a notation concept (e.g. `tcy`,
    /// `傍点`), or a diagnostic code (e.g. `aozora::lex::unclosed_bracket`,
    /// or the short `unclosed_bracket`).
    #[arg(value_name = "TARGET")]
    pub(crate) kind: String,
}

/// `aozora spec schema <which>` arguments.
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
/// for `aozora spec kinds` (both `--format human` and `--format json`).
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
    match args.format.resolved() {
        // `resolved()` never returns `Auto`, but match exhaustively.
        OutputFormat::Human | OutputFormat::Auto => {
            for t in &tables {
                write_table(&mut stdout, t.title, t.blurb, t.rows.iter().copied())?;
            }
        }
        OutputFormat::Json => write_kinds_json(&mut stdout, &tables)?,
    }
    Ok(ExitCode::SUCCESS)
}

/// Emit the `{"schemaVersion":1,"data":{<jsonKey>:[{tag,summary}]}}` envelope.
/// Single-line / compact, matching the shape (two keys / camelCase) of the
/// `inspect` wire envelopes, though `schemaVersion` here is a CLI-local counter
/// distinct from the wire `SCHEMA_VERSION`.
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
    let envelope = serde_json::json!({ "schemaVersion": 1, "data": data });
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

/// Print the static ［＃…］ slug catalogue as the shared `aozora::json`
/// envelope. Reads no document input; byte-identical to every binding's
/// `slugs_json()` output.
pub(crate) fn run_slugs() -> Result<ExitCode> {
    let mut stdout = io::stdout().lock();
    writeln!(stdout, "{}", json::slugs()).context("write slugs to stdout")?;
    Ok(ExitCode::SUCCESS)
}

/// Print the explainer for `args.kind`. Resolves a `NodeKind` handbook page,
/// a notation concept, or a diagnostic code (see [`resolve_explain`]). On an
/// unrecognised target it exits non-zero with a localized message that offers
/// the nearest known target ("did you mean …?") plus a hint pointing back at
/// `aozora spec kinds`.
pub(crate) fn run_explain(args: &ExplainArgs, lang: &LanguageIdentifier) -> Result<ExitCode> {
    match resolve_explain(&args.kind, lang) {
        Some(text) => {
            let mut stdout = io::stdout().lock();
            writeln!(stdout, "{text}").context("write explain to stdout")?;
            Ok(ExitCode::SUCCESS)
        }
        None => bail!("{}", unknown_target_message(&args.kind, lang)),
    }
}

/// Resolve `target` to its explainer prose, in the deterministic precedence
/// `NodeKind tag > concept > diagnostic code`.
///
/// The three layers do not collide: node-page and concept keys are disjoint by
/// construction, and diagnostic codes always carry `_` and/or `::` (which the
/// node-page / concept keys never do), so a full or short code only ever
/// reaches the last layer. The order is a guarantee, not a coincidence — a key
/// that is both a node-page tag and a concept always renders the node page.
fn resolve_explain(target: &str, lang: &LanguageIdentifier) -> Option<String> {
    explain_kind(target)
        .or_else(|| explain_concept(target, lang))
        .or_else(|| explain_diagnostic(target, lang))
}

/// The localized "unknown explain target" error, with a "did you mean `Y`?"
/// tail when a near neighbour exists ([`nearest_target`]) and a hint pointing
/// at `aozora spec kinds`. Human-only: the suggestion and prose respect `lang`,
/// while the exit code (the machine axis) is unchanged — this string is what
/// `run_explain` bails with.
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
// Short, single-line summaries used by `aozora spec kinds` rows. The full
// multi-paragraph prose for each `NodeKind` lives in this crate under
// `src/node-docs/<kind>.md` and is surfaced verbatim by
// `aozora explain <kind>` via `include_str!`.

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
        NodeKind::Container => "Inline-attached container (字下げ系の wrap).",
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
        _ => "(unrecognised InternalCheckCode — this build is missing a summary; please report it)",
    }
}

/// Embedded prose pages for `aozora explain <tag>`, keyed by the
/// `NodeKind::as_json_tag` wire tag — the same string `aozora spec kinds`
/// prints and every driver crate emits. Bodies are `include_str!`d from
/// this crate's `src/node-docs/`.
///
/// The key must be the wire tag and nothing else. Three entries used to
/// be keyed by their filename instead (`tateChuYoko`, `sashie`,
/// `annotation`), so `aozora spec kinds` advertised `combineUpright` /
/// `illustration` / `directive` and `aozora explain` rejected all three
/// — while the pages sat right here, reachable only under names the CLI
/// never printed. `explain_reaches_every_documented_kind` pins the
/// round-trip now.
const NODE_PAGES: &[(&str, &str)] = &[
    ("ruby", include_str!("node-docs/ruby.md")),
    ("bouten", include_str!("node-docs/bouten.md")),
    ("combineUpright", include_str!("node-docs/tate-chu-yoko.md")),
    ("gaiji", include_str!("node-docs/gaiji.md")),
    ("indent", include_str!("node-docs/indent.md")),
    ("alignEnd", include_str!("node-docs/align-end.md")),
    ("warichu", include_str!("node-docs/warichu.md")),
    ("pageBreak", include_str!("node-docs/page-break.md")),
    ("sectionBreak", include_str!("node-docs/section-break.md")),
    ("heading", include_str!("node-docs/aozora-heading.md")),
    ("headingHint", include_str!("node-docs/heading-hint.md")),
    ("illustration", include_str!("node-docs/sashie.md")),
    ("kaeriten", include_str!("node-docs/kaeriten.md")),
    ("directive", include_str!("node-docs/annotation.md")),
    ("angleQuote", include_str!("node-docs/angle-quote.md")),
    ("container", include_str!("node-docs/container.md")),
    ("containerOpen", include_str!("node-docs/container-open.md")),
    (
        "containerClose",
        include_str!("node-docs/container-close.md"),
    ),
];

/// The prose page for a wire tag, falling back to the one-line summary
/// `aozora spec kinds` prints for kinds that have no page.
///
/// Without the fallback the two commands contradict each other: `explain`
/// rejects `center` and tells the reader to run `spec kinds`, which lists
/// `center` with a summary. Seven kinds are in that position. A tag the
/// tool advertises must explain to something.
fn explain_kind(tag: &str) -> Option<String> {
    if let Some((_, body)) = NODE_PAGES.iter().find(|(t, _)| *t == tag) {
        return Some((*body).to_owned());
    }
    let kind = NodeKind::ALL.iter().find(|k| k.as_json_tag() == tag)?;
    Some(format!(
        "# {tag}\n\n{}\n\nNo detailed page for this kind yet. \
         `aozora spec kinds` lists every tag, and the notation itself is at \
         https://p4suta.github.io/aozora-notation-spec/\n",
        describe_node(*kind)
    ))
}

/// Explain a diagnostic code: `aozora explain aozora::lex::unclosed_bracket`
/// (or the short `unclosed_bracket`). Prints the same code / severity / URL
/// that `aozora check` attaches to the diagnostic — the machine axis, sourced
/// from [`aozora::Diagnostic::explain`] so the two never diverge. The localized
/// title / body prose and the section labels (repro / fixed / see) are pulled
/// from the `aozora-i18n` catalog by code + `lang`; the repro / fixed example
/// pair is the language-neutral Aozora notation carried by `info`.
fn explain_diagnostic(arg: &str, lang: &LanguageIdentifier) -> Option<String> {
    // Accept the full `aozora::lex::<token>` / `aozora::lint::<token>` code or
    // the bare trailing token; expand the short form against both namespaces.
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

// ---- notation concepts ---------------------------------------------
//
// Concept / notation-family keys the reader is likely to type but that are
// not a one-to-one `NodeKind` page: abbreviations (`tcy`) and Japanese
// names (`傍点`, `ルビ`, …). Each key routes to a concept slug whose
// localized title / body prose lives in aozora-i18n as
// `concept-<slug>-{title,body}` (en / ja / zh).
//
// No key here may be a `NodeKind` wire tag. `resolve_explain` tries node
// pages first, so such an entry is unreachable — and worse, it hides the
// node page when that page is keyed by anything else. `combineUpright` sat here
// and did exactly that: the page was registered as `tateChuYoko`, so the
// tag `aozora spec kinds` prints resolved to this short concept blurb
// instead of the node page. `concepts_never_shadow_a_node_page` pins it.

/// `(typed key, concept slug)`. Several keys may share one slug (aliases). The
/// slug names the `concept-<slug>-{title,body}` catalog keys in aozora-i18n.
const CONCEPTS: &[(&str, &str)] = &[
    ("tcy", "tcy"),
    ("縦中横", "tcy"),
    ("ルビ", "ruby"),
    ("外字", "gaiji"),
    ("傍点", "bouten"),
    ("割注", "warichu"),
    ("返り点", "kaeriten"),
    ("kanbun", "kaeriten"),
];

/// Explain a notation concept: `aozora explain tcy` / `aozora explain 傍点`.
/// Renders the localized concept title + body from aozora-i18n. `None` when
/// `key` names no concept.
fn explain_concept(key: &str, lang: &LanguageIdentifier) -> Option<String> {
    let slug = CONCEPTS.iter().find(|(k, _)| *k == key).map(|(_, s)| *s)?;
    let title = i18n::t(lang, &format!("concept-{slug}-title"));
    let body = i18n::t(lang, &format!("concept-{slug}-body"));
    Some(format!("{title}\n\n{body}"))
}

// ---- "did you mean" suggestion -------------------------------------

/// Every target `resolve_explain` accepts, in a fixed order: node-page tags,
/// concept keys, then each diagnostic code in both its full (`aozora::lex::…`)
/// and short (trailing token) form. This is exactly the suggestion pool, so a
/// "did you mean `Y`?" hint always names something `explain` can actually
/// resolve — the fixed order also makes the nearest-neighbour tie-break
/// deterministic.
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

/// The nearest known [`known_targets`] entry to `target` by Levenshtein
/// distance, or `None` when even the closest is too far to be a plausible
/// typo. The cutoff is rustc's `find_best_match_for_name` rule —
/// `max(len, 3) / 3` — so short garbage like `bogus` yields no suggestion
/// while a one- or two-character slip is caught. Ties resolve to the earliest
/// entry in [`known_targets`]'s fixed order.
fn nearest_target(target: &str) -> Option<&'static str> {
    let threshold = target.chars().count().max(3) / 3;
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

/// The Levenshtein edit distance (insert / delete / substitute, unit cost)
/// between `a` and `b`, over Unicode scalar values so Japanese concept keys
/// compare per character rather than per UTF-8 byte. A hand-rolled two-row DP
/// — no dependency for one small function on the cold error path.
fn levenshtein(a: &str, b: &str) -> usize {
    let b: Vec<char> = b.chars().collect();
    // `prev[j]` = distance between the processed prefix of `a` and `b[..j]`.
    // Row 0 is the cost of deleting each prefix of `b` (all inserts).
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

    // `resolve` is the pure TTY-decision seam behind `kinds --format auto`. On
    // a terminal `Auto` renders `Human` (tables); piped, it renders `Json`.
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
        let fallback =
            "(unrecognised PairKind variant — this build is missing a summary; please report it).";
        for k in PairKind::ALL {
            assert_ne!(describe_pair(k), fallback, "{k:?} hit the fallback arm");
        }
    }

    /// `NodeKind` is `#[non_exhaustive]`, so `describe_node`'s `_` arm is
    /// mandatory from this crate and the compiler can never flag a missing
    /// variant. `BodyEnd` / `ForcedBreak` / `Emphasis` fell through it for
    /// as long as they existed, and `aozora spec kinds` printed the
    /// apology to users as those variants' summary. Only a walk of `ALL`
    /// catches that — which is why `describe_pair` has had this test and
    /// `describe_node` only had one asserting the variants it remembered.
    #[test]
    fn describe_node_covers_every_variant_without_fallback() {
        let fallback =
            "(unrecognised NodeKind variant — this build is missing a summary; please report it).";
        for k in NodeKind::ALL {
            assert_ne!(describe_node(k), fallback, "{k:?} hit the fallback arm");
        }
    }

    /// Every `NODE_PAGES` key must be the wire tag `aozora spec kinds`
    /// prints — the name `explain`'s own error tells the reader to look up.
    /// Three pages were keyed by their filename instead, so the tags
    /// `combineUpright` / `illustration` / `directive` were rejected while
    /// their pages sat in the binary.
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

    /// Each page opens with `Inspect tag: `x``, which is the command the
    /// reader is meant to type next. It must be the key that reaches the
    /// page. Three pages named their old filename-derived key, so the page
    /// you reached by `aozora explain directive` told you to run
    /// `aozora explain annotation`, which is rejected.
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

    /// The round-trip the CLI promises: `explain`'s error says to run
    /// `spec kinds`, so every tag that command advertises must explain to
    /// something. Nine of twenty-five did not — and `explain` answered
    /// seven of those by telling the reader to consult the very list they
    /// came from.
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

    /// `resolve_explain` tries node pages before concepts, so a concept
    /// keyed on a wire tag is dead — and hides the node page whenever that
    /// page is keyed by anything else. `combineUpright` was both at once.
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
        let fallback =
            "(unrecognised InternalCheckCode — this build is missing a summary; please report it)";
        for c in InternalCheckCode::ALL {
            assert_ne!(describe_internal(c), fallback, "{c:?} hit the fallback arm");
        }
    }

    // ---- edit-distance seam --------------------------------------------

    #[test]
    fn levenshtein_base_cases() {
        assert_eq!(levenshtein("", ""), 0);
        assert_eq!(levenshtein("ruby", "ruby"), 0);
        // One insert, one delete, one substitute.
        assert_eq!(levenshtein("ruby", "rubyx"), 1);
        assert_eq!(levenshtein("rubyx", "ruby"), 1);
        assert_eq!(levenshtein("ruby", "rubi"), 1);
        // Distance from the empty string is the other's length.
        assert_eq!(levenshtein("", "gaiji"), 5);
        assert_eq!(levenshtein("gaiji", ""), 5);
    }

    #[test]
    fn levenshtein_transposition_costs_two_edits() {
        // A swapped pair is two unit edits under plain Levenshtein — the value
        // the `unclsoed_bracket` typo relies on staying under threshold.
        assert_eq!(levenshtein("unclsoed_bracket", "unclosed_bracket"), 2);
    }

    #[test]
    fn levenshtein_counts_unicode_scalars_not_bytes() {
        // Japanese concept keys must compare per character; `傍点` is 6 UTF-8
        // bytes but two scalars, so a one-character change is distance 1.
        assert_eq!(levenshtein("傍点", "傍点"), 0);
        assert_eq!(levenshtein("傍点", "傍線"), 1);
        assert_eq!(levenshtein("縦中横", "縦横"), 1);
    }

    #[test]
    fn nearest_target_suggests_close_typos() {
        // A one/two-edit slip resolves to the intended target across all three
        // pools: node-page tag, diagnostic code (short + full), and concept.
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
        // `ル字` sits at edit distance 1 from exactly two known targets — the
        // concept keys `ルビ` and `外字` (each a one-substitution slip) — and no
        // closer, so it is a genuine two-way tie at the global minimum, inside
        // the length-scaled threshold. The pool visits `ルビ` before `外字`, and
        // the tie-break keeps the *earliest* entry: a strict `<` update never
        // displaces an equal-distance incumbent. (Flip it to `<=` and `外字`
        // would win instead.)
        assert_eq!(nearest_target("ル字"), Some("ルビ"));
    }

    #[test]
    fn nearest_target_declines_when_nothing_is_close() {
        // `bogus` is far from every known target — no misleading suggestion.
        assert_eq!(nearest_target("bogus"), None);
        // The cutoff is length-scaled: a single stray char on a short word is
        // still too far to guess at.
        assert_eq!(nearest_target("zzzz"), None);
    }

    #[test]
    fn known_targets_are_all_actually_resolvable() {
        // The suggestion pool must never name a target `explain` cannot resolve
        // — every entry resolves through one of the three layers.
        let en = lang_en();
        for target in known_targets() {
            assert!(
                resolve_explain(target, &en).is_some(),
                "suggestion pool entry `{target}` does not resolve",
            );
        }
    }

    // ---- resolution order: NodeKind tag > concept > diagnostic code ----

    fn lang_en() -> LanguageIdentifier {
        "en".parse().expect("`en` parses")
    }

    #[test]
    fn resolve_explain_prefers_node_page_over_concept() {
        // `ruby` is both a handbook page and the `ルビ` concept's family; the
        // node page wins, so the output is the handbook chapter, not the blurb.
        let text = resolve_explain("ruby", &lang_en()).expect("ruby resolves");
        assert!(text.contains("NodeKind::Ruby"), "node page: {text:?}");
    }

    #[test]
    fn resolve_explain_serves_concepts_that_lack_a_node_page() {
        // A concept-only key (Japanese alias / abbreviation) resolves to the
        // localized concept prose, distinct from any node page.
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
        // Full and short code forms both land on the diagnostic layer.
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
        // A near-miss carries the localized "did you mean" tail *and* the
        // `aozora spec kinds` pointer the CLI's error-hint test pins.
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
