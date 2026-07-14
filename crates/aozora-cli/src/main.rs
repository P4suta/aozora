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
//!   `container-pairs` / `diagnostics` / `gaiji`), or the static
//!   `slugs` catalogue. The data counterpart to `aozora schema
//!   <kind>`, byte-identical to every binding's `*_json()` output.
//! - `aozora pandoc FILE [--format FMT]` — project the parsed
//!   document to a Pandoc AST. Without `--format`, prints Pandoc JSON
//!   to stdout (consumable by `pandoc -f json -t FMT`); with
//!   `--format`, spawns `pandoc` and pipes the JSON through it.
//!
//! Introspection (no input required, prints typed contracts):
//! - `aozora kinds` — table of every `NodeKind` / `PairKind` /
//!   `Severity` / `DiagnosticSource` / `Sentinel` /
//!   `InternalCheckCode` variant with its wire tag and a one-line
//!   summary.
//! - `aozora schema {diagnostics|nodes|pairs|container-pairs}` —
//!   pretty-prints the JSON Schema for one of the four
//!   envelopes. Sourced from `aozora::json::schema_*` (`schema`
//!   feature on the `aozora` crate).
//! - `aozora explain <kind>` — embedded handbook chapter for the
//!   given `NodeKind`, surfaced via `include_str!`.
//!
//! Onboarding (set up / inspect a working environment):
//! - `aozora init [DIR]` — scaffold a project: a commented
//!   `.aozora.toml`, a sample `hon.aozora`, and a `.gitignore`.
//! - `aozora doctor` — end-user runtime self-check: the discovered
//!   config, the effective settings and their sources, PATH tools,
//!   and terminal colour capabilities.
//!
//! Tooling:
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

mod color;
mod completions;
mod config;
mod diagnostics_render;
mod doctor;
mod init;
mod input;
mod introspect;
mod logging;
mod manpage;
mod timing;
mod watch;

use std::env;
use std::ffi::OsString;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::{Command as Process, ExitCode, Stdio};

use aozora::{
    DiagnosticSource, Document, json,
    render::{DirectiveNormalization, RenderOptions, SerializeOptions},
};
// The formatter crate owns both the source-encoding value-enum (so both
// frontends share one decoder) and the colour-policy enum; re-exported
// crate-wide so `config` can name them.
pub(crate) use aozora_fmt::{ColorChoice, Encoding};
use aozora_i18n::{self as i18n, LanguageIdentifier};

use anyhow::{Context, Result};
use clap::builder::styling::{AnsiColor, Style, Styles};
use clap::{Parser, Subcommand, ValueEnum};
use tracing::debug;

use crate::completions::CompletionsArgs;
use crate::diagnostics_render::DiagFormat;
use crate::init::InitArgs;
use crate::introspect::{ExplainArgs, KindsArgs, SchemaArgs};
use crate::manpage::ManArgs;
use crate::timing::Timer;

/// Help / usage styling: bold-green headers and usage line (the single
/// accent), cyan literals (flag names and their values), plain placeholders.
/// clap only emits these ANSI codes when its `color` feature decides colour is
/// on, so `NO_COLOR` / `CLICOLOR` / a piped stream drop them automatically. The
/// `kinds` / `schema` tables are comfy-table, not clap, so they stay
/// monochrome regardless.
const HELP_STYLES: Styles = Styles::styled()
    .header(AnsiColor::Green.on_default().bold())
    .usage(AnsiColor::Green.on_default().bold())
    .literal(AnsiColor::Cyan.on_default())
    .placeholder(Style::new());

#[derive(Debug, Parser)]
#[command(
    name = "aozora",
    about = "Aozora Bunko notation parser CLI",
    version = aozora_buildstamp::VERSION,
    propagate_version = true,
    styles = HELP_STYLES,
    after_long_help = "Examples:
  aozora init myproject              # scaffold a new project
  aozora check FILE.txt              # lex + report diagnostics
  aozora render FILE.txt > out.html  # render to HTML
  aozora inspect nodes FILE.txt         # parsed nodes as JSON
  aozora fmt --check FILE.txt        # CI format gate
  aozora explain unclosed_bracket    # explain a diagnostic code
  aozora completions zsh             # shell completion script

Document subcommands read stdin when given '-' or no path."
)]
struct Cli {
    #[command(subcommand)]
    command: Command,

