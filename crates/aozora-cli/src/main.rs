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
//! All document-level subcommands accept `-` (or no path argument)
//! to read from stdin. Encoding defaults to UTF-8; pass
//! `--encoding sjis` (or `-E sjis`) to decode a Shift_JIS Aozora
//! Bunko file before parsing.

#![forbid(unsafe_code)]

mod introspect;

use std::env;
use std::ffi::OsString;
use std::fs;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command as Process, ExitCode, Stdio};

use aozora::Document;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand, ValueEnum};

use crate::introspect::{ExplainArgs, KindsArgs, SchemaArgs};

#[derive(Debug, Parser)]
#[command(
    name = "aozora",
    about = "Aozora Bunko notation parser CLI",
    version,
    propagate_version = true
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
    /// Tabulate every `NodeKind` / `PairKind` / `Severity` /
    /// `DiagnosticSource` / `Sentinel` / `InternalCheckCode`
    /// variant with its wire tag.
    Kinds(KindsArgs),
    /// Pretty-print the JSON Schema for one of the four wire envelopes.
    Schema(SchemaArgs),
    /// Print short prose for a `NodeKind` camelCase tag.
    Explain(ExplainArgs),
    /// Project the parsed document to a Pandoc AST.
    /// Without `--format`, prints Pandoc JSON to stdout (consumable
    /// by `pandoc -f json -t <FORMAT>`); with `--format`, spawns
    /// pandoc and pipes the JSON through it.
    Pandoc(PandocArgs),
}

#[derive(Debug, Parser)]
struct CheckArgs {
    /// Input path; pass `-` (or omit) to read from stdin.
    #[arg(default_value = "-")]
    file: PathBuf,

    /// Exit non-zero on any diagnostic.
    #[arg(long, short = 's')]
    strict: bool,

    /// Source encoding.
    #[arg(long, short = 'E', value_enum, default_value_t = Encoding::Utf8)]
    encoding: Encoding,
}

#[derive(Debug, Parser)]
struct FmtArgs {
    /// Input path; pass `-` (or omit) to read from stdin.
    #[arg(default_value = "-")]
    file: PathBuf,

    /// Exit non-zero if the formatted output differs from the input
    /// (after the lexer's sanitize phase: BOM strip, CRLF→LF). Mutually
    /// exclusive with `--write`.
    #[arg(long, conflicts_with = "write")]
    check: bool,

    /// Overwrite the input file with the formatted output. Ignored
    /// when reading from stdin.
    #[arg(long, conflicts_with = "check")]
    write: bool,

    /// Source encoding.
    #[arg(long, short = 'E', value_enum, default_value_t = Encoding::Utf8)]
    encoding: Encoding,
}

#[derive(Debug, Parser)]
struct RenderArgs {
    /// Input path; pass `-` (or omit) to read from stdin.
    #[arg(default_value = "-")]
    file: PathBuf,

    /// Source encoding.
    #[arg(long, short = 'E', value_enum, default_value_t = Encoding::Utf8)]
    encoding: Encoding,
}

#[derive(Debug, Parser)]
struct PandocArgs {
    /// Input path; pass `-` (or omit) to read from stdin.
    #[arg(default_value = "-")]
    file: PathBuf,

    /// Source encoding.
    #[arg(long, short = 'E', value_enum, default_value_t = Encoding::Utf8)]
    encoding: Encoding,

    /// Pandoc output format (e.g. `html`, `epub`, `latex`, `docx`).
    /// When set, the binary spawns `pandoc -f json -t <FORMAT>` and
    /// pipes the generated JSON through it; otherwise the Pandoc
    /// JSON itself goes to stdout.
    #[arg(long, short = 't')]
    format: Option<String>,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum Encoding {
    Utf8,
    Sjis,
}

fn main() -> ExitCode {
    let raw: Vec<OsString> = env::args_os().collect();
    let cli = Cli::parse_from(raw);

    let result = match cli.command {
        Command::Check(opts) => run_check(&opts),
        Command::Fmt(opts) => run_fmt(&opts),
        Command::Render(opts) => run_render(&opts),
        Command::Kinds(opts) => introspect::run_kinds(&opts),
        Command::Schema(opts) => introspect::run_schema(&opts),
        Command::Explain(opts) => introspect::run_explain(&opts),
        Command::Pandoc(opts) => run_pandoc(&opts),
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
    let source = read_source(&args.file, args.encoding)?;
    let doc = Document::new(source);
    let tree = doc.parse();

    if tree.diagnostics().is_empty() {
        return Ok(ExitCode::SUCCESS);
    }

    let mut stderr = io::stderr().lock();
    for diag in tree.diagnostics() {
        let _drop = writeln!(stderr, "{diag}");
    }

    if args.strict {
        Ok(ExitCode::FAILURE)
    } else {
        Ok(ExitCode::SUCCESS)
    }
}

fn run_fmt(args: &FmtArgs) -> Result<ExitCode> {
    let source = read_source(&args.file, args.encoding)?;
    let doc = Document::new(source.clone());
    let formatted = doc.parse().serialize();

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
            display_path(&args.file)
        );
        return Ok(ExitCode::from(1));
    }

    if args.write && args.file.as_os_str() != "-" {
        fs::write(&args.file, &formatted)
            .with_context(|| format!("failed to write {}", display_path(&args.file)))?;
        return Ok(ExitCode::SUCCESS);
    }

    let mut stdout = io::stdout().lock();
    stdout
        .write_all(formatted.as_bytes())
        .context("failed to write to stdout")?;
    Ok(ExitCode::SUCCESS)
}

fn run_render(args: &RenderArgs) -> Result<ExitCode> {
    let source = read_source(&args.file, args.encoding)?;
    let doc = Document::new(source);
    let html = doc.parse().to_html();
    let mut stdout = io::stdout().lock();
    stdout
        .write_all(html.as_bytes())
        .context("failed to write to stdout")?;
    Ok(ExitCode::SUCCESS)
}

fn run_pandoc(args: &PandocArgs) -> Result<ExitCode> {
    let source = read_source(&args.file, args.encoding)?;
    let doc = Document::new(source);
    let pandoc = aozora_pandoc::to_pandoc(&doc.parse());
    let json = serde_json::to_string(&pandoc).context("serialize Pandoc AST")?;

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
