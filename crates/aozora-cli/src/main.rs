#![expect(
    clippy::expect_used,
    reason = "document commands validate source inputs before constructing parser spans"
)]

//! Parses commands and dispatches document, interactive, and reference operations.

#![forbid(unsafe_code)]

mod atomic_write;
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
mod lab_worker;
mod logging;
mod lsp;
mod manpage;
mod output;
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
use std::thread;

pub(crate) use crate::fmt::{ColorChoice, Encoding};
use crate::i18n::LanguageIdentifier;
use aozora::pandoc::to_pandoc;
use aozora::{DiagnosticSource, DirectiveNormalization, RenderOptions, SerializeOptions, json};

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

const HELP_STYLES: Styles = Styles::styled()
    .header(AnsiColor::Green.on_default().bold())
    .usage(AnsiColor::Green.on_default().bold())
    .literal(AnsiColor::Cyan.on_default())
    .placeholder(Style::new());

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

    /// Choose `auto`, `always`, or `never` for diagnostic color.
    /// `auto` uses terminal detection and honours `NO_COLOR`, `CLICOLOR`, and `CLICOLOR_FORCE`.
    /// Defaults to `.aozora.toml`'s `color`, then `auto`.
    /// Applies to graphical diagnostics and `fmt --diff`.
    /// `spec kinds` tables stay monochrome.
    /// Accepts placement after any subcommand.
    #[arg(long, global = true, value_name = "WHEN", value_enum)]
    color: Option<ColorChoice>,

    /// Increase stderr log verbosity with `-v` for info, `-vv` for debug, or `-vvv` for trace.
    /// Preserves stdout and diagnostic output.
    /// `AOZORA_LOG` overrides this setting.
    /// One `-v` cancels `--quiet`.
    /// Accepts placement after any subcommand.
    #[arg(long, short = 'v', global = true, action = clap::ArgAction::Count)]
    verbose: u8,

    /// Restrict stderr logging to errors.
    /// Preserves stdout.
    /// One `-v` cancels this setting.
    /// `AOZORA_LOG` overrides it.
    /// Accepts placement after any subcommand.
    #[arg(long, short = 'q', global = true)]
    quiet: bool,

    /// Choose the human message language with a `BCP-47` tag: `en`, `ja`, or `zh`.
    /// Applies to the stdin guard, watch banner, and `explain`.
    /// Resolves through `--lang > AOZORA_LANG > .aozora.toml lang > LANG`, then English.
    /// Unknown locales use English.
    /// Preserves JSON, short diagnostics, codes, exit status, schemas, and source encoding.
    /// Accepts placement after any subcommand.
    #[arg(long, global = true, value_name = "LOCALE")]
    lang: Option<String>,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Run the lexer over a file and report diagnostics.
    Check(CheckArgs),
    /// Report notation lints from `aozora::lint::*` with suggested canonical spellings for directive near-misses.
    /// `check` reports every diagnostic.
    /// `--fix` rewrites flagged near-misses in place with the Tier1 autofix.
    Lint(LintArgs),
    #[command(
        about = "Format documents: round-trip parse ∘ to_source to the canonical form. \
        Reads stdin, one file, many files, or directories; `--check` / `--diff` / `--list` \
        verify without writing, `--write` rewrites in place, `--fix` also canonicalises \
        flagged directive near-misses"
    )]
    Fmt(FmtCmd),
    /// Render Aozora notation to HTML on stdout.
    Render(RenderArgs),
    /// Emit a document's JSON envelope for `nodes`, `pairs`, `container-pairs`, `diagnostics`, or `gaiji`.
    /// `spec schema <kind>` prints the matching schema.
    /// Output matches every binding's `*_json()` output byte for byte.
    Inspect(InspectArgs),
    /// Print prose for a `NodeKind` tag or notation concept, or help / severity / URL for a diagnostic code.
    Explain(ExplainArgs),
    /// Query typed contracts without document input.
    /// `kinds` lists enums and wire tags.
    /// `schema <which>` emits JSON Schema.
    /// `slugs` lists the static `［＃…］` catalogue.
    Spec(SpecArgs),
    /// Project the parsed document to a Pandoc abstract syntax tree.
    /// Without `--to`, print JSON to stdout for `pandoc -f json -t <FORMAT>`.
    /// With `--to`, pipe the JSON through pandoc.
    Pandoc(PandocArgs),
    /// Check discovered `.aozora.toml` configuration and effective settings with their sources.
    /// Report `pandoc` availability on `PATH` and terminal color support.
    /// Exit 0 when all checks pass or 1 for a blocking configuration problem.
    /// Contributors can also use `just doctor` to check their development environment.
    Doctor,
    /// Scaffold an Aozora notation project in `[DIR]`, defaulting to the working directory.
    /// Create a commented `.aozora.toml`, a `.gitignore`, and `hon.aozora` with ruby, emphasis dots, and indentation for `render` and `check`.
    /// Preserve existing files unless `--force` permits replacement, so repeated runs are safe.
    /// Skip optional files with `--no-sample` and `--no-gitignore`.
    Init(InitArgs),
    /// Explore notation interactively with parsed nodes, HTML, Pandoc abstract syntax trees, and diagnostics for each line.
    /// Use the same engine as the document subcommands and web playground.
    /// Control the session with `:mode`, `:lang`, `:encoding`, `:load`, `:help`, and `:quit`.
    /// Rustyline provides terminal line editing and history.
    /// Piped stdin supports scripts.
    Repl(ReplArgs),
    /// Open a full-screen editor with source, preview, and diagnostic panes.
    /// Preview HTML, nodes, or Pandoc through the document subcommands' engine.
    /// Edits refresh all panes after a debounce delay, providing an alternative to `--watch`.
    /// `Ctrl-S` saves, `Ctrl-L` cycles language, `Ctrl-P` cycles preview, and `Ctrl-Q` quits.
    /// Supply `[FILE]` to open a document.
    Tui(TuiArgs),
    /// Run the aozora language server in this process over stdio until the client disconnects.
    /// Accept `--stdio` for editor compatibility.
    /// Stdout carries `JSON-RPC` and stderr carries logs.
    Lsp(LspArgs),
    /// Print a shell completion script for `bash`, `zsh`, `fish`, `powershell`, `elvish`, or `nushell` to stdout.
    /// Generate scripts from the installed binary's command tree.
    /// Release tarballs include them under `completions/`.
    Completions(CompletionsArgs),
    /// Render a roff man page for the root command or a named subcommand.
    /// This hidden command generates the pages under `man/man1/` in release tarballs.
    #[command(hide = true)]
    Man(ManArgs),
    /// Speak the release-lab protocol with one JSON value per line over stdin and stdout.
    #[command(hide = true)]
    LabWorker,
}