    /// When to colourise diagnostics: `auto` (colour on a terminal,
    /// honouring `NO_COLOR` / `CLICOLOR` / `CLICOLOR_FORCE`), `always`, or
    /// `never`. Global — accepted after any subcommand. Governs `check`'s
    /// graphical diagnostics; `kinds` tables are always monochrome.
    #[arg(long, global = true, value_name = "WHEN", default_value = "auto")]
    color: ColorChoice,

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
    /// `diagnostics` / `gaiji` — or the static `slugs` catalogue. The
    /// data counterpart to `schema`: `schema <kind>` prints the JSON
    /// Schema, `inspect <kind>` prints a document's data in that schema,
    /// byte-identical to every binding's `*_json()` output.
    Inspect(InspectArgs),
    /// Tabulate every `NodeKind` / `PairKind` / `Severity` /
    /// `DiagnosticSource` / `Sentinel` / `InternalCheckCode`
    /// variant with its wire tag.
    Kinds(KindsArgs),
    /// Pretty-print the JSON Schema for one of the four JSON envelopes.
    Schema(SchemaArgs),
    /// Print prose for a `NodeKind` tag or notation concept, or help /
    /// severity / URL for a diagnostic code.
    Explain(ExplainArgs),
    /// Project the parsed document to a Pandoc AST.
    /// Without `--format`, prints Pandoc JSON to stdout (consumable
    /// by `pandoc -f json -t <FORMAT>`); with `--format`, spawns
    /// pandoc and pipes the JSON through it.
    Pandoc(PandocArgs),
    /// Run an end-user runtime self-check: the discovered `.aozora.toml` and
    /// the effective settings (with each value's source), whether `pandoc` and
    /// `aozora-lsp` are on `PATH`, and the terminal's colour capabilities.
    /// Exits 0 when all-green, 1 on a blocking problem (a malformed config).
    /// The runtime counterpart to the contributor-facing `just doctor`.
    Doctor,
    /// Scaffold a new Aozora notation project into `[DIR]` (default the
    /// working directory): a commented `.aozora.toml`, a sample `hon.aozora`
    /// exercising ruby / 傍点 / 字下げ so `render` and `check` work
    /// immediately, and a `.gitignore`. Existing files are kept untouched
    /// unless `--force`; idempotent. `--no-sample` / `--no-gitignore` opt out.
    Init(InitArgs),
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

/// Where to read a single document and how to decode it — the input source
/// shared by the FILE-taking document subcommands (check / render / inspect /
/// pandoc). `fmt` reads many PATHs, so it uses `aozora_fmt::FmtArgs` instead.
#[derive(Debug, Parser)]
struct InputArgs {
    /// Input path; pass `-` (or omit) to read from stdin.
    #[arg(default_value = "-")]
    file: PathBuf,

    /// Source encoding. Falls back to `AOZORA_ENCODING`, then the
    /// `encoding` key in `.aozora.toml`, then auto-detection.
    #[arg(long, short = 'E', value_enum, env = "AOZORA_ENCODING")]
    encoding: Option<Encoding>,
}

/// Cross-cutting document-subcommand behaviour, independent of how input is
/// read: the `.aozora.toml` source and the timing / watch controls. Flattened
/// by every document subcommand — including `fmt` — so these flags are declared
/// once and behave identically across the CLI.
#[derive(Debug, Parser)]
struct CrossCutArgs {
    /// Read settings from this `.aozora.toml` instead of searching
    /// upward from the working directory.
    #[arg(long, value_name = "PATH")]
    config: Option<PathBuf>,

    /// Print per-phase timing (read / parse / output) to stderr. Writes
    /// only to stderr, so stdout stays byte-identical — safe to leave on
    /// inside a `render` / `inspect` pipeline. The report auto-selects its
    /// view like `check`'s diagnostics: aligned `human` lines when stderr is
    /// a terminal, the `{schemaVersion:1,data:{phases,totalNanos}}` envelope
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
    cross: CrossCutArgs,
}

impl CommonArgs {
    /// Load the effective `.aozora.toml`: an explicit `--config`, else an
    /// upward search from the working directory, else all-default.
    fn load_config(&self) -> Result<config::ConfigFile> {
        let cwd = env::current_dir().context("failed to read the working directory")?;
        config::ConfigFile::resolve(self.cross.config.as_deref(), &cwd)
    }

    /// Effective encoding: `-E/--encoding` (or `AOZORA_ENCODING`, both via
    /// clap), else the config's `encoding`, else auto-detect.
    fn resolved_encoding(&self, cfg: &config::ConfigFile) -> Encoding {
        self.input.encoding.or(cfg.encoding).unwrap_or_default()
    }
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

