//! `aozora` command-line frontend.
//!
//! Subcommands fall into two groups:
//!
//! Document-level (consume input, produce output):
//! - `aozora check FILE [--strict]` — run the lexer over `FILE` and
//!   report diagnostics. Exit 0 when no diagnostics; exit 1 otherwise
//!   if `--strict`, else exit 0 with diagnostics on stderr.
//! - `aozora lint FILE [--fix]` — report the advisory
//!   notation-hygiene lints (`aozora::lint::*`) — non-canonical
//!   directive near-misses. `--fix` rewrites the flagged near-misses
//!   in place (the Tier1 autofix; same transform as `fmt --fix
//!   --write`).
//! - `aozora fmt FILE [--check | --write]` — round-trip
//!   `parse ∘ to_source`. `--check` exits non-zero if the formatted
//!   output differs from `FILE`; `--write` overwrites `FILE`. Default
//!   is print-to-stdout.
//! - `aozora render FILE` — render `FILE` to HTML on stdout.
//! - `aozora inspect <kind> FILE` — emit the parsed document's JSON
//!   for one `aozora::json` envelope (`nodes` / `pairs` /
//!   `container-pairs` / `diagnostics` / `gaiji`). The data
//!   counterpart to `aozora spec schema <kind>`, byte-identical to
//!   every binding's `*_json()` output.
//! - `aozora pandoc FILE [--to FMT]` — project the parsed
//!   document to a Pandoc AST. Without `--to`, prints Pandoc JSON
//!   to stdout (consumable by `pandoc -f json -t FMT`); with
//!   `--to`, spawns `pandoc` and pipes the JSON through it.
//!
//! Introspection (no input required, prints typed contracts):
//! - `aozora explain <target>` — embedded handbook chapter for a
//!   `NodeKind` tag or notation concept, or the help / severity / URL
//!   for a diagnostic code.
//! - `aozora spec kinds` — table of every `NodeKind` / `PairKind` /
//!   `Severity` / `DiagnosticSource` / `Sentinel` /
//!   `InternalCheckCode` variant with its wire tag and a one-line
//!   summary.
//! - `aozora spec schema {config|diagnostics|nodes|pairs|container-pairs}` —
//!   pretty-prints the JSON Schema for the configuration file or a
//!   document envelope. Document schemas are sourced from
//!   `aozora::json::schema_*` (`schema` feature on the `aozora` crate).
//! - `aozora spec slugs` — the static ［＃…］ slug catalogue (no input).
//!
//! Onboarding (set up / inspect a working environment):
//! - `aozora init [DIR]` — scaffold a project: a commented
//!   `.aozora.toml`, a sample `hon.aozora`, and a `.gitignore`.
//! - `aozora doctor` — end-user runtime self-check: the discovered
//!   config, the effective settings and their sources, PATH tools,
//!   and terminal colour capabilities.
//!
//! Tooling:
//! - `aozora lsp [--stdio]` — run the aozora language server in-process,
//!   speaking LSP over stdio. `--stdio` is accepted (and ignored) for
//!   editor compatibility.
//! - `aozora completions <shell>` — print a shell completion script
//!   (bash / zsh / fish / powershell / elvish / nushell), generated
//!   from the live command tree.
//!
//! All document-level subcommands accept `-` (or no path argument)
//! to read from stdin. Encoding is auto-detected by default (UTF-8 if
//! the bytes are valid UTF-8, otherwise Shift_JIS); pass
//! `--encoding {utf8,sjis}` (or `-E …`) to force a specific decoder.

#![allow(
    missing_docs,
    reason = "CLI binary — internal pub items are not a documented public API surface"
)]
#![allow(
    clippy::doc_markdown,
    reason = "--help text is human-facing prose; identifiers like to_source need not be code-spanned in command help"
)]
#![forbid(unsafe_code)]

mod buildstamp;
mod color;
mod completions;
mod config;
mod diagnostics_render;
mod doctor;
mod fmt;
mod i18n;
mod init;
mod input;
mod introspect;
mod logging;
mod lsp;
mod manpage;
mod repl;
mod timing;
mod tui;
mod watch;
mod which;
mod wire;

use std::env;
use std::ffi::OsString;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::{Command as Process, ExitCode, Stdio};

use aozora::pandoc::to_pandoc;
use aozora::{DiagnosticSource, DirectiveNormalization, RenderOptions, SerializeOptions, json};
// The formatter crate owns both the source-encoding value-enum (so every
// subcommand shares one decoder) and the colour-policy enum; re-exported
// crate-wide so `config` can name them.
pub(crate) use crate::fmt::{ColorChoice, Encoding};
use crate::i18n::LanguageIdentifier;

use anyhow::{Context, Result};
use clap::builder::PossibleValue;
use clap::builder::styling::{AnsiColor, Style, Styles};
use clap::{Parser, Subcommand, ValueEnum};
use tracing::debug;

use crate::completions::CompletionsArgs;
use crate::diagnostics_render::DiagFormat;
use crate::init::InitArgs;
use crate::introspect::{ExplainArgs, KindsArgs, SchemaArgs};
use crate::lsp::LspArgs;
use crate::manpage::ManArgs;
use crate::repl::ReplArgs;
use crate::timing::Timer;
use crate::tui::TuiArgs;

/// Help / usage styling: bold-green headers and usage line (the single
/// accent), cyan literals (flag names and their values), plain placeholders.
/// clap only emits these ANSI codes when its `color` feature decides colour is
/// on, so `NO_COLOR` / `CLICOLOR` / a piped stream drop them automatically. The
/// `spec kinds` / `spec schema` tables are comfy-table, not clap, so they stay
/// monochrome regardless.
const HELP_STYLES: Styles = Styles::styled()
    .header(AnsiColor::Green.on_default().bold())
    .usage(AnsiColor::Green.on_default().bold())
    .literal(AnsiColor::Cyan.on_default())
    .placeholder(Style::new());

/// Top-level `--help` layout. clap can only file every subcommand under one
/// flat `Commands:` heading, so the daily verbs and the occasionally-consulted
/// reference material (`spec`, `explain`) all read as one undifferentiated
/// wall. This template files them into task-oriented groups instead —
/// Documents / Interactive / Introspection / Setup & tooling — with a terse
/// one-line index blurb each (each subcommand's own `--help` still carries the
/// full prose). `{usage-heading}` keeps clap's styled `Usage:`; the group and
/// `Options:` headings render through clap's plain-text arg writer, so the
/// whole block degrades to monochrome on a pipe. A `#[test]` asserts every
/// visible subcommand appears here, so a newly added command cannot silently
/// go missing from the grouped index.
const HELP_TEMPLATE: &str = "\
{about-with-newline}
{usage-heading} {usage}

Documents:
  check        Parse a document and report every diagnostic
  lint         Report advisory notation-hygiene lints (`--fix` rewrites)
  fmt          Canonicalise via parse ∘ to_source (`--check` / `--write` / `--diff`)
  render       Render Aozora notation to HTML on stdout
  inspect      Emit a document's JSON views (nodes / pairs / gaiji / …)
  pandoc       Project to a Pandoc AST (50+ output formats)

