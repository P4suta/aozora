#![expect(
    clippy::expect_used,
    reason = "doctor constructs a valid static version requirement"
)]

//! `aozora doctor` — the end-user runtime self-check.
//!
//! Distinct from the contributor `just doctor`, which probes the *development*
//! toolchain, git hooks, signing, and optional system dependencies. This one answers the
//! reader's question — "will `aozora` behave the way I think it will *here*?" —
//! by reporting four things and nothing it has to reach the network for:
//!
//! 1. **Configuration** — the discovered `.aozora.toml` (project) and XDG
//!    `config.toml` (global), and whether the effective file parses with no
//!    unknown keys. A malformed file is the one *blocking* failure (exit 1).
//! 2. **Effective settings** — the resolved `encoding` / `format` / `strict` /
//!    `color` / `lang`, each tagged with the source that decided it
//!    (`flag` / `env …` / `project` / `global` / `default`).
//! 3. **External tools** — whether `pandoc` is on `PATH` (with its version);
//!    it is optional, so its absence is advisory, never blocking. (The LSP is
//!    built into this binary — `aozora lsp` — so there is no daemon to probe.)
//! 4. **Terminal** — whether stdout / stderr are TTYs, the `NO_COLOR` /
//!    `CLICOLOR` state, and the colour the CLI would actually emit.
//!
//! The section headings, status words, and hints are localized through
//! the `i18n` catalog; the setting / tool identifiers, enum tags, source labels, and
//! tool versions woven in are machine vocabulary and stay literal in every
//! locale (ADR-0033). The report goes to stdout; the exit code (0 all-green /
//! 1 blocking) is the machine signal.

