//! `aozora tui` — a full-screen, live editor for the notation.
//!
//! The editor-free counterpart to `--watch`: instead of re-running a command
//! when a file on disk changes, the TUI hosts the buffer itself in a
//! three-pane layout — a source EDIT pane (left), a live PREVIEW pane (right),
//! and a DIAGNOSTICS pane (bottom) — and re-parses / re-renders / re-diagnoses
//! on every keystroke, debounced. It is the terminal-native sibling of the web
//! playground with an editor attached.
//!
//! Like [`crate::repl`], it owns **no** parsing, rendering, or diagnostic
//! logic. The preview is the exact engine surface the document subcommands
//! emit — [`aozora::Tree::to_html`] (as `aozora render`), [`json::nodes`] (as
//! `aozora inspect nodes`), and [`aozora_pandoc::to_pandoc`] (as `aozora
//! pandoc`) — and the diagnostics pane is [`aozora::diagnostics_text`], the
//! portable plain-text report. So the panes can never disagree with the rest
//! of the CLI, and the TUI defines no machine surface of its own.
//!
//! Keys, all held with `Ctrl` so every unmodified keystroke reaches the
//! editor:
//!
//! - `Ctrl-S` — save the buffer to the opened file (UTF-8).
//! - `Ctrl-L` — cycle the chrome language (en → ja → zh).
//! - `Ctrl-P` — cycle the preview view (html → nodes → pandoc).
//! - `Ctrl-Q` — quit.
//!
//! The heart is the pure [`derive()`] function (`source + preview + lang →
//! preview text + diagnostics`), the pure [`command_for`] keybind decoder, and
//! the pure [`all_terminals`] tty guard; the state transitions
//! ([`App::toggle_lang`], [`App::toggle_preview`], [`App::save`], the
//! [`App::recompute_due`] debounce test) and the [`render`] layout are all
//! exercised headlessly — over `ratatui`'s
//! [`TestBackend`](ratatui::backend::TestBackend) and by direct state calls —
//! so the whole surface is unit-tested without a terminal. Only the terminal
//! I/O shells ([`run`]'s alternate-screen setup and the [`run_app`] /
//! [`event_loop`] draw / input loop) need a real tty.
//!
//! The chrome (pane titles, the keybind legend, save / error status) is
//! localized through `aozora-i18n`; the preview and diagnostic *bytes* are the
//! machine axis and stay identical in every locale (ADR-0033). The edit buffer
//! is Unicode text and `Ctrl-S` always writes UTF-8 — the engine still decodes
//! a Shift_JIS file on open (`-E sjis`), but a Shift_JIS write-back is out of
//! scope.

use std::fs;
use std::io::{self, IsTerminal};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use aozora::{Document, json};
use aozora_i18n::{self as i18n, FluentArgs, LanguageIdentifier};
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

/// Coalesce a burst of keystrokes into a single re-parse: the preview and
/// diagnostics recompute once the buffer has been quiet for this long.
const DEBOUNCE: Duration = Duration::from_millis(150);

/// How long [`event_loop`] blocks waiting for input before checking whether
/// the debounce window has elapsed. Kept below [`DEBOUNCE`] so a pending
/// recompute fires promptly even when the typist has stopped.
const POLL: Duration = Duration::from_millis(50);

/// `aozora tui` — start the full-screen live editor.
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
    /// Optional file to open in the editor. Omit to start with an empty
    /// buffer; `Ctrl-S` then reports that there is no file to save.
    #[arg(value_name = "FILE")]
    file: Option<PathBuf>,

    /// Which projection the preview pane shows at startup: `html` (the
    /// default, as `aozora render`), `nodes` (as `aozora inspect nodes`), or
    /// `pandoc` (the Pandoc AST). Cycle it live with `Ctrl-P`.
    #[arg(long, value_enum, default_value_t = Preview::default())]
    preview: Preview,

    /// Source encoding for opening `[FILE]`: `auto` (default), `utf8`, or
    /// `sjis`. The edit buffer is Unicode; `Ctrl-S` always saves UTF-8.
    #[arg(long, short = 'E', value_enum, default_value_t = Encoding::default())]
    encoding: Encoding,
}

