//! `aozora repl` — an interactive read-eval-print loop for the notation.
//!
//! The terminal counterpart to the web playground: type one line of Aozora
//! notation and see its parsed `nodes`, rendered `html`, projected Pandoc AST,
//! and diagnostics immediately. Ideal for learning the notation and prototyping
//! a fragment without creating a file.
//!
//! It owns **no** parsing, rendering, or diagnostic logic. Every view is the
//! exact engine surface the other subcommands emit — [`json::nodes`] (as
//! `aozora inspect nodes`), [`aozora::Snapshot::to_html`] (as `aozora render`),
//! [`aozora::pandoc::to_pandoc`] (as `aozora pandoc`), and
//! [`aozora::diagnostics_text`] (the portable plain-text report) — so the REPL
//! can never disagree with the rest of the CLI. It defines no machine surface
//! of its own: the output is human chrome around unmodified engine bytes.
//!
//! Meta-commands, all prefixed with `:`, tune the session without leaving it:
//!
//! - `:mode {nodes,html,pandoc,all}` — which view(s) to show (default `all`).
//! - `:lang {en,ja,zh}` — the message language of the chrome (resolved through
//!   the same [`crate::i18n::resolve`] the `--lang` flag uses).
//! - `:encoding {auto,utf8,sjis}` — the decoder `:load` applies (typed lines
//!   are already UTF-8, so this only affects file loads).
//! - `:load FILE` — parse a file's contents through the current mode.
//! - `:help` — list the commands.
//! - `:quit` — leave (Ctrl-D does the same).
//!
//! The heart is the pure [`eval`] function (`source + mode + lang → output
//! string`) and the pure [`Repl::handle`] line dispatcher: both are exercised
//! without a terminal by the unit tests. Line editing and history come from
//! `rustyline` on a TTY; a piped stdin falls through to a plain line reader, so
//! `printf '…\n:quit\n' | aozora repl` is scriptable.
//!
//! The chrome (banner, labels, acknowledgements, help, errors) is localized
//! through the `i18n` catalog; the view *bytes* — node JSON, HTML, Pandoc JSON, and
//! the English diagnostic report — are the machine axis and stay identical in
//! every locale (ADR-0033).

use std::io::{self, IsTerminal, Write};
use std::path::Path;
use std::process::ExitCode;

use crate::fmt::{decode, read_file};
use crate::i18n::{self as i18n, FluentArgs, LanguageIdentifier};
use anyhow::{Context, Result};
use aozora::pandoc::to_pandoc;
use aozora::{Document, json};
use clap::{Parser, ValueEnum};
use rustyline::DefaultEditor;
use rustyline::error::ReadlineError;
use tracing::debug;

use crate::Encoding;

/// The interactive prompt shown before each typed line on a terminal.
const PROMPT: &str = "aozora> ";

/// `aozora repl` — start the interactive notation loop.
#[derive(Debug, Parser)]
#[command(after_long_help = "Examples:
  aozora repl                    # start the loop (all views)
  aozora repl --mode html        # start showing only rendered HTML
  aozora repl -E sjis            # decode :load files as Shift_JIS

Inside the loop, type notation to see it parsed, or a :command:
  :mode html   :lang ja   :encoding sjis   :load FILE   :help   :quit")]
pub(crate) struct ReplArgs {
    /// Which view(s) to show for each line: `nodes`, `html`, `pandoc`, or
    /// `all` (the default). Changeable in-session with `:mode`.
    #[arg(long, value_enum, default_value_t = Mode::default())]
    mode: Mode,

    /// Source encoding for `:load`: `auto` (default), `utf8`, or `sjis`. Typed
    /// lines are already UTF-8, so this only governs file loads. Changeable
    /// in-session with `:encoding`.
    #[arg(long, short = 'E', value_enum, default_value_t = Encoding::default())]
    encoding: Encoding,
}