Interactive:
  repl         Interactive read-eval-print loop — type notation, see output
  tui          Full-screen live editor with preview and diagnostics

Introspection:
  explain      Explain a diagnostic code, NodeKind tag, or notation concept
  spec         Query the tool's own contracts (kinds / schema / slugs)

Setup & tooling:
  init         Scaffold a new project (`.aozora.toml` + a sample document)
  doctor       Runtime self-check — config, PATH tools, terminal capabilities
  lsp          Run the aozora language server in-process (LSP over stdio)
  completions  Print a shell completion script (bash / zsh / fish / …)

Options:
{options}{after-help}";

#[derive(Debug, Parser)]
#[command(
    name = "aozora",
    about = "Aozora Bunko notation parser CLI",
    version = crate::buildstamp::VERSION,
    propagate_version = true,
    styles = HELP_STYLES,
    help_template = HELP_TEMPLATE,
    after_long_help = "Examples:
  aozora init myproject              # scaffold a new project
  aozora check FILE.txt              # lex + report diagnostics
  aozora render FILE.txt > out.html  # render to HTML
  aozora inspect nodes FILE.txt      # parsed nodes as JSON
  aozora fmt --check FILE.txt        # CI format gate
  aozora explain unclosed_bracket    # explain a diagnostic code
  aozora spec kinds                  # the parser's typed vocabulary
  aozora completions zsh             # shell completion script

Document subcommands read stdin when given '-' or no path."
)]
struct Cli {
    #[command(subcommand)]
    command: Command,

    /// When to colourise diagnostics: `auto` (colour on a terminal,
    /// honouring `NO_COLOR` / `CLICOLOR` / `CLICOLOR_FORCE`), `always`, or
    /// `never`. Falls back to the `color` key in `.aozora.toml`, then `auto`.
    /// Global — accepted after any subcommand.
    /// Governs `check`'s graphical diagnostics and `fmt --diff`; `spec kinds`
    /// tables are always monochrome.
    #[arg(long, global = true, value_name = "WHEN", value_enum)]
    color: Option<ColorChoice>,

    /// Increase log verbosity (repeatable): `-v` info, `-vv` debug, `-vvv`
    /// trace. Logs go to stderr only, so stdout — and the JSON / short
    /// diagnostic streams — stay byte-identical. `AOZORA_LOG` overrides this;
    /// cancels against `--quiet` (one axis). Global.
    #[arg(long, short = 'v', global = true, action = clap::ArgAction::Count)]
    verbose: u8,

    /// Lower log verbosity to errors only — the opposite end of the
    /// `--verbose` axis (a single `-v` cancels it). Affects only stderr
    /// logging, never stdout. `AOZORA_LOG` overrides it. Global.
    #[arg(long, short = 'q', global = true)]
    quiet: bool,

    /// Language for human messages (the stdin guard, the `--watch` banner,
    /// and `explain`), as a BCP-47 tag: `en` (the default), `ja`, or `zh`.
    /// Highest priority in `--lang > AOZORA_LANG > .aozora.toml lang > LANG`;
    /// unknown locales fall back to `en`. Never affects machine output
    /// (json / short / codes / exit / schema) or `--encoding`. Global.
    #[arg(long, global = true, value_name = "LOCALE")]
    lang: Option<String>,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Run the lexer over a file and report diagnostics.
    Check(CheckArgs),
    /// Report notation-hygiene lints (`aozora::lint::*`) — non-canonical
    /// directive near-misses and their suggested canonical spelling. The
    /// authoring-hygiene view (`check` reports every diagnostic); `--fix`
    /// rewrites the flagged near-misses in place (the Tier1 autofix).
    Lint(LintArgs),
    /// Format documents: round-trip parse ∘ to_source to the canonical
    /// form. Reads stdin, one file, many files, or directories; `--check`
    /// / `--diff` / `--list` verify without writing, `--write` rewrites in
    /// place, `--fix` also canonicalises flagged directive near-misses.
    Fmt(FmtCmd),
    /// Render Aozora notation to HTML on stdout.
    Render(RenderArgs),
    /// Emit a parsed document's JSON for one `aozora::json`
    /// envelope — `nodes` / `pairs` / `container-pairs` /
    /// `diagnostics` / `gaiji`. The data counterpart to `spec schema`:
    /// `spec schema <kind>` prints the JSON Schema, `inspect <kind>`
    /// prints a document's data in that schema, byte-identical to every
    /// binding's `*_json()` output.
    Inspect(InspectArgs),
    /// Print prose for a `NodeKind` tag or notation concept, or help /
    /// severity / URL for a diagnostic code.
    Explain(ExplainArgs),
    /// Query the tool's own typed contracts — no document input. Groups the
    /// introspection subcommands: `kinds` (the enum / wire-tag tables),
    /// `schema <which>` (a JSON envelope's JSON Schema), and `slugs` (the
    /// static ［＃…］ catalogue).
    Spec(SpecArgs),
    /// Project the parsed document to a Pandoc AST.
    /// Without `--to`, prints Pandoc JSON to stdout (consumable
    /// by `pandoc -f json -t <FORMAT>`); with `--to`, spawns
    /// pandoc and pipes the JSON through it.
    Pandoc(PandocArgs),
    /// Run an end-user runtime self-check: the discovered `.aozora.toml` and
    /// the effective settings (with each value's source), whether `pandoc` is
    /// on `PATH`, and the terminal's colour capabilities.
    /// Exits 0 when all-green, 1 on a blocking problem (a malformed config).
    /// The runtime counterpart to the contributor-facing `just doctor`.
    Doctor,
    /// Scaffold a new Aozora notation project into `[DIR]` (default the
    /// working directory): a commented `.aozora.toml`, a sample `hon.aozora`
    /// exercising ruby / 傍点 / 字下げ so `render` and `check` work
    /// immediately, and a `.gitignore`. Existing files are kept untouched
    /// unless `--force`; idempotent. `--no-sample` / `--no-gitignore` opt out.
    Init(InitArgs),
    /// Start an interactive read-eval-print loop: type a line of notation and
    /// see its parsed nodes, rendered HTML, Pandoc AST, and diagnostics
    /// immediately (the terminal counterpart to the web playground). Reuses the
    /// same parse / render / json engine as the document subcommands, so its
    /// views can never disagree with them. `:mode` / `:lang` / `:encoding` /
    /// `:load` / `:help` / `:quit` tune the session; line editing and history
    /// come from rustyline on a terminal, and a piped stdin is scriptable.
    Repl(ReplArgs),
    /// Open a full-screen live editor: a source EDIT pane, a live PREVIEW pane
    /// (rendered HTML / nodes / Pandoc), and a DIAGNOSTICS pane, all refreshed
    /// on every keystroke (debounced) through the same parse / render / json
    /// engine as the document subcommands. The editor-free counterpart to
    /// `--watch`. `Ctrl-S` saves, `Ctrl-L` cycles language, `Ctrl-P` cycles the
    /// preview view, `Ctrl-Q` quits. An optional `[FILE]` opens for editing.
    Tui(TuiArgs),
    /// Run the aozora language server in-process, speaking LSP over stdio
    /// until the client disconnects. `--stdio` is accepted (and ignored) for
    /// editor compatibility; stdout carries the JSON-RPC wire protocol and logs
    /// go to stderr.
    Lsp(LspArgs),
    /// Print a shell completion script (`bash` / `zsh` / `fish` /
    /// `powershell` / `elvish` / `nushell`) on stdout. Generated from
    /// the live command tree, so it always matches the installed
    /// binary; release tarballs also ship these under `completions/`.
    Completions(CompletionsArgs),
    /// Render a roff man page (the top-level page, or a named
    /// subcommand's). Hidden: man pages ship in the release tarball
    /// under `man/man1/` rather than being invoked by hand.
    #[command(hide = true)]
    Man(ManArgs),
}