/// Which projection the PREVIEW pane shows. Each maps to the exact engine
/// surface a document subcommand emits, so the pane bytes never diverge.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, ValueEnum)]
enum Preview {
    /// Rendered HTML (as `aozora render`).
    #[default]
    Html,
    /// The parsed source nodes as JSON (as `aozora inspect nodes`).
    Nodes,
    /// The Pandoc AST as JSON (as `aozora pandoc`, pretty-printed for the pane).
    Pandoc,
}

impl Preview {
    /// The next view in the `Ctrl-P` cycle (html → nodes → pandoc → html).
    const fn next(self) -> Self {
        match self {
            Self::Html => Self::Nodes,
            Self::Nodes => Self::Pandoc,
            Self::Pandoc => Self::Html,
        }
    }

    /// The canonical value-enum tag (`html` / `nodes` / `pandoc`) — the
    /// spelling `--preview` accepts and the pane title / footer show.
    const fn tag(self) -> &'static str {
        match self {
            Self::Html => "html",
            Self::Nodes => "nodes",
            Self::Pandoc => "pandoc",
        }
    }
}

/// The recomputed panes: the preview text, the diagnostic report, and the
/// diagnostic count (for the pane title). The pure product of [`derive()`].
#[derive(Debug, Clone, PartialEq, Eq)]
struct Derived {
    /// The PREVIEW pane body — the selected engine projection of the source.
    preview: String,
    /// The DIAGNOSTICS pane body — the plain-text report, or the localized
    /// "no diagnostics" placeholder for a clean parse.
    diagnostics: String,
    /// How many diagnostics fired (shown in the pane title; `0` is clean).
    diag_count: usize,
}

/// Re-parse `source` and project it through `preview`, alongside its
/// diagnostics. Pure — the same parse tree every other subcommand produces, so
/// the HTML matches `aozora render`, the nodes match `aozora inspect nodes`,
/// and the report matches `aozora`'s portable diagnostic text exactly.
fn derive(source: &str, preview: Preview, lang: &LanguageIdentifier) -> Derived {
    let doc = Document::new(source);
    let tree = doc.parse();

    let preview_text = match preview {
        Preview::Html => tree.to_html(),
        Preview::Nodes => json::nodes(&tree),
        Preview::Pandoc => {
            let owned = doc.lex();
            // Pretty-printed for the pane (the compact `aozora pandoc` bytes
            // would wrap into an unreadable blob); serializing cannot fail in
            // practice, so surface the error text rather than panicking.
            serde_json::to_string_pretty(&aozora_pandoc::to_pandoc(&owned))
                .unwrap_or_else(|err| err.to_string())
        }
    };

    // The always-shown diagnostics: the engine's portable plain-text report
    // (English machine axis); an empty report becomes the localized placeholder.
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

/// The transient footer status: the default keybind legend, or a save
/// acknowledgement / error that replaces it until the next language / preview
/// toggle clears it.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Status {
    /// No transient message — the footer shows only the keybind legend.
    Legend,
    /// A success note (e.g. `saved book.txt`), already localized.
    Info(String),
    /// An error note (e.g. no file to save), already localized.
    Error(String),
}

/// An app-level command decoded from a key event. Anything that is not a
/// command is [`Command::Edit`] — the key is forwarded to the text editor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Command {
    /// Write the buffer to the opened file (`Ctrl-S`).
    Save,
    /// Advance the chrome language (`Ctrl-L`).
    ToggleLang,
    /// Advance the preview view (`Ctrl-P`).
    TogglePreview,
    /// Leave the editor (`Ctrl-Q`).
    Quit,
    /// Not a command — hand the key to the source editor.
    Edit,
}

/// Decode a key event into an app [`Command`]. Only `Ctrl`-modified letters are
/// commands, so every unmodified keystroke (including plain letters and the
/// editor's own `Ctrl` shortcuts we do not claim) falls through to
/// [`Command::Edit`]. Pure — the whole keymap is unit-tested here.
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