use std::env;
use std::fmt;
use std::io::{self, IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

use crate::i18n::{self as i18n, FluentArgs, LanguageIdentifier};
use anyhow::{Context, Result};
use clap::ValueEnum;

use crate::config::{ConfigFile, Layers, strict_active};
use crate::diagnostics_render::DiagFormat;
use crate::which::which;
use crate::{ColorChoice, Encoding};

/// Everything `main` already decided about colour: the flag layer it decided
/// *from* that doctor cannot re-read for itself, and the choice it installed.
///
/// Passed whole rather than re-derived, because a second derivation could drift
/// from the hook this process is actually running under. The raw flag
/// attributes the settings row to its [`Source`]; `resolved` is what the report
/// states as effective and what the terminal section weighs.
#[derive(Debug, Clone, Copy)]
pub(crate) struct ColorFacts {
    /// The parsed global `--color`.
    pub(crate) flag: Option<ColorChoice>,
    /// The choice [`color::resolve`](crate::color::resolve) folded from that
    /// flag plus the config, and installed process-wide.
    pub(crate) resolved: ColorChoice,
}

/// Run the runtime self-check: gather the facts, render the localized report to
/// stdout, and exit 0 (all-green) or 1 (a blocking problem — a malformed
/// config).
///
/// `lang` is likewise passed both raw and resolved, for the reason [`ColorFacts`]
/// documents: `lang_flag` attributes the row, `lang` states it.
pub(crate) fn run(
    color: ColorFacts,
    lang_flag: Option<&str>,
    lang: &LanguageIdentifier,
) -> Result<ExitCode> {
    let doctor = Doctor::gather(color, lang_flag, lang);
    let (report, blocking) = doctor.render(lang);
    let mut stdout = io::stdout().lock();
    stdout
        .write_all(report.as_bytes())
        .context("failed to write the doctor report to stdout")?;
    Ok(if blocking == 0 {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(1)
    })
}

/// Which precedence layer supplied an effective setting's value. Rendered as a
/// literal, un-localized label (machine vocabulary, mirroring the documented
/// `flag > env > project > global > default` order).
#[derive(Debug, Clone, Copy)]
enum Source {
    /// A command-line flag (`--color` / `--lang`).
    Flag,
    /// An environment variable, named for the report.
    Env(&'static str),
    /// The project `.aozora.toml`.
    Project,
    /// The XDG global `config.toml`.
    Global,
    /// The built-in default.
    Default,
}

impl Source {
    /// The literal source label shown in the settings dump.
    fn label(self) -> String {
        match self {
            Self::Flag => "flag".to_owned(),
            Self::Env(var) => format!("env {var}"),
            Self::Project => "project".to_owned(),
            Self::Global => "global".to_owned(),
            Self::Default => "default".to_owned(),
        }
    }
}

/// One row of the effective-settings dump: a literal identifier label and how
/// that setting resolved — either a clean value + [`Source`], or an
/// environment value the runtime would reject.
#[derive(Debug)]
struct SettingRow {
    label: &'static str,
    resolution: Resolution,
}

/// How one effective setting resolved. A clean value is reported with the
/// [`Source`] that decided it; a set-but-invalid environment variable is *not*
/// a clean setting — the runtime's clap parser would reject it (exit 2), so
/// doctor surfaces it as a blocking problem instead.
#[derive(Debug)]
enum Resolution {
    /// A resolved value (an enum tag / boolean / locale — machine vocabulary)
    /// and the layer that decided it.
    Resolved { value: String, source: Source },
    /// `var` is set to `raw`, which the CLI runtime's clap parser rejects (a
    /// case-sensitive `ValueEnum` mismatch, or a bool that is not exactly
    /// `true` / `false`). Rendered as a blocking problem, not a setting.
    RejectedEnv { var: &'static str, raw: String },
}

/// The parsed state of an environment-backed setting, mirroring how clap reads
/// it: absent, a valid value, or a set-but-invalid value the runtime rejects.
#[derive(Debug)]
enum EnvSetting<T> {
    Unset,
    Valid(T),
    Invalid(String),
}

/// The configuration section's facts: the directory the upward search began
/// from, and either the two discovered layer paths or the actionable loader
/// message for a malformed file (the one blocking failure).
#[derive(Debug)]
struct ConfigReport {
    cwd: PathBuf,
    outcome: Result<ConfigPaths, String>,
}

/// The two discovered config file paths (each `None` when that layer is absent).
#[derive(Debug)]
struct ConfigPaths {
    project: Option<PathBuf>,
    global: Option<PathBuf>,
}

/// A PATH-tool probe result: absent, or present with an optional detail
/// (`pandoc`'s version line).
#[derive(Debug)]
enum ToolStatus {
    Missing,
    Found { detail: Option<String> },
}

/// The terminal-capability facts feeding the colour decision.
#[derive(Debug)]
struct TerminalReport {
    stdout_tty: bool,
    stderr_tty: bool,
    /// The `NO_COLOR` / `CLICOLOR` values when set (the raw string), else `None`.
    no_color: Option<String>,
    clicolor: Option<String>,
    /// The colour the CLI would actually emit, resolved from the flag + env +
    /// stderr TTY exactly as the runtime hook does.
    colour_on: bool,
}

/// Every fact the report renders, gathered once from the environment so
/// [`render`](Doctor::render) is a pure function of them (unit-tested directly
/// with fabricated facts, end-to-end via the pinned-env snapshot).
#[derive(Debug)]
struct Doctor {
    config: ConfigReport,
    settings: Vec<SettingRow>,
    pandoc: ToolStatus,
    terminal: TerminalReport,
}

impl Doctor {
    /// Probe the environment: load the config layers, resolve the effective
    /// settings and their sources, probe the PATH tools, and read the terminal
    /// capabilities.
    fn gather(color: ColorFacts, lang_flag: Option<&str>, lang: &LanguageIdentifier) -> Self {
        let cwd = env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        let (config, layers) = load_config(&cwd);
        let settings = resolve_settings(&layers, color, lang_flag, lang);
        // The colour `main` resolved, not the raw flag: the terminal section
        // reports what this run would actually emit, so it must weigh the
        // config layer the hook was installed from.
        let terminal = terminal_report(color.resolved);
        Self {
            config,
            settings,
            pandoc: probe_pandoc(),
            terminal,
        }
    }

    /// Render the localized human report and the count of blocking problems.
    fn render(&self, lang: &LanguageIdentifier) -> (String, usize) {
        let mut out = String::new();
        let blocking = self
            .write_report(&mut out, lang)
            .expect("writing to a String is infallible");
        (out, blocking)
    }

    /// Assemble the report into `out`, returning the blocking-problem count.
    /// Split from [`render`](Self::render) so the whole body threads `?` on the
    /// infallible `String` writes and pays the `expect` exactly once.
    fn write_report(
        &self,
        out: &mut String,
        lang: &LanguageIdentifier,
    ) -> Result<usize, fmt::Error> {
        use std::fmt::Write as _;
        let mut blocking = 0usize;

        writeln!(out, "{}", i18n::t(lang, "doctor-title"))?;

        // -- Configuration --
        writeln!(out, "\n{}", i18n::t(lang, "doctor-config-heading"))?;
        match &self.config.outcome {
            Ok(paths) => {
                let project = paths.project.as_ref().map_or_else(
                    || {
                        let mut args = FluentArgs::new();
                        args.set("dir", self.config.cwd.display().to_string());
                        i18n::tf(lang, "doctor-project-none", &args)
                    },
                    |path| path.display().to_string(),
                );
                writeln!(out, "  {:<22} {project}", "project .aozora.toml")?;
                let global = paths.global.as_ref().map_or_else(
                    || i18n::t(lang, "doctor-global-none"),
                    |path| path.display().to_string(),
                );
                writeln!(out, "  {:<22} {global}", "global config.toml")?;
                writeln!(out, "  {}", i18n::t(lang, "doctor-parse-ok"))?;
            }
            Err(error) => {
                blocking += 1;
                let mut args = FluentArgs::new();
                args.set("error", error.clone());
                writeln!(out, "  {}", i18n::tf(lang, "doctor-parse-error", &args))?;
            }
        }

        // -- Effective settings -- (each rejected env is a blocking problem)
        blocking += write_settings(out, lang, &self.settings)?;

        // -- External tools --
        writeln!(out, "\n{}", i18n::t(lang, "doctor-tools-heading"))?;
        render_tool(out, lang, "pandoc", &self.pandoc, "doctor-hint-pandoc")?;

        // -- Terminal --
        writeln!(out, "\n{}", i18n::t(lang, "doctor-terminal-heading"))?;
        let yes = i18n::t(lang, "doctor-terminal-yes");
        let no = i18n::t(lang, "doctor-terminal-no");
        let tty = |on: bool| if on { &yes } else { &no };
        writeln!(out, "  {:<18}{}", "stdout", tty(self.terminal.stdout_tty))?;
        writeln!(out, "  {:<18}{}", "stderr", tty(self.terminal.stderr_tty))?;
        writeln!(
            out,
            "  {:<18}{}",
            "NO_COLOR",
            env_state(lang, self.terminal.no_color.as_deref())
        )?;
        writeln!(
            out,
            "  {:<18}{}",
            "CLICOLOR",
            env_state(lang, self.terminal.clicolor.as_deref())
        )?;
        let colour = if self.terminal.colour_on {
            i18n::t(lang, "doctor-colour-on")
        } else {
            i18n::t(lang, "doctor-colour-off")
        };
        writeln!(
            out,
            "  {:<18}{colour}",
            i18n::t(lang, "doctor-colour-label")
        )?;

        // -- Summary --
        if blocking == 0 {
            writeln!(out, "\n{}", i18n::t(lang, "doctor-all-passed"))?;
        } else {
            let mut args = FluentArgs::new();
            args.set("count", blocking.to_string());
            writeln!(out, "\n{}", i18n::tf(lang, "doctor-problems", &args))?;
        }

        Ok(blocking)
    }
}

/// Render the effective-settings section into `out`, returning the count of
/// blocking problems it found — one per [`Resolution::RejectedEnv`] row, an
/// environment value the CLI runtime would reject rather than a clean setting.
fn write_settings(
    out: &mut String,
    lang: &LanguageIdentifier,
    settings: &[SettingRow],
) -> Result<usize, fmt::Error> {
    use std::fmt::Write as _;
    let mut blocking = 0usize;
    writeln!(out, "\n{}", i18n::t(lang, "doctor-settings-heading"))?;
    for row in settings {
        match &row.resolution {
            Resolution::Resolved { value, source } => {
                writeln!(out, "  {:<10} {value:<8} {}", row.label, source.label())?;
            }
            Resolution::RejectedEnv { var, raw } => {
                blocking += 1;
                let mut args = FluentArgs::new();
                args.set("var", (*var).to_owned());
                args.set("value", raw.clone());
                writeln!(
                    out,
                    "  {:<10} {}",
                    row.label,
                    i18n::tf(lang, "doctor-setting-rejected", &args)
                )?;
            }
        }
    }
    Ok(blocking)
}

/// Render one PATH-tool row (plus a hint line when it is missing) into `out`.
#[expect(
    clippy::too_many_arguments,
    reason = "the five inputs (sink, language, tool name, its status, and its localized hint key) are each distinct; a bundle struct would move the arity without clarifying it"
)]
fn render_tool(
    out: &mut String,
    lang: &LanguageIdentifier,
    name: &str,
    status: &ToolStatus,
    hint_key: &str,
) -> fmt::Result {
    use std::fmt::Write as _;
    match status {
        ToolStatus::Found { detail } => {
            let shown = detail.as_deref().unwrap_or_default();
            writeln!(out, "  {name:<12} {shown}")
        }
        ToolStatus::Missing => {
            writeln!(out, "  {name:<12} {}", i18n::t(lang, "doctor-tool-missing"))?;
            writeln!(out, "    ↳ {}", i18n::t(lang, hint_key))
        }
    }
}

/// The localized `set (value)` / `unset` state of an environment variable.
fn env_state(lang: &LanguageIdentifier, value: Option<&str>) -> String {
    value.map_or_else(
        || i18n::t(lang, "doctor-env-unset"),
        |raw| {
            let mut args = FluentArgs::new();
            args.set("value", raw.to_owned());
            i18n::tf(lang, "doctor-env-set", &args)
        },
    )
}

/// Load the two config layers for the report. On a malformed file the loader
/// error becomes the (blocking) configuration-error message and the settings
/// dump falls back to empty layers, so `env` / `default` sources still resolve.
fn load_config(cwd: &Path) -> (ConfigReport, Layers) {
    match ConfigFile::layers(cwd) {
        Ok(layers) => {
            let paths = ConfigPaths {
                project: layers.project_path.clone(),
                global: layers.global_path.clone(),
            };
            (
                ConfigReport {
                    cwd: cwd.to_owned(),
                    outcome: Ok(paths),
                },
                layers,
            )
        }
        Err(error) => (
            ConfigReport {
                cwd: cwd.to_owned(),
                outcome: Err(format!("{error:#}")),
            },
            Layers::default(),
        ),
    }
}

/// Resolve the effective settings and the source that decided each,
/// mirroring the runtime **exactly** (that fidelity is the whole point — a
/// re-implementation that drifts is worse than useless):
///
/// - `encoding` / `format` — the value-enums layer env > project > global >
///   default (`args.x.or(cfg.x).unwrap_or_default()`), and the env value is
///   parsed *case-sensitively*, exactly as clap's `ValueEnum` does here (the
///   args declare no `ignore_case`). A set-but-invalid env is the runtime's
///   hard rejection (exit 2), surfaced as a blocking problem — never a clean
///   setting. See [`resolve_value_enum`].
/// - `strict` — **not** layered: the runtime computes `args.strict ||
///   cfg.strict.unwrap_or(false)` (a boolean OR, main.rs), so a config
///   `strict = true` forces strict ON even when the env / flag says `false`,
///   and a config `strict = false` is indistinguishable from unset. The env is
///   parsed with clap's bool parser (exactly `true` / `false`). See
///   [`resolve_strict`].
/// - `color` and `lang` are the two settings doctor carries a flag for (both
///   `--color` / `--lang` are global), so `main` has already resolved each for
///   this very process: their values are reported as passed in and only
///   *attributed* here — `color` over `flag > project > global > default`
///   ([`colour_source`]), `lang` over `--lang > AOZORA_LANG > config.lang >
///   LANG` ([`lang_source`]). Neither can produce the
///   [`Resolution::RejectedEnv`] row the subcommand-local env vars above can:
///   colour reads no environment variable at all, and an unknown `AOZORA_LANG`
///   locale negotiates rather than fails.
fn resolve_settings(
    layers: &Layers,
    color: ColorFacts,
    lang_flag: Option<&str>,
    lang: &LanguageIdentifier,
) -> Vec<SettingRow> {
    let encoding = resolve_value_enum(
        env_value_enum::<Encoding>("AOZORA_ENCODING"),
        "AOZORA_ENCODING",
        layers.project.encoding,
        layers.global.encoding,
        Encoding::default(),
    );
    let format = resolve_value_enum(
        env_value_enum::<DiagFormat>("AOZORA_FORMAT"),
        "AOZORA_FORMAT",
        layers.project.format,
        layers.global.format,
        DiagFormat::default(),
    );
    let strict = resolve_strict(
        env_bool("AOZORA_STRICT"),
        layers.project.strict,
        layers.global.strict,
    );
    // The colour `main` resolved, reported as-is: re-deriving the value here
    // could drift from the hook actually installed. Only its attribution is
    // recomputed, from the same layers `color::resolve` folded.
    let color = Resolution::Resolved {
        value: enum_tag(&color.resolved),
        source: colour_source(color.flag, layers.project.color, layers.global.color),
    };
    let lang_res = Resolution::Resolved {
        value: lang.to_string(),
        source: lang_source(
            lang_flag,
            env::var("AOZORA_LANG").ok().as_deref(),
            layers.project.lang.as_deref(),
            layers.global.lang.as_deref(),
            env::var("LANG").ok().as_deref(),
        ),
    };
    vec![
        SettingRow {
            label: "encoding",
            resolution: encoding,
        },
        SettingRow {
            label: "format",
            resolution: format,
        },
        SettingRow {
            label: "strict",
            resolution: strict,
        },
        SettingRow {
            label: "color",
            resolution: color,
        },
        SettingRow {
            label: "lang",
            resolution: lang_res,
        },
    ]
}

/// Resolve one env > project > global > default value-enum setting into a
/// [`Resolution`]. A set-but-invalid env is a [`Resolution::RejectedEnv`] (the
/// runtime rejects it); otherwise the first present layer wins, tagged with its
/// [`Source`].
///
/// Used for the settings doctor *simulates* rather than resolves — `encoding` /
/// `format`, whose flags belong to the document subcommands, so doctor reads
/// their env vars itself to answer "what would `aozora check` use here?".
/// `color` / `lang` are real resolutions this process already performed, so
/// they attribute through [`colour_source`] / [`lang_source`] instead.
#[expect(
    clippy::too_many_arguments,
    reason = "the four layers (env outcome + var name, project, global, default) are each independent; a bundle struct would move the arity without clarifying it"
)]
fn resolve_value_enum<T: ValueEnum + Copy>(
    from_env: EnvSetting<T>,
    env_var: &'static str,
    project: Option<T>,
    global: Option<T>,
    default: T,
) -> Resolution {
    match from_env {
        EnvSetting::Invalid(raw) => Resolution::RejectedEnv { var: env_var, raw },
        EnvSetting::Valid(value) => Resolution::Resolved {
            value: enum_tag(&value),
            source: Source::Env(env_var),
        },
        EnvSetting::Unset => {
            let (value, source) = layer(project, global, default);
            Resolution::Resolved {
                value: enum_tag(&value),
                source,
            }
        }
    }
}