/// Where to read a single document.
///
/// Shared by the FILE-taking document subcommands. `fmt` reads many paths, so
/// it owns only the common document settings.
#[derive(Debug, Parser)]
struct InputArgs {
    /// Input path; pass `-` (or omit) to read from stdin.
    #[arg(default_value = "-")]
    file: PathBuf,
}

/// Settings shared by every document subcommand.
#[derive(Debug, Parser)]
struct DocumentArgs {
    /// Source encoding. Falls back to `AOZORA_ENCODING`, then the
    /// `encoding` key in `.aozora.toml`, then auto-detection.
    #[arg(long, short = 'E', value_enum, env = "AOZORA_ENCODING")]
    encoding: Option<Encoding>,

    /// How to render diagnostics: `human`, `json`, or `short`.
    ///
    /// Defaults to `human` on a terminal and `json` when piped. Falls back to
    /// `AOZORA_FORMAT`, then `.aozora.toml`.
    #[arg(long, value_enum, env = "AOZORA_FORMAT")]
    format: Option<DiagFormat>,

    /// Exit non-zero when parsing emits a diagnostic.
    ///
    /// Also settable via `AOZORA_STRICT` or `.aozora.toml`.
    #[arg(long, short = 's', env = "AOZORA_STRICT")]
    strict: bool,

    /// Read settings from this `.aozora.toml` instead of searching
    /// upward from the working directory.
    #[arg(long, value_name = "PATH")]
    config: Option<PathBuf>,

    /// Print per-phase timing (read / parse / output) to stderr. Writes
    /// only to stderr, so stdout stays byte-identical — safe to leave on
    /// inside a `render` / `inspect` pipeline. The report auto-selects its
    /// view like `check`'s diagnostics: aligned `human` lines when stderr is
    /// a terminal, a versioned `{schemaVersion,data:{phases,totalNanos}}` envelope
    /// when it is piped.
    #[arg(long)]
    timing: bool,

    /// Re-run on every change to the input file (foreground; Ctrl-C to
    /// stop). Requires a file path — not available on stdin.
    #[arg(long)]
    watch: bool,
}

/// The single-FILE document flags: input source + cross-cutting behaviour,
/// flattened by check / render / inspect / pandoc.
#[derive(Debug, Parser)]
struct CommonArgs {
    #[command(flatten)]
    input: InputArgs,
    #[command(flatten)]
    document: DocumentArgs,
}

impl CommonArgs {
    /// Load the effective `.aozora.toml`: an explicit `--config`, else an
    /// upward search from the working directory, else all-default.
    fn load_config(&self) -> Result<config::ConfigFile> {
        let cwd = env::current_dir().context("failed to read the working directory")?;
        config::ConfigFile::resolve(self.document.config.as_deref(), &cwd)
    }

    /// Effective encoding: `-E/--encoding` (or `AOZORA_ENCODING`, both via
    /// clap), else the config's `encoding`, else auto-detect.
    fn resolved_encoding(&self, cfg: &config::ConfigFile) -> Encoding {
        self.document.encoding.or(cfg.encoding).unwrap_or_default()
    }
}

#[derive(Clone, Copy, Debug)]
struct DiagnosticPolicy {
    format: DiagFormat,
    strict: bool,
}