/// Where to read a single document.
#[derive(Debug, Parser)]
struct InputArgs {
    /// Read this path, or use `-` or no path to read stdin.
    #[arg(default_value = "-")]
    file: PathBuf,
}

/// Settings shared by every document subcommand.
#[derive(Debug, Parser)]
struct DocumentArgs {
    /// Source encoding.
    /// Falls back to `AOZORA_ENCODING`, then the `encoding` key in `.aozora.toml`, then automatic detection.
    #[arg(long, short = 'E', value_enum, env = "AOZORA_ENCODING")]
    encoding: Option<Encoding>,

    /// How to render diagnostics: `human`, `json`, or `short`.
    ///
    /// Defaults to `human` on a terminal and `json` when piped.
    /// Falls back to `AOZORA_FORMAT`, then `.aozora.toml`.
    #[arg(long, value_enum, env = "AOZORA_FORMAT")]
    format: Option<DiagFormat>,

    /// Exit non-zero when parsing emits a diagnostic.
    ///
    /// Also settable via `AOZORA_STRICT` or `.aozora.toml`.
    #[arg(long, short = 's', env = "AOZORA_STRICT")]
    strict: bool,

    /// Read this `.aozora.toml` instead of searching upward from the working directory.
    #[arg(long, value_name = "PATH")]
    config: Option<PathBuf>,

    /// Print read, parse, and output timings to stderr, preserving stdout for pipelines.
    /// Use aligned human output on a terminal or a versioned `{schemaVersion,data:{phases,totalNanos}}` JSON envelope when piped.
    #[arg(long)]
    timing: bool,

    /// Run again whenever the input file changes, until `Ctrl-C`.
    /// Requires a path rather than stdin.
    #[arg(long)]
    watch: bool,
}

#[derive(Debug, Parser)]
struct CommonArgs {
    #[command(flatten)]
    input: InputArgs,
    #[command(flatten)]
    document: DocumentArgs,
}