/// The mutable editor session: the source buffer, the active preview view and
/// chrome language, the opened file, the cached [`Derived`] panes, and the
/// debounce / dirty bookkeeping. Everything a command mutates lives here, kept
/// free of terminal I/O so the transitions are unit-testable.
struct App {
    /// The source EDIT pane's buffer (owns cursor / selection / undo).
    editor: TextArea<'static>,
    /// The active preview projection.
    preview: Preview,
    /// The chrome language (pane titles, legend, status).
    lang: LanguageIdentifier,
    /// The opened file, or `None` for an unbacked buffer.
    path: Option<PathBuf>,
    /// Whether the buffer has unsaved edits since the last save / open.
    modified: bool,
    /// The cached preview + diagnostics, recomputed on the debounce.
    derived: Derived,
    /// The transient footer status.
    status: Status,
    /// When an edit last happened and a recompute is still pending, or `None`
    /// once the panes are up to date.
    pending: Option<Instant>,
}

impl App {
    /// Build a session over an in-memory `source` — the seam the tests and
    /// [`App::new`] share, with no file I/O. The initial panes are derived up
    /// front so the first frame is already live.
    fn from_source(
        source: &str,
        path: Option<PathBuf>,
        preview: Preview,
        lang: LanguageIdentifier,
    ) -> Self {
        // Split on '\n' (not `str::lines`) so a trailing newline survives as a
        // final empty line — `source()` then round-trips the buffer faithfully.
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

    /// Build a session from parsed args: read and decode `[FILE]` through the
    /// shared formatter reader / decoder (the exact byte path `aozora render
    /// FILE` uses), or start empty.
    fn new(args: &TuiArgs, lang: &LanguageIdentifier) -> Result<Self> {
        let (source, path) = match &args.file {
            Some(path) => {
                let raw = aozora_fmt::read_file(path)
                    .with_context(|| format!("failed to read {}", path.display()))?;
                let text = aozora_fmt::decode(&raw, args.encoding)?;
                (text, Some(path.clone()))
            }
            None => (String::new(), None),
        };
        Ok(Self::from_source(&source, path, args.preview, lang.clone()))
    }

    /// The current buffer as one source string (lines rejoined with `\n`).
    fn source(&self) -> String {
        self.editor.lines().join("\n")
    }

    /// Re-derive the panes from the live buffer and clear the pending flag.
    fn recompute(&mut self) {
        self.derived = derive(&self.source(), self.preview, &self.lang);
        self.pending = None;
    }

    /// Record that the buffer changed: mark it dirty and open the debounce
    /// window so [`recompute_due`](Self::recompute_due) fires once it lapses.
    fn mark_edited(&mut self) {
        self.modified = true;
        self.pending = Some(Instant::now());
    }

    /// Whether a pending edit's debounce window has elapsed by `now` — the pure
    /// debounce decision the event loop polls. `false` when nothing is pending.
    fn recompute_due(&self, now: Instant, window: Duration) -> bool {
        self.pending
            .is_some_and(|since| now.saturating_duration_since(since) >= window)
    }

    /// Advance the preview view (`Ctrl-P`), clear any status, and re-derive so
    /// the pane switches immediately.
    fn toggle_preview(&mut self) {
        self.preview = self.preview.next();
        self.status = Status::Legend;
        self.recompute();
    }

    /// Advance the chrome language (`Ctrl-L`), clear any status, and re-derive
    /// so the localized placeholders / titles switch immediately.
    fn toggle_lang(&mut self) {
        self.lang = next_lang(&self.lang);
        self.status = Status::Legend;
        self.recompute();
    }

    /// Save the buffer to the opened file as UTF-8 (`Ctrl-S`). With no file the
    /// status reports there is nothing to save; a write error is shown inline
    /// and is never fatal. Sets the footer status either way.
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

/// The next language in the `Ctrl-L` cycle (en → ja → zh → en). The tags are
/// the CLI's available locales; an unrecognised current language restarts the
/// cycle at `ja` (treating it as English's successor).
fn next_lang(current: &LanguageIdentifier) -> LanguageIdentifier {
    const CYCLE: [&str; 3] = ["en", "ja", "zh"];
    let cur = current.to_string();
    let idx = CYCLE.iter().position(|tag| *tag == cur).unwrap_or(0);
    CYCLE[(idx + 1) % CYCLE.len()]
        .parse()
        .expect("built-in locale tag parses")
}

/// Write the editor buffer to `path` as UTF-8. Split out so the byte write is
/// unit-testable over a tempfile.
fn write_source(path: &Path, source: &str) -> io::Result<()> {
    fs::write(path, source)
}

/// The SOURCE pane title: `source`, plus the file path and a `modified` marker
/// when present. Localized.
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

/// The PREVIEW pane title: `preview · <view>` (the view tag stays literal).
fn preview_title(app: &App) -> String {
    format!(
        "{} · {}",
        i18n::t(&app.lang, "tui-title-preview"),
        app.preview.tag()
    )
}

/// The DIAGNOSTICS pane title: `diagnostics`, with `· <n>` appended when any
/// fired. Localized.
fn diagnostics_title(app: &App) -> String {
    let base = i18n::t(&app.lang, "tui-title-diagnostics");
    if app.derived.diag_count == 0 {
        base
    } else {
        format!("{base} · {}", app.derived.diag_count)
    }
}

/// The keybind legend shown in the footer, with the current language and
/// preview view woven in. Localized action words, literal key glyphs / tags.
fn footer_legend(preview: Preview, lang: &LanguageIdentifier) -> String {
    let save = i18n::t(lang, "tui-key-save");
    let lang_word = i18n::t(lang, "tui-key-lang");
    let preview_word = i18n::t(lang, "tui-key-preview");
    let quit = i18n::t(lang, "tui-key-quit");
    let cur_lang = lang.to_string();
    let cur_preview = preview.tag();
    format!("^S {save}   ^L {lang_word}·{cur_lang}   ^P {preview_word}·{cur_preview}   ^Q {quit}")
}

/// The footer line: the dim keybind legend, plus the transient status (green
/// for a save, red for an error) when one is set.
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

/// Lay out and draw the three panes and the footer into `frame`. Pure over
/// `app` — no terminal state — so it renders identically onto `ratatui`'s
/// `TestBackend` in the unit tests as onto a real terminal.
fn render(app: &App, frame: &mut Frame<'_>) {
    // Rows: the pane body, then a one-line footer.
    let [body, footer_area] =
        Layout::vertical([Constraint::Min(3), Constraint::Length(1)]).areas(frame.area());
    // The body: the top panes over the diagnostics pane.
    let [top, diag_area] =
        Layout::vertical([Constraint::Percentage(60), Constraint::Percentage(40)]).areas(body);
    // The top: source (left) beside preview (right).
    let [src_area, prev_area] =
        Layout::horizontal([Constraint::Percentage(50), Constraint::Percentage(50)]).areas(top);

    // Source EDIT pane.
    let src_block = Block::bordered().title(source_title(app));
    let src_inner = src_block.inner(src_area);
    frame.render_widget(src_block, src_area);
    frame.render_widget(&app.editor, src_inner);

    // Live PREVIEW pane.
    let prev_block = Block::bordered().title(preview_title(app));
    let prev_inner = prev_block.inner(prev_area);
    frame.render_widget(prev_block, prev_area);
    frame.render_widget(
        Paragraph::new(app.derived.preview.as_str()).wrap(Wrap { trim: false }),
        prev_inner,
    );

    // DIAGNOSTICS pane.
    let diag_block = Block::bordered().title(diagnostics_title(app));
    let diag_inner = diag_block.inner(diag_area);
    frame.render_widget(diag_block, diag_area);
    frame.render_widget(
        Paragraph::new(app.derived.diagnostics.as_str()).wrap(Wrap { trim: false }),
        diag_inner,
    );

    // Footer: keybind legend + transient status.
    frame.render_widget(Paragraph::new(footer_line(app)), footer_area);
}

/// Whether the TUI can run: it reads key events from stdin and draws the panes
/// to stdout, so it needs **every** stream it drives to be a real terminal — a
/// pipe on any end (`… | aozora tui` or `aozora tui | …`) means it cannot
/// operate. Pure over the per-stream tty flags (`[stdin, stdout]` in [`run`]),
/// so the "all, not any" rule is unit-tested rather than resting on the
/// untestable real-tty guard.
fn all_terminals(stream_ttys: [bool; 2]) -> bool {
    stream_ttys.iter().all(|&is_tty| is_tty)
}

/// Start the editor: refuse a non-terminal (the TUI needs a tty for rendering
/// and key input — a piped invocation gets an actionable error, not a hang),
/// open the optional file, then run the loop. Always exits 0 on a clean quit.
///
/// Real-tty only past the guard ([`run_app`] enters raw mode + the alternate
/// screen), so the sweep cannot exercise it; its one decision — the pure
/// [`all_terminals`] predicate — is unit-tested, and the refusal path is
/// covered end-to-end by the `tui_without_a_terminal_refuses` smoke test.
#[cfg_attr(test, mutants::skip)]
pub(crate) fn run(args: &TuiArgs, lang: &LanguageIdentifier) -> Result<ExitCode> {
    if !all_terminals([io::stdin().is_terminal(), io::stdout().is_terminal()]) {
        anyhow::bail!("{}", i18n::t(lang, "tui-no-tty"));
    }
    let app = App::new(args, lang)?;
    run_app(app)
}

/// Enter raw mode + the alternate screen, run [`event_loop`], then restore the
/// terminal on every exit path (clean quit, error, or panic-unwind of the
/// loop). Needs a real terminal, so it is skipped by the mutation sweep and
/// covered only by the pure [`render`] / [`command_for`] / `App` tests below.
#[cfg_attr(test, mutants::skip)]
fn run_app(mut app: App) -> Result<ExitCode> {
    terminal::enable_raw_mode().context("failed to enable raw mode")?;
    let mut stdout = io::stdout();
    execute!(stdout, terminal::EnterAlternateScreen)
        .context("failed to enter the alternate screen")?;
    let backend = CrosstermBackend::new(stdout);
    let mut term = Terminal::new(backend).context("failed to create the terminal")?;

    let outcome = event_loop(&mut term, &mut app);

    // Restore the terminal unconditionally, even if the loop errored, so a
    // failure never leaves the user's shell in raw mode / the alternate screen.
    let _drop = terminal::disable_raw_mode();
    let _drop = execute!(term.backend_mut(), terminal::LeaveAlternateScreen);
    let _drop = term.show_cursor();

    outcome
}

/// The draw / input loop: redraw when something changed, poll for a key, run
/// its [`Command`] (or forward it to the editor), and fire the debounced
/// recompute once the buffer goes quiet. Returns on `Ctrl-Q`. Real-tty only —
/// skipped by the sweep; the pure seams it drives are unit-tested.
#[cfg_attr(test, mutants::skip)]
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
    use ratatui::backend::TestBackend;
    use ratatui::buffer::{Buffer, Cell};

    use super::*;

    fn lang(tag: &str) -> LanguageIdentifier {
        tag.parse().expect("test locale tag parses")
    }

    fn app(source: &str, preview: Preview, tag: &str) -> App {
        App::from_source(source, None, preview, lang(tag))
    }

    /// A key event for `Ctrl` + a letter.
    fn ctrl(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL)
    }