impl DocumentArgs {
    fn diagnostic_policy(&self, cfg: &config::ConfigFile) -> DiagnosticPolicy {
        DiagnosticPolicy {
            format: self.format.or(cfg.format).unwrap_or_default(),
            strict: config::strict_active(self.strict, cfg.strict),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DocumentOutcome {
    Success,
    Strict,
    Internal,
}

impl DocumentOutcome {
    fn exit_code(self) -> ExitCode {
        match self {
            Self::Success => ExitCode::SUCCESS,
            Self::Strict => ExitCode::from(1),
            Self::Internal => ExitCode::from(3),
        }
    }
}

fn report_diagnostics(
    policy: DiagnosticPolicy,
    report: DiagnosticReport<'_>,
) -> Result<DocumentOutcome> {
    let DiagnosticReport {
        path,
        source,
        diagnostics,
        lang,
    } = report;
    if diagnostics.is_empty() {
        return Ok(DocumentOutcome::Success);
    }
    diagnostics_render::render(
        policy.format,
        &display_path(path),
        source,
        diagnostics,
        lang,
    )
    .context("failed to write diagnostics")?;
    Ok(
        if diagnostics
            .iter()
            .any(|diagnostic| diagnostic.source() == DiagnosticSource::Internal)
        {
            DocumentOutcome::Internal
        } else if policy.strict {
            DocumentOutcome::Strict
        } else {
            DocumentOutcome::Success
        },
    )
}

#[derive(Clone, Copy)]
struct DiagnosticReport<'a> {
    path: &'a Path,
    source: &'a str,
    diagnostics: &'a [aozora::Diagnostic],
    lang: &'a LanguageIdentifier,
}

#[derive(Debug, Parser)]
#[command(after_long_help = "Examples:
  aozora check src.txt          # human on a TTY, json when piped
  aozora check --strict src.txt # any diagnostic -> exit 1
  aozora check -E sjis file.txt # Shift_JIS source
  aozora check --timing src.txt # print read/parse/render timing
  cat src.txt | aozora check    # read from stdin")]
struct CheckArgs {
    #[command(flatten)]
    common: CommonArgs,
}

#[derive(Debug, Parser)]
#[command(after_long_help = "Examples:
  aozora lint src.txt           # report notation-hygiene lints
  aozora lint --strict src.txt  # any lint -> exit 1 (CI gate)
  aozora lint --fix src.txt     # rewrite flagged near-misses in place
  cat src.txt | aozora lint     # read from stdin (--fix needs a file)")]
struct LintArgs {
    #[command(flatten)]
    common: CommonArgs,

    /// Rewrite the flagged directive near-misses to their canonical spelling
    /// in place — the zero-false-positive Tier1 autofix. This is the same
    /// source transform as `aozora fmt --fix --write` (one shared engine), so
    /// it also canonicalises ruby bars and whitespace, not only directives.
    /// Needs a file path; it cannot rewrite stdin.
    #[arg(long)]
    fix: bool,
}

/// `aozora fmt` — the formatter's full surface (positional PATHs, `--check` /
/// `--write` / `--diff` / `--list` / `--json` / `--fix` / `-E/--encoding`),
/// backed by the shared `crate::fmt` engine, plus the CLI's cross-cutting
/// `--config` / `--timing` / `--watch`.
#[derive(Debug, Parser)]
struct FmtCmd {
    #[command(flatten)]
    fmt: fmt::FmtArgs,

    #[command(flatten)]
    document: DocumentArgs,
}

#[derive(Debug, Parser)]
struct RenderArgs {
    #[command(flatten)]
    common: CommonArgs,

    /// Render verified non-canonical directive near-misses as if they were
    /// their canonical spelling (e.g. `［＃「梅」は小書き］` renders 梅 as
    /// small-letter emphasis instead of an inert hidden directive span) — the
    /// same near-misses `aozora::lint::non_canonical_directive` flags and
    /// `aozora fmt --fix` rewrites. Opt-in and read-only: without this flag an
    /// unrecognised directive stays a hidden `aozora-directive` span, and this
    /// never rewrites the input source (a read-only projection applies
    /// `--normalize`; a source rewrite uses `fmt --fix`). Consults the
    /// zero-false-positive Tier1 catalogue only.
    #[arg(long)]
    normalize: bool,

    /// Additionally reduce the lossy / judgment "degraded" forms Tier1 refuses
    /// (Tier2) — e.g. `［＃中文字、ゴシック体］` renders bold, and
    /// `［＃ここから最後まで３字下げ］` an indent that runs to the document end.
    /// Implies `--normalize`, is render-only, and never rewrites the source
    /// (a Tier2 reduction applied in error can reach only this render
    /// output). Looser than `--normalize`. See ADR-0026.
    #[arg(long)]
    degraded: bool,
}

/// `aozora inspect <kind>` — which JSON envelope to emit. The data
/// counterpart to `SchemaKind`: `spec schema nodes` prints the contract,
/// `inspect nodes` prints a document's data in that contract.
///
/// The nesting is the one structural split that matters: `Gaiji` reads
/// raw source, so it is unreachable through a parse; every other kind
/// projects the parse tree. Encoding that difference in the type is what
/// lets the dispatch stay a two-arm match with no `unreachable!` stand-in.
#[derive(Debug, Clone, Copy)]
enum InspectKind {
    Gaiji,
    Tree(TreeKind),
}

/// The `inspect` kinds that project the parse tree.
#[derive(Debug, Clone, Copy)]
enum TreeKind {
    Diagnostics,
    Nodes,
    Pairs,
    ContainerPairs,
}

impl TreeKind {
    /// Render this projection of `tree` to its JSON string. Every arm
    /// delegates to `aozora::json`, the single authority shared with the
    /// Python / WASM / C bindings, so the bytes match every surface.
    fn render(self, snapshot: &aozora::Snapshot) -> String {
        match self {
            Self::Diagnostics => json::diagnostics(snapshot.diagnostics()),
            Self::Nodes => json::nodes(snapshot),
            Self::Pairs => json::pairs(snapshot),
            Self::ContainerPairs => json::container_pairs(snapshot),
        }
    }
}

// Hand-written rather than derived: `ValueEnum` cannot derive through the
// `Tree(TreeKind)` tuple variant. The `value_variants` order is the order
// `--help` lists the kinds.
impl ValueEnum for InspectKind {
    fn value_variants<'a>() -> &'a [Self] {
        &[
            Self::Tree(TreeKind::Diagnostics),
            Self::Tree(TreeKind::Nodes),
            Self::Tree(TreeKind::Pairs),
            Self::Tree(TreeKind::ContainerPairs),
            Self::Gaiji,
        ]
    }

    fn to_possible_value(&self) -> Option<PossibleValue> {
        Some(match self {
            Self::Tree(TreeKind::Diagnostics) => PossibleValue::new("diagnostics")
                .help("Per-diagnostic `{ kind, severity, source, span, codepoint? }`"),
            Self::Tree(TreeKind::Nodes) => PossibleValue::new("nodes")
                .help("Per-source-node `{ kind, span }`, sorted by `span.start`"),
            Self::Tree(TreeKind::Pairs) => {
                PossibleValue::new("pairs").help("Per-matched-pair `{ kind, open, close }`")
            }
            Self::Tree(TreeKind::ContainerPairs) => PossibleValue::new("container-pairs")
                .help("Per-container-pair `{ kind, open, close }` (source coordinates)"),
            Self::Gaiji => PossibleValue::new("gaiji")
                .help("Per-外字-reference `{ span, description, mencode, codepoint, resolved }`"),
        })
    }
}

#[derive(Debug, Parser)]
#[command(after_long_help = "Examples:
  aozora inspect nodes src.txt           # source nodes as JSON
  cat src.txt | aozora inspect pairs     # matched pairs from stdin
  aozora inspect gaiji -E sjis file.txt  # resolved gaiji references
  aozora spec schema nodes               # the *contract* for `inspect nodes`")]
struct InspectArgs {
    /// Which JSON envelope to emit.
    #[arg(value_enum)]
    which: InspectKind,

    #[command(flatten)]
    common: CommonArgs,
}

#[derive(Debug, Parser)]
struct PandocArgs {
    #[command(flatten)]
    common: CommonArgs,

    /// Pandoc output format (e.g. `html`, `epub`, `latex`, `docx`).
    /// When set, the binary spawns `pandoc -f json -t <FORMAT>` and
    /// pipes the generated JSON through it; otherwise the Pandoc
    /// JSON itself goes to stdout.
    #[arg(long, short = 't')]
    to: Option<String>,
}

/// `aozora spec <command>` — introspect the parser's own typed contracts.
/// These read no document input; each prints a machine contract (the enum /
/// wire-tag tables, a JSON envelope's JSON Schema, or the static slug
/// catalogue). Grouping them under one noun keeps the reference material out
/// of the daily-verb `--help`.
#[derive(Debug, Parser)]
#[command(after_long_help = "Examples:
  aozora spec kinds                # enum / wire-tag tables (json when piped)
  aozora spec kinds --format json  # force the machine envelope
  aozora spec schema nodes         # the JSON Schema for the `nodes` envelope
  aozora spec slugs                # the static ［＃…］ slug catalogue")]
struct SpecArgs {
    #[command(subcommand)]
    command: SpecCommand,
}

#[derive(Debug, Subcommand)]
enum SpecCommand {
    /// Tabulate every `NodeKind` / `PairKind` / `Severity` /
    /// `DiagnosticSource` / `Sentinel` / `InternalCheckCode` variant with its
    /// wire tag. `--format` selects human tables or the JSON envelope.
    Kinds(KindsArgs),
    /// Pretty-print the configuration or document-envelope JSON Schema.
    Schema(SchemaArgs),
    /// Print the static ［＃…］ slug catalogue as the shared `aozora::json`
    /// envelope — reads no document input.
    Slugs,
}

fn main() -> ExitCode {
    let raw: Vec<OsString> = env::args_os().collect();
    let cli = Cli::parse_from(raw);

    // Install the stderr tracing subscriber once, before any subcommand runs.
    // `-v`/`-q` set the default level (default `warn`); `AOZORA_LOG` overrides.
    // Writes only to stderr, so stdout and the machine diagnostic streams stay
    // byte-identical at any verbosity. (`--help` / `--version` already exited
    // during `parse_from`, so this never runs for them.)
    logging::init(cli.verbose, cli.quiet);

    // Read the `.aozora.toml` layers once, up front: both decisions below are
    // config-backed and both must be settled before any subcommand runs, so
    // the file is resolved here rather than twice. Tolerant by design — see
    // `early_config`.
    let cfg = early_config(&cli.command);

    // Install the colour hook before any diagnostic `Report` is constructed:
    // miette captures its handler at construction time (see `color::install`).
    // Precedence: `--color > .aozora.toml color` (project over global), then
    // `auto` — at which point `NO_COLOR` / `CLICOLOR` / `CLICOLOR_FORCE` and
    // the stderr TTY decide. Colour has no `AOZORA_*` rung of its own; see
    // `color`'s module docs.
    let color = color::resolve(cli.color, cfg.color);
    color::install(color);

    // Resolve the human-message language once, up front, so every surface
    // (stdin guard, watch banner, explain footer / labels) speaks the same
    // language. Precedence: `--lang > AOZORA_LANG > .aozora.toml lang > LANG`,
    // then English. The machine axis never consults it.
    let lang = resolve_lang(cli.lang.as_deref(), cfg.lang.as_deref());

    let result = match cli.command {
        Command::Check(opts) => run_check(&opts, &lang),
        Command::Lint(opts) => run_lint(&opts, &lang),
        Command::Fmt(opts) => run_fmt(&opts, color, cli.quiet, &lang),
        Command::Render(opts) => run_render(&opts, &lang),
        Command::Inspect(opts) => run_inspect(&opts, &lang),
        Command::Explain(opts) => introspect::run_explain(&opts, &lang),
        Command::Spec(opts) => run_spec(&opts),
        Command::Pandoc(opts) => run_pandoc(&opts, &lang),
        Command::Doctor => doctor::run(
            doctor::ColorFacts {
                flag: cli.color,
                resolved: color,
            },
            cli.lang.as_deref(),
            &lang,
        ),
        Command::Init(opts) => init::run(&opts, &lang),
        Command::Repl(opts) => repl::run(&opts, &lang),
        Command::Tui(opts) => tui::run(&opts, &lang),
        Command::Lsp(opts) => lsp::run(&opts),
        Command::Completions(opts) => Ok(completions::run_completions(&opts)),
        Command::Man(opts) => manpage::run_man(&opts),
    };

    match result {
        Ok(code) => code,
        Err(err) => match classify_err(&err) {
            // A reader that closed our stdout pipe early (`aozora render … |
            // head`) is a normal, silent success, not an error — see ADR-0029.
            ErrDisposition::SilentSuccess => ExitCode::SUCCESS,
            ErrDisposition::Usage => {
                let _drop = writeln!(io::stderr(), "aozora: {err:#}");
                ExitCode::from(2)
            }
        },
    }
}

/// Resolve the human-message language from the full precedence chain
/// `--lang > AOZORA_LANG > .aozora.toml lang > LANG`, then English.
///
/// `explicit` is the parsed `--lang`. `AOZORA_LANG` and `LANG` are read from
/// the environment — `LANG` for *message* language only, never for encoding
/// (ADR-0033). `config_lang` is the `lang` key from [`early_config`].
fn resolve_lang(explicit: Option<&str>, config_lang: Option<&str>) -> LanguageIdentifier {
    i18n::resolve(
        explicit,
        env::var("AOZORA_LANG").ok().as_deref(),
        config_lang,
        env::var("LANG").ok().as_deref(),
    )
}

/// The effective `.aozora.toml` for `command`, read *tolerantly* — the file
/// layer of the two settings `main` must decide before any subcommand runs:
/// the colour hook (miette captures its handler at construction time) and the
/// message language. Honours the subcommand's own `--config PATH`, so the early
/// decisions read the very file the subcommand will.
///
/// Any error (an unreadable working directory, a malformed file) yields an
/// all-default config, keeping both decisions infallible. Nothing is swallowed:
/// the subcommand re-loads the same layers through
/// [`CommonArgs::load_config`] and surfaces a malformed file as its own hard
/// error there — colour and language simply must not be the surface that
/// reports it, since the reporting itself depends on them.
fn early_config(command: &Command) -> config::ConfigFile {
    env::current_dir()
        .ok()
        .and_then(|cwd| config::ConfigFile::resolve(command_config_path(command), &cwd).ok())
        .unwrap_or_default()
}

/// The `--config PATH` override carried by `command`, if it takes one. Only the
/// document subcommands flatten `DocumentArgs`; the introspection / tooling
/// subcommands read no config file, so they contribute no config layer.
fn command_config_path(command: &Command) -> Option<&Path> {
    match command {
        Command::Check(a) => a.common.document.config.as_deref(),
        Command::Lint(a) => a.common.document.config.as_deref(),
        Command::Render(a) => a.common.document.config.as_deref(),
        Command::Inspect(a) => a.common.document.config.as_deref(),
        Command::Pandoc(a) => a.common.document.config.as_deref(),
        Command::Fmt(a) => a.document.config.as_deref(),
        Command::Explain(_)
        | Command::Spec(_)
        | Command::Doctor
        | Command::Init(_)
        | Command::Repl(_)
        | Command::Tui(_)
        | Command::Lsp(_)
        | Command::Completions(_)
        | Command::Man(_) => None,
    }
}

/// How a top-level error maps to the process's disposition — the pure decision
/// behind `main`'s final `match`, split out so both boundaries are unit-testable
/// without a real broken pipe or a 4 GiB input.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ErrDisposition {
    /// Broken pipe: exit 0 with nothing on stderr (ADR-0029).
    SilentSuccess,
    /// Bad input, configuration, arguments, or runtime prerequisites.
    Usage,
}

/// Classify a top-level `err` into its [`ErrDisposition`].
fn classify_err(err: &anyhow::Error) -> ErrDisposition {
    if fmt::is_broken_pipe(err) {
        ErrDisposition::SilentSuccess
    } else {
        ErrDisposition::Usage
    }
}

/// Run `once`, or — with `--watch` — run it now and re-run on every
/// change to the input file. `--watch` on stdin is a usage error (2).
fn run_watched(
    common: &CommonArgs,
    lang: &LanguageIdentifier,
    once: impl Fn() -> Result<ExitCode>,
) -> Result<ExitCode> {
    if !common.document.watch {
        return once();
    }
    if common.input.file.as_os_str() == "-" {
        let _drop = writeln!(
            io::stderr(),
            "aozora: --watch needs a file path; it cannot watch stdin"
        );
        return Ok(ExitCode::from(2));
    }
    watch::watch(&common.input.file, lang, once)
}

fn run_check(args: &CheckArgs, lang: &LanguageIdentifier) -> Result<ExitCode> {
    if let Some(code) = input::guard_stdin(&args.common.input.file, "check", lang) {
        return Ok(code);
    }
    run_watched(&args.common, lang, || run_check_once(args, lang))
}

fn run_check_once(args: &CheckArgs, lang: &LanguageIdentifier) -> Result<ExitCode> {
    let cfg = args.common.load_config()?;
    let encoding = args.common.resolved_encoding(&cfg);
    let policy = args.common.document.diagnostic_policy(&cfg);

    let mut timer = Timer::new(args.common.document.timing);
    let source = timer.measure("read", || read_source(&args.common.input.file, encoding))?;
    let doc = aozora::parse(source).expect("source fits parser span limit");
    let tree = timer.measure("parse", || doc.snapshot());
    let diagnostics = tree.diagnostics();

    let outcome = timer.measure("render", || {
        report_diagnostics(
            policy,
            DiagnosticReport {
                path: &args.common.input.file,
                source: doc.source(),
                diagnostics,
                lang,
            },
        )
    })?;

    timer.report()?;
    Ok(outcome.exit_code())
}

fn run_lint(args: &LintArgs, lang: &LanguageIdentifier) -> Result<ExitCode> {
    if let Some(code) = input::guard_stdin(&args.common.input.file, "lint", lang) {
        return Ok(code);
    }
    run_watched(&args.common, lang, || run_lint_once(args, lang))
}

fn run_lint_once(args: &LintArgs, lang: &LanguageIdentifier) -> Result<ExitCode> {
    let cfg = args.common.load_config()?;
    let encoding = args.common.resolved_encoding(&cfg);
    let policy = args.common.document.diagnostic_policy(&cfg);
    let path = &args.common.input.file;

    if args.fix {
        return run_lint_fix(
            path,
            LintFixSettings {
                encoding,
                policy,
                lang,
            },
        );
    }

    let mut timer = Timer::new(args.common.document.timing);
    let source = timer.measure("read", || read_source(path, encoding))?;
    let doc = aozora::parse(source).expect("source fits parser span limit");
    let tree = timer.measure("parse", || doc.snapshot());
    let lints = lint_diagnostics(&tree);
    timer.report()?;

    report_diagnostics(
        policy,
        DiagnosticReport {
            path,
            source: doc.source(),
            diagnostics: &lints,
            lang,
        },
    )
    .map(DocumentOutcome::exit_code)
}

/// The resolved lint settings threaded through `aozora lint --fix` alongside
/// the input path: the decoder, the diagnostic output format, whether residue
/// is fatal under `--strict`, and the language for rendered messages.
#[derive(Clone, Copy)]
struct LintFixSettings<'a> {
    encoding: Encoding,
    policy: DiagnosticPolicy,
    lang: &'a LanguageIdentifier,
}

