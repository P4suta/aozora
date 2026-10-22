#![expect(
    clippy::expect_used,
    reason = "TUI initialization validates parser bounds and terminal state"
)]

//! Runs the terminal editor through [`run`].
//! ADR-0033 keeps preview and diagnostic output independent of the interface language.

use std::io::{self, IsTerminal};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::{Duration, Instant};

use crate::atomic_write;
use crate::fmt::{decode, read_file};
use crate::i18n::{self as i18n, FluentArgs, LanguageIdentifier};
use anyhow::{Context, Result};
use aozora::json;
use aozora::pandoc::to_pandoc;
use clap::{Parser, ValueEnum};
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use crossterm::{execute, terminal};
use ratatui::Frame;
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Layout};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Paragraph, Wrap};
use tui_textarea::TextArea;

use crate::Encoding;

const DEBOUNCE: Duration = Duration::from_millis(150);

const POLL: Duration = Duration::from_millis(50);

const MIN_BODY_ROWS: u16 = 3;
const EDITOR_HEIGHT_PERCENT: u16 = 60;
const DIAGNOSTICS_HEIGHT_PERCENT: u16 = 40;
const TOP_PANE_WIDTH_PERCENT: u16 = 50;

/// Start the full-screen live editor.
#[derive(Debug, Parser)]
#[command(after_long_help = "Examples:
  aozora tui                     # start with an empty buffer
  aozora tui hon.aozora          # open a file to edit
  aozora tui --preview nodes     # start the preview pane in nodes view
  aozora tui -E sjis book.txt    # decode a Shift_JIS file on open

Keys (while running):
  Ctrl-S  save the buffer to the file      Ctrl-L  cycle message language
  Ctrl-P  cycle the preview view           Ctrl-Q  quit
Every unmodified keystroke edits the source pane; the preview and diagnostics
panes refresh automatically, debounced.")]
pub(crate) struct TuiArgs {
    /// Open this file in the editor.
    /// Omit to start with an empty buffer, where `Ctrl-S` reports that no file is available.
    #[arg(value_name = "FILE")]
    file: Option<PathBuf>,

    /// Choose the initial preview: `html`, `nodes`, or `pandoc`.
    /// Defaults to HTML, matching `aozora render`.
    /// The other views match `aozora inspect nodes` and `aozora pandoc`.
    /// Cycle views with `Ctrl-P`.
    #[arg(long, value_enum, default_value_t = Preview::default())]
    preview: Preview,

    /// Choose the source encoding for `[FILE]`: `auto`, `utf8`, or `sjis`.
    /// Defaults to `auto`.
    /// The Unicode edit buffer always saves as UTF-8 with `Ctrl-S`.
    #[arg(long, short = 'E', value_enum, default_value_t = Encoding::default())]
    encoding: Encoding,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, ValueEnum)]
enum Preview {
    /// Render HTML as `aozora render` does.
    #[default]
    Html,
    /// Show source nodes as JSON, matching `aozora inspect nodes`.
    Nodes,
    /// Show the Pandoc abstract syntax tree as formatted JSON, matching `aozora pandoc`.
    Pandoc,
}

impl Preview {
    const fn next(self) -> Self {
        match self {
            Self::Html => Self::Nodes,
            Self::Nodes => Self::Pandoc,
            Self::Pandoc => Self::Html,
        }
    }