    /// Every cell's symbol concatenated row-major — enough to assert that an
    /// ASCII substring (a pane title, a keybind glyph) is on screen.
    fn buffer_text(buffer: &Buffer) -> String {
        buffer.content.iter().map(Cell::symbol).collect()
    }

    // --- Preview: the cycle and its tags ---

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

    // --- next_lang: the language cycle ---

    #[test]
    fn next_lang_cycles_en_ja_zh() {
        assert_eq!(next_lang(&lang("en")), lang("ja"));
        assert_eq!(next_lang(&lang("ja")), lang("zh"));
        assert_eq!(next_lang(&lang("zh")), lang("en"));
    }

    // --- derive: each view reuses the exact engine bytes ---

    #[test]
    fn derive_html_matches_the_render_engine() {
        let source = "｜青空《あおぞら》";
        let d = derive(source, Preview::Html, &lang("en"));
        let expected = Document::new(source).parse().to_html();
        assert!(!expected.is_empty(), "fixture renders to non-empty HTML");
        assert_eq!(d.preview, expected, "preview is the render bytes");
        assert_eq!(d.diag_count, 0, "clean fixture");
    }

    #[test]
    fn derive_nodes_matches_the_inspect_engine() {
        let source = "青空《あおぞら》";
        let d = derive(source, Preview::Nodes, &lang("en"));
        let expected = json::nodes(&Document::new(source).parse());
        assert_eq!(d.preview, expected, "preview is the inspect-nodes bytes");
    }