/// `aozora lint --fix`: apply the Tier1 autofix in place through the same
/// guarded engine `fmt --fix --write` uses, then re-lint the result and report
/// anything the autofix could not resolve.
fn run_lint_fix(path: &Path, settings: LintFixSettings<'_>) -> Result<ExitCode> {
    let LintFixSettings {
        encoding,
        policy,
        lang,
    } = settings;
    if path.as_os_str() == "-" {
        anyhow::bail!("lint --fix needs a file path; it cannot rewrite stdin");
    }
    let opts = SerializeOptions::default().directives(DirectiveNormalization::Canonical);
    let fmt = fmt::read_and_format(path, opts, encoding)?;
    fmt::write_back(path, &fmt, opts)?;

    // Re-lint the written form: the Tier1 autofix resolves every flagged
    // near-miss, so this is normally empty, but reporting the residue keeps
    // `--fix` honest if a body was flagged yet declined a canonical.
    let doc = aozora::parse(fmt.new).expect("source fits parser span limit");
    let tree = doc.snapshot();
    let residual = lint_diagnostics(&tree);
    if residual.is_empty() {
        return Ok(ExitCode::SUCCESS);
    }
    report_diagnostics(
        policy,
        DiagnosticReport {
            path,
            source: doc.source(),
            diagnostics: &residual,
            lang,
        },
    )
    .map(DocumentOutcome::exit_code)
}

