#![expect(
    clippy::expect_used,
    reason = "REPL initialization validates parser bounds and history state"
)]

use std::io::{self, IsTerminal, Write};
use std::path::Path;
use std::process::ExitCode;

use crate::fmt::{decode, read_file};
use crate::i18n::{self as i18n, FluentArgs, LanguageIdentifier};
use crate::output;
use anyhow::{Context, Result};
use aozora::json;
use aozora::pandoc::to_pandoc;
use clap::{Parser, ValueEnum};
use rustyline::DefaultEditor;
use rustyline::error::ReadlineError;
use tracing::debug;

use crate::Encoding;

const PROMPT: &str = "aozora> ";

/// Start the interactive notation loop.
#[derive(Debug, Parser)]
#[command(after_long_help = "Examples:
  aozora repl                    # start the loop (all views)
  aozora repl --mode html        # start showing only rendered HTML
  aozora repl -E sjis            # decode :load files as Shift_JIS

Inside the loop, type notation to see it parsed, or a :command:
  :mode html   :lang ja   :encoding sjis   :load FILE   :help   :quit")]
pub(crate) struct ReplArgs {
    /// Choose `nodes`, `html`, `pandoc`, or `all` for each input line.
    /// Defaults to `all`.
    /// Change the view with `:mode`.
    #[arg(long, value_enum, default_value_t = Mode::default())]
    mode: Mode,

    /// Choose the source encoding for `:load`: `auto`, `utf8`, or `sjis`.
    /// Defaults to `auto`.
    /// Typed lines already use UTF-8, so this setting only affects file loads.
    /// Change it with `:encoding`.
    #[arg(long, short = 'E', value_enum, default_value_t = Encoding::default())]
    encoding: Encoding,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, ValueEnum)]
enum Mode {
    /// Show source nodes as JSON, matching `aozora inspect nodes`.
    Nodes,
    /// Render HTML as `aozora render` does.
    Html,
    /// Show the Pandoc abstract syntax tree as JSON, matching `aozora pandoc`.
    Pandoc,
    /// Show nodes, HTML, and Pandoc together.
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

#[derive(Debug)]
struct Repl {
    mode: Mode,
    lang: LanguageIdentifier,
    encoding: Encoding,
}

#[derive(Debug, PartialEq, Eq)]
enum Effect {
    Print(String),
    Silent,
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

    fn handle(&mut self, line: &str) -> Effect {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            return Effect::Silent;
        }
        if let Some(rest) = trimmed.strip_prefix(':') {
            return self.meta(rest);
        }
        Effect::Print(eval(line, self.mode, &self.lang))
    }

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
        self.lang = i18n::resolve(Some(arg), None, None, None);
        let tag = self.lang.to_string();
        Effect::Print(self.line("repl-lang-set", "lang", &tag))
    }

    fn load(&self, arg: &str) -> Effect {
        if arg.is_empty() {
            return Effect::Print(self.usage("load", "FILE"));
        }
        let path = Path::new(arg);
        match read_source(path, self.encoding) {
            Ok(source) => {
                let mut out = self.line("repl-loaded", "path", &path.display().to_string());
                out.push('\n');
                out.push_str(&eval(&source, self.mode, &self.lang));
                Effect::Print(out)
            }
            Err(err) => Effect::Print(self.load_error(path, &err)),
        }
    }

    fn line(&self, key: &str, field: &str, value: &str) -> String {
        let mut args = FluentArgs::new();
        args.set(field, value.to_owned());
        i18n::tf(&self.lang, key, &args)
    }

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
        args.set("error", format!("{err:#}"));
        i18n::tf(&self.lang, "repl-load-error", &args)
    }
}

fn eval(source: &str, mode: Mode, lang: &LanguageIdentifier) -> String {
    let doc = aozora::parse(source).expect("source fits parser span limit");
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
        let pandoc = serde_json::to_string(&to_pandoc(&snapshot))
            .expect("Pandoc AST serialization is infallible");
        sections.push(section(&i18n::t(lang, "repl-label-pandoc"), &pandoc));
    }

    let report = aozora::diagnostics_text(source, tree.diagnostics());
    let diag = if report.trim().is_empty() {
        i18n::t(lang, "repl-diag-none")
    } else {
        report
    };
    sections.push(section(&i18n::t(lang, "repl-label-diag"), &diag));

    sections.join("\n\n")
}

fn section(label: &str, body: &str) -> String {
    format!("{label}\n{}", body.trim_end_matches('\n'))
}

fn value_tag<T: ValueEnum>(value: &T) -> String {
    value
        .to_possible_value()
        .expect("every REPL value-enum variant has a stable tag")
        .get_name()
        .to_owned()
}

fn value_choices<T: ValueEnum>() -> String {
    T::value_variants()
        .iter()
        .filter_map(T::to_possible_value)
        .map(|value| value.get_name().to_owned())
        .collect::<Vec<_>>()
        .join(", ")
}

fn help(lang: &LanguageIdentifier) -> String {
    i18n::t(lang, "repl-help")
}

