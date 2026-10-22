#![expect(
    clippy::expect_used,
    reason = "doctor constructs a valid static version requirement"
)]

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
use crate::output;
use crate::which::which;
use crate::{ColorChoice, Encoding};

#[derive(Debug, Clone, Copy)]
pub(crate) struct ColorFacts {
    pub(crate) flag: Option<ColorChoice>,
    pub(crate) resolved: ColorChoice,
}

pub(crate) fn run(
    color: ColorFacts,
    lang_flag: Option<&str>,
    lang: &LanguageIdentifier,
) -> Result<ExitCode> {
    let doctor = Doctor::gather(color, lang_flag, lang);
    let (report, blocking) = doctor.render(lang);
    let mut stdout = output::stdout();
    stdout
        .write_all(report.as_bytes())
        .context("failed to write the doctor report to stdout")?;
    Ok(if blocking == 0 {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(1)
    })
}

#[derive(Debug, Clone, Copy)]
enum Source {
    Flag,
    Env(&'static str),
    Project,
    Global,
    Default,
}

impl Source {
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

#[derive(Debug)]
struct SettingRow {
    label: &'static str,
    resolution: Resolution,
}

#[derive(Debug)]
enum Resolution {
    Resolved { value: String, source: Source },
    RejectedEnv { var: &'static str, raw: String },
}

#[derive(Debug)]
enum EnvSetting<T> {
    Unset,
    Valid(T),
    Invalid(String),
}

#[derive(Debug)]
struct ConfigReport {
    cwd: PathBuf,
    outcome: Result<ConfigPaths, String>,
}

#[derive(Debug)]
struct ConfigPaths {
    project: Option<PathBuf>,
    global: Option<PathBuf>,
}

#[derive(Debug)]
enum ToolStatus {
    Missing,
    Found { detail: Option<String> },
}

#[derive(Debug)]
struct TerminalReport {
    stdout_tty: bool,
    stderr_tty: bool,
    no_color: Option<String>,
    clicolor: Option<String>,
    colour_on: bool,
}

#[derive(Debug)]
struct Doctor {
    config: ConfigReport,
    settings: Vec<SettingRow>,
    pandoc: ToolStatus,
    terminal: TerminalReport,
}

impl Doctor {
    fn gather(color: ColorFacts, lang_flag: Option<&str>, lang: &LanguageIdentifier) -> Self {
        let cwd = env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        let (config, layers) = load_config(&cwd);
        let settings = resolve_settings(&layers, color, lang_flag, lang);
        let terminal = terminal_report(color.resolved);
        Self {
            config,
            settings,
            pandoc: probe_pandoc(),
            terminal,
        }
    }

    fn render(&self, lang: &LanguageIdentifier) -> (String, usize) {
        let mut out = String::new();
        let blocking = self
            .write_report(&mut out, lang)
            .expect("writing to a String is infallible");
        (out, blocking)
    }