/// Diagnostics surfaced by `lint`: notation-hygiene findings plus any internal
/// parser failure that must retain the process-wide exit-3 contract.
fn lint_diagnostics(snapshot: &aozora::Snapshot) -> Vec<aozora::Diagnostic> {
    snapshot
        .diagnostics()
        .iter()
        .filter(|diagnostic| is_lint_output(diagnostic))
        .cloned()
        .collect()
}

fn is_lint_output(diagnostic: &aozora::Diagnostic) -> bool {
    diagnostic.is_lint() || diagnostic.source() == DiagnosticSource::Internal
}

fn run_fmt(
    args: &FmtCmd,
    color: ColorChoice,
    quiet: bool,
    lang: &LanguageIdentifier,
) -> Result<ExitCode> {
    // Anti-hang guard: fmt reading an interactive TTY with no file would block
    // forever. `resolve` reports whether the paths degrade to stdin.
    if matches!(fmt::resolve(args.fmt.paths()), Ok(fmt::Input::Stdin))
        && let Some(code) = input::guard_stdin(Path::new("-"), "fmt", lang)
    {
        return Ok(code);
    }
    fmt_watched(args, lang, || run_fmt_once(args, color, quiet, lang))
}

/// The concrete file paths among fmt's PATHs — every path that is not the `-`
/// stdin marker. `--watch` needs exactly one; split out so the stdin filter is
/// unit-testable.
fn watch_target_paths(paths: &[PathBuf]) -> Vec<&PathBuf> {
    paths.iter().filter(|p| p.as_os_str() != "-").collect()
}

/// `--watch` for `fmt`: re-run on every change to the single input file.
/// fmt takes many PATHs, so watch requires exactly one non-stdin path.
fn fmt_watched(
    args: &FmtCmd,
    lang: &LanguageIdentifier,
    once: impl Fn() -> Result<ExitCode>,
) -> Result<ExitCode> {
    if args.document.watch {
        let files = watch_target_paths(args.fmt.paths());
        let [path] = files.as_slice() else {
            let _drop = writeln!(
                io::stderr(),
                "aozora fmt: --watch needs exactly one file path (not stdin or multiple paths)"
            );
            return Ok(ExitCode::from(2));
        };
        watch::watch(path, lang, once)
    } else {
        once()
    }
}

fn run_fmt_once(
    args: &FmtCmd,
    color: ColorChoice,
    quiet: bool,
    lang: &LanguageIdentifier,
) -> Result<ExitCode> {
    // Fold `.aozora.toml` into the effective encoding (flag/env > config >
    // auto), then hand off to the shared formatter engine.
    let cwd = env::current_dir().context("failed to read the working directory")?;
    let cfg = config::ConfigFile::resolve(args.document.config.as_deref(), &cwd)?;
    let encoding = args.document.encoding.or(cfg.encoding).unwrap_or_default();
    let policy = args.document.diagnostic_policy(&cfg);

    // The presentation policy: `--color` drives the `--diff` hunks, while
    // `--quiet` and the resolved message language govern the TTY-gated
    // directory-fmt progress bar and localized summary on stderr.
    let presentation = fmt::Presentation {
        color,
        quiet,
        lang: lang.clone(),
        diagnostic_format: policy.format,
        strict: policy.strict,
    };
    let mut timer = Timer::new(args.document.timing);
    let code = timer.measure("format", || {
        fmt::run_engine(&args.fmt, encoding, "aozora fmt", &presentation)
    });
    timer.report()?;
    Ok(code)
}