/// Which view(s) each evaluated line prints. Diagnostics are always shown
/// after the selected views — they are the point of the loop — so this only
/// selects the *representation* of the parse.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, ValueEnum)]
enum Mode {
    /// The parsed source nodes as JSON (as `aozora inspect nodes`).
    Nodes,
    /// The rendered HTML (as `aozora render`).
    Html,
    /// The Pandoc AST as JSON (as `aozora pandoc`).
    Pandoc,
    /// Every view at once: nodes, HTML, and Pandoc.
    #[default]
    All,
}

impl Mode {
    fn shows_nodes(self) -> bool {
        matches!(self, Self::Nodes | Self::All)
    }

    fn shows_html(self) -> bool {
        matches!(self, Self::Html | Self::All)
    }

    fn shows_pandoc(self) -> bool {
        matches!(self, Self::Pandoc | Self::All)
    }
}

/// The mutable session state a meta-command tunes: the active view mode, the
/// chrome language, and the `:load` decoder.
#[derive(Debug)]
struct Repl {
    mode: Mode,
    lang: LanguageIdentifier,
    encoding: Encoding,
}

/// What handling one input line produces — the pure result [`Repl::handle`]
/// returns, kept free of I/O so the whole dispatch is unit-testable.
#[derive(Debug, PartialEq, Eq)]
enum Effect {
    /// Emit this text (an evaluated line, an acknowledgement, help, or an
    /// error), then keep looping. Carries no trailing newline; the caller adds
    /// exactly one.
    Print(String),
    /// A blank line: emit nothing, keep looping.
    Silent,
    /// Leave the loop (`:quit` or Ctrl-D).
    Quit,
}

impl Repl {
    fn new(mode: Mode, lang: LanguageIdentifier, encoding: Encoding) -> Self {
        Self {
            mode,
            lang,
            encoding,
        }
    }

