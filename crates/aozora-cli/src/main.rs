//! `aozora` command-line frontend.
//!
//! Subcommands fall into two groups:
//!
//! Document-level (consume input, produce output):
//! - `aozora check FILE [--strict]` — run the lexer over `FILE` and
//!   report diagnostics. Exit 0 when no diagnostics; exit 1 otherwise
//!   if `--strict`, else exit 0 with diagnostics on stderr.
//! - `aozora fmt FILE [--check | --write]` — round-trip
//!   `parse ∘ serialize`. `--check` exits non-zero if the formatted
//!   output differs from `FILE`; `--write` overwrites `FILE`. Default
//!   is print-to-stdout.
//! - `aozora render FILE` — render `FILE` to HTML on stdout.
//! - `aozora wire <kind> FILE` — emit the parsed document's wire JSON
//!   for one `aozora::wire` envelope (`nodes` / `pairs` /
//!   `container-pairs` / `diagnostics` / `gaiji`), or the static
//!   `slugs` catalogue. The data counterpart to `aozora schema
//!   <kind>`, byte-identical to every binding's `*_json()` output.
//!
//! Introspection (no input required, prints typed contracts):
//! - `aozora kinds` — table of every `NodeKind` / `PairKind` /
//!   `Severity` / `DiagnosticSource` / `Sentinel` /
//!   `InternalCheckCode` variant with its wire tag and a one-line
//!   summary.
//! - `aozora schema {diagnostics|nodes|pairs|container-pairs}` —
//!   pretty-prints the JSON Schema for one of the four wire
//!   envelopes. Sourced from `aozora::wire::schema_*` (`schema`
//!   feature on the `aozora` crate).
//! - `aozora explain <kind>` — embedded handbook chapter for the
//!   given `NodeKind`, surfaced via `include_str!`.
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

#![forbid(unsafe_code)]

mod completions;
mod diagnostics_render;
mod introspect;
mod manpage;
mod timing;

use std::borrow::Cow;
use std::env;
use std::ffi::OsString;
use std::fs;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command as Process, ExitCode, Stdio};

use aozora::{DiagnosticSource, Document, wire};

use anyhow::{Context, Result};
use clap::{Parser, Subcommand, ValueEnum};

use crate::completions::CompletionsArgs;
use crate::diagnostics_render::DiagFormat;
use crate::introspect::{ExplainArgs, KindsArgs, SchemaArgs};
use crate::manpage::ManArgs;
use crate::timing::{Timer, TimingFormat};

#[derive(Debug, Parser)]
#[command(
    name = "aozora",
    about = "Aozora Bunko notation parser CLI",
    version,
    propagate_version = true,
    after_long_help = "Examples:
  aozora check FILE.txt              # lex + report diagnostics
  aozora render FILE.txt > out.html  # render to HTML
  aozora wire nodes FILE.txt         # parsed nodes as wire JSON
  aozora fmt --check FILE.txt        # CI format gate
  aozora explain unclosed_bracket    # explain a diagnostic code
  aozora completions zsh             # shell completion script

Document subcommands read stdin when given '-' or no path."
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Run the lexer over a file and report diagnostics.
    Check(CheckArgs),
    /// Round-trip parse ∘ serialize and emit the canonical form.
    Fmt(FmtArgs),
    /// Render Aozora notation to HTML on stdout.
    Render(RenderArgs),
    /// Emit a parsed document's wire JSON for one `aozora::wire`
    /// envelope — `nodes` / `pairs` / `container-pairs` /
    /// `diagnostics` / `gaiji` — or the static `slugs` catalogue. The
    /// data counterpart to `schema`: `schema <kind>` prints the JSON
    /// Schema, `wire <kind>` prints a document's data in that schema,
    /// byte-identical to every binding's `*_json()` output.
    Wire(WireArgs),
    /// Tabulate every `NodeKind` / `PairKind` / `Severity` /
    /// `DiagnosticSource` / `Sentinel` / `InternalCheckCode`
    /// variant with its wire tag.
    Kinds(KindsArgs),
    /// Pretty-print the JSON Schema for one of the four wire envelopes.
    Schema(SchemaArgs),
    /// Print prose for a `NodeKind` tag, or help / severity / URL for a
    /// diagnostic code.
    Explain(ExplainArgs),
    /// Project the parsed document to a Pandoc AST.
    /// Without `--format`, prints Pandoc JSON to stdout (consumable
    /// by `pandoc -f json -t <FORMAT>`); with `--format`, spawns
    /// pandoc and pipes the JSON through it.
    Pandoc(PandocArgs),
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