impl CommonArgs {
    fn load_config(&self) -> Result<config::ConfigFile> {
        let cwd = env::current_dir().context("failed to read the working directory")?;
        config::ConfigFile::resolve(self.document.config.as_deref(), &cwd)
    }

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
    const INTERNAL_EXIT_CODE: u8 = 3;

    fn exit_code(self) -> ExitCode {
        match self {
            Self::Success => ExitCode::SUCCESS,
            Self::Strict => ExitCode::from(1),
            Self::Internal => ExitCode::from(Self::INTERNAL_EXIT_CODE),
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

    /// Rewrite flagged directive near-misses in place with the Tier1 autofix, which has no false positives.
    /// Use the same transform as `aozora fmt --fix --write` to canonicalize directives, ruby bars, and whitespace.
    /// Requires a path rather than stdin.
    #[arg(long)]
    fix: bool,
}

/// Format documents with shared configuration, timing, and watch settings.
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

    /// Render verified directive near-misses using their canonical spelling without changing the source.
    /// For example, `［＃「梅」は小書き］` renders `梅` as small-letter emphasis.
    /// Uses only the Tier1 catalogue with no false positives, matching `aozora::lint::non_canonical_directive` and `aozora fmt --fix`.
    /// Without this flag, unrecognized directives remain hidden `aozora-directive` spans.
    #[arg(long)]
    normalize: bool,

    /// Also render the lossy Tier2 forms that Tier1 refuses.
    /// For example, `［＃中文字、ゴシック体］` renders bold and `［＃ここから最後まで３字下げ］` indents through the document end.
    /// Implies `--normalize` with looser reductions that affect only rendered output, preserving the source even for mistaken reductions.
    /// See ADR-0026.
    #[arg(long)]
    degraded: bool,
}

#[derive(Debug, Clone, Copy)]
enum InspectKind {
    Gaiji,
    Tree(TreeKind),
}

#[derive(Debug, Clone, Copy)]
enum TreeKind {
    Diagnostics,
    Nodes,
    Pairs,
    ContainerPairs,
}

impl TreeKind {
    fn render(self, snapshot: &aozora::Snapshot) -> String {
        match self {
            Self::Diagnostics => json::diagnostics(snapshot.diagnostics()),
            Self::Nodes => json::nodes(snapshot),
            Self::Pairs => json::pairs(snapshot),
            Self::ContainerPairs => json::container_pairs(snapshot),
        }
    }
}

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

    /// Choose a Pandoc output format such as `html`, `epub`, `latex`, or `docx`.
    /// Runs `pandoc -f json -t <FORMAT>` with the generated JSON.
    /// Omit to print Pandoc JSON to stdout.
    #[arg(long, short = 't')]
    to: Option<String>,
}

/// Inspect typed contracts without document input.
/// Print enum and wire-tag tables, JSON Schema, or the static slug catalogue.
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
    /// List each variant and wire tag for `NodeKind`, `PairKind`, `Severity`, `DiagnosticSource`, `Sentinel`, and `InternalCheckCode`.
    /// `--format` selects human tables or the JSON envelope.
    Kinds(KindsArgs),
    /// Pretty-print the configuration or document-envelope JSON Schema.
    Schema(SchemaArgs),
    /// Print the static `［＃…］` slug catalogue as the shared `aozora::json` envelope without document input.
    Slugs,
}

fn main() -> ExitCode {
    let raw: Vec<OsString> = env::args_os().collect();
    let cli = Cli::parse_from(raw);

    logging::init(cli.verbose, cli.quiet);

    let cfg = early_config(&cli.command);

    let color = color::resolve(cli.color, cfg.color);
    color::install(color);

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
        Command::Completions(opts) => completions::run_completions(&opts),
        Command::Man(opts) => manpage::run_man(&opts),
        Command::LabWorker => lab_worker::run(),
    };

    match result {
        Ok(code) => code,
        Err(err) => match classify_err(&err) {
            // ADR-0029 treats a closed output pipe as success.
            ErrDisposition::SilentSuccess => ExitCode::SUCCESS,
            ErrDisposition::Usage => {
                let _drop = writeln!(io::stderr(), "aozora: {err:#}");
                ExitCode::from(2)
            }
        },
    }
}

/// ADR-0033 separates message language from source encoding.
fn resolve_lang(explicit: Option<&str>, config_lang: Option<&str>) -> LanguageIdentifier {
    i18n::resolve(
        explicit,
        env::var("AOZORA_LANG").ok().as_deref(),
        config_lang,
        env::var("LANG").ok().as_deref(),
    )
}