    /// Dispatch one input `line`: a blank line is [`Effect::Silent`], a
    /// `:`-prefixed line is a meta-command, and anything else is notation
    /// evaluated through the current mode. Pure apart from `:load`'s file read.
    fn handle(&mut self, line: &str) -> Effect {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            return Effect::Silent;
        }
        if let Some(rest) = trimmed.strip_prefix(':') {
            return self.meta(rest);
        }
        // Evaluate the raw line (not the trimmed copy) so leading notation
        // whitespace, which can be significant, survives into the parse.
        Effect::Print(eval(line, self.mode, &self.lang))
    }

    /// Handle a meta-command: `rest` is the text after the leading `:`, e.g.
    /// `mode html` or `load book.txt`. Splits the verb from its argument and
    /// routes to the matching setter, returning the acknowledgement / error to
    /// print (or [`Effect::Quit`]).
    fn meta(&mut self, rest: &str) -> Effect {
        let (cmd, arg) = match rest.split_once(char::is_whitespace) {
            Some((cmd, arg)) => (cmd, arg.trim()),
            None => (rest, ""),
        };
        match cmd {
            "quit" | "q" | "exit" => Effect::Quit,
            "help" | "h" | "?" => Effect::Print(help(&self.lang)),
            "mode" | "m" => self.set_mode(arg),
            "lang" | "l" => self.set_lang(arg),
            "encoding" | "enc" | "e" => self.set_encoding(arg),
            "load" => self.load(arg),
            other => Effect::Print(self.unknown(other)),
        }
    }

    fn set_mode(&mut self, arg: &str) -> Effect {
        match Mode::from_str(arg, true) {
            Ok(mode) => {
                self.mode = mode;
                Effect::Print(self.line("repl-mode-set", "mode", &value_tag(&mode)))
            }
            Err(_) => Effect::Print(self.usage("mode", &value_choices::<Mode>())),
        }
    }

    fn set_encoding(&mut self, arg: &str) -> Effect {
        match Encoding::from_str(arg, true) {
            Ok(encoding) => {
                self.encoding = encoding;
                Effect::Print(self.line("repl-encoding-set", "encoding", &value_tag(&encoding)))
            }
            Err(_) => Effect::Print(self.usage("encoding", &value_choices::<Encoding>())),
        }
    }

    fn set_lang(&mut self, arg: &str) -> Effect {
        if arg.is_empty() {
            return Effect::Print(self.usage("lang", "en, ja, zh"));
        }
        // Resolve through the same negotiation the `--lang` flag uses: an
        // unknown tag settles on English (with no error), exactly as the flag.
        self.lang = i18n::resolve(Some(arg), None, None, None);
        let tag = self.lang.to_string();
        Effect::Print(self.line("repl-lang-set", "lang", &tag))
    }

    /// `:load FILE` — read the file with the current decoder and evaluate its
    /// whole contents through the current mode. A read/decode failure is
    /// reported inline (the loop keeps running); it is never fatal.
    fn load(&self, arg: &str) -> Effect {
        if arg.is_empty() {
            return Effect::Print(self.usage("load", "FILE"));
        }
        let path = Path::new(arg);
        match read_source(path, self.encoding) {
            Ok(source) => {
                // The `loaded <path>` header, then the file's evaluation.
                let mut out = self.line("repl-loaded", "path", &path.display().to_string());
                out.push('\n');
                out.push_str(&eval(&source, self.mode, &self.lang));
                Effect::Print(out)
            }
            Err(err) => Effect::Print(self.load_error(path, &err)),
        }
    }

    /// A localized single-`{$field}` line — a `:mode` / `:lang` / `:encoding`
    /// acknowledgement (`mode → all`) or the `:load` header (`loaded FILE`).
    fn line(&self, key: &str, field: &str, value: &str) -> String {
        let mut args = FluentArgs::new();
        args.set(field, value.to_owned());
        i18n::tf(&self.lang, key, &args)
    }

    /// The `usage: :cmd EXPECTED` line for a missing or invalid argument.
    fn usage(&self, cmd: &str, expected: &str) -> String {
        let mut args = FluentArgs::new();
        args.set("cmd", cmd.to_owned());
        args.set("expected", expected.to_owned());
        i18n::tf(&self.lang, "repl-usage", &args)
    }

    fn unknown(&self, cmd: &str) -> String {
        let mut args = FluentArgs::new();
        args.set("cmd", cmd.to_owned());
        i18n::tf(&self.lang, "repl-unknown-meta", &args)
    }

    fn load_error(&self, path: &Path, err: &anyhow::Error) -> String {
        let mut args = FluentArgs::new();
        args.set("path", path.display().to_string());
        // `{err:#}` flattens the anyhow context chain into one line; it is the
        // English engine message (machine axis), woven into the localized frame.
        args.set("error", format!("{err:#}"));
        i18n::tf(&self.lang, "repl-load-error", &args)
    }
}

/// Evaluate one `source` fragment through `mode`: render the selected views
/// plus the always-shown diagnostics into a single labelled block (no trailing
/// newline). Pure — the same parse tree every other subcommand would produce,
/// so the bytes match `inspect nodes` / `render` / `pandoc` exactly.
fn eval(source: &str, mode: Mode, lang: &LanguageIdentifier) -> String {
    let doc = Document::new(source);
    let tree = doc.snapshot();

    let mut sections: Vec<String> = Vec::new();
    if mode.shows_nodes() {
        sections.push(section(
            &i18n::t(lang, "repl-label-nodes"),
            &json::nodes(&tree),
        ));
    }
    if mode.shows_html() {
        sections.push(section(&i18n::t(lang, "repl-label-html"), &tree.to_html()));
    }
    if mode.shows_pandoc() {
        let snapshot = doc.snapshot();
        // Serializing the Pandoc AST cannot fail in practice; surface the error
        // text rather than panicking if that assumption ever breaks.
        let pandoc =
            serde_json::to_string(&to_pandoc(&snapshot)).unwrap_or_else(|err| err.to_string());
        sections.push(section(&i18n::t(lang, "repl-label-pandoc"), &pandoc));
    }

    // Diagnostics are always shown — the whole point of the loop. Reuse the
    // engine's portable plain-text report (English machine axis); an empty
    // report becomes the localized "no diagnostics" line.
    let report = aozora::diagnostics_text(source, tree.diagnostics());
    let diag = if report.trim().is_empty() {
        i18n::t(lang, "repl-diag-none")
    } else {
        report
    };
    sections.push(section(&i18n::t(lang, "repl-label-diag"), &diag));

    sections.join("\n\n")
}

