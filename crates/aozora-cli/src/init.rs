#![expect(
    clippy::expect_used,
    reason = "the embedded project template is a valid static document"
)]

use std::fmt;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use crate::i18n::{self as i18n, LanguageIdentifier};
use crate::{atomic_write, output};
use anyhow::{Context, Result};
use clap::Parser;

const CONFIG_NAME: &str = ".aozora.toml";
const SAMPLE_NAME: &str = "hon.aozora";
const GITIGNORE_NAME: &str = ".gitignore";

const CONFIG_TEMPLATE: &str = r#"# .aozora.toml — project configuration for the aozora CLI.
#
# Settings resolve highest-priority first:
#   flag > env > this file > global (~/.config/aozora/config.toml) > default
# Every key below is optional and shown at its default; uncomment one to
# override it. Unknown keys are rejected, so a typo fails loudly rather than
# being silently ignored.

# Source encoding used to read documents.
#   "auto" (default) — UTF-8 when the bytes are valid UTF-8, else Shift_JIS
#   "utf8" | "sjis"  — force one decoder
# encoding = "auto"

# How `check` and `lint` render diagnostics.
#   "auto" (default) — human on a terminal, json when piped
#   "human" | "json" | "short"
# format = "auto"

# Treat any diagnostic (or lint) as a failure and exit non-zero. This is a
# boolean OR with the --strict flag / AOZORA_STRICT, so `true` forces strict
# on; `false` is indistinguishable from leaving it unset.
# strict = false

# When to colourise diagnostics and `fmt --diff`. Outranked by --color.
#   "auto" (default) — colour on a terminal, honouring NO_COLOR / CLICOLOR
#   "always" | "never" — a decided choice, so it outranks NO_COLOR / CLICOLOR
#                        exactly as `--color always` / `--color never` do
# color = "auto"

# Language for human messages only (never affects machine output or encoding).
#   A BCP-47 tag: "en" (default), "ja", or "zh". Outranked by --lang /
#   AOZORA_LANG.
# lang = "en"
"#;

const SAMPLE_DOC: &str = "青空文庫記法サンプル\n\
\n\
｜青梅《おうめ》の宿場を朝早くに発った。\n\
\n\
［＃ここから２字下げ］\n\
これは字下げされた一節です。\n\
記法を試すときの手本にしてください。\n\
［＃ここで字下げ終わり］\n\
\n\
ここが肝心な点だ。［＃「肝心」に傍点］\n";

const GITIGNORE: &str = "# Rendered HTML output (from `aozora render`)\n\
/out/\n\
*.html\n\
\n\
# Editor / OS cruft\n\
.DS_Store\n\
Thumbs.db\n\
*~\n\
.*.swp\n";

/// `aozora init [DIR]` — scaffold a project directory.
#[derive(Debug, Parser)]
#[command(after_long_help = "Examples:
  aozora init                 # scaffold the current directory
  aozora init myproject       # scaffold ./myproject (created if absent)
  aozora init --no-sample     # config + .gitignore only
  aozora init --force         # overwrite files that already exist")]
pub(crate) struct InitArgs {
    /// Directory to scaffold into (created if it does not exist).
    /// Defaults to the current directory.
    #[arg(default_value = ".", value_name = "DIR")]
    dir: PathBuf,

    /// Overwrite files that already exist.
    /// Without it, an existing file is kept untouched — never silently clobbered — and reported as skipped.
    #[arg(long)]
    force: bool,

    /// Skip the sample `hon.aozora` document.
    #[arg(long)]
    no_sample: bool,

    /// Skip the `.gitignore`.
    #[arg(long)]
    no_gitignore: bool,
}

pub(crate) fn run(args: &InitArgs, lang: &LanguageIdentifier) -> Result<ExitCode> {
    fs::create_dir_all(&args.dir)
        .with_context(|| format!("failed to create directory {}", args.dir.display()))?;

    let mut results = Vec::new();
    for (name, contents) in scaffold_files(args) {
        let outcome = write_scaffold(&args.dir.join(name), contents, args.force)?;
        results.push(FileResult { name, outcome });
    }

    let report = render_report(&results, !args.no_sample, lang);
    output::stdout()
        .write_all(report.as_bytes())
        .context("failed to write the init report to stdout")?;
    Ok(ExitCode::SUCCESS)
}

fn scaffold_files(args: &InitArgs) -> Vec<(&'static str, &'static str)> {
    let mut files = vec![(CONFIG_NAME, CONFIG_TEMPLATE)];
    if !args.no_sample {
        files.push((SAMPLE_NAME, SAMPLE_DOC));
    }
    if !args.no_gitignore {
        files.push((GITIGNORE_NAME, GITIGNORE));
    }
    files
}