/// Flags shared by every document subcommand: where to read, how to
/// decode, and whether to print timing. Flattened into each so `file`
/// and `-E/--encoding` are declared once and `--timing` has a single
/// home (it spans check / render / fmt / wire / pandoc).
#[derive(Debug, Parser)]
struct CommonArgs {
    /// Input path; pass `-` (or omit) to read from stdin.
    #[arg(default_value = "-")]
    file: PathBuf,

    /// Source encoding.
    #[arg(long, short = 'E', value_enum, default_value_t = Encoding::Auto)]
    encoding: Encoding,

    /// Print per-phase timing (read / parse / output) to stderr. Writes
    /// only to stderr, so stdout stays byte-identical — safe to leave on
    /// inside a `render` / `wire` pipeline.
    #[arg(long)]
    timing: bool,

    /// Timing report format: `human` (aligned lines + total, the
    /// default) or `json` (a `{schema_version, phases, total_nanos}`
    /// envelope for scripts and agents). Ignored without `--timing`.
    #[arg(long, value_enum, default_value_t = TimingFormat::Human)]
    timing_format: TimingFormat,
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

    /// Exit non-zero on any diagnostic.
    #[arg(long, short = 's')]
    strict: bool,

    /// How to render diagnostics: `human` (graphical snippet, the
    /// default on a terminal), `json` (the `aozora::wire` envelope, the
    /// default when stderr is piped — the machine / agent path), or
    /// `short` (one grep-able line per diagnostic).
    #[arg(long, value_enum, default_value_t = DiagFormat::Auto)]
    diagnostic_format: DiagFormat,
}

#[derive(Debug, Parser)]
struct FmtArgs {
    #[command(flatten)]
    common: CommonArgs,

    /// Exit non-zero if the formatted output differs from the input
    /// (after the lexer's sanitize phase: BOM strip, CRLF→LF). Mutually
    /// exclusive with `--write`.
    #[arg(long, conflicts_with = "write")]
    check: bool,

    /// Overwrite the input file with the formatted output. Ignored
    /// when reading from stdin.
    #[arg(long, conflicts_with = "check")]
    write: bool,
}

#[derive(Debug, Parser)]
struct RenderArgs {
    #[command(flatten)]
    common: CommonArgs,
}