fn read_source(path: &Path, encoding: Encoding) -> Result<String> {
    let raw = read_file(path).with_context(|| format!("failed to read {}", path.display()))?;
    decode(&raw, encoding)
}

pub(crate) fn run(args: &ReplArgs, lang: &LanguageIdentifier) -> Result<ExitCode> {
    let mut repl = Repl::new(args.mode, lang.clone(), args.encoding);

    let mut stdout = output::stdout();
    writeln!(stdout, "{}", i18n::t(lang, "repl-banner")).context("failed to write REPL banner")?;

    if io::stdin().is_terminal() {
        interactive(&mut repl)
    } else {
        scripted(&mut repl)
    }
}

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

fn interactive(repl: &mut Repl) -> Result<ExitCode> {
    let mut editor = DefaultEditor::new().context("failed to initialise the line editor")?;
    loop {
        match editor.readline(PROMPT) {
            Ok(line) => {
                if let Err(err) = editor.add_history_entry(line.as_str()) {
                    debug!(%err, "could not record REPL history entry");
                }
                let mut stdout = output::stdout();
                let keep_going = step(repl, &line, &mut stdout)?;
                if !keep_going {
                    return Ok(ExitCode::SUCCESS);
                }
            }
            Err(ReadlineError::Interrupted) => {}
            Err(ReadlineError::Eof) => return Ok(ExitCode::SUCCESS),
            Err(err) => return Err(err).context("line editor error"),
        }
    }
}

fn scripted(repl: &mut Repl) -> Result<ExitCode> {
    let input = io::read_to_string(io::stdin()).context("failed to read stdin")?;
    let mut stdout = output::stdout();
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

    fn printed(effect: Effect) -> String {
        match effect {
            Effect::Print(text) => text,
            other => panic!("expected Effect::Print, got {other:?}"),
        }
    }

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
        assert!(Mode::All.shows_nodes() && Mode::All.shows_html() && Mode::All.shows_pandoc());
    }

    #[test]
    fn eval_html_matches_the_render_engine() {
        let source = "｜青空《あおぞら》";
        let out = eval(source, Mode::Html, &lang("en"));
        let expected = aozora::parse(source)
            .expect("source fits parser span limit")
            .snapshot()
            .to_html();
        assert!(
            !expected.is_empty(),
            "fixture must render to non-empty HTML"
        );
        assert!(
            out.contains(&expected),
            "html view carries render bytes: {out}"
        );
        assert!(out.contains("青空") && out.contains("あおぞら"), "{out}");
        assert!(out.contains("html:"), "{out}");
        assert!(!out.contains("nodes:") && !out.contains("pandoc:"), "{out}");
    }

    #[test]
    fn eval_nodes_matches_the_inspect_engine() {
        let source = "青空《あおぞら》";
        let out = eval(source, Mode::Nodes, &lang("en"));
        let expected = json::nodes(
            &aozora::parse(source)
                .expect("source fits parser span limit")
                .snapshot(),
        );
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
        let out = eval("bad \u{E001} char", Mode::Html, &lang("en"));
        assert!(
            !out.contains("(no diagnostics)"),
            "diagnostic expected: {out}"
        );
        assert!(out.contains("aozora::"), "diagnostic code shown: {out}");
    }

    #[test]
    fn eval_diagnostics_stay_english_under_a_localized_chrome() {
        let ja = eval("bad \u{E001} char", Mode::Html, &lang("ja"));
        let en = eval("bad \u{E001} char", Mode::Html, &lang("en"));
        let code = "aozora::";
        let ja_code = &ja[ja.find(code).expect("ja has the code")..];
        let en_code = &en[en.find(code).expect("en has the code")..];
        assert_eq!(ja_code, en_code, "diagnostic report is language-invariant");
    }

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

    #[test]
    fn meta_mode_switches_the_view_and_acknowledges() {
        let mut r = repl();
        let ack = printed(r.handle(":mode html"));
        assert_eq!(r.mode, Mode::Html, "state updated");
        assert!(ack.contains("html"), "ack names the new mode: {ack}");
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

    #[test]
    fn meta_lang_switches_the_chrome_language() {
        let mut r = repl();
        let ack = printed(r.handle(":lang ja"));
        assert_eq!(r.lang, lang("ja"), "language updated");
        assert!(ack.contains("ja"), "ack names the language: {ack}");
        let out = printed(r.handle("青空《あおぞら》"));
        assert!(!out.contains("(no diagnostics)"), "chrome localized: {out}");
    }

    #[test]
    fn meta_lang_unknown_tag_settles_on_english() {
        let mut r = repl();
        r.handle(":lang xx");
        assert_eq!(r.lang, lang("en"), "unknown tag → en");
    }

    #[test]
    fn meta_lang_without_an_argument_shows_usage() {
        let msg = printed(repl().handle(":lang"));
        assert!(msg.contains("usage"), "missing arg → usage: {msg}");
    }

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

    #[test]
    fn meta_unknown_command_is_reported() {
        let msg = printed(repl().handle(":frobnicate"));
        assert!(msg.contains("frobnicate"), "names the bad command: {msg}");
    }

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