/// The project > global > default walk for a `Copy` setting once the env layer
/// is out, returning the value and the [`Source`] that decided it.
fn layer<T: Copy>(project: Option<T>, global: Option<T>, default: T) -> (T, Source) {
    project
        .map(|value| (value, Source::Project))
        .or_else(|| global.map(|value| (value, Source::Global)))
        .unwrap_or((default, Source::Default))
}

/// Resolve `strict` the way the runtime does — a boolean OR, **not** a layered
/// override (main.rs: `args.strict || cfg.strict.unwrap_or(false)`). Doctor
/// carries no `--strict` flag, so `args.strict` reduces to the parsed env value
/// (`false` when unset); the merged config value (project winning) folds in with
/// `||`. The consequence, and the reason a re-implemented layering was wrong: a
/// config `strict = true` forces strict ON even when `AOZORA_STRICT=false`, and
/// a config `strict = false` is a no-op. The source is whatever turns strict on
/// (env over project over global); the default-off otherwise. A rejected env is
/// the runtime's hard error, same as the value-enums.
fn resolve_strict(
    from_env: EnvSetting<bool>,
    project: Option<bool>,
    global: Option<bool>,
) -> Resolution {
    let env_strict = match from_env {
        EnvSetting::Invalid(raw) => {
            return Resolution::RejectedEnv {
                var: "AOZORA_STRICT",
                raw,
            };
        }
        EnvSetting::Valid(value) => Some(value),
        EnvSetting::Unset => None,
    };
    let flag_or_env = env_strict.unwrap_or(false);
    // The merged config value, project winning (ConfigFile::merge), exactly as
    // `cfg.strict` reaches the runtime. The effective value comes from the same
    // helper `run_check` / `run_lint` use, so the report cannot drift from it.
    let cfg_strict = project.or(global);
    let effective = strict_active(flag_or_env, cfg_strict);
    let source = if flag_or_env {
        Source::Env("AOZORA_STRICT")
    } else if cfg_strict == Some(true) {
        // Config forces strict ON despite a false / unset env — attribute it
        // truthfully to the config layer that carried the `true`.
        if project == Some(true) {
            Source::Project
        } else {
            Source::Global
        }
    } else {
        Source::Default
    };
    Resolution::Resolved {
        value: effective.to_string(),
        source,
    }
}