    fn write_report(
        &self,
        out: &mut String,
        lang: &LanguageIdentifier,
    ) -> Result<usize, fmt::Error> {
        use std::fmt::Write as _;
        let mut blocking = 0usize;

        writeln!(out, "{}", i18n::t(lang, "doctor-title"))?;

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

        blocking += write_settings(out, lang, &self.settings)?;

        writeln!(out, "\n{}", i18n::t(lang, "doctor-tools-heading"))?;
        render_tool(out, lang, "pandoc", &self.pandoc, "doctor-hint-pandoc")?;

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

fn layer<T: Copy>(project: Option<T>, global: Option<T>, default: T) -> (T, Source) {
    project
        .map(|value| (value, Source::Project))
        .or_else(|| global.map(|value| (value, Source::Global)))
        .unwrap_or((default, Source::Default))
}

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
    let cfg_strict = project.or(global);
    let effective = strict_active(flag_or_env, cfg_strict);
    let source = if flag_or_env {
        Source::Env("AOZORA_STRICT")
    } else if cfg_strict == Some(true) {
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

fn present(source: Option<&str>) -> bool {
    source.is_some_and(|value| !value.trim().is_empty())
}

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

fn classify_env<T>(raw: Option<String>, parse: impl Fn(&str) -> Option<T>) -> EnvSetting<T> {
    let Some(value) = raw else {
        return EnvSetting::Unset;
    };
    parse(&value).map_or(EnvSetting::Invalid(value), EnvSetting::Valid)
}

fn env_value_enum<T: ValueEnum>(var: &str) -> EnvSetting<T> {
    classify_env(env::var(var).ok(), |raw| T::from_str(raw, false).ok())
}

fn env_bool(var: &str) -> EnvSetting<bool> {
    classify_env(env::var(var).ok(), |raw| raw.parse::<bool>().ok())
}

fn enum_tag<T: ValueEnum>(value: &T) -> String {
    value
        .to_possible_value()
        .map_or_else(|| "?".to_owned(), |possible| possible.get_name().to_owned())
}

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

fn probe_pandoc() -> ToolStatus {
    which("pandoc").map_or(ToolStatus::Missing, |path| ToolStatus::Found {
        detail: pandoc_version(&path),
    })
}

fn pandoc_version(path: &Path) -> Option<String> {
    let output = Command::new(path).arg("--version").output().ok()?;
    first_line(&output.stdout)
}

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

    fn resolved(resolution: Resolution) -> (String, Source) {
        match resolution {
            Resolution::Resolved { value, source } => (value, source),
            Resolution::RejectedEnv { var, raw } => {
                panic!("expected a resolved setting, got RejectedEnv {var}={raw:?}")
            }
        }
    }

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

    #[test]
    fn resolve_strict_config_true_forces_on_over_a_false_env() {
        let (value, source) = resolved(resolve_strict(EnvSetting::Valid(false), Some(true), None));
        assert_eq!(
            value, "true",
            "config true forces strict on despite env false"
        );
        assert!(
            matches!(source, Source::Project),
            "sourced to the project config"
        );

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
        let (value, source) = resolved(resolve_strict(EnvSetting::Unset, None, None));
        assert_eq!(value, "false");
        assert!(matches!(source, Source::Default), "default-off");

        let (value, source) = resolved(resolve_strict(EnvSetting::Unset, Some(false), None));
        assert_eq!(value, "false");
        assert!(matches!(source, Source::Default), "config false is a no-op");

        let (value, source) = resolved(resolve_strict(EnvSetting::Unset, Some(false), Some(true)));
        assert_eq!(value, "false", "project false shadows global true");
        assert!(matches!(source, Source::Default));
    }

    #[test]
    fn resolve_strict_rejects_a_non_bool_env() {
        let resolution = resolve_strict(EnvSetting::Invalid("on".to_owned()), None, None);
        assert!(
            matches!(resolution, Resolution::RejectedEnv { var: "AOZORA_STRICT", ref raw } if raw == "on"),
            "a non-`true`/`false` env is rejected: {resolution:?}"
        );
    }

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

    #[test]
    fn colour_source_walks_flag_then_project_then_global_then_default() {
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
        assert!(
            matches!(
                colour_source(Some(ColorChoice::Auto), Some(ColorChoice::Never), None),
                Source::Flag
            ),
            "an explicit --color auto is the flag deciding, not the default"
        );
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
        assert!(
            colour_on(ColorChoice::Auto, false, true, Some("0"), Some("1")),
            "CLICOLOR_FORCE wins"
        );
        assert!(
            !colour_on(ColorChoice::Auto, true, true, None, Some("0")),
            "NO_COLOR off"
        );
        assert!(
            !colour_on(ColorChoice::Auto, true, false, Some("0"), None),
            "CLICOLOR=0 off"
        );
        assert!(
            colour_on(ColorChoice::Auto, true, false, None, None),
            "tty -> on"
        );
        assert!(
            !colour_on(ColorChoice::Auto, false, false, None, None),
            "no tty -> off"
        );
    }

    #[test]
    fn classify_env_splits_unset_valid_and_invalid() {
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

    fn green_row(label: &'static str, value: &str) -> SettingRow {
        SettingRow {
            label,
            resolution: Resolution::Resolved {
                value: value.to_owned(),
                source: Source::Default,
            },
        }
    }

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