/// Uses defaults on configuration errors so diagnostic presentation can initialize before commands report those errors.
fn early_config(command: &Command) -> config::ConfigFile {
    env::current_dir()
        .ok()
        .and_then(|cwd| config::ConfigFile::resolve(command_config_path(command), &cwd).ok())
        .unwrap_or_default()
}

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
        | Command::Man(_)
        | Command::LabWorker => None,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ErrDisposition {
    /// ADR-0029 requires silent success for a closed output pipe.
    SilentSuccess,
    Usage,
}

fn classify_err(err: &anyhow::Error) -> ErrDisposition {
    if output::is_broken_pipe(err) {
        ErrDisposition::SilentSuccess
    } else {
        ErrDisposition::Usage
    }
}

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

#[derive(Clone, Copy)]
struct LintFixSettings<'a> {
    encoding: Encoding,
    policy: DiagnosticPolicy,
    lang: &'a LanguageIdentifier,
}

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
    if matches!(fmt::resolve(args.fmt.paths()), Ok(fmt::Input::Stdin))
        && let Some(code) = input::guard_stdin(Path::new("-"), "fmt", lang)
    {
        return Ok(code);
    }
    fmt_watched(args, lang, || run_fmt_once(args, color, quiet, lang))
}

fn watch_target_paths(paths: &[PathBuf]) -> Vec<&PathBuf> {
    paths.iter().filter(|p| p.as_os_str() != "-").collect()
}

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
    let cwd = env::current_dir().context("failed to read the working directory")?;
    let cfg = config::ConfigFile::resolve(args.document.config.as_deref(), &cwd)?;
    let encoding = args.document.encoding.or(cfg.encoding).unwrap_or_default();
    let policy = args.document.diagnostic_policy(&cfg);

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
    let opts = RenderOptions::default().directives(if args.degraded {
        DirectiveNormalization::Degraded
    } else if args.normalize {
        DirectiveNormalization::Canonical
    } else {
        DirectiveNormalization::Off
    });
    let html = timer.measure("render", || tree.to_html_with(opts));
    let mut stdout = output::stdout();
    stdout
        .write_all(html.as_bytes())
        .context("failed to write to stdout")?;
    timer.report()?;
    Ok(outcome.exit_code())
}

fn run_inspect(args: &InspectArgs, lang: &LanguageIdentifier) -> Result<ExitCode> {
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
    let mut stdout = output::stdout();
    writeln!(stdout, "{json}").context("failed to write to stdout")?;
    timer.report()?;
    Ok(outcome.exit_code())
}

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
        let mut stdout = output::stdout();
        stdout
            .write_all(json.as_bytes())
            .context("write Pandoc JSON to stdout")?;
        return Ok(outcome.exit_code());
    };

    debug!(format, "spawning `pandoc -f json -t <format>` subprocess");
    let mut child = Process::new("pandoc")
        .args(["-f", "json", "-t", format])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .with_context(|| {
            const PANDOC_INSTALL_URL: &str = "https://pandoc.org";
            format!(
                "failed to spawn `pandoc`; install it from {PANDOC_INSTALL_URL} or omit \
             --to to emit Pandoc JSON instead"
            )
        })?;
    let mut child_stdout = child.stdout.take().context("piped stdout")?;
    let stdout_forwarder = match thread::Builder::new()
        .name("aozora-pandoc-stdout".to_owned())
        .spawn(move || {
            let mut stdout = output::stdout();
            io::copy(&mut child_stdout, &mut stdout)
        }) {
        Ok(forwarder) => forwarder,
        Err(error) => {
            let _drop = child.kill();
            let _drop = child.wait();
            return Err(error).context("failed to spawn the pandoc stdout forwarding thread");
        }
    };
    let mut stdin = child.stdin.take().context("piped stdin")?;
    let write_result = stdin.write_all(json.as_bytes());
    drop(stdin);
    let status = child.wait().context("wait for pandoc")?;
    let forward_result = stdout_forwarder
        .join()
        .map_err(|_| anyhow::anyhow!("pandoc stdout forwarding thread panicked"))?;
    if let Some(code) = finish_pandoc_pipes(write_result, forward_result)? {
        return Ok(code);
    }
    Ok(pandoc_exit_code(outcome, status.success()))
}