    /// Exit non-zero on any diagnostic. Also settable via `AOZORA_STRICT`
    /// or the `strict` key in `.aozora.toml`.
    #[arg(long, short = 's', env = "AOZORA_STRICT")]
    strict: bool,

    /// How to render diagnostics: `human` (graphical snippet, the
    /// default on a terminal), `json` (the `aozora::json` envelope, the
    /// default when stderr is piped — the machine / agent path), or
    /// `short` (one grep-able line per diagnostic). Falls back to
    /// `AOZORA_FORMAT`, then `.aozora.toml`.
    #[arg(long, value_enum, env = "AOZORA_FORMAT")]
    format: Option<DiagFormat>,
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

    /// Exit non-zero if any lint fired. Also settable via `AOZORA_STRICT`
    /// or the `strict` key in `.aozora.toml`. Shared with `check`.
    #[arg(long, short = 's', env = "AOZORA_STRICT")]
    strict: bool,

    /// How to render lints: `human` / `json` / `short` — the same views and
    /// `.aozora.toml` / `AOZORA_FORMAT` fallbacks as `check`.
    #[arg(long, value_enum, env = "AOZORA_FORMAT")]
    format: Option<DiagFormat>,

    /// Rewrite the flagged directive near-misses to their canonical spelling
    /// in place — the zero-false-positive Tier1 autofix. This is the same
    /// source transform as `aozora fmt --fix --write` (one shared engine), so
    /// it also canonicalises ruby bars and whitespace, not only directives.
    /// Needs a file path; it cannot rewrite stdin.
    #[arg(long)]
    fix: bool,
}

/// `aozora fmt` — the standalone formatter's full surface (positional PATHs,
/// `--check` / `--write` / `--diff` / `--list` / `--json` / `--fix` /
/// `-E/--encoding`), backed by the one `aozora-fmt` engine, plus the CLI's
/// cross-cutting `--config` / `--timing` / `--watch`. Both frontends share the
/// single `aozora_fmt::FmtArgs` definition, so the canonical form and the flag
/// vocabulary can never drift between `aozora fmt` and `aozora-fmt`.
#[derive(Debug, Parser)]
struct FmtCmd {
    #[command(flatten)]
    fmt: aozora_fmt::FmtArgs,

    #[command(flatten)]
    cross: CrossCutArgs,
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
    /// (a Tier2 misfire can reach only this render output). Looser than
    /// `--normalize`. See ADR-0026.
    #[arg(long)]
    degraded: bool,
}

/// `aozora inspect <kind>` — which JSON envelope to emit. The data
/// counterpart to `SchemaKind`: `schema nodes` prints the contract,
/// `inspect nodes` prints a document's data in that contract.
#[derive(Debug, Clone, Copy, ValueEnum)]
enum InspectKind {
    /// Per-diagnostic `{ kind, severity, source, span, codepoint? }`.
    Diagnostics,
    /// Per-source-node `{ kind, span }`, sorted by `span.start`.
    Nodes,
    /// Per-matched-pair `{ kind, open, close }`.
    Pairs,
    /// Per-container-pair `{ kind, open, close }` (normalized coordinates).
    ContainerPairs,
    /// Per-外字-reference `{ span, description, mencode, codepoint, resolved }`.
    #[value(name = "gaiji", alias = "gaiji-resolutions")]
    GaijiResolutions,
    /// The static ［＃…］ slug catalogue — reads no document input.
    Slugs,
}

#[derive(Debug, Parser)]
#[command(after_long_help = "Examples:
  aozora inspect nodes src.txt           # source nodes as JSON
  cat src.txt | aozora inspect pairs     # matched pairs from stdin
  aozora inspect gaiji -E sjis file.txt  # resolved gaiji references
  aozora inspect slugs                   # the static slug catalogue")]
struct InspectArgs {
    /// Which JSON envelope to emit.
    #[arg(value_enum)]
    which: InspectKind,

    // `common.file` is unused by `slugs` (a static catalogue with no
    // document input); every other kind reads it.
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
    format: Option<String>,
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

    // Install the colour hook before any diagnostic `Report` is constructed:
    // miette captures its handler at construction time (see `color::install`).
    color::install(cli.color);

    // Resolve the human-message language once, up front, so every surface
    // (stdin guard, watch banner, explain footer / labels) speaks the same
    // language. Precedence: `--lang > AOZORA_LANG > .aozora.toml lang > LANG`,
    // then English. The machine axis never consults it. (Introspection
    // subcommands that read no config resolve without a config layer.)
    let lang = resolve_lang(cli.lang.as_deref(), &cli.command);