/// The [`Source`] that decides the message language, mirroring
/// `crate::i18n::resolve`: the first present-and-non-blank of `--lang`,
/// `AOZORA_LANG`, `config.lang` (project over global), then `LANG`; else the
/// built-in English default.
#[expect(
    clippy::too_many_arguments,
    reason = "the five language sources mirror crate::i18n::resolve's precedence chain one-to-one; each is a distinct layer"
)]
fn lang_source(
    flag: Option<&str>,
    aozora_lang: Option<&str>,
    project: Option<&str>,
    global: Option<&str>,
    sys_lang: Option<&str>,
) -> Source {
    if present(flag) {
        Source::Flag
    } else if present(aozora_lang) {
        Source::Env("AOZORA_LANG")
    } else if present(project) {
        Source::Project
    } else if present(global) {
        Source::Global
    } else if present(sys_lang) {
        Source::Env("LANG")
    } else {
        Source::Default
    }
}

/// True when a language source is present and not blank — the "decides"
/// predicate `crate::i18n::resolve` uses when walking its precedence chain.
fn present(source: Option<&str>) -> bool {
    source.is_some_and(|value| !value.trim().is_empty())
}

/// The [`Source`] that decides the colour choice, mirroring `color::resolve`
/// layer for layer: `--color`, then the project / global `color` key, then the
/// built-in `auto`.
///
/// There is no environment rung between the flag and the file — colour defers
/// to the standard `NO_COLOR` / `CLICOLOR` / `CLICOLOR_FORCE` signals on the
/// `auto` path instead of carrying an `AOZORA_*` variable — so this walks the
/// very layers the installed hook was folded from and cannot drift from them.
fn colour_source(
    flag: Option<ColorChoice>,
    project: Option<ColorChoice>,
    global: Option<ColorChoice>,
) -> Source {
    if flag.is_some() {
        Source::Flag
    } else if project.is_some() {
        Source::Project
    } else if global.is_some() {
        Source::Global
    } else {
        Source::Default
    }
}