    const fn tag(self) -> &'static str {
        match self {
            Self::Html => "html",
            Self::Nodes => "nodes",
            Self::Pandoc => "pandoc",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Derived {
    preview: String,
    diagnostics: String,
    diag_count: usize,
}

fn derive(source: &str, preview: Preview, lang: &LanguageIdentifier) -> Derived {
    let doc = aozora::parse(source).expect("source fits parser span limit");
    let tree = doc.snapshot();

    let preview_text = match preview {
        Preview::Html => tree.to_html(),
        Preview::Nodes => json::nodes(&tree),
        Preview::Pandoc => {
            let snapshot = doc.snapshot();
            serde_json::to_string_pretty(&to_pandoc(&snapshot))
                .expect("Pandoc AST serialization is infallible")
        }
    };

    let diags = tree.diagnostics();
    let report = aozora::diagnostics_text(source, diags);
    let diagnostics = if report.trim().is_empty() {
        i18n::t(lang, "tui-diag-none")
    } else {
        report
    };

    Derived {
        preview: preview_text,
        diagnostics,
        diag_count: diags.len(),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Status {
    Legend,
    Info(String),
    Error(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Command {
    Save,
    ToggleLang,
    TogglePreview,
    Quit,
    Edit,
}

fn command_for(key: KeyEvent) -> Command {
    if key.modifiers.contains(KeyModifiers::CONTROL) {
        match key.code {
            KeyCode::Char('s') => return Command::Save,
            KeyCode::Char('l') => return Command::ToggleLang,
            KeyCode::Char('p') => return Command::TogglePreview,
            KeyCode::Char('q') => return Command::Quit,
            _ => {}
        }
    }
    Command::Edit
}

struct App {
    editor: TextArea<'static>,
    preview: Preview,
    lang: LanguageIdentifier,
    path: Option<PathBuf>,
    modified: bool,
    derived: Derived,
    status: Status,
    /// Time of the last edit awaiting a preview refresh.
    pending: Option<Instant>,
}

impl App {
    fn from_source(
        source: &str,
        path: Option<PathBuf>,
        preview: Preview,
        lang: LanguageIdentifier,
    ) -> Self {
        let lines: Vec<String> = source.split('\n').map(str::to_owned).collect();
        let derived = derive(source, preview, &lang);
        Self {
            editor: TextArea::new(lines),
            preview,
            lang,
            path,
            modified: false,
            derived,
            status: Status::Legend,
            pending: None,
        }
    }

    fn new(args: &TuiArgs, lang: &LanguageIdentifier) -> Result<Self> {
        let (source, path) = match &args.file {
            Some(path) => {
                let raw = read_file(path)
                    .with_context(|| format!("failed to read {}", path.display()))?;
                let text = decode(&raw, args.encoding)?;
                (text, Some(path.clone()))
            }
            None => (String::new(), None),
        };
        Ok(Self::from_source(&source, path, args.preview, lang.clone()))
    }

    fn source(&self) -> String {
        self.editor.lines().join("\n")
    }

    fn recompute(&mut self) {
        self.derived = derive(&self.source(), self.preview, &self.lang);
        self.pending = None;
    }

    fn mark_edited(&mut self) {
        self.modified = true;
        self.pending = Some(Instant::now());
    }

    fn recompute_due(&self, now: Instant, window: Duration) -> bool {
        self.pending
            .is_some_and(|since| now.saturating_duration_since(since) >= window)
    }

    fn toggle_preview(&mut self) {
        self.preview = self.preview.next();
        self.status = Status::Legend;
        self.recompute();
    }

    fn toggle_lang(&mut self) {
        self.lang = next_lang(&self.lang);
        self.status = Status::Legend;
        self.recompute();
    }

    fn save(&mut self) {
        let Some(path) = self.path.clone() else {
            self.status = Status::Error(i18n::t(&self.lang, "tui-no-file"));
            return;
        };
        let source = self.source();
        match write_source(&path, &source) {
            Ok(()) => {
                self.modified = false;
                let mut args = FluentArgs::new();
                args.set("path", path.display().to_string());
                self.status = Status::Info(i18n::tf(&self.lang, "tui-saved", &args));
            }
            Err(err) => {
                let mut args = FluentArgs::new();
                args.set("path", path.display().to_string());
                args.set("error", err.to_string());
                self.status = Status::Error(i18n::tf(&self.lang, "tui-save-error", &args));
            }
        }
    }
}

fn next_lang(current: &LanguageIdentifier) -> LanguageIdentifier {
    const CYCLE: [&str; 3] = ["en", "ja", "zh"];
    let cur = current.to_string();
    let idx = CYCLE.iter().position(|tag| *tag == cur).unwrap_or(0);
    CYCLE[(idx + 1) % CYCLE.len()]
        .parse()
        .expect("built-in locale tag parses")
}

fn write_source(path: &Path, source: &str) -> io::Result<()> {
    atomic_write::replace(path, source.as_bytes())
}

fn source_title(app: &App) -> String {
    let mut title = i18n::t(&app.lang, "tui-title-source");
    if let Some(path) = &app.path {
        title = format!("{title} · {}", path.display());
    }
    if app.modified {
        title = format!("{title} · {}", i18n::t(&app.lang, "tui-modified"));
    }
    title
}

fn preview_title(app: &App) -> String {
    format!(
        "{} · {}",
        i18n::t(&app.lang, "tui-title-preview"),
        app.preview.tag()
    )
}

fn diagnostics_title(app: &App) -> String {
    let base = i18n::t(&app.lang, "tui-title-diagnostics");
    if app.derived.diag_count == 0 {
        base
    } else {
        format!("{base} · {}", app.derived.diag_count)
    }
}

fn footer_legend(preview: Preview, lang: &LanguageIdentifier) -> String {
    let save = i18n::t(lang, "tui-key-save");
    let lang_word = i18n::t(lang, "tui-key-lang");
    let preview_word = i18n::t(lang, "tui-key-preview");
    let quit = i18n::t(lang, "tui-key-quit");
    let cur_lang = lang.to_string();
    let cur_preview = preview.tag();
    format!("^S {save}   ^L {lang_word}·{cur_lang}   ^P {preview_word}·{cur_preview}   ^Q {quit}")
}

fn footer_line(app: &App) -> Line<'static> {
    let dim = Style::default().add_modifier(Modifier::DIM);
    let mut spans = vec![Span::styled(footer_legend(app.preview, &app.lang), dim)];
    match &app.status {
        Status::Legend => {}
        Status::Info(msg) => {
            spans.push(Span::raw("   "));
            spans.push(Span::styled(msg.clone(), Style::default().fg(Color::Green)));
        }
        Status::Error(msg) => {
            spans.push(Span::raw("   "));
            spans.push(Span::styled(msg.clone(), Style::default().fg(Color::Red)));
        }
    }
    Line::from(spans)
}

fn render(app: &App, frame: &mut Frame<'_>) {
    let [body, footer_area] =
        Layout::vertical([Constraint::Min(MIN_BODY_ROWS), Constraint::Length(1)])
            .areas(frame.area());
    let [top, diag_area] = Layout::vertical([
        Constraint::Percentage(EDITOR_HEIGHT_PERCENT),
        Constraint::Percentage(DIAGNOSTICS_HEIGHT_PERCENT),
    ])
    .areas(body);
    let [src_area, prev_area] = Layout::horizontal([
        Constraint::Percentage(TOP_PANE_WIDTH_PERCENT),
        Constraint::Percentage(TOP_PANE_WIDTH_PERCENT),
    ])
    .areas(top);

    let src_block = Block::bordered().title(source_title(app));
    let src_inner = src_block.inner(src_area);
    frame.render_widget(src_block, src_area);
    frame.render_widget(&app.editor, src_inner);

    let prev_block = Block::bordered().title(preview_title(app));
    let prev_inner = prev_block.inner(prev_area);
    frame.render_widget(prev_block, prev_area);
    frame.render_widget(
        Paragraph::new(app.derived.preview.as_str()).wrap(Wrap { trim: false }),
        prev_inner,
    );

    let diag_block = Block::bordered().title(diagnostics_title(app));
    let diag_inner = diag_block.inner(diag_area);
    frame.render_widget(diag_block, diag_area);
    frame.render_widget(
        Paragraph::new(app.derived.diagnostics.as_str()).wrap(Wrap { trim: false }),
        diag_inner,
    );

    frame.render_widget(Paragraph::new(footer_line(app)), footer_area);
}

fn all_terminals(stream_ttys: [bool; 2]) -> bool {
    stream_ttys.iter().all(|&is_tty| is_tty)
}

/// Rejects redirected input or output before opening the editor.
pub(crate) fn run(args: &TuiArgs, lang: &LanguageIdentifier) -> Result<ExitCode> {
    if !all_terminals([io::stdin().is_terminal(), io::stdout().is_terminal()]) {
        anyhow::bail!("{}", i18n::t(lang, "tui-no-tty"));
    }
    let app = App::new(args, lang)?;
    run_app(app)
}

fn run_app(mut app: App) -> Result<ExitCode> {
    let mut session = TerminalSession::enter()?;
    let outcome = event_loop(&mut session.terminal, &mut app);
    session.finish(outcome)
}

struct TerminalSession<W: io::Write> {
    terminal: Terminal<CrosstermBackend<W>>,
    raw_mode: bool,
    alternate_screen: bool,
    cursor: bool,
}

impl TerminalSession<io::Stdout> {
    fn enter() -> Result<Self> {
        terminal::enable_raw_mode().context("failed to enable raw mode")?;
        let mut stdout = io::stdout();
        if let Err(err) = execute!(stdout, terminal::EnterAlternateScreen) {
            let primary = anyhow::Error::new(err).context("failed to enter the alternate screen");
            let cleanup = terminal::disable_raw_mode().context("failed to disable raw mode");
            return Err(merge_errors(primary, cleanup.err()));
        }
        let terminal = match Terminal::new(CrosstermBackend::new(stdout)) {
            Ok(terminal) => terminal,
            Err(err) => {
                let primary = anyhow::Error::new(err).context("failed to create the terminal");
                let alternate = execute!(io::stdout(), terminal::LeaveAlternateScreen)
                    .context("failed to leave the alternate screen");
                let raw = terminal::disable_raw_mode().context("failed to disable raw mode");
                let cleanup = combine_cleanup(alternate, raw);
                return Err(merge_errors(primary, cleanup.err()));
            }
        };
        Ok(Self {
            terminal,
            raw_mode: true,
            alternate_screen: true,
            cursor: true,
        })
    }
}

impl<W: io::Write> TerminalSession<W> {
    fn finish(mut self, outcome: Result<ExitCode>) -> Result<ExitCode> {
        let cleanup = self.restore();
        match (outcome, cleanup) {
            (Ok(code), Ok(())) => Ok(code),
            (Err(err), Ok(())) | (Ok(_), Err(err)) => Err(err),
            (Err(primary), Err(cleanup)) => Err(merge_errors(primary, Some(cleanup))),
        }
    }

    fn restore(&mut self) -> Result<()> {
        let cursor = if self.cursor {
            let result = self
                .terminal
                .show_cursor()
                .context("failed to restore the terminal cursor");
            if result.is_ok() {
                self.cursor = false;
            }
            result
        } else {
            Ok(())
        };
        let alternate = if self.alternate_screen {
            let result = execute!(self.terminal.backend_mut(), terminal::LeaveAlternateScreen)
                .context("failed to leave the alternate screen");
            if result.is_ok() {
                self.alternate_screen = false;
            }
            result
        } else {
            Ok(())
        };
        let raw = if self.raw_mode {
            let result = terminal::disable_raw_mode().context("failed to disable raw mode");
            if result.is_ok() {
                self.raw_mode = false;
            }
            result
        } else {
            Ok(())
        };
        combine_cleanup(combine_cleanup(cursor, alternate), raw)
    }
}

impl<W: io::Write> Drop for TerminalSession<W> {
    fn drop(&mut self) {
        let _drop = self.restore();
    }
}

fn combine_cleanup(first: Result<()>, second: Result<()>) -> Result<()> {
    match (first, second) {
        (Ok(()), Ok(())) => Ok(()),
        (Err(err), Ok(())) | (Ok(()), Err(err)) => Err(err),
        (Err(primary), Err(cleanup)) => Err(merge_errors(primary, Some(cleanup))),
    }
}

fn merge_errors(primary: anyhow::Error, cleanup: Option<anyhow::Error>) -> anyhow::Error {
    match cleanup {
        Some(cleanup) => anyhow::anyhow!("{primary:#}; additionally, {cleanup:#}"),
        None => primary,
    }
}

fn event_loop(
    term: &mut Terminal<CrosstermBackend<io::Stdout>>,
    app: &mut App,
) -> Result<ExitCode> {
    let mut needs_redraw = true;
    loop {
        if needs_redraw {
            term.draw(|frame| render(app, frame))
                .context("failed to draw the TUI")?;
            needs_redraw = false;
        }
        if event::poll(POLL).context("failed to poll for terminal input")? {
            if let Event::Key(key) = event::read().context("failed to read terminal input")?
                && key.kind == KeyEventKind::Press
            {
                match command_for(key) {
                    Command::Quit => return Ok(ExitCode::SUCCESS),
                    Command::Save => app.save(),
                    Command::ToggleLang => app.toggle_lang(),
                    Command::TogglePreview => app.toggle_preview(),
                    Command::Edit => {
                        if app.editor.input(key) {
                            app.mark_edited();
                        }
                    }
                }
            }
            needs_redraw = true;
        }
        if app.recompute_due(Instant::now(), DEBOUNCE) {
            app.recompute();
            needs_redraw = true;
        }
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::fs;
    use std::rc::Rc;

    use ratatui::backend::TestBackend;
    use ratatui::buffer::{Buffer, Cell};
    use ratatui::layout::Rect;
    use ratatui::{TerminalOptions, Viewport};

    use super::*;

    fn lang(tag: &str) -> LanguageIdentifier {
        tag.parse().expect("test locale tag parses")
    }

    fn app(source: &str, preview: Preview, tag: &str) -> App {
        App::from_source(source, None, preview, lang(tag))
    }

    fn ctrl(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL)
    }

    fn buffer_text(buffer: &Buffer) -> String {
        buffer.content.iter().map(Cell::symbol).collect()
    }

    #[test]
    fn preview_next_cycles_html_nodes_pandoc() {
        assert_eq!(Preview::Html.next(), Preview::Nodes);
        assert_eq!(Preview::Nodes.next(), Preview::Pandoc);
        assert_eq!(Preview::Pandoc.next(), Preview::Html);
    }

    #[test]
    fn preview_tag_names_each_view() {
        assert_eq!(Preview::Html.tag(), "html");
        assert_eq!(Preview::Nodes.tag(), "nodes");
        assert_eq!(Preview::Pandoc.tag(), "pandoc");
    }

    #[test]
    fn next_lang_cycles_en_ja_zh() {
        assert_eq!(next_lang(&lang("en")), lang("ja"));
        assert_eq!(next_lang(&lang("ja")), lang("zh"));
        assert_eq!(next_lang(&lang("zh")), lang("en"));
    }

    #[test]
    fn derive_html_matches_the_render_engine() {
        let source = "｜青空《あおぞら》";
        let d = derive(source, Preview::Html, &lang("en"));
        let expected = aozora::parse(source)
            .expect("source fits parser span limit")
            .snapshot()
            .to_html();
        assert!(!expected.is_empty(), "fixture renders to non-empty HTML");
        assert_eq!(d.preview, expected, "preview is the render bytes");
        assert_eq!(d.diag_count, 0, "clean fixture");
    }

    #[test]
    fn derive_nodes_matches_the_inspect_engine() {
        let source = "青空《あおぞら》";
        let d = derive(source, Preview::Nodes, &lang("en"));
        let expected = json::nodes(
            &aozora::parse(source)
                .expect("source fits parser span limit")
                .snapshot(),
        );
        assert_eq!(d.preview, expected, "preview is the inspect-nodes bytes");
    }

    #[test]
    fn derive_pandoc_projects_the_ast() {
        let d = derive("青空《あおぞら》", Preview::Pandoc, &lang("en"));
        assert!(
            d.preview.contains("pandoc-api-version"),
            "pandoc AST shown: {}",
            d.preview
        );
    }

    #[test]
    fn derive_clean_source_reports_no_diagnostics() {
        let d = derive("青空《あおぞら》", Preview::Html, &lang("en"));
        assert_eq!(d.diag_count, 0);
        assert_eq!(d.diagnostics, "(no diagnostics)", "localized placeholder");
    }

    #[test]
    fn derive_surfaces_a_diagnostic_from_the_engine() {
        let d = derive("bad \u{E001} char", Preview::Html, &lang("en"));
        assert!(d.diag_count > 0, "diagnostic expected");
        assert!(
            d.diagnostics.contains("aozora::"),
            "namespaced code shown verbatim: {}",
            d.diagnostics
        );
    }

    #[test]
    fn derive_diagnostics_stay_english_under_a_localized_chrome() {
        let ja = derive("bad \u{E001} char", Preview::Html, &lang("ja"));
        let en = derive("bad \u{E001} char", Preview::Html, &lang("en"));
        assert_eq!(
            ja.diagnostics, en.diagnostics,
            "report is language-invariant"
        );
    }

    #[test]
    fn command_for_maps_the_ctrl_keys() {
        assert_eq!(command_for(ctrl('s')), Command::Save);
        assert_eq!(command_for(ctrl('l')), Command::ToggleLang);
        assert_eq!(command_for(ctrl('p')), Command::TogglePreview);
        assert_eq!(command_for(ctrl('q')), Command::Quit);
    }

    #[test]
    fn command_for_forwards_everything_else_to_the_editor() {
        assert_eq!(
            command_for(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE)),
            Command::Edit
        );
        assert_eq!(command_for(ctrl('a')), Command::Edit);
        assert_eq!(
            command_for(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
            Command::Edit
        );
    }

    #[test]
    fn all_terminals_needs_every_stream() {
        assert!(all_terminals([true, true]), "both ttys → interactive");
        assert!(
            !all_terminals([true, false]),
            "piped stdout → not interactive"
        );
        assert!(
            !all_terminals([false, true]),
            "piped stdin → not interactive"
        );
        assert!(
            !all_terminals([false, false]),
            "both piped → not interactive"
        );
    }

    #[test]
    fn terminal_finish_preserves_the_application_exit_code() {
        let terminal = Terminal::with_options(
            CrosstermBackend::new(io::stdout()),
            TerminalOptions {
                viewport: Viewport::Fixed(Rect::new(0, 0, 1, 1)),
            },
        )
        .expect("a fixed viewport does not inspect the host terminal");
        let session = TerminalSession {
            terminal,
            raw_mode: false,
            alternate_screen: false,
            cursor: false,
        };

        assert_eq!(
            session
                .finish(Ok(ExitCode::from(7)))
                .expect("disabled cleanup is infallible"),
            ExitCode::from(7)
        );
    }

    #[test]
    fn terminal_drop_restores_an_enabled_cursor() {
        #[derive(Clone, Default)]
        struct SharedWriter(Rc<RefCell<Vec<u8>>>);

        impl io::Write for SharedWriter {
            fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
                self.0.borrow_mut().extend_from_slice(bytes);
                Ok(bytes.len())
            }

            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }

        let writer = SharedWriter::default();
        let terminal = Terminal::with_options(
            CrosstermBackend::new(writer.clone()),
            TerminalOptions {
                viewport: Viewport::Fixed(Rect::new(0, 0, 1, 1)),
            },
        )
        .expect("a fixed viewport does not inspect the host terminal");
        let session = TerminalSession {
            terminal,
            raw_mode: false,
            alternate_screen: false,
            cursor: true,
        };

        drop(session);

        assert!(!writer.0.borrow().is_empty());
    }

    #[test]
    fn cleanup_combiner_preserves_single_and_multiple_failures() {
        let first = combine_cleanup(Err(anyhow::anyhow!("first")), Ok(()))
            .expect_err("the first cleanup failure is retained");
        assert_eq!(first.to_string(), "first");

        let second = combine_cleanup(Ok(()), Err(anyhow::anyhow!("second")))
            .expect_err("the second cleanup failure is retained");
        assert_eq!(second.to_string(), "second");

        let both = combine_cleanup(
            Err(anyhow::anyhow!("first")),
            Err(anyhow::anyhow!("second")),
        )
        .expect_err("both cleanup failures are retained");
        assert!(both.to_string().contains("first"));
        assert!(both.to_string().contains("second"));
    }

    #[test]
    fn from_source_derives_the_initial_panes() {
        let a = app("青空《あおぞら》", Preview::Html, "en");
        assert_eq!(a.source(), "青空《あおぞら》", "buffer holds the source");
        assert!(!a.derived.preview.is_empty(), "panes derived up front");
        assert!(!a.modified, "a freshly opened buffer is clean");
    }

    #[test]
    fn source_round_trips_a_trailing_newline() {
        let a = app("a\n", Preview::Html, "en");
        assert_eq!(a.source(), "a\n");
    }

    #[test]
    fn toggle_preview_advances_the_view_and_recomputes() {
        let mut a = app("青空《あおぞら》", Preview::Html, "en");
        let html = a.derived.preview.clone();
        a.toggle_preview();
        assert_eq!(a.preview, Preview::Nodes, "view advanced");
        assert_ne!(a.derived.preview, html, "panes recomputed for the new view");
        assert_eq!(
            a.derived.preview,
            json::nodes(
                &aozora::parse("青空《あおぞら》")
                    .expect("source fits parser span limit")
                    .snapshot()
            ),
            "now showing the nodes bytes"
        );
    }

    #[test]
    fn toggle_lang_localizes_the_chrome() {
        let mut a = app("青空《あおぞら》", Preview::Html, "en");
        assert_eq!(
            a.derived.diagnostics, "(no diagnostics)",
            "english placeholder"
        );
        a.toggle_lang();
        assert_eq!(a.lang, lang("ja"), "language advanced");
        assert_ne!(
            a.derived.diagnostics, "(no diagnostics)",
            "placeholder localized"
        );
    }

    #[test]
    fn mark_edited_sets_modified_and_opens_the_debounce_window() {
        let mut a = app("x", Preview::Html, "en");
        assert!(a.pending.is_none(), "nothing pending at rest");
        a.mark_edited();
        assert!(a.modified, "buffer marked dirty");
        assert!(a.pending.is_some(), "debounce window opened");
    }

    #[test]
    fn recompute_due_respects_the_debounce_window() {
        let mut a = app("x", Preview::Html, "en");
        let base = Instant::now();
        assert!(!a.recompute_due(base, DEBOUNCE));
        a.pending = Some(base);
        assert!(!a.recompute_due(base, DEBOUNCE), "0 elapsed < window");
        assert!(a.recompute_due(base + DEBOUNCE, DEBOUNCE), "window elapsed");
    }

    #[test]
    fn recompute_clears_the_pending_window() {
        let mut a = app("x", Preview::Html, "en");
        a.mark_edited();
        a.recompute();
        assert!(a.pending.is_none(), "recompute clears the pending flag");
    }

    #[test]
    fn save_writes_the_buffer_and_acknowledges() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("source.aozora");
        fs::write(&path, b"").expect("seed temp file");
        let mut a = App::from_source(
            "青空《あおぞら》",
            Some(path.clone()),
            Preview::Html,
            lang("en"),
        );
        a.modified = true;
        a.save();
        let written = fs::read_to_string(&path).expect("read back");
        assert_eq!(written, "青空《あおぞら》", "buffer written to disk");
        assert!(!a.modified, "save clears the dirty flag");
        match &a.status {
            Status::Info(msg) => assert!(
                msg.contains(&path.display().to_string()),
                "status names the path: {msg}"
            ),
            other => panic!("expected an Info status, got {other:?}"),
        }
    }

    #[test]
    fn save_without_a_file_reports_no_file() {
        let mut a = app("青空《あおぞら》", Preview::Html, "en");
        a.save();
        assert!(
            matches!(a.status, Status::Error(_)),
            "no-file is an error status"
        );
    }

    #[test]
    fn source_title_shows_the_path_and_modified_marker() {
        let mut a = App::from_source(
            "x",
            Some(PathBuf::from("hon.aozora")),
            Preview::Html,
            lang("en"),
        );
        assert_eq!(source_title(&a), "source · hon.aozora", "clean: path only");
        a.modified = true;
        assert!(
            source_title(&a).contains("modified"),
            "dirty buffer marks the title: {}",
            source_title(&a)
        );
    }

    #[test]
    fn preview_title_names_the_active_view() {
        let a = app("x", Preview::Nodes, "en");
        assert_eq!(preview_title(&a), "preview · nodes");
    }

    #[test]
    fn diagnostics_title_appends_the_count_when_nonzero() {
        let clean = app("青空《あおぞら》", Preview::Html, "en");
        assert_eq!(diagnostics_title(&clean), "diagnostics", "clean: no count");
        let dirty = app("bad \u{E001} char", Preview::Html, "en");
        assert!(
            diagnostics_title(&dirty).starts_with("diagnostics · "),
            "count appended: {}",
            diagnostics_title(&dirty)
        );
    }

    #[test]
    fn footer_legend_lists_every_keybind_and_the_current_state() {
        let legend = footer_legend(Preview::Nodes, &lang("en"));
        for key in ["^S", "^L", "^P", "^Q"] {
            assert!(legend.contains(key), "legend lists `{key}`: {legend}");
        }
        assert!(
            legend.contains("·nodes"),
            "shows the current view: {legend}"
        );
        assert!(
            legend.contains("·en"),
            "shows the current language: {legend}"
        );
    }

    #[test]
    fn render_draws_the_three_panes_and_footer() {
        let a = app("青空《あおぞら》", Preview::Html, "en");
        let mut term = Terminal::new(TestBackend::new(80, 24)).expect("test terminal");
        term.draw(|frame| render(&a, frame)).expect("draw");
        let text = buffer_text(term.backend().buffer());
        for label in ["source", "preview", "diagnostics"] {
            assert!(text.contains(label), "`{label}` pane drawn: {text}");
        }
        assert!(
            text.contains("^S") && text.contains("^Q"),
            "footer legend drawn"
        );
        assert!(
            text.contains("no diagnostics"),
            "clean-parse placeholder drawn"
        );
    }

    #[test]
    fn render_shows_a_diagnostic_in_the_bottom_pane() {
        let a = app("bad \u{E001} char", Preview::Html, "en");
        let mut term = Terminal::new(TestBackend::new(80, 24)).expect("test terminal");
        term.draw(|frame| render(&a, frame)).expect("draw");
        let text = buffer_text(term.backend().buffer());
        assert!(
            text.contains("aozora"),
            "diagnostic drawn in the pane: {text}"
        );
    }
}