    let result = match cli.command {
        Command::Check(opts) => run_check(&opts, &lang),
        Command::Lint(opts) => run_lint(&opts, &lang),
        Command::Fmt(opts) => run_fmt(&opts, cli.color, &lang),
        Command::Render(opts) => run_render(&opts, &lang),
        Command::Inspect(opts) => run_inspect(&opts, &lang),
        Command::Kinds(opts) => introspect::run_kinds(&opts),
        Command::Schema(opts) => introspect::run_schema(&opts),
        Command::Explain(opts) => introspect::run_explain(&opts, &lang),
        Command::Pandoc(opts) => run_pandoc(&opts, &lang),
        Command::Doctor => doctor::run(cli.color, cli.lang.as_deref(), &lang),
        Command::Init(opts) => init::run(&opts, &lang),
        Command::Completions(opts) => Ok(completions::run_completions(&opts)),
        Command::Man(opts) => manpage::run_man(&opts),
    };

    match result {
        Ok(code) => code,
        Err(err) => match classify_err(&err) {
            // A reader that closed our stdout pipe early (`aozora render … |
            // head`) is a normal, silent success, not an error — see ADR-0029.
            ErrDisposition::SilentSuccess => ExitCode::SUCCESS,
            // Input past the parser core's u32 span limit is a usage error (2),
            // not the generic failure (1): the graceful rejection the py/wasm
            // bindings already give, instead of the lexer assert's SIGABRT.
            ErrDisposition::Usage => {
                let _drop = writeln!(io::stderr(), "aozora: {err:#}");
                ExitCode::from(2)
            }
            ErrDisposition::Failure => {
                let _drop = writeln!(io::stderr(), "aozora: {err:#}");
                ExitCode::FAILURE
            }
        },
    }
}

/// Resolve the human-message language from the full precedence chain
/// `--lang > AOZORA_LANG > .aozora.toml lang > LANG`, then English.
///
/// `explicit` is the parsed `--lang`. `AOZORA_LANG` and `LANG` are read from
/// the environment — `LANG` for *message* language only, never for encoding
/// (ADR-0033). The config `lang` comes from a tolerant load of the same
/// `.aozora.toml` layers the subcommand will use.
fn resolve_lang(explicit: Option<&str>, command: &Command) -> LanguageIdentifier {
    let config_lang = config_lang(command);
    i18n::resolve(
        explicit,
        env::var("AOZORA_LANG").ok().as_deref(),
        config_lang.as_deref(),
        env::var("LANG").ok().as_deref(),
    )
}

/// The `lang` key from the effective `.aozora.toml` for `command`, or `None`.
/// Best-effort: any error (unreadable cwd, malformed config) resolves to `None`
/// so language resolution stays infallible — the subcommand re-loads the config
/// and surfaces a malformed file through its own normal error path.
fn config_lang(command: &Command) -> Option<String> {
    let cwd = env::current_dir().ok()?;
    config::ConfigFile::resolve(command_config_path(command), &cwd)
        .ok()?
        .lang
}

/// The `--config PATH` override carried by `command`, if it takes one. Only the
/// document subcommands flatten `CrossCutArgs`; the introspection / tooling
/// subcommands read no config file, so they contribute no config layer.
fn command_config_path(command: &Command) -> Option<&Path> {
    match command {
        Command::Check(a) => a.common.cross.config.as_deref(),
        Command::Lint(a) => a.common.cross.config.as_deref(),
        Command::Render(a) => a.common.cross.config.as_deref(),
        Command::Inspect(a) => a.common.cross.config.as_deref(),
        Command::Pandoc(a) => a.common.cross.config.as_deref(),
        Command::Fmt(a) => a.cross.config.as_deref(),
        Command::Kinds(_)
        | Command::Schema(_)
        | Command::Explain(_)
        | Command::Doctor
        | Command::Init(_)
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
    /// Oversize input: usage error, exit 2, message on stderr.
    Usage,
    /// Anything else: generic failure, exit 1, message on stderr.
    Failure,
}

/// Classify a top-level `err` into its [`ErrDisposition`]. Broken pipe wins over
/// oversize wins over the generic failure, matching `main`'s arm order.
fn classify_err(err: &anyhow::Error) -> ErrDisposition {
    if aozora_fmt::is_broken_pipe(err) {
        ErrDisposition::SilentSuccess
    } else if aozora_fmt::is_oversize_input(err) {
        ErrDisposition::Usage
    } else {
        ErrDisposition::Failure
    }
}

/// Run `once`, or — with `--watch` — run it now and re-run on every
/// change to the input file. `--watch` on stdin is a usage error (2).
fn run_watched(
    common: &CommonArgs,
    lang: &LanguageIdentifier,
    once: impl Fn() -> Result<ExitCode>,
) -> Result<ExitCode> {
    if !common.cross.watch {
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
    let format = args.format.or(cfg.format).unwrap_or_default();
    let strict = config::strict_active(args.strict, cfg.strict);

    let mut timer = Timer::new(args.common.cross.timing);
    let source = timer.measure("read", || read_source(&args.common.input.file, encoding))?;
    let doc = Document::new(source);
    let tree = timer.measure("parse", || doc.parse());
    let diagnostics = tree.diagnostics();

    let code = if diagnostics.is_empty() {
        ExitCode::SUCCESS
    } else {
        timer
            .measure("render", || {
                diagnostics_render::render(
                    format,
                    &display_path(&args.common.input.file),
                    &doc,
                    diagnostics,
                    lang,
                )
            })
            .context("failed to write diagnostics")?;

        // Exit-code contract (documented in `aozora check --help` and
        // AGENTS.md): 3 = an Internal diagnostic fired (a library bug, not
        // bad input), 1 = `--strict` with at least one diagnostic, 0 = input
        // diagnostics were printed but tolerated.
        if diagnostics
            .iter()
            .any(|d| d.source() == DiagnosticSource::Internal)
        {
            ExitCode::from(3)
        } else if strict {
            ExitCode::from(1)
        } else {
            ExitCode::SUCCESS
        }
    };

    timer.report()?;
    Ok(code)
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
    let format = args.format.or(cfg.format).unwrap_or_default();
    let strict = config::strict_active(args.strict, cfg.strict);
    let path = &args.common.input.file;

    if args.fix {
        return run_lint_fix(path, encoding, format, strict, lang);
    }

    let mut timer = Timer::new(args.common.cross.timing);
    let source = timer.measure("read", || read_source(path, encoding))?;
    let doc = Document::new(source);
    let tree = timer.measure("parse", || doc.parse());
    let lints = lint_diagnostics(&tree);
    timer.report()?;

    if lints.is_empty() {
        return Ok(ExitCode::SUCCESS);
    }
    diagnostics_render::render(format, &display_path(path), &doc, &lints, lang)
        .context("failed to write lints")?;
    // Lint codes are advisory and never `Internal`, so there is no exit-3 arm
    // (unlike `check`): tolerated by default, exit 1 under `--strict` for CI.
    Ok(if strict {
        ExitCode::from(1)
    } else {
        ExitCode::SUCCESS
    })
}

/// `aozora lint --fix`: apply the Tier1 autofix in place through the same
/// guarded engine `fmt --fix --write` uses, then re-lint the result and report
/// anything the autofix could not resolve.
#[allow(
    clippy::too_many_arguments,
    reason = "a path plus the four resolved lint settings (encoding / format / strict / lang), each independent; a bundle struct would not read more clearly"
)]
fn run_lint_fix(
    path: &Path,
    encoding: Encoding,
    format: DiagFormat,
    strict: bool,
    lang: &LanguageIdentifier,
) -> Result<ExitCode> {
    if path.as_os_str() == "-" {
        anyhow::bail!("lint --fix needs a file path; it cannot rewrite stdin");
    }
    let opts = SerializeOptions {
        directives: DirectiveNormalization::Canonical,
    };
    let fmt = aozora_fmt::read_and_format(path, opts, encoding)?;
    aozora_fmt::write_back(path, &fmt, opts)?;

    // Re-lint the written form: the Tier1 autofix resolves every flagged
    // near-miss, so this is normally empty, but reporting the residue keeps
    // `--fix` honest if a body was flagged yet declined a canonical.
    let doc = Document::new(fmt.new);
    let tree = doc.parse();
    let residual = lint_diagnostics(&tree);
    if residual.is_empty() {
        return Ok(ExitCode::SUCCESS);
    }
    diagnostics_render::render(format, &display_path(path), &doc, &residual, lang)
        .context("failed to write lints")?;
    Ok(if strict {
        ExitCode::from(1)
    } else {
        ExitCode::SUCCESS
    })
}

/// The notation-hygiene lints (`aozora::lint::*`) from a parsed tree — the
/// advisory subset `aozora lint` reports, filtered from every diagnostic.
fn lint_diagnostics(tree: &aozora::Tree<'_>) -> Vec<aozora::Diagnostic> {
    tree.diagnostics()
        .iter()
        .filter(|d| d.is_lint())
        .cloned()
        .collect()
}

fn run_fmt(args: &FmtCmd, color: ColorChoice, lang: &LanguageIdentifier) -> Result<ExitCode> {
    // Anti-hang guard: fmt reading an interactive TTY with no file would block
    // forever. `resolve` reports whether the paths degrade to stdin.
    if matches!(
        aozora_fmt::resolve(args.fmt.paths()),
        Ok(aozora_fmt::Input::Stdin)
    ) && let Some(code) = input::guard_stdin(Path::new("-"), "fmt", lang)
    {
        return Ok(code);
    }
    fmt_watched(args, lang, || run_fmt_once(args, color))
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
    if !args.cross.watch {
        return once();
    }
    let files = watch_target_paths(args.fmt.paths());
    let [path] = files.as_slice() else {
        let _drop = writeln!(
            io::stderr(),
            "aozora fmt: --watch needs exactly one file path (not stdin or multiple paths)"
        );
        return Ok(ExitCode::from(2));
    };
    watch::watch(path, lang, once)
}

fn run_fmt_once(args: &FmtCmd, color: ColorChoice) -> Result<ExitCode> {
    // Fold `.aozora.toml` into the effective encoding (flag/env > config >
    // auto), then hand off to the single shared engine — the same code the
    // standalone `aozora-fmt` binary runs, so behaviour can never diverge.
    let cwd = env::current_dir().context("failed to read the working directory")?;
    let cfg = config::ConfigFile::resolve(args.cross.config.as_deref(), &cwd)?;
    let encoding = args.fmt.encoding().or(cfg.encoding).unwrap_or_default();

    let mut timer = Timer::new(args.cross.timing);
    let code = timer.measure("format", || {
        aozora_fmt::run_engine(&args.fmt, encoding, color, "aozora fmt")
    });
    timer.report()?;
    Ok(code)
}

fn run_render(args: &RenderArgs, lang: &LanguageIdentifier) -> Result<ExitCode> {
    if let Some(code) = input::guard_stdin(&args.common.input.file, "render", lang) {
        return Ok(code);
    }
    run_watched(&args.common, lang, || run_render_once(args))
}

fn run_render_once(args: &RenderArgs) -> Result<ExitCode> {
    let cfg = args.common.load_config()?;
    let encoding = args.common.resolved_encoding(&cfg);
    let mut timer = Timer::new(args.common.cross.timing);
    let source = timer.measure("read", || read_source(&args.common.input.file, encoding))?;
    let doc = Document::new(source);
    let tree = timer.measure("parse", || doc.parse());
    let opts = RenderOptions {
        // --degraded implies --normalize and adds Tier2; --normalize alone is
        // Tier1; neither is the byte-identical default.
        directives: if args.degraded {
            DirectiveNormalization::Degraded
        } else if args.normalize {
            DirectiveNormalization::Canonical
        } else {
            DirectiveNormalization::Off
        },
    };
    let html = timer.measure("render", || tree.to_html_with(opts));
    let mut stdout = io::stdout().lock();
    stdout
        .write_all(html.as_bytes())
        .context("failed to write to stdout")?;
    timer.report()?;
    Ok(ExitCode::SUCCESS)
}

fn run_inspect(args: &InspectArgs, lang: &LanguageIdentifier) -> Result<ExitCode> {
    // `slugs` is a static catalogue that reads no input, so it must stay
    // usable on a bare terminal — guard only the kinds that read stdin.
    if inspect_reads_stdin(args.which) {
        let cmd = inspect_cmd(args.which);
        if let Some(code) = input::guard_stdin(&args.common.input.file, &cmd, lang) {
            return Ok(code);
        }
    }
    run_watched(&args.common, lang, || run_inspect_once(args))
}

/// Does this `inspect` kind read document input from stdin? Every kind but the
/// static `slugs` catalogue does — split out so the guard predicate is testable.
fn inspect_reads_stdin(kind: InspectKind) -> bool {
    !matches!(kind, InspectKind::Slugs)
}

/// The stdin-hint command string for an `inspect` kind, e.g.
/// `inspect nodes` — the value-enum tag mirrors what the user typed, so the
/// hint's `aozora <cmd> <FILE>` is copy-pasteable.
fn inspect_cmd(kind: InspectKind) -> String {
    let tag = kind
        .to_possible_value()
        .expect("every non-skipped InspectKind variant has a value-enum name")
        .get_name()
        .to_owned();
    format!("inspect {tag}")
}

fn run_inspect_once(args: &InspectArgs) -> Result<ExitCode> {
    let mut timer = Timer::new(args.common.cross.timing);
    let json = inspect_json(args, &mut timer)?;
    let mut stdout = io::stdout().lock();
    writeln!(stdout, "{json}").context("failed to write to stdout")?;
    timer.report()?;
    Ok(ExitCode::SUCCESS)
}

/// Project the requested JSON envelope to its JSON string. `slugs` is a
/// static catalogue (no input read); `gaiji` scans raw source; every
/// other kind walks the parse tree. All arms delegate to
/// `aozora::json`, the single authority shared with the Python / WASM /
/// C bindings, so the bytes are identical across every surface.
fn inspect_json(args: &InspectArgs, timer: &mut Timer) -> Result<String> {
    if matches!(args.which, InspectKind::Slugs) {
        return Ok(json::slugs());
    }
    let cfg = args.common.load_config()?;
    let encoding = args.common.resolved_encoding(&cfg);
    let source = timer.measure("read", || read_source(&args.common.input.file, encoding))?;
    if matches!(args.which, InspectKind::GaijiResolutions) {
        return Ok(timer.measure("serialize", || json::gaiji(&source)));
    }
    let doc = Document::new(source);
    let tree = timer.measure("parse", || doc.parse());
    Ok(timer.measure("serialize", || match args.which {
        InspectKind::Nodes => json::nodes(&tree),
        InspectKind::Pairs => json::pairs(&tree),
        InspectKind::ContainerPairs => json::container_pairs(&tree),
        InspectKind::Diagnostics => json::diagnostics(tree.diagnostics()),
        InspectKind::Slugs | InspectKind::GaijiResolutions => {
            unreachable!("slugs and gaiji are emitted before the parse step")
        }
    }))
}

fn run_pandoc(args: &PandocArgs, lang: &LanguageIdentifier) -> Result<ExitCode> {
    if let Some(code) = input::guard_stdin(&args.common.input.file, "pandoc", lang) {
        return Ok(code);
    }
    run_watched(&args.common, lang, || run_pandoc_once(args))
}

fn run_pandoc_once(args: &PandocArgs) -> Result<ExitCode> {
    let cfg = args.common.load_config()?;
    let encoding = args.common.resolved_encoding(&cfg);
    let mut timer = Timer::new(args.common.cross.timing);
    let source = timer.measure("read", || read_source(&args.common.input.file, encoding))?;
    let doc = Document::new(source);
    let owned = timer.measure("parse", || doc.lex());
    let json = timer
        .measure("pandoc", || {
            serde_json::to_string(&aozora_pandoc::to_pandoc(&owned))
        })
        .context("serialize Pandoc AST")?;
    timer.report()?;

    let Some(format) = args.format.as_deref() else {
        // No --format: emit Pandoc JSON. Downstream invocations
        // ( `aozora pandoc input.txt | pandoc -f json -t epub` )
        // pick up the bytes verbatim.
        let mut stdout = io::stdout().lock();
        stdout
            .write_all(json.as_bytes())
            .context("write Pandoc JSON to stdout")?;
        return Ok(ExitCode::SUCCESS);
    };

    // --format set: pipe through `pandoc -f json -t <format>`.
    debug!(format, "spawning `pandoc -f json -t <format>` subprocess");
    let mut child = Process::new("pandoc")
        .args(["-f", "json", "-t", format])
        .stdin(Stdio::piped())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .spawn()
        .with_context(|| {
            "failed to spawn `pandoc`; install it from https://pandoc.org or omit \
             --format to emit Pandoc JSON instead"
        })?;
    let mut stdin = child.stdin.take().context("piped stdin")?;
    stdin
        .write_all(json.as_bytes())
        .context("write Pandoc JSON to pandoc stdin")?;
    drop(stdin);
    let status = child.wait().context("wait for pandoc")?;
    Ok(if status.success() {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    })
}

fn read_source(path: &Path, encoding: Encoding) -> Result<String> {
    debug!(
        source = %display_path(path),
        ?encoding,
        "reading and decoding input"
    );
    // The formatter crate owns both the guarded readers and the decoder, so
    // `check`/`render`/`inspect`/`pandoc` and both `fmt` frontends read and
    // resolve bytes identically — including the oversize-input rejection
    // (before the read for files, mid-read for stdin, and after decode for
    // Shift_JIS → UTF-8 expansion).
    let raw = if path.as_os_str() == "-" {
        aozora_fmt::read_stdin()?
    } else {
        aozora_fmt::read_file(path)?
    };
    aozora_fmt::decode(&raw, encoding)
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
    use super::*;

    // --- classify_err: main's final error disposition (main.rs:417 guard) ---

    #[test]
    fn classify_err_maps_broken_pipe_to_silent_success() {
        let err = anyhow::Error::new(io::Error::from(io::ErrorKind::BrokenPipe));
        assert_eq!(classify_err(&err), ErrDisposition::SilentSuccess);
    }

    #[test]
    fn classify_err_maps_oversize_input_to_usage() {
        // is_broken_pipe is false, is_oversize_input true -> Usage (exit 2).
        // Forcing the oversize guard false would misroute this to Failure.
        let err = anyhow::Error::new(aozora_fmt::OversizeInput {
            bytes: aozora_fmt::MAX_SOURCE_BYTES + 1,
        });
        assert_eq!(classify_err(&err), ErrDisposition::Usage);
    }

    #[test]
    fn classify_err_maps_other_errors_to_failure() {
        // Neither guard matches -> Failure (exit 1). Forcing the oversize guard
        // true would misroute this to Usage.
        let err = anyhow::anyhow!("some unrelated failure");
        assert_eq!(classify_err(&err), ErrDisposition::Failure);
    }

    // --- watch_target_paths: fmt --watch stdin filter (main.rs:611 `!=`) ---

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

    // --- inspect_reads_stdin: run_inspect guard predicate (main.rs:676 `!`) ---

    #[test]
    fn inspect_reads_stdin_true_for_document_kinds() {
        assert!(inspect_reads_stdin(InspectKind::Nodes));
        assert!(inspect_reads_stdin(InspectKind::Pairs));
        assert!(inspect_reads_stdin(InspectKind::ContainerPairs));
        assert!(inspect_reads_stdin(InspectKind::Diagnostics));
        assert!(inspect_reads_stdin(InspectKind::GaijiResolutions));
    }

    #[test]
    fn inspect_reads_stdin_false_for_the_static_slugs_catalogue() {
        // Deleting the `!` would flip both this and the document-kind cases.
        assert!(!inspect_reads_stdin(InspectKind::Slugs));
    }

    // --- inspect_cmd: the copy-pasteable stdin hint (main.rs:689 body) ---

    #[test]
    fn inspect_cmd_returns_exact_command_strings() {
        // Pins every value-enum tag, killing both String::new() and
        // "xyzzy".into() whole-body replacements.
        assert_eq!(inspect_cmd(InspectKind::Nodes), "inspect nodes");
        assert_eq!(inspect_cmd(InspectKind::Pairs), "inspect pairs");
        assert_eq!(
            inspect_cmd(InspectKind::ContainerPairs),
            "inspect container-pairs"
        );
        assert_eq!(inspect_cmd(InspectKind::Diagnostics), "inspect diagnostics");
        assert_eq!(inspect_cmd(InspectKind::GaijiResolutions), "inspect gaiji");
        assert_eq!(inspect_cmd(InspectKind::Slugs), "inspect slugs");
    }

    // --- run_pandoc_once: real return differs from the default (main.rs:742) ---

    #[test]
    fn run_pandoc_once_propagates_read_errors() {
        // A nonexistent input file makes the read fail before any output, so the
        // real function returns Err — distinguishing it from the mutant body
        // `Ok(Default::default())`, i.e. `Ok(ExitCode::SUCCESS)`.
        let args =
            PandocArgs::try_parse_from(["pandoc", "/nonexistent/aozora-pandoc-missing-9c1f2a.txt"])
                .expect("pandoc args parse");
        run_pandoc_once(&args).unwrap_err();
    }

    // --- command_config_path: threads --config into language resolution
    //     (main.rs:510 body) ---

    #[test]
    fn command_config_path_carries_a_document_subcommands_override() {
        // A document subcommand's `--config PATH` must reach `config_lang`; the
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
        // The introspection subcommands flatten no `CrossCutArgs`, so they carry
        // no config layer — the `None` arm, not the whole-body mutant.
        let cli = Cli::try_parse_from(["aozora", "kinds"]).expect("cli parses");
        assert_eq!(command_config_path(&cli.command), None);
    }
}
