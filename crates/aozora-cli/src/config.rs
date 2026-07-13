//! `.aozora.toml` discovery and loading.
//!
//! Settings resolve **flag > env > project > global > default**: clap
//! handles the flag-and-env half (the document flags carry `env =
//! "AOZORA_*"`), and the caller folds in the config value with `Option::or`
//! before falling back to the built-in default. This module owns the file
//! half — two layers and how they compose:
//!
//! - **project** — the nearest `.aozora.toml`, found by walking up from the
//!   working directory (the git / rustfmt idiom: project-local config
//!   without having to locate the repo root).
//! - **global** — `$XDG_CONFIG_HOME/aozora/config.toml`, or
//!   `$HOME/.config/aozora/config.toml` when `XDG_CONFIG_HOME` is unset or
//!   empty (the XDG Base Directory default): a user-wide baseline beneath
//!   the project layer.
//!
//! [`ConfigFile::merge`] overlays the two field-wise — every setting the
//! project file sets wins, and anything it leaves unset falls through to the
//! global file. An explicit `--config PATH` is a full escape hatch: it
//! bypasses BOTH the upward search and the global layer, loading that one
//! file alone.
//!
//! `serde` + `toml` only — no new external crate (ADR-0013); XDG is resolved
//! by reading the two environment variables by hand. Unknown keys are
//! rejected (`deny_unknown_fields`) so a mistyped setting fails loudly
//! instead of being silently ignored.

use std::env;
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::Deserialize;
use tracing::debug;

use crate::diagnostics_render::DiagFormat;
use crate::{ColorChoice, Encoding};

/// The project file searched for, walking up from the working directory.
const CONFIG_NAME: &str = ".aozora.toml";

/// A deserialized config file — a project `.aozora.toml` or the global
/// `config.toml`. Every field is optional: a present value becomes the
/// default for that setting, still overridable by a higher-precedence layer
/// (project over global) or by a flag / environment variable.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
pub(crate) struct ConfigFile {
    pub encoding: Option<Encoding>,
    pub format: Option<DiagFormat>,
    pub strict: Option<bool>,
    pub color: Option<ColorChoice>,
}

impl ConfigFile {
    /// Resolve the effective config. An explicit `--config PATH` (a hard
    /// error if unreadable or malformed) is a full escape hatch — used
    /// alone, bypassing both discovery layers. Otherwise the nearest
    /// `.aozora.toml` (project) is overlaid field-wise on the XDG
    /// `config.toml` (global), project winning; an absent layer contributes
    /// an all-default config.
    pub(crate) fn resolve(explicit: Option<&Path>, cwd: &Path) -> Result<Self> {
        if let Some(path) = explicit {
            debug!(config = %path.display(), "config precedence: explicit --config (bypasses discovery + global)");
            return Self::load(path);
        }
        let discovered = discover(cwd);
        if let Some(path) = &discovered {
            debug!(config = %path.display(), "config precedence: nearest project .aozora.toml wins");
        } else {
            debug!("config precedence: no project .aozora.toml; defaults over global config.toml");
        }
        let project = discovered.map_or_else(|| Ok(Self::default()), |path| Self::load(&path))?;
        let global = Self::load_global()?;
        Ok(Self::merge(&project, &global))
    }

    /// Field-wise overlay: every setting present in `project` wins; anything
    /// it leaves unset falls through to `global`. Every field is `Copy`, so
    /// the all-`Option` shape reduces the merge to a per-field
    /// [`Option::or`], and the two layers compose without either clobbering
    /// the other's unrelated keys.
    fn merge(project: &Self, global: &Self) -> Self {
        Self {
            encoding: project.encoding.or(global.encoding),
            format: project.format.or(global.format),
            strict: project.strict.or(global.strict),
            color: project.color.or(global.color),
        }
    }

    /// Load the global config when its file exists, else an all-default
    /// config. A malformed global file is a hard error, exactly like the
    /// project file.
    fn load_global() -> Result<Self> {
        match global_config_path() {
            Some(path) if path.is_file() => Self::load(&path),
            _ => Ok(Self::default()),
        }
    }

    fn load(path: &Path) -> Result<Self> {
        let text = fs::read_to_string(path)
            .with_context(|| format!("failed to read config {}", path.display()))?;
        toml::from_str(&text).with_context(|| format!("invalid config {}", path.display()))
    }
}

/// Walk up from `start` to the filesystem root, returning the first
/// directory that holds a `.aozora.toml`.
fn discover(start: &Path) -> Option<PathBuf> {
    start
        .ancestors()
        .map(|dir| dir.join(CONFIG_NAME))
        .find(|candidate| candidate.is_file())
}