fn write_scaffold(path: &Path, contents: &str, force: bool) -> Result<Outcome> {
    let exists = path
        .try_exists()
        .with_context(|| format!("failed to inspect {}", path.display()))?;
    if exists && !force {
        return Ok(Outcome::Skipped);
    }
    atomic_write::replace(path, contents.as_bytes())
        .with_context(|| format!("failed to write {}", path.display()))?;
    Ok(if exists {
        Outcome::Overwritten
    } else {
        Outcome::Created
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Outcome {
    Created,
    Overwritten,
    Skipped,
}

impl Outcome {
    fn key(self) -> &'static str {
        match self {
            Self::Created => "init-created",
            Self::Overwritten => "init-overwritten",
            Self::Skipped => "init-skipped",
        }
    }
}

#[derive(Debug)]
struct FileResult {
    name: &'static str,
    outcome: Outcome,
}

fn render_report(results: &[FileResult], sample: bool, lang: &LanguageIdentifier) -> String {
    let mut out = String::new();
    write_report(&mut out, results, sample, lang).expect("writing to a String is infallible");
    out
}

fn write_report(
    out: &mut String,
    results: &[FileResult],
    sample: bool,
    lang: &LanguageIdentifier,
) -> fmt::Result {
    use std::fmt::Write as _;

    writeln!(out, "{}", i18n::t(lang, "init-heading"))?;
    writeln!(out)?;
    for result in results {
        let word = i18n::t(lang, result.outcome.key());
        if result.outcome == Outcome::Skipped {
            let hint = i18n::t(lang, "init-skipped-hint");
            writeln!(out, "  {:<16} {word} ({hint})", result.name)?;
        } else {
            writeln!(out, "  {:<16} {word}", result.name)?;
        }
    }

    writeln!(out)?;
    writeln!(out, "{}", i18n::t(lang, "init-next-steps"))?;
    if sample {
        step(
            out,
            &format!("aozora render {SAMPLE_NAME}"),
            lang,
            "init-step-render",
        )?;
        step(
            out,
            &format!("aozora check {SAMPLE_NAME}"),
            lang,
            "init-step-check",
        )?;
    }
    step(out, "aozora doctor", lang, "init-step-doctor")?;
    Ok(())
}

fn step(out: &mut String, command: &str, lang: &LanguageIdentifier, key: &str) -> fmt::Result {
    use std::fmt::Write as _;
    writeln!(out, "  {command:<27}# {}", i18n::t(lang, key))
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::*;
    use crate::config::ConfigFile;

    fn lang(tag: &str) -> LanguageIdentifier {
        tag.parse().expect("test locale tag parses")
    }

    fn init_args(dir: &Path) -> InitArgs {
        InitArgs {
            dir: dir.to_owned(),
            force: false,
            no_sample: false,
            no_gitignore: false,
        }
    }

    #[test]
    fn config_template_covers_every_config_field() {
        let fields = config_field_names();
        assert!(
            fields.len() >= 5,
            "expected serde to enumerate the config fields, got {fields:?}"
        );
        for field in &fields {
            assert!(
                CONFIG_TEMPLATE.contains(field.as_str()),
                "the .aozora.toml template does not mention the `{field}` config key — \
                 add a documented line for it so the scaffold stays in sync with ConfigFile",
            );
        }
    }

    fn config_field_names() -> Vec<String> {
        let error = toml::from_str::<ConfigFile>("this-is-not-a-config-key = true")
            .expect_err("an unknown key must be rejected by deny_unknown_fields");
        error
            .to_string()
            .split('`')
            .skip(1)
            .step_by(2)
            .skip(1)
            .map(str::to_owned)
            .collect()
    }

    #[test]
    fn write_scaffold_creates_an_absent_file() {
        let dir = TempDir::new().expect("tempdir");
        let path = dir.path().join("new.toml");
        let outcome = write_scaffold(&path, "hello\n", false).expect("write");
        assert_eq!(outcome, Outcome::Created);
        assert_eq!(fs::read_to_string(&path).expect("read back"), "hello\n");
    }

    #[test]
    fn write_scaffold_skips_an_existing_file_without_force() {
        let dir = TempDir::new().expect("tempdir");
        let path = dir.path().join("keep.toml");
        fs::write(&path, "original\n").expect("seed");
        let outcome = write_scaffold(&path, "replacement\n", false).expect("write");
        assert_eq!(outcome, Outcome::Skipped);
        assert_eq!(fs::read_to_string(&path).expect("read back"), "original\n");
    }

    #[test]
    fn write_scaffold_overwrites_an_existing_file_with_force() {
        let dir = TempDir::new().expect("tempdir");
        let path = dir.path().join("clobber.toml");
        fs::write(&path, "original\n").expect("seed");
        let outcome = write_scaffold(&path, "replacement\n", true).expect("write");
        assert_eq!(outcome, Outcome::Overwritten);
        assert_eq!(
            fs::read_to_string(&path).expect("read back"),
            "replacement\n"
        );
    }

    #[test]
    fn scaffold_files_defaults_to_all_three() {
        let dir = TempDir::new().expect("tempdir");
        let names: Vec<_> = scaffold_files(&init_args(dir.path()))
            .into_iter()
            .map(|(name, _)| name)
            .collect();
        assert_eq!(names, [CONFIG_NAME, SAMPLE_NAME, GITIGNORE_NAME]);
    }

    #[test]
    fn scaffold_files_honours_no_sample_and_no_gitignore() {
        let dir = TempDir::new().expect("tempdir");
        let no_sample = scaffold_files(&InitArgs {
            no_sample: true,
            ..init_args(dir.path())
        });
        assert_eq!(
            no_sample.iter().map(|(n, _)| *n).collect::<Vec<_>>(),
            [CONFIG_NAME, GITIGNORE_NAME],
            "--no-sample drops the sample document",
        );
        let bare = scaffold_files(&InitArgs {
            no_sample: true,
            no_gitignore: true,
            ..init_args(dir.path())
        });
        assert_eq!(
            bare.iter().map(|(n, _)| *n).collect::<Vec<_>>(),
            [CONFIG_NAME],
            "both opt-outs leave only the config template",
        );
    }

    #[test]
    fn outcome_key_maps_each_variant() {
        assert_eq!(Outcome::Created.key(), "init-created");
        assert_eq!(Outcome::Overwritten.key(), "init-overwritten");
        assert_eq!(Outcome::Skipped.key(), "init-skipped");
    }

    fn all_created() -> Vec<FileResult> {
        vec![
            FileResult {
                name: CONFIG_NAME,
                outcome: Outcome::Created,
            },
            FileResult {
                name: SAMPLE_NAME,
                outcome: Outcome::Created,
            },
            FileResult {
                name: GITIGNORE_NAME,
                outcome: Outcome::Created,
            },
        ]
    }

    #[test]
    fn render_report_all_created_is_exact_english() {
        let report = render_report(&all_created(), true, &lang("en"));
        assert_eq!(
            report,
            concat!(
                "aozora init — scaffold a project\n",
                "\n",
                "  .aozora.toml     created\n",
                "  hon.aozora       created\n",
                "  .gitignore       created\n",
                "\n",
                "Next steps:\n",
                "  aozora render hon.aozora   # render the sample to HTML\n",
                "  aozora check hon.aozora    # report diagnostics\n",
                "  aozora doctor              # verify the effective configuration\n",
            ),
        );
    }

    #[test]
    fn render_report_skipped_row_carries_the_force_hint() {
        let results = vec![FileResult {
            name: CONFIG_NAME,
            outcome: Outcome::Skipped,
        }];
        let report = render_report(&results, false, &lang("en"));
        assert!(
            report.contains(".aozora.toml     skipped (already exists; use --force to overwrite)"),
            "the skipped row names the file and the --force escape hatch: {report:?}",
        );
    }

    #[test]
    fn render_report_omits_sample_steps_without_a_sample() {
        let results = vec![FileResult {
            name: CONFIG_NAME,
            outcome: Outcome::Created,
        }];
        let report = render_report(&results, false, &lang("en"));
        assert!(
            !report.contains("hon.aozora"),
            "no sample steps when the sample is absent: {report:?}",
        );
        assert!(
            report.contains("aozora doctor"),
            "the doctor next-step is always shown: {report:?}",
        );
    }

    #[test]
    fn render_report_localizes_chrome_for_japanese() {
        let report = render_report(&all_created(), true, &lang("ja"));
        assert!(report.contains("作成"), "ja created word: {report:?}");
        assert!(
            report.contains(".aozora.toml"),
            "file names stay literal: {report:?}",
        );
    }

    #[test]
    fn config_template_parses_and_is_all_default() {
        let cfg: ConfigFile =
            toml::from_str(CONFIG_TEMPLATE).expect("the scaffolded .aozora.toml is valid TOML");
        assert!(cfg.encoding.is_none(), "encoding stays defaulted");
        assert!(cfg.format.is_none(), "format stays defaulted");
        assert!(cfg.strict.is_none(), "strict stays defaulted");
        assert!(cfg.color.is_none(), "color stays defaulted");
        assert!(cfg.lang.is_none(), "lang stays defaulted");
    }
}