/// Whether the CLI would emit colour, mirroring the signals miette consults for
/// `--color auto`: `CLICOLOR_FORCE` (non-`0`) forces on, then `NO_COLOR`
/// (present) forces off, then `CLICOLOR=0` forces off, else the stderr TTY
/// decides. `always` / `never` short-circuit the whole chain.
#[expect(
    clippy::too_many_arguments,
    clippy::fn_params_excessive_bools,
    reason = "the five inputs are the distinct colour signals miette weighs (the flag, the stderr TTY, and the NO_COLOR / CLICOLOR / CLICOLOR_FORCE env vars); a struct would not clarify them"
)]
fn colour_on(
    choice: ColorChoice,
    stderr_tty: bool,
    no_color: bool,
    clicolor: Option<&str>,
    clicolor_force: Option<&str>,
) -> bool {
    match choice {
        ColorChoice::Always => true,
        ColorChoice::Never => false,
        ColorChoice::Auto => {
            if clicolor_force.is_some_and(|value| value != "0") {
                true
            } else if no_color || clicolor.is_some_and(|value| value == "0") {
                false
            } else {
                stderr_tty
            }
        }
    }
}

/// Classify a raw environment value into an [`EnvSetting`] with `parse`: absent
/// (`None`) is [`EnvSetting::Unset`], a value `parse` accepts is
/// [`EnvSetting::Valid`], and one it rejects is [`EnvSetting::Invalid`] (kept
/// verbatim for the report). The parse is passed in so the env-reading and the
/// clap-faithful parsing are unit-testable apart.
fn classify_env<T>(raw: Option<String>, parse: impl Fn(&str) -> Option<T>) -> EnvSetting<T> {
    let Some(value) = raw else {
        return EnvSetting::Unset;
    };
    parse(&value).map_or(EnvSetting::Invalid(value), EnvSetting::Valid)
}

/// Read a value-enum environment variable and classify it *exactly* as clap
/// would for the matching `--encoding` / `--format` arg: `ValueEnum::from_str`
/// with `ignore_case = false`, since those args declare no `ignore_case`. A
/// value of the wrong case (`SJIS`, `JSON`) is therefore [`EnvSetting::Invalid`],
/// mirroring the runtime's exit-2 rejection rather than silently accepting it.
fn env_value_enum<T: ValueEnum>(var: &str) -> EnvSetting<T> {
    classify_env(env::var(var).ok(), |raw| T::from_str(raw, false).ok())
}

/// Read `AOZORA_STRICT` and classify it *exactly* as clap parses the `--strict`
/// bool: only the literal `true` / `false` are accepted (clap's `BoolValueParser`
/// is precisely `bool::from_str` — case-sensitive, un-trimmed), so `on` / `off`
/// / `1` / `0` / `yes` are [`EnvSetting::Invalid`], the runtime's exit-2
/// rejection.
fn env_bool(var: &str) -> EnvSetting<bool> {
    classify_env(env::var(var).ok(), |raw| raw.parse::<bool>().ok())
}

/// A value-enum's wire tag (`auto` / `utf8` / …) — the machine-vocabulary value
/// shown in the settings dump.
fn enum_tag<T: ValueEnum>(value: &T) -> String {
    value
        .to_possible_value()
        .map_or_else(|| "?".to_owned(), |possible| possible.get_name().to_owned())
}

/// Read the terminal-capability facts and resolve the effective colour.
/// `choice` is the colour `main` already resolved through the full layer chain
/// (`color::resolve`), so the reported `colour_on` is this run's real outcome.
fn terminal_report(choice: ColorChoice) -> TerminalReport {
    let stderr_tty = io::stderr().is_terminal();
    let no_color = env::var("NO_COLOR").ok();
    let clicolor = env::var("CLICOLOR").ok();
    let clicolor_force = env::var("CLICOLOR_FORCE").ok();
    let colour_on = colour_on(
        choice,
        stderr_tty,
        env::var_os("NO_COLOR").is_some(),
        clicolor.as_deref(),
        clicolor_force.as_deref(),
    );
    TerminalReport {
        stdout_tty: io::stdout().is_terminal(),
        stderr_tty,
        no_color,
        clicolor,
        colour_on,
    }
}

/// Probe `pandoc` on PATH, reading its first `--version` line when present.
fn probe_pandoc() -> ToolStatus {
    which("pandoc").map_or(ToolStatus::Missing, |path| ToolStatus::Found {
        detail: pandoc_version(&path),
    })
}

/// The first line of `pandoc --version`, or `None` if it cannot be read.
fn pandoc_version(path: &Path) -> Option<String> {
    let output = Command::new(path).arg("--version").output().ok()?;
    first_line(&output.stdout)
}