fn run_render(args: &RenderArgs, lang: &LanguageIdentifier) -> Result<ExitCode> {
    if let Some(code) = input::guard_stdin(&args.common.input.file, "render", lang) {
        return Ok(code);
    }
    run_watched(&args.common, lang, || run_render_once(args, lang))
}

fn run_render_once(args: &RenderArgs, lang: &LanguageIdentifier) -> Result<ExitCode> {
    let cfg = args.common.load_config()?;
    let encoding = args.common.resolved_encoding(&cfg);
    let policy = args.common.document.diagnostic_policy(&cfg);
    let mut timer = Timer::new(args.common.document.timing);
    let source = timer.measure("read", || read_source(&args.common.input.file, encoding))?;
    let doc = aozora::parse(source).expect("source fits parser span limit");
    let tree = timer.measure("parse", || doc.snapshot());
    let outcome = report_diagnostics(
        policy,
        DiagnosticReport {
            path: &args.common.input.file,
            source: doc.source(),
            diagnostics: tree.diagnostics(),
            lang,
        },
    )?;
    if outcome == DocumentOutcome::Internal {
        timer.report()?;
        return Ok(outcome.exit_code());
    }
    // --degraded implies --normalize and adds Tier2; --normalize alone is
    // Tier1; neither is the byte-identical default.
    let opts = RenderOptions::default().directives(if args.degraded {
        DirectiveNormalization::Degraded
    } else if args.normalize {
        DirectiveNormalization::Canonical
    } else {
        DirectiveNormalization::Off
    });
    let html = timer.measure("render", || tree.to_html_with(opts));
    let mut stdout = io::stdout().lock();
    stdout
        .write_all(html.as_bytes())
        .context("failed to write to stdout")?;
    timer.report()?;
    Ok(outcome.exit_code())
}

fn run_inspect(args: &InspectArgs, lang: &LanguageIdentifier) -> Result<ExitCode> {
    // Every `inspect` kind reads a document (the static slug catalogue is a
    // `spec slugs` view, not an `inspect` one), so the guard is unconditional.
    if let Some(code) = input::guard_stdin(&args.common.input.file, "inspect", lang) {
        return Ok(code);
    }
    run_watched(&args.common, lang, || run_inspect_once(args, lang))
}

fn run_inspect_once(args: &InspectArgs, lang: &LanguageIdentifier) -> Result<ExitCode> {
    let mut timer = Timer::new(args.common.document.timing);
    let (json, outcome) = inspect_json(args, lang, &mut timer)?;
    if outcome == DocumentOutcome::Internal {
        timer.report()?;
        return Ok(outcome.exit_code());
    }
    let mut stdout = io::stdout().lock();
    writeln!(stdout, "{json}").context("failed to write to stdout")?;
    timer.report()?;
    Ok(outcome.exit_code())
}

/// Project the requested JSON envelope to its JSON string.
fn inspect_json(
    args: &InspectArgs,
    lang: &LanguageIdentifier,
    timer: &mut Timer,
) -> Result<(String, DocumentOutcome)> {
    let cfg = args.common.load_config()?;
    let encoding = args.common.resolved_encoding(&cfg);
    let policy = args.common.document.diagnostic_policy(&cfg);
    let source = timer.measure("read", || read_source(&args.common.input.file, encoding))?;
    let doc = aozora::parse(source).expect("source fits parser span limit");
    let tree = timer.measure("parse", || doc.snapshot());
    let outcome = report_diagnostics(
        policy,
        DiagnosticReport {
            path: &args.common.input.file,
            source: doc.source(),
            diagnostics: tree.diagnostics(),
            lang,
        },
    )?;
    let json = match args.which {
        InspectKind::Gaiji => timer.measure("serialize", || json::gaiji(&tree)),
        InspectKind::Tree(kind) => timer.measure("serialize", || kind.render(&tree)),
    };
    Ok((json, outcome))
}

/// Dispatch `aozora spec <command>` to the introspection renderers. These read
/// no document input — each prints a machine contract to stdout.
fn run_spec(args: &SpecArgs) -> Result<ExitCode> {
    match &args.command {
        SpecCommand::Kinds(opts) => introspect::run_kinds(opts),
        SpecCommand::Schema(opts) => introspect::run_schema(opts),
        SpecCommand::Slugs => introspect::run_slugs(),
    }
}

fn run_pandoc(args: &PandocArgs, lang: &LanguageIdentifier) -> Result<ExitCode> {
    if let Some(code) = input::guard_stdin(&args.common.input.file, "pandoc", lang) {
        return Ok(code);
    }
    run_watched(&args.common, lang, || run_pandoc_once(args, lang))
}

fn run_pandoc_once(args: &PandocArgs, lang: &LanguageIdentifier) -> Result<ExitCode> {
    let cfg = args.common.load_config()?;
    let encoding = args.common.resolved_encoding(&cfg);
    let policy = args.common.document.diagnostic_policy(&cfg);
    let mut timer = Timer::new(args.common.document.timing);
    let source = timer.measure("read", || read_source(&args.common.input.file, encoding))?;
    let doc = aozora::parse(source).expect("source fits parser span limit");
    let snapshot = timer.measure("parse", || doc.snapshot());
    let outcome = report_diagnostics(
        policy,
        DiagnosticReport {
            path: &args.common.input.file,
            source: doc.source(),
            diagnostics: snapshot.diagnostics(),
            lang,
        },
    )?;
    if outcome == DocumentOutcome::Internal {
        timer.report()?;
        return Ok(outcome.exit_code());
    }
    let json = timer
        .measure("pandoc", || serde_json::to_string(&to_pandoc(&snapshot)))
        .context("serialize Pandoc AST")?;
    timer.report()?;

    let Some(format) = args.to.as_deref() else {
        // No --to: emit Pandoc JSON. Downstream invocations
        // ( `aozora pandoc input.txt | pandoc -f json -t epub` )
        // pick up the bytes verbatim.
        let mut stdout = io::stdout().lock();
        stdout
            .write_all(json.as_bytes())
            .context("write Pandoc JSON to stdout")?;
        return Ok(outcome.exit_code());
    };

    // --to set: pipe through `pandoc -f json -t <format>`.
    debug!(format, "spawning `pandoc -f json -t <format>` subprocess");
    let mut child = Process::new("pandoc")
        .args(["-f", "json", "-t", format])
        .stdin(Stdio::piped())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .spawn()
        .with_context(|| {
            "failed to spawn `pandoc`; install it from https://pandoc.org or omit \
             --to to emit Pandoc JSON instead"
        })?;
    let mut stdin = child.stdin.take().context("piped stdin")?;
    stdin
        .write_all(json.as_bytes())
        .context("write Pandoc JSON to pandoc stdin")?;
    drop(stdin);
    let status = child.wait().context("wait for pandoc")?;
    Ok(pandoc_exit_code(outcome, status.success()))
}