    #[test]
    fn derive_pandoc_projects_the_ast() {
        // The pandoc view carries the AST (pretty-printed): a Pandoc document
        // is a JSON object with the `pandoc-api-version` key.
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
        // A private-use sentinel reliably fires a diagnostic (the same fixture
        // the engine's diagnostics_text test uses).
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
        // The chrome placeholder localizes; the diagnostic report bytes (the
        // machine axis) do not — identical to the English run.
        let ja = derive("bad \u{E001} char", Preview::Html, &lang("ja"));
        let en = derive("bad \u{E001} char", Preview::Html, &lang("en"));
        assert_eq!(
            ja.diagnostics, en.diagnostics,
            "report is language-invariant"
        );
    }

    // --- command_for: the keymap ---

    #[test]
    fn command_for_maps_the_ctrl_keys() {
        assert_eq!(command_for(ctrl('s')), Command::Save);
        assert_eq!(command_for(ctrl('l')), Command::ToggleLang);
        assert_eq!(command_for(ctrl('p')), Command::TogglePreview);
        assert_eq!(command_for(ctrl('q')), Command::Quit);
    }

    #[test]
    fn command_for_forwards_everything_else_to_the_editor() {
        // A plain letter is an edit, and an unclaimed Ctrl combo is too.
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

    // --- all_terminals: the tty guard ---

    #[test]
    fn all_terminals_needs_every_stream() {
        // Every stream the TUI drives must be a real tty; a pipe on any end (or
        // both) is not interactive — the "all, not any" rule the `run` guard
        // rests on. Order is `[stdin, stdout]`.
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

    // --- App transitions ---

    #[test]
    fn from_source_derives_the_initial_panes() {
        let a = app("青空《あおぞら》", Preview::Html, "en");
        assert_eq!(a.source(), "青空《あおぞら》", "buffer holds the source");
        assert!(!a.derived.preview.is_empty(), "panes derived up front");
        assert!(!a.modified, "a freshly opened buffer is clean");
    }

    #[test]
    fn source_round_trips_a_trailing_newline() {
        // Splitting on '\n' (not `lines`) keeps a final empty line.
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
            json::nodes(&Document::new("青空《あおぞら》").parse()),
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
        // The clean-parse placeholder now localizes (no longer the English text).
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
        // Nothing pending → never due.
        assert!(!a.recompute_due(base, DEBOUNCE));
        a.pending = Some(base);
        // Before the window elapses → not due; at/after the window → due.
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

    // --- save: the file write and its statuses ---

    #[test]
    fn save_writes_the_buffer_and_acknowledges() {
        let file = tempfile::Builder::new()
            .suffix(".aozora")
            .tempfile()
            .expect("temp file");
        let path = file.path().to_path_buf();
        let mut a = App::from_source(
            "青空《あおぞら》",
            Some(path.clone()),
            Preview::Html,
            lang("en"),
        );
        a.modified = true;
        a.save();
        // The bytes reached disk, the dirty flag cleared, and the status names
        // the saved path.
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

    // --- titles / legend ---

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

    // --- render: the headless TestBackend draw ---

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
        // A clean parse shows the localized "no diagnostics" placeholder.
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
        // The engine's diagnostic code reaches the diagnostics pane.
        assert!(
            text.contains("aozora"),
            "diagnostic drawn in the pane: {text}"
        );
    }
}