/// The first non-empty, trimmed line of captured stdout — the version-string
/// parsing split from the spawn so it is unit-testable without a real binary.
fn first_line(stdout: &[u8]) -> Option<String> {
    let text = String::from_utf8_lossy(stdout);
    let first = text.lines().next()?.trim();
    (!first.is_empty()).then(|| first.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lang(tag: &str) -> LanguageIdentifier {
        tag.parse().expect("test locale tag parses")
    }

    /// Unwrap a [`Resolution::Resolved`] to `(value, source)`, panicking on a
    /// `RejectedEnv` — a test asserting a clean setting expects no rejection.
    fn resolved(resolution: Resolution) -> (String, Source) {
        match resolution {
            Resolution::Resolved { value, source } => (value, source),
            Resolution::RejectedEnv { var, raw } => {
                panic!("expected a resolved setting, got RejectedEnv {var}={raw:?}")
            }
        }
    }

    // ---- resolve_value_enum: env > project > global > default, and the
    //      case-sensitive env that a bad case rejects (BUG 2) ----

    #[test]
    fn resolve_value_enum_prefers_env_over_every_lower_layer() {
        let (value, source) = resolved(resolve_value_enum(
            EnvSetting::Valid(Encoding::Utf8),
            "AOZORA_ENCODING",
            Some(Encoding::Sjis),
            Some(Encoding::Auto),
            Encoding::default(),
        ));
        assert_eq!(value, "utf8", "env wins the value");
        assert!(
            matches!(source, Source::Env("AOZORA_ENCODING")),
            "env is the reported source"
        );
    }

    #[test]
    fn resolve_value_enum_falls_to_project_then_global_then_default() {
        let (v, s) = resolved(resolve_value_enum(
            EnvSetting::Unset,
            "V",
            Some(Encoding::Sjis),
            Some(Encoding::Auto),
            Encoding::default(),
        ));
        assert_eq!(v, "sjis", "project wins when env is absent");
        assert!(matches!(s, Source::Project), "project source");

        let (v, s) = resolved(resolve_value_enum(
            EnvSetting::Unset,
            "V",
            None,
            Some(Encoding::Sjis),
            Encoding::default(),
        ));
        assert_eq!(v, "sjis", "global fills when project is unset");
        assert!(matches!(s, Source::Global), "global source");

        let (v, s) = resolved(resolve_value_enum::<Encoding>(
            EnvSetting::Unset,
            "V",
            None,
            None,
            Encoding::default(),
        ));
        assert_eq!(v, "auto", "default when nothing is set");
        assert!(matches!(s, Source::Default), "default source");
    }

    #[test]
    fn resolve_value_enum_rejects_a_bad_env_value_over_the_lower_layers() {
        // A set-but-invalid env is the runtime's exit-2 rejection, NOT a silent
        // fall-through to project/global — even with a valid project value.
        let resolution = resolve_value_enum(
            EnvSetting::<Encoding>::Invalid("SJIS".to_owned()),
            "AOZORA_ENCODING",
            Some(Encoding::Utf8),
            None,
            Encoding::default(),
        );
        assert!(
            matches!(resolution, Resolution::RejectedEnv { var: "AOZORA_ENCODING", ref raw } if raw == "SJIS"),
            "a bad-case env is rejected, not resolved to the project layer: {resolution:?}"
        );
    }

    // ---- resolve_strict: the boolean OR, NOT a layered override (BUG 1) ----

    #[test]
    fn resolve_strict_config_true_forces_on_over_a_false_env() {
        // BUG 1: AOZORA_STRICT=false + config strict=true — the runtime's
        // `false || true` is ON, attributed to the config layer that carried it.
        let (value, source) = resolved(resolve_strict(EnvSetting::Valid(false), Some(true), None));
        assert_eq!(
            value, "true",
            "config true forces strict on despite env false"
        );
        assert!(
            matches!(source, Source::Project),
            "sourced to the project config"
        );

        // The same when the true lives in the global layer.
        let (value, source) = resolved(resolve_strict(EnvSetting::Valid(false), None, Some(true)));
        assert_eq!(value, "true");
        assert!(
            matches!(source, Source::Global),
            "sourced to the global config"
        );
    }

    #[test]
    fn resolve_strict_env_true_wins_and_is_sourced_to_env() {
        let (value, source) = resolved(resolve_strict(EnvSetting::Valid(true), Some(false), None));
        assert_eq!(value, "true", "env true turns strict on");
        assert!(matches!(source, Source::Env("AOZORA_STRICT")), "env source");
    }

    #[test]
    fn resolve_strict_is_off_by_default_and_config_false_is_a_no_op() {
        // Nothing set: off, default.
        let (value, source) = resolved(resolve_strict(EnvSetting::Unset, None, None));
        assert_eq!(value, "false");
        assert!(matches!(source, Source::Default), "default-off");

        // A config `strict = false` cannot force anything in an OR — it is
        // indistinguishable from unset, so the source stays default.
        let (value, source) = resolved(resolve_strict(EnvSetting::Unset, Some(false), None));
        assert_eq!(value, "false");
        assert!(matches!(source, Source::Default), "config false is a no-op");

        // project false shadows a global true (ConfigFile::merge: project wins),
        // so the merged config is false and strict stays off.
        let (value, source) = resolved(resolve_strict(EnvSetting::Unset, Some(false), Some(true)));
        assert_eq!(value, "false", "project false shadows global true");
        assert!(matches!(source, Source::Default));
    }

    #[test]
    fn resolve_strict_rejects_a_non_bool_env() {
        // `on` is truthy to a shell but not to clap's bool parser (exit 2).
        let resolution = resolve_strict(EnvSetting::Invalid("on".to_owned()), None, None);
        assert!(
            matches!(resolution, Resolution::RejectedEnv { var: "AOZORA_STRICT", ref raw } if raw == "on"),
            "a non-`true`/`false` env is rejected: {resolution:?}"
        );
    }

    // ---- lang_source: mirrors crate::i18n::resolve's precedence ----

    #[test]
    fn lang_source_walks_the_full_precedence_chain() {
        assert!(
            matches!(
                lang_source(Some("ja"), Some("zh"), Some("en"), None, Some("ko")),
                Source::Flag
            ),
            "--lang wins outright"
        );
        assert!(
            matches!(
                lang_source(None, Some("zh"), Some("en"), None, Some("ko")),
                Source::Env("AOZORA_LANG")
            ),
            "AOZORA_LANG beats config + LANG"
        );
        assert!(
            matches!(
                lang_source(None, None, Some("en"), Some("zh"), Some("ko")),
                Source::Project
            ),
            "project config beats global + LANG"
        );
        assert!(
            matches!(
                lang_source(None, None, None, Some("zh"), Some("ko")),
                Source::Global
            ),
            "global config beats LANG"
        );
        assert!(
            matches!(
                lang_source(None, None, None, None, Some("ko")),
                Source::Env("LANG")
            ),
            "LANG is the last real source"
        );
        assert!(
            matches!(lang_source(None, None, None, None, None), Source::Default),
            "default when nothing decides"
        );
    }

    #[test]
    fn lang_source_treats_blank_as_absent() {
        // A present-but-blank higher source is skipped, matching resolve().
        assert!(
            matches!(
                lang_source(Some("  "), Some(""), None, None, Some("ja")),
                Source::Env("LANG")
            ),
            "blank flag / env fall through to LANG"
        );
    }

    #[test]
    fn present_is_true_only_for_non_blank() {
        assert!(present(Some("ja")), "a real value is present");
        assert!(!present(Some("   ")), "whitespace is not present");
        assert!(!present(Some("")), "empty is not present");
        assert!(!present(None), "None is not present");
    }

    // ---- colour resolution ----

    #[test]
    fn colour_source_walks_flag_then_project_then_global_then_default() {
        // Each layer in turn is the highest one set, so each must decide once.
        assert!(
            matches!(
                colour_source(
                    Some(ColorChoice::Always),
                    Some(ColorChoice::Never),
                    Some(ColorChoice::Never)
                ),
                Source::Flag
            ),
            "the flag outranks every layer beneath it"
        );
        assert!(
            matches!(
                colour_source(None, Some(ColorChoice::Never), Some(ColorChoice::Always)),
                Source::Project
            ),
            "the project .aozora.toml beats the global config"
        );
        assert!(
            matches!(
                colour_source(None, None, Some(ColorChoice::Always)),
                Source::Global
            ),
            "the global config decides when the project leaves colour unset"
        );
        assert!(
            matches!(colour_source(None, None, None), Source::Default),
            "nothing set -> the built-in default"
        );
    }

    #[test]
    fn colour_source_reads_the_layer_that_is_set_not_the_value_it_holds() {
        // An explicit `--color auto` is a real choice, not the default: it is
        // the flag layer deciding.
        assert!(
            matches!(
                colour_source(Some(ColorChoice::Auto), Some(ColorChoice::Never), None),
                Source::Flag
            ),
            "an explicit --color auto is the flag deciding, not the default"
        );
        // Likewise a config `color = "auto"` is the project layer deciding.
        assert!(
            matches!(
                colour_source(None, Some(ColorChoice::Auto), None),
                Source::Project
            ),
            "a config auto is the project deciding, not the default"
        );
    }

    #[test]
    fn colour_on_honours_explicit_always_and_never() {
        assert!(
            colour_on(ColorChoice::Always, false, true, Some("0"), None),
            "always ignores every signal"
        );
        assert!(
            !colour_on(ColorChoice::Never, true, false, None, Some("1")),
            "never ignores every signal"
        );
    }

    #[test]
    fn colour_on_auto_follows_the_env_then_tty_chain() {
        // CLICOLOR_FORCE (non-zero) forces on, over NO_COLOR / a non-tty.
        assert!(
            colour_on(ColorChoice::Auto, false, true, Some("0"), Some("1")),
            "CLICOLOR_FORCE wins"
        );
        // A zero force is not a force; NO_COLOR then decides.
        assert!(
            !colour_on(ColorChoice::Auto, true, true, None, Some("0")),
            "NO_COLOR off"
        );
        // CLICOLOR=0 disables when no force / no NO_COLOR.
        assert!(
            !colour_on(ColorChoice::Auto, true, false, Some("0"), None),
            "CLICOLOR=0 off"
        );
        // Nothing set: the stderr TTY decides.
        assert!(
            colour_on(ColorChoice::Auto, true, false, None, None),
            "tty -> on"
        );
        assert!(
            !colour_on(ColorChoice::Auto, false, false, None, None),
            "no tty -> off"
        );
    }

    // ---- small parsers / formatters ----

    #[test]
    fn classify_env_splits_unset_valid_and_invalid() {
        // The pure classifier over a fixed parser: absent, accepted, rejected.
        let parse = |raw: &str| (raw == "ok").then_some(7u8);
        assert!(matches!(classify_env(None, parse), EnvSetting::Unset));
        assert!(matches!(
            classify_env(Some("ok".to_owned()), parse),
            EnvSetting::Valid(7)
        ));
        assert!(
            matches!(classify_env(Some("no".to_owned()), parse), EnvSetting::Invalid(ref raw) if raw == "no"),
            "an unparsable value is kept verbatim as Invalid"
        );
    }

    #[test]
    fn value_enum_env_parsing_is_case_sensitive_like_clap() {
        // The exact clap parser doctor mirrors: `from_str(_, false)`. Lowercase
        // tags parse; any other case is rejected (the runtime's exit-2 path),
        // NOT silently accepted the way `ignore_case = true` used to.
        assert_eq!(Encoding::from_str("sjis", false).ok(), Some(Encoding::Sjis));
        assert!(
            Encoding::from_str("SJIS", false).is_err(),
            "uppercase is rejected"
        );
        assert!(
            Encoding::from_str("Sjis", false).is_err(),
            "mixed case is rejected"
        );
        assert!(
            matches!(DiagFormat::from_str("json", false), Ok(DiagFormat::Json)),
            "the lowercase tag parses"
        );
        assert!(
            DiagFormat::from_str("JSON", false).is_err(),
            "uppercase format is rejected"
        );
    }

    #[test]
    fn strict_env_parsing_matches_claps_bool_parser() {
        // clap's BoolValueParser is exactly `bool::from_str`: only the literal
        // `true` / `false`, case-sensitive, un-trimmed. The shell-ish spellings
        // the old parser accepted are all rejected by the runtime (exit 2).
        assert_eq!("true".parse::<bool>().ok(), Some(true));
        assert_eq!("false".parse::<bool>().ok(), Some(false));
        for rejected in [
            "on", "off", "1", "0", "yes", "no", "TRUE", "False", " true ",
        ] {
            assert!(
                rejected.parse::<bool>().is_err(),
                "{rejected:?} is not a clap bool"
            );
        }
    }

    #[test]
    fn enum_tag_reads_the_wire_tag() {
        assert_eq!(enum_tag(&Encoding::Auto), "auto");
        assert_eq!(enum_tag(&Encoding::Sjis), "sjis");
        assert_eq!(enum_tag(&ColorChoice::Never), "never");
        assert_eq!(enum_tag(&DiagFormat::Short), "short");
    }

    #[test]
    fn source_label_is_the_literal_precedence_word() {
        assert_eq!(Source::Flag.label(), "flag");
        assert_eq!(Source::Env("AOZORA_STRICT").label(), "env AOZORA_STRICT");
        assert_eq!(Source::Project.label(), "project");
        assert_eq!(Source::Global.label(), "global");
        assert_eq!(Source::Default.label(), "default");
    }

    #[test]
    fn env_state_localizes_set_and_unset() {
        assert_eq!(env_state(&lang("en"), None), "unset");
        assert_eq!(env_state(&lang("en"), Some("1")), "set (1)");
    }

    // ---- tool probing: PATH search + version-line parsing ----

    #[test]
    fn first_line_takes_the_first_non_empty_trimmed_line() {
        assert_eq!(
            first_line(b"pandoc 3.1.11\nCopyright ...\n"),
            Some("pandoc 3.1.11".to_owned()),
            "the first line is the version"
        );
        assert_eq!(
            first_line(b"  spaced  \nrest"),
            Some("spaced".to_owned()),
            "trimmed"
        );
        assert_eq!(first_line(b""), None, "empty output yields nothing");
        assert_eq!(
            first_line(b"   \n"),
            None,
            "a blank first line yields nothing"
        );
    }

    // ---- render: the full report is a pure function of the facts ----

    /// A clean settings row resolved to a `default`-source value — the shape
    /// every all-green row takes.
    fn green_row(label: &'static str, value: &str) -> SettingRow {
        SettingRow {
            label,
            resolution: Resolution::Resolved {
                value: value.to_owned(),
                source: Source::Default,
            },
        }
    }

    /// An all-green report: no config files, every setting defaulted, both
    /// tools missing, a piped (non-TTY) terminal with colour off.
    fn all_green() -> Doctor {
        Doctor {
            config: ConfigReport {
                cwd: PathBuf::from("/x"),
                outcome: Ok(ConfigPaths {
                    project: None,
                    global: None,
                }),
            },
            settings: vec![
                green_row("encoding", "auto"),
                green_row("format", "auto"),
                green_row("strict", "false"),
                green_row("color", "auto"),
                green_row("lang", "en"),
            ],
            pandoc: ToolStatus::Missing,
            terminal: TerminalReport {
                stdout_tty: false,
                stderr_tty: false,
                no_color: None,
                clicolor: None,
                colour_on: false,
            },
        }
    }

    #[test]
    fn render_all_green_is_exact_english_and_exits_zero() {
        let (report, blocking) = all_green().render(&lang("en"));
        assert_eq!(blocking, 0, "no blocking problems");
        assert_eq!(
            report,
            concat!(
                "aozora doctor — runtime self-check\n",
                "\n",
                "Configuration\n",
                "  project .aozora.toml   none (searched up from /x)\n",
                "  global config.toml     none\n",
                "  configuration parsed cleanly (no unknown keys)\n",
                "\n",
                "Effective settings\n",
                "  encoding   auto     default\n",
                "  format     auto     default\n",
                "  strict     false    default\n",
                "  color      auto     default\n",
                "  lang       en       default\n",
                "\n",
                "External tools\n",
                "  pandoc       not found on PATH\n",
                "    ↳ needed for `aozora pandoc -t FMT`; install from https://pandoc.org\n",
                "\n",
                "Terminal\n",
                "  stdout            not a terminal\n",
                "  stderr            not a terminal\n",
                "  NO_COLOR          unset\n",
                "  CLICOLOR          unset\n",
                "  effective colour  off\n",
                "\n",
                "All checks passed.\n",
            ),
        );
    }

    #[test]
    fn render_config_error_is_blocking_and_names_the_problem() {
        let mut doctor = all_green();
        doctor.config.outcome =
            Err("invalid config /x/.aozora.toml: unknown field `colour`".to_owned());
        let (report, blocking) = doctor.render(&lang("en"));
        assert_eq!(blocking, 1, "a malformed config is one blocking problem");
        assert!(
            report.contains(
                "configuration error: invalid config /x/.aozora.toml: unknown field `colour`"
            ),
            "the actionable loader message is surfaced: {report:?}"
        );
        assert!(
            report.contains("1 problem(s) found."),
            "the summary counts the blocking problem: {report:?}"
        );
        assert!(
            !report.contains("configuration parsed cleanly"),
            "no clean-parse line on the error path: {report:?}"
        );
    }

    #[test]
    fn render_rejected_env_setting_is_blocking_and_not_a_clean_row() {
        // A set-but-invalid env (here AOZORA_ENCODING=SJIS) is the runtime's
        // exit-2 rejection: doctor must surface it as a blocking problem, never
        // print it as a clean effective setting.
        let mut doctor = all_green();
        doctor.settings[0] = SettingRow {
            label: "encoding",
            resolution: Resolution::RejectedEnv {
                var: "AOZORA_ENCODING",
                raw: "SJIS".to_owned(),
            },
        };
        let (report, blocking) = doctor.render(&lang("en"));
        assert_eq!(blocking, 1, "a rejected env is one blocking problem");
        assert!(
            report.contains(
                "AOZORA_ENCODING=SJIS is set but not a valid value; aozora would reject it"
            ),
            "the rejection is surfaced actionably: {report:?}"
        );
        assert!(
            !report.contains("encoding   auto"),
            "no clean fall-through row for the rejected setting: {report:?}"
        );
        assert!(
            report.contains("1 problem(s) found."),
            "the summary counts the rejected env: {report:?}"
        );
    }

    #[test]
    fn render_shows_a_found_tool_detail() {
        let mut doctor = all_green();
        doctor.pandoc = ToolStatus::Found {
            detail: Some("pandoc 3.1.11".to_owned()),
        };
        let (report, _) = doctor.render(&lang("en"));
        assert!(
            report.contains("pandoc       pandoc 3.1.11"),
            "the found version is shown without a missing hint: {report:?}"
        );
        assert!(
            !report.contains("not found on PATH\n    ↳ needed for `aozora pandoc"),
            "no pandoc hint when it is present: {report:?}"
        );
    }

    #[test]
    fn render_localizes_headings_for_japanese() {
        // The prose axis follows --lang; a spot-check that ja headings differ
        // from en confirms the report routes through the catalog (the machine
        // tags / labels stay literal, verified by the exact en test above).
        let (report, _) = all_green().render(&lang("ja"));
        assert!(
            report.contains("設定"),
            "ja configuration heading: {report:?}"
        );
        assert!(
            report.contains("encoding"),
            "setting identifiers stay literal"
        );
    }
}