fn pandoc_exit_code(outcome: DocumentOutcome, child_succeeded: bool) -> ExitCode {
    if child_succeeded {
        outcome.exit_code()
    } else {
        ExitCode::from(2)
    }
}

fn read_source(path: &Path, encoding: Encoding) -> Result<String> {
    debug!(
        source = %display_path(path),
        ?encoding,
        "reading and decoding input"
    );
    // The formatter crate owns both the guarded readers and the decoder, so
    // `check`/`render`/`inspect`/`pandoc` and `fmt` read and
    // resolve bytes identically — including the oversize-input rejection
    // (before the read for files, mid-read for stdin, and after decode for
    // Shift_JIS → UTF-8 expansion).
    let raw = if path.as_os_str() == "-" {
        fmt::read_stdin()?
    } else {
        fmt::read_file(path)?
    };
    fmt::decode(&raw, encoding)
}

fn display_path(path: &Path) -> String {
    if path.as_os_str() == "-" {
        String::from("<stdin>")
    } else {
        path.display().to_string()
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use super::*;

    // --- classify_err: main's final error disposition ---

    #[test]
    fn classify_err_maps_broken_pipe_to_silent_success() {
        let err = anyhow::Error::new(io::Error::from(io::ErrorKind::BrokenPipe));
        assert_eq!(classify_err(&err), ErrDisposition::SilentSuccess);
    }

    #[test]
    fn classify_err_maps_oversize_input_to_usage() {
        let err = anyhow::Error::new(fmt::OversizeInput {
            bytes: fmt::MAX_SOURCE_BYTES + 1,
        });
        assert_eq!(classify_err(&err), ErrDisposition::Usage);
    }

    #[test]
    fn classify_err_maps_other_errors_to_usage() {
        let err = anyhow::anyhow!("some unrelated failure");
        assert_eq!(classify_err(&err), ErrDisposition::Usage);
    }

    #[test]
    fn lint_output_keeps_lints_and_internal_failures_only() {
        let span = aozora::Span::new(0, 1);
        assert!(is_lint_output(
            &aozora::Diagnostic::non_canonical_directive(span, "canonical",)
        ));
        assert!(is_lint_output(&aozora::Diagnostic::internal(
            span,
            aozora::InternalCheckCode::RegistryOutOfOrder,
        )));
        assert!(!is_lint_output(&aozora::Diagnostic::source_contains_pua(
            span, '\u{E001}',
        )));
    }

    // --- watch_target_paths: fmt --watch stdin filter ---

    #[test]
    fn watch_target_paths_drops_stdin_marker() {
        let paths = vec![PathBuf::from("-"), PathBuf::from("a.txt")];
        let targets = watch_target_paths(&paths);
        // With `==` instead of `!=` this would keep only "-" and drop "a.txt".
        assert_eq!(targets.len(), 1);
        assert_eq!(targets[0].as_os_str(), "a.txt");
    }

    #[test]
    fn watch_target_paths_keeps_all_concrete_paths() {
        let paths = vec![PathBuf::from("a.txt"), PathBuf::from("b.txt")];
        let targets = watch_target_paths(&paths);
        assert_eq!(targets.len(), 2);
        assert_eq!(targets[0].as_os_str(), "a.txt");
        assert_eq!(targets[1].as_os_str(), "b.txt");
    }

    #[test]
    fn watch_target_paths_drops_a_lone_stdin_marker() {
        let paths = vec![PathBuf::from("-")];
        // With `==` this would keep the single "-" entry.
        assert!(watch_target_paths(&paths).is_empty());
    }

    #[test]
    fn fmt_without_watch_runs_once() {
        let args = FmtCmd::try_parse_from(["fmt"]).expect("fmt args");
        let calls = Cell::new(0);
        fmt_watched(&args, &resolve_lang(None, None), || {
            calls.set(calls.get() + 1);
            Ok(ExitCode::SUCCESS)
        })
        .expect("fmt dispatch");
        assert_eq!(calls.get(), 1);
    }

    // --- run_pandoc_once: real return differs from the default ---

    #[test]
    fn run_pandoc_once_propagates_read_errors() {
        // A nonexistent input file makes the read fail before any output, so the
        // real function returns Err — distinguishing it from the mutant body
        // `Ok(Default::default())`, i.e. `Ok(ExitCode::SUCCESS)`.
        let args =
            PandocArgs::try_parse_from(["pandoc", "/nonexistent/aozora-pandoc-missing-9c1f2a.txt"])
                .expect("pandoc args parse");
        run_pandoc_once(&args, &resolve_lang(None, None)).unwrap_err();
    }

    #[test]
    fn pandoc_output_and_diagnostic_formats_are_independent() {
        let args = PandocArgs::try_parse_from([
            "pandoc",
            "--format",
            "short",
            "--to",
            "html",
            "input.txt",
        ])
        .expect("pandoc args parse");
        assert!(matches!(
            args.common.document.format,
            Some(DiagFormat::Short)
        ));
        assert_eq!(args.to.as_deref(), Some("html"));
        PandocArgs::try_parse_from(["pandoc", "--format", "html", "input.txt"]).unwrap_err();
    }

    #[test]
    fn pandoc_child_failure_is_an_operational_error() {
        assert_eq!(
            pandoc_exit_code(DocumentOutcome::Success, false),
            ExitCode::from(2),
        );
        assert_eq!(
            pandoc_exit_code(DocumentOutcome::Strict, true),
            ExitCode::from(1),
        );
    }

    // --- command_config_path: threads --config into the early colour /
    //     language resolution ---

    #[test]
    fn command_config_path_carries_a_document_subcommands_override() {
        // A document subcommand's `--config PATH` must reach `early_config`; the
        // whole-body `None` mutant would silently drop every explicit config.
        let cli = Cli::try_parse_from(["aozora", "check", "--config", "custom.toml", "-"])
            .expect("cli parses");
        assert_eq!(
            command_config_path(&cli.command),
            Some(Path::new("custom.toml")),
        );
    }

    #[test]
    fn command_config_path_is_none_for_configless_subcommands() {
        // The introspection subcommands flatten no `DocumentArgs`, so they carry
        // no config layer — the `None` arm, not the whole-body mutant.
        let cli = Cli::try_parse_from(["aozora", "spec", "kinds"]).expect("cli parses");
        assert_eq!(command_config_path(&cli.command), None);
    }

    #[test]
    fn top_level_help_lists_every_visible_subcommand() {
        // The grouped `--help` template (`HELP_TEMPLATE`) names each command by
        // hand, so a newly added subcommand could silently go missing from the
        // index. Assert every visible subcommand — all but the hidden `man` and
        // clap's auto-generated `help` — appears in the rendered top-level help.
        use clap::CommandFactory;
        let mut cmd = Cli::command();
        let help = cmd.render_long_help().to_string();
        let missing: Vec<String> = cmd
            .get_subcommands()
            .filter(|s| !s.is_hide_set() && s.get_name() != "help")
            .map(|s| s.get_name().to_owned())
            .filter(|name| !help.contains(name))
            .collect();
        assert!(
            missing.is_empty(),
            "grouped top-level --help omits {missing:?}:\n{help}",
        );
    }
}