/// The global config path, reading the environment. Split from its pure
/// [`global_config_path_from`] seam so the XDG-over-HOME precedence stays
/// unit-testable without mutating the process environment.
fn global_config_path() -> Option<PathBuf> {
    global_config_path_from(env::var_os("XDG_CONFIG_HOME"), env::var_os("HOME"))
}

/// Derive the global config path from the two raw env values, following the
/// XDG Base Directory spec: `$XDG_CONFIG_HOME/aozora/config.toml`, or
/// `$HOME/.config/aozora/config.toml` when `XDG_CONFIG_HOME` is unset or
/// empty. `None` when neither variable yields a base directory (a stripped
/// environment) — the global layer is then simply absent. Resolved by hand
/// to avoid a `dirs`-style dependency (ADR-0013).
fn global_config_path_from(xdg: Option<OsString>, home: Option<OsString>) -> Option<PathBuf> {
    let base = match xdg {
        // A set-but-empty `XDG_CONFIG_HOME` is not an absolute path, so per
        // the spec it is treated as unset: fall through to the HOME default.
        Some(dir) if !dir.is_empty() => PathBuf::from(dir),
        _ => PathBuf::from(home.filter(|home| !home.is_empty())?).join(".config"),
    };
    Some(base.join("aozora").join("config.toml"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A `ConfigFile` with every field unset — the neutral base each case
    /// tweaks with `..`.
    fn empty() -> ConfigFile {
        ConfigFile::default()
    }

    // --- merge: the project-over-global field-wise overlay ---

    #[test]
    fn merge_project_wins_each_field_it_sets() {
        let project = ConfigFile {
            strict: Some(true),
            color: Some(ColorChoice::Never),
            ..empty()
        };
        let global = ConfigFile {
            encoding: Some(Encoding::Sjis),
            format: Some(DiagFormat::Json),
            strict: Some(false),
            color: Some(ColorChoice::Always),
        };
        let merged = ConfigFile::merge(&project, &global);
        // Project's set fields win outright...
        assert_eq!(merged.strict, Some(true));
        assert_eq!(merged.color, Some(ColorChoice::Never));
        // ...and the fields it left unset fall through to global — per-field,
        // not all-or-nothing: a whole-`project` return would drop these two.
        assert_eq!(merged.encoding, Some(Encoding::Sjis));
        assert!(matches!(merged.format, Some(DiagFormat::Json)));
    }

    #[test]
    fn merge_global_fills_fields_project_leaves_unset() {
        let global = ConfigFile {
            strict: Some(true),
            color: Some(ColorChoice::Always),
            ..empty()
        };
        let merged = ConfigFile::merge(&empty(), &global);
        assert_eq!(merged.strict, Some(true));
        assert_eq!(merged.color, Some(ColorChoice::Always));
    }

    #[test]
    fn merge_of_two_empties_stays_all_unset() {
        let merged = ConfigFile::merge(&empty(), &empty());
        assert_eq!(merged.strict, None);
        assert_eq!(merged.color, None);
        assert_eq!(merged.encoding, None);
        assert!(merged.format.is_none());
    }

    // --- global_config_path_from: the XDG-over-HOME precedence seam ---

    #[test]
    fn global_path_prefers_xdg_when_set() {
        let path = global_config_path_from(
            Some(OsString::from("/cfg")),
            Some(OsString::from("/home/u")),
        );
        assert_eq!(path, Some(PathBuf::from("/cfg/aozora/config.toml")));
    }

    #[test]
    fn global_path_falls_back_to_home_dot_config_when_xdg_unset() {
        let path = global_config_path_from(None, Some(OsString::from("/home/u")));
        assert_eq!(
            path,
            Some(PathBuf::from("/home/u/.config/aozora/config.toml"))
        );
    }

    #[test]
    fn global_path_treats_empty_xdg_as_unset() {
        // A set-but-empty XDG_CONFIG_HOME is ignored; the HOME default wins.
        let path = global_config_path_from(Some(OsString::new()), Some(OsString::from("/home/u")));
        assert_eq!(
            path,
            Some(PathBuf::from("/home/u/.config/aozora/config.toml"))
        );
    }

    #[test]
    fn global_path_absent_without_xdg_or_home() {
        assert_eq!(global_config_path_from(None, None), None);
        // An empty HOME is likewise no base directory.
        assert_eq!(global_config_path_from(None, Some(OsString::new())), None);
    }
}