/// `aozora wire <kind>` — which wire envelope to emit. The data
/// counterpart to `SchemaKind`: `schema nodes` prints the contract,
/// `wire nodes` prints a document's data in that contract.
#[derive(Debug, Clone, Copy, ValueEnum)]
enum WireKind {
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
  aozora wire nodes src.txt           # source nodes as JSON
  cat src.txt | aozora wire pairs     # matched pairs from stdin
  aozora wire gaiji -E sjis file.txt  # resolved gaiji references
  aozora wire slugs                   # the static slug catalogue")]
struct WireArgs {
    /// Which wire envelope to emit.
    #[arg(value_enum)]
    which: WireKind,

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

#[derive(Debug, Clone, Copy, Default, ValueEnum)]
enum Encoding {
    /// Detect the source encoding: valid UTF-8 is used as-is, otherwise
    /// the bytes are decoded as Shift_JIS. The right default — Aozora
    /// files ship as Shift_JIS, but UTF-8 mirrors are common, and the
    /// caller should not have to know which they have.
    #[default]
    Auto,
    /// Force UTF-8; error if the input is not valid UTF-8.
    Utf8,
    /// Force Shift_JIS decoding.
    Sjis,
}

fn main() -> ExitCode {
    let raw: Vec<OsString> = env::args_os().collect();
    let cli = Cli::parse_from(raw);

    let result = match cli.command {
        Command::Check(opts) => run_check(&opts),
        Command::Fmt(opts) => run_fmt(&opts),
        Command::Render(opts) => run_render(&opts),
        Command::Wire(opts) => run_wire(&opts),
        Command::Kinds(opts) => introspect::run_kinds(&opts),
        Command::Schema(opts) => introspect::run_schema(&opts),
        Command::Explain(opts) => introspect::run_explain(&opts),
        Command::Pandoc(opts) => run_pandoc(&opts),
        Command::Completions(opts) => Ok(completions::run_completions(&opts)),
        Command::Man(opts) => manpage::run_man(&opts),
    };

    match result {
        Ok(code) => code,
        Err(err) => {
            let _drop = writeln!(io::stderr(), "aozora: {err:#}");
            ExitCode::FAILURE
        }
    }
}

fn run_check(args: &CheckArgs) -> Result<ExitCode> {
    let mut timer = Timer::new(args.common.timing, args.common.timing_format);
    let source = timer.measure("read", || {
        read_source(&args.common.file, args.common.encoding)
    })?;
    let doc = Document::new(source);
    let tree = timer.measure("parse", || doc.parse());
    let diagnostics = tree.diagnostics();

    let code = if diagnostics.is_empty() {
        ExitCode::SUCCESS
    } else {
        timer
            .measure("render", || {
                diagnostics_render::render(
                    args.diagnostic_format,
                    &display_path(&args.common.file),
                    &doc,
                    diagnostics,
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
        } else if args.strict {
            ExitCode::from(1)
        } else {
            ExitCode::SUCCESS
        }
    };

    timer.report()?;
    Ok(code)
}

fn run_fmt(args: &FmtArgs) -> Result<ExitCode> {
    let mut timer = Timer::new(args.common.timing, args.common.timing_format);
    let source = timer.measure("read", || {
        read_source(&args.common.file, args.common.encoding)
    })?;
    let doc = Document::new(source.clone());
    let tree = timer.measure("parse", || doc.parse());
    let formatted = timer.measure("serialize", || tree.serialize());
    // Timing covers read/parse/serialize; the comparison and I/O below
    // are not the parse cost a reader cares about, so report here.
    timer.report()?;

    // The lexer's Phase 0 sanitize strips BOM and normalises CRLF→LF;
    // the canonical form is fixed-point on the sanitized input, not
    // the raw bytes — apply the same normalisation to compare apples
    // to apples.
    let sanitized = source
        .strip_prefix('\u{feff}')
        .unwrap_or(&source)
        .replace("\r\n", "\n");

    if args.check {
        if formatted == sanitized {
            return Ok(ExitCode::SUCCESS);
        }
        let _drop = writeln!(
            io::stderr(),
            "aozora fmt: {} would be reformatted",
            display_path(&args.common.file)
        );
        return Ok(ExitCode::from(1));
    }

    if args.write && args.common.file.as_os_str() != "-" {
        fs::write(&args.common.file, &formatted)
            .with_context(|| format!("failed to write {}", display_path(&args.common.file)))?;
        return Ok(ExitCode::SUCCESS);
    }

    let mut stdout = io::stdout().lock();
    stdout
        .write_all(formatted.as_bytes())
        .context("failed to write to stdout")?;
    Ok(ExitCode::SUCCESS)
}

fn run_render(args: &RenderArgs) -> Result<ExitCode> {
    let mut timer = Timer::new(args.common.timing, args.common.timing_format);
    let source = timer.measure("read", || {
        read_source(&args.common.file, args.common.encoding)
    })?;
    let doc = Document::new(source);
    let tree = timer.measure("parse", || doc.parse());
    let html = timer.measure("render", || tree.to_html());
    let mut stdout = io::stdout().lock();
    stdout
        .write_all(html.as_bytes())
        .context("failed to write to stdout")?;
    timer.report()?;
    Ok(ExitCode::SUCCESS)
}

fn run_wire(args: &WireArgs) -> Result<ExitCode> {
    let mut timer = Timer::new(args.common.timing, args.common.timing_format);
    let json = wire_json(args, &mut timer)?;
    let mut stdout = io::stdout().lock();
    writeln!(stdout, "{json}").context("failed to write to stdout")?;
    timer.report()?;
    Ok(ExitCode::SUCCESS)
}

/// Project the requested wire envelope to its JSON string. `slugs` is a
/// static catalogue (no input read); `gaiji` scans raw source; every
/// other kind walks the parse tree. All arms delegate to
/// `aozora::wire`, the single authority shared with the Python / WASM /
/// C bindings, so the bytes are identical across every surface.
fn wire_json(args: &WireArgs, timer: &mut Timer) -> Result<String> {
    if matches!(args.which, WireKind::Slugs) {
        return Ok(wire::serialize_slugs());
    }
    let source = timer.measure("read", || {
        read_source(&args.common.file, args.common.encoding)
    })?;
    if matches!(args.which, WireKind::GaijiResolutions) {
        return Ok(timer.measure("serialize", || wire::serialize_gaiji_resolutions(&source)));
    }
    let doc = Document::new(source);
    let tree = timer.measure("parse", || doc.parse());
    Ok(timer.measure("serialize", || match args.which {
        WireKind::Nodes => wire::serialize_nodes(&tree),
        WireKind::Pairs => wire::serialize_pairs(&tree),
        WireKind::ContainerPairs => wire::serialize_container_pairs(&tree),
        WireKind::Diagnostics => wire::serialize_diagnostics(tree.diagnostics()),
        WireKind::Slugs | WireKind::GaijiResolutions => {
            unreachable!("slugs and gaiji are emitted before the parse step")
        }
    }))
}

fn run_pandoc(args: &PandocArgs) -> Result<ExitCode> {
    let mut timer = Timer::new(args.common.timing, args.common.timing_format);
    let source = timer.measure("read", || {
        read_source(&args.common.file, args.common.encoding)
    })?;
    let doc = Document::new(source);
    let tree = timer.measure("parse", || doc.parse());
    let json = timer
        .measure("pandoc", || {
            serde_json::to_string(&aozora_pandoc::to_pandoc(&tree))
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
    let raw = if path.as_os_str() == "-" {
        let mut buf = Vec::new();
        io::stdin()
            .read_to_end(&mut buf)
            .context("failed to read from stdin")?;
        buf
    } else {
        fs::read(path).with_context(|| format!("failed to read {}", display_path(path)))?
    };

    match encoding {
        Encoding::Auto => aozora_encoding::decode_auto(&raw)
            .map(Cow::into_owned)
            .map_err(|e| anyhow::anyhow!("input is neither valid UTF-8 nor Shift_JIS: {e}")),
        Encoding::Utf8 => String::from_utf8(raw)
            .map_err(|e| e.utf8_error())
            .context("input is not valid UTF-8 (use --encoding sjis for Aozora Bunko files)"),
        Encoding::Sjis => aozora_encoding::decode_sjis(&raw)
            .map_err(|e| anyhow::anyhow!("Shift_JIS decode failed: {e}")),
    }
}

fn display_path(path: &Path) -> String {
    if path.as_os_str() == "-" {
        String::from("<stdin>")
    } else {
        path.display().to_string()
    }
}