fn finish_pandoc_pipes(
    write_result: io::Result<()>,
    forward_result: io::Result<u64>,
) -> Result<Option<ExitCode>> {
    forward_result.context("forward pandoc stdout")?;
    if let Err(error) = write_result {
        if error.kind() == io::ErrorKind::BrokenPipe {
            return Ok(Some(ExitCode::from(2)));
        }
        return Err(error).context("write Pandoc JSON to pandoc stdin");
    }
    Ok(None)
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

    struct BrokenPipeWriter;

    impl Write for BrokenPipeWriter {
        fn write(&mut self, _buf: &[u8]) -> io::Result<usize> {
            Err(io::ErrorKind::BrokenPipe.into())
        }

        fn flush(&mut self) -> io::Result<()> {
            Err(io::ErrorKind::BrokenPipe.into())
        }
    }

    fn stdout_broken_pipe_error() -> anyhow::Error {
        let mut stdout = output::guard(BrokenPipeWriter);
        anyhow::Error::new(stdout.write_all(b"x").unwrap_err())
    }

    #[test]
    fn classify_err_maps_broken_pipe_to_silent_success() {
        let err = stdout_broken_pipe_error();
        assert_eq!(classify_err(&err), ErrDisposition::SilentSuccess);
    }

    #[test]
    fn classify_err_does_not_silence_unmarked_broken_pipe() {
        let err = anyhow::Error::new(io::Error::from(io::ErrorKind::BrokenPipe))
            .context("failed to write a file");
        assert_eq!(classify_err(&err), ErrDisposition::Usage);
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

    #[test]
    fn watch_target_paths_drops_stdin_marker() {
        let paths = vec![PathBuf::from("-"), PathBuf::from("a.txt")];
        let targets = watch_target_paths(&paths);
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
        assert_eq!(watch_target_paths(&paths), Vec::<&PathBuf>::new());
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

    #[test]
    fn run_pandoc_once_propagates_read_errors() {
        let args =
            PandocArgs::try_parse_from(["pandoc", "/nonexistent/aozora-pandoc-missing-9c1f2a.txt"])
                .expect("pandoc args parse");
        run_pandoc_once(&args, &resolve_lang(None, None)).unwrap_err();
    }

    #[test]
    fn pandoc_stdout_broken_pipe_takes_priority_over_child_stdin_broken_pipe() {
        struct BrokenWriter;

        impl Write for BrokenWriter {
            fn write(&mut self, _buf: &[u8]) -> io::Result<usize> {
                Err(io::ErrorKind::BrokenPipe.into())
            }

            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }

        let mut stdout = output::guard(BrokenWriter);
        let forward_result = stdout.write_all(b"x").map(|()| 1);
        let write_result = Err(io::ErrorKind::BrokenPipe.into());

        let error = finish_pandoc_pipes(write_result, forward_result)
            .expect_err("stdout closure must remain visible");

        assert!(output::is_broken_pipe(&error));
    }

    #[test]
    fn pandoc_child_stdin_broken_pipe_is_an_operational_error() {
        let result = finish_pandoc_pipes(Err(io::ErrorKind::BrokenPipe.into()), Ok(0))
            .expect("pandoc closing stdin is a classified operational failure");

        assert_eq!(result, Some(ExitCode::from(2)));
    }

    #[test]
    fn pandoc_child_stdin_non_pipe_error_is_propagated() {
        let error = finish_pandoc_pipes(Err(io::ErrorKind::PermissionDenied.into()), Ok(0))
            .expect_err("non-pipe stdin errors must retain their cause");
        let io_error = error
            .downcast_ref::<io::Error>()
            .expect("stdin error remains in the anyhow chain");

        assert_eq!(io_error.kind(), io::ErrorKind::PermissionDenied);
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

    #[test]
    fn command_config_path_carries_a_document_subcommands_override() {
        let cli = Cli::try_parse_from(["aozora", "check", "--config", "custom.toml", "-"])
            .expect("cli parses");
        assert_eq!(
            command_config_path(&cli.command),
            Some(Path::new("custom.toml")),
        );
    }

    #[test]
    fn command_config_path_is_none_for_configless_subcommands() {
        let cli = Cli::try_parse_from(["aozora", "spec", "kinds"]).expect("cli parses");
        assert_eq!(command_config_path(&cli.command), None);
    }

    #[test]
    fn top_level_help_lists_every_visible_subcommand() {
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