/// A `label\n<body>` section, the body stripped of trailing newlines so the
/// caller's `\n\n` join spaces sections uniformly.
fn section(label: &str, body: &str) -> String {
    format!("{label}\n{}", body.trim_end_matches('\n'))
}

/// The canonical value-enum tag for `value`, e.g. `Mode::All` → `all` — the
/// spelling shown in an acknowledgement and accepted back by `:mode` / `:encoding`.
fn value_tag<T: ValueEnum>(value: &T) -> String {
    value
        .to_possible_value()
        .expect("every REPL value-enum variant has a stable tag")
        .get_name()
        .to_owned()
}

/// The comma-separated list of every accepted tag for value-enum `T`, e.g.
/// `nodes, html, pandoc, all` — woven into a `usage:` line.
fn value_choices<T: ValueEnum>() -> String {
    T::value_variants()
        .iter()
        .filter_map(T::to_possible_value)
        .map(|value| value.get_name().to_owned())
        .collect::<Vec<_>>()
        .join(", ")
}

/// The localized `:help` body listing the meta-commands.
fn help(lang: &LanguageIdentifier) -> String {
    i18n::t(lang, "repl-help")
}

/// Read and decode a `:load` target through the shared formatter reader /
/// decoder — the exact byte path `read_source` in `main` uses, so a loaded
/// file parses identically to `aozora render FILE`.
fn read_source(path: &Path, encoding: Encoding) -> Result<String> {
    let raw = read_file(path).with_context(|| format!("failed to read {}", path.display()))?;
    decode(&raw, encoding)
}

/// Run the loop: print the banner, then read lines — with `rustyline` editing /
/// history on a terminal, or a plain reader when stdin is piped — until `:quit`
/// or EOF. Always exits 0; per-line errors are reported inline and never fatal.
pub(crate) fn run(args: &ReplArgs, lang: &LanguageIdentifier) -> Result<ExitCode> {
    let mut repl = Repl::new(args.mode, lang.clone(), args.encoding);

    let mut stdout = io::stdout().lock();
    writeln!(stdout, "{}", i18n::t(lang, "repl-banner")).context("failed to write REPL banner")?;
    drop(stdout);

    if io::stdin().is_terminal() {
        interactive(&mut repl)
    } else {
        scripted(&mut repl)
    }
}

/// Emit one line's [`Effect`] to `out`, returning whether the loop continues.
/// The single seam both the interactive and scripted readers drive.
fn step(repl: &mut Repl, line: &str, out: &mut impl Write) -> io::Result<bool> {
    match repl.handle(line) {
        Effect::Print(text) => {
            writeln!(out, "{text}")?;
            Ok(true)
        }
        Effect::Silent => Ok(true),
        Effect::Quit => Ok(false),
    }
}

/// The terminal path: `rustyline` supplies line editing and in-session history.
/// Ctrl-C abandons the current line and re-prompts; Ctrl-D (EOF) leaves.
// mutants::skip — this path only runs on a real terminal (`DefaultEditor` and
// `readline` require a TTY), so the sweep host cannot exercise it; its sole
// decision — stop on `step`'s `false`, keep looping otherwise — is the same
// continue/stop seam the piped-stdin `scripted` twin drives end-to-end and the
// `step_*` unit tests pin. Reinforcing it would need a pseudo-terminal harness.
#[cfg_attr(test, mutants::skip)]
fn interactive(repl: &mut Repl) -> Result<ExitCode> {
    let mut editor = DefaultEditor::new().context("failed to initialise the line editor")?;
    loop {
        match editor.readline(PROMPT) {
            Ok(line) => {
                if let Err(err) = editor.add_history_entry(line.as_str()) {
                    debug!(%err, "could not record REPL history entry");
                }
                let mut stdout = io::stdout().lock();
                let keep_going = step(repl, &line, &mut stdout)?;
                if !keep_going {
                    return Ok(ExitCode::SUCCESS);
                }
            }
            // Ctrl-C: discard the current line and prompt again.
            Err(ReadlineError::Interrupted) => {}
            // Ctrl-D on an empty line: leave the loop.
            Err(ReadlineError::Eof) => return Ok(ExitCode::SUCCESS),
            Err(err) => return Err(err).context("line editor error"),
        }
    }
}

/// The piped-stdin path: read the (finite) input plainly — no editing or
/// history, since there is no terminal — and evaluate it line by line, so
/// `printf '…\n:quit\n' | aozora repl` works in scripts and tests. `:quit`
/// stops early; otherwise it runs to EOF.
fn scripted(repl: &mut Repl) -> Result<ExitCode> {
    let input = io::read_to_string(io::stdin()).context("failed to read stdin")?;
    let mut stdout = io::stdout().lock();
    for line in input.lines() {
        if !step(repl, line, &mut stdout)? {
            break;
        }
    }
    Ok(ExitCode::SUCCESS)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lang(tag: &str) -> LanguageIdentifier {
        tag.parse().expect("test locale tag parses")
    }

    fn repl() -> Repl {
        Repl::new(Mode::All, lang("en"), Encoding::Auto)
    }

    /// The `Effect::Print` text, or a panic naming the wrong variant.
    fn printed(effect: Effect) -> String {
        match effect {
            Effect::Print(text) => text,
            other => panic!("expected Effect::Print, got {other:?}"),
        }
    }

    // --- Mode::shows_*: the view-selection predicates ---

    #[test]
    fn mode_predicates_select_the_right_views() {
        assert!(
            Mode::Nodes.shows_nodes() && !Mode::Nodes.shows_html() && !Mode::Nodes.shows_pandoc()
        );
        assert!(!Mode::Html.shows_nodes() && Mode::Html.shows_html() && !Mode::Html.shows_pandoc());
        assert!(
            !Mode::Pandoc.shows_nodes()
                && !Mode::Pandoc.shows_html()
                && Mode::Pandoc.shows_pandoc()
        );
        // `All` shows every view.
        assert!(Mode::All.shows_nodes() && Mode::All.shows_html() && Mode::All.shows_pandoc());
    }

    // --- eval: each view reuses the exact engine bytes ---

    #[test]
    fn eval_html_matches_the_render_engine() {
        let source = "｜青空《あおぞら》";
        let out = eval(source, Mode::Html, &lang("en"));
        // The HTML view is byte-for-byte the `aozora render` output.
        let expected = Document::new(source).snapshot().to_html();
        assert!(
            !expected.is_empty(),
            "fixture must render to non-empty HTML"
        );
        assert!(
            out.contains(&expected),
            "html view carries render bytes: {out}"
        );
        // The ruby base and reading both survive into the shown HTML.
        assert!(out.contains("青空") && out.contains("あおぞら"), "{out}");
        // Only the HTML view (plus diagnostics) is shown — no nodes / pandoc.
        assert!(out.contains("html:"), "{out}");
        assert!(!out.contains("nodes:") && !out.contains("pandoc:"), "{out}");
    }

    #[test]
    fn eval_nodes_matches_the_inspect_engine() {
        let source = "青空《あおぞら》";
        let out = eval(source, Mode::Nodes, &lang("en"));
        let expected = json::nodes(&Document::new(source).snapshot());
        assert!(
            out.contains(expected.trim_end()),
            "nodes view is inspect bytes: {out}"
        );
        assert!(out.contains("nodes:") && !out.contains("html:"), "{out}");
    }

    #[test]
    fn eval_all_shows_every_view_and_diagnostics() {
        let out = eval("青空《あおぞら》", Mode::All, &lang("en"));
        for label in ["nodes:", "html:", "pandoc:", "diagnostics:"] {
            assert!(
                out.contains(label),
                "`{label}` missing from all-view: {out}"
            );
        }
    }

    #[test]
    fn eval_clean_line_reports_no_diagnostics() {
        let out = eval("青空《あおぞら》", Mode::Html, &lang("en"));
        assert!(out.contains("(no diagnostics)"), "clean parse: {out}");
    }

    #[test]
    fn eval_surfaces_a_diagnostic_from_the_engine() {
        // A private-use sentinel reliably fires a diagnostic (the same fixture
        // the engine's own diagnostics_text test uses).
        let out = eval("bad \u{E001} char", Mode::Html, &lang("en"));
        assert!(
            !out.contains("(no diagnostics)"),
            "diagnostic expected: {out}"
        );
        // The engine's report carries the namespaced code; the REPL shows it verbatim.
        assert!(out.contains("aozora::"), "diagnostic code shown: {out}");
    }

    #[test]
    fn eval_diagnostics_stay_english_under_a_localized_chrome() {
        // The chrome label localizes; the diagnostic report bytes (machine axis)
        // do not — they are identical to the English run.
        let ja = eval("bad \u{E001} char", Mode::Html, &lang("ja"));
        let en = eval("bad \u{E001} char", Mode::Html, &lang("en"));
        let code = "aozora::";
        let ja_code = &ja[ja.find(code).expect("ja has the code")..];
        let en_code = &en[en.find(code).expect("en has the code")..];
        assert_eq!(ja_code, en_code, "diagnostic report is language-invariant");
    }

    // --- handle: blank / source / meta routing ---

    #[test]
    fn handle_blank_line_is_silent() {
        assert_eq!(repl().handle("   "), Effect::Silent);
        assert_eq!(repl().handle(""), Effect::Silent);
    }

    #[test]
    fn handle_notation_line_evaluates_it() {
        let printed = printed(repl().handle("青空《あおぞら》"));
        assert!(printed.contains("青空"), "notation is evaluated: {printed}");
    }

    // --- :quit / :help ---

    #[test]
    fn meta_quit_and_aliases_leave_the_loop() {
        assert_eq!(repl().handle(":quit"), Effect::Quit);
        assert_eq!(repl().handle(":q"), Effect::Quit);
        assert_eq!(repl().handle(":exit"), Effect::Quit);
    }

    #[test]
    fn meta_help_lists_the_commands() {
        let help = printed(repl().handle(":help"));
        for cmd in [":mode", ":lang", ":encoding", ":load", ":quit"] {
            assert!(help.contains(cmd), "help mentions `{cmd}`: {help}");
        }
    }

    // --- :mode ---

    #[test]
    fn meta_mode_switches_the_view_and_acknowledges() {
        let mut r = repl();
        let ack = printed(r.handle(":mode html"));
        assert_eq!(r.mode, Mode::Html, "state updated");
        assert!(ack.contains("html"), "ack names the new mode: {ack}");
        // The switch takes effect on the next evaluated line.
        let out = printed(r.handle("青空《あおぞら》"));
        assert!(
            out.contains("html:") && !out.contains("nodes:"),
            "mode applied: {out}"
        );
    }

    #[test]
    fn meta_mode_rejects_an_unknown_value_with_usage() {
        let mut r = repl();
        let msg = printed(r.handle(":mode bogus"));
        assert_eq!(r.mode, Mode::All, "state unchanged on a bad value");
        assert!(
            msg.contains("usage") && msg.contains("nodes"),
            "usage lists choices: {msg}"
        );
    }

    // --- :encoding ---

    #[test]
    fn meta_encoding_switches_the_decoder() {
        let mut r = repl();
        let ack = printed(r.handle(":encoding sjis"));
        assert_eq!(r.encoding, Encoding::Sjis, "decoder updated");
        assert!(ack.contains("sjis"), "ack names the decoder: {ack}");
    }

    #[test]
    fn meta_encoding_rejects_an_unknown_value() {
        let mut r = repl();
        let msg = printed(r.handle(":encoding klingon"));
        assert_eq!(r.encoding, Encoding::Auto, "unchanged on a bad value");
        assert!(msg.contains("usage"), "usage shown: {msg}");
    }

    // --- :lang ---

    #[test]
    fn meta_lang_switches_the_chrome_language() {
        let mut r = repl();
        let ack = printed(r.handle(":lang ja"));
        assert_eq!(r.lang, lang("ja"), "language updated");
        assert!(ack.contains("ja"), "ack names the language: {ack}");
        // Chrome now localizes: the "no diagnostics" line is Japanese, not English.
        let out = printed(r.handle("青空《あおぞら》"));
        assert!(!out.contains("(no diagnostics)"), "chrome localized: {out}");
    }

    #[test]
    fn meta_lang_unknown_tag_settles_on_english() {
        // Mirrors `--lang`: an unknown tag negotiates to English, not an error.
        let mut r = repl();
        r.handle(":lang xx");
        assert_eq!(r.lang, lang("en"), "unknown tag → en");
    }

    #[test]
    fn meta_lang_without_an_argument_shows_usage() {
        let msg = printed(repl().handle(":lang"));
        assert!(msg.contains("usage"), "missing arg → usage: {msg}");
    }

    // --- :load ---

    #[test]
    fn meta_load_reads_a_file_and_evaluates_it() {
        use std::io::Write as _;
        let mut file = tempfile::Builder::new()
            .suffix(".aozora")
            .tempfile()
            .expect("temp file");
        write!(file, "｜青空《あおぞら》").expect("seed file");
        file.flush().expect("flush");

        let mut r = repl();
        let out = printed(r.handle(&format!(":load {}", file.path().display())));
        // The header names the loaded path, and the file's content is evaluated.
        assert!(
            out.contains(&file.path().display().to_string()),
            "load header: {out}"
        );
        assert!(out.contains("青空"), "file content evaluated: {out}");
    }

    #[test]
    fn meta_load_reports_a_missing_file_inline() {
        let mut r = repl();
        let out = printed(r.handle(":load /no/such/aozora-repl-4b1c9d.txt"));
        // A read failure is a printed, non-fatal error — the loop keeps going.
        assert!(
            out.contains("/no/such/aozora-repl-4b1c9d.txt"),
            "names the path: {out}"
        );
    }

    #[test]
    fn meta_load_without_a_path_shows_usage() {
        let msg = printed(repl().handle(":load"));
        assert!(msg.contains("usage") && msg.contains("FILE"), "{msg}");
    }

    // --- unknown meta ---

    #[test]
    fn meta_unknown_command_is_reported() {
        let msg = printed(repl().handle(":frobnicate"));
        assert!(msg.contains("frobnicate"), "names the bad command: {msg}");
    }

    // --- value helpers ---

    #[test]
    fn value_tag_round_trips_each_mode() {
        assert_eq!(value_tag(&Mode::Nodes), "nodes");
        assert_eq!(value_tag(&Mode::Html), "html");
        assert_eq!(value_tag(&Mode::Pandoc), "pandoc");
        assert_eq!(value_tag(&Mode::All), "all");
    }

    #[test]
    fn value_choices_lists_every_mode() {
        assert_eq!(value_choices::<Mode>(), "nodes, html, pandoc, all");
    }

    // --- step: the shared read/emit seam ---

    #[test]
    fn step_prints_and_continues_for_a_source_line() {
        let mut r = repl();
        let mut out = Vec::new();
        let keep = step(&mut r, "青空《あおぞら》", &mut out).expect("step");
        assert!(keep, "a source line keeps the loop running");
        let text = String::from_utf8(out).expect("utf8");
        assert!(
            text.contains("青空") && text.ends_with('\n'),
            "printed with newline: {text}"
        );
    }

    #[test]
    fn step_quit_stops_the_loop() {
        let mut r = repl();
        let mut out = Vec::new();
        let keep = step(&mut r, ":quit", &mut out).expect("step");
        assert!(!keep, ":quit stops the loop");
        assert!(out.is_empty(), ":quit emits nothing");
    }

    #[test]
    fn step_blank_line_emits_nothing_but_continues() {
        let mut r = repl();
        let mut out = Vec::new();
        let keep = step(&mut r, "   ", &mut out).expect("step");
        assert!(keep && out.is_empty(), "blank line: silent, continues");
    }
}
