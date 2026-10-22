use std::env;
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::Deserialize;
use tracing::debug;

use crate::diagnostics_render::DiagFormat;
use crate::{ColorChoice, Encoding};

const CONFIG_NAME: &str = ".aozora.toml";

/// Project or global defaults for CLI settings.
#[derive(Debug, Default, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
#[schemars(title = "AozoraConfig")]
pub(crate) struct ConfigFile {
    pub encoding: Option<Encoding>,
    pub format: Option<DiagFormat>,
    pub strict: Option<bool>,
    /// When to colourise diagnostics; `always` and `never` override terminal colour detection.
    pub color: Option<ColorChoice>,
    /// Human-message language as a BCP-47 tag; unknown locales fall back to English.
    pub lang: Option<String>,
}

#[derive(Debug, Default)]
pub(crate) struct Layers {
    pub project_path: Option<PathBuf>,
    pub project: ConfigFile,
    pub global_path: Option<PathBuf>,
    pub global: ConfigFile,
}

impl ConfigFile {
    pub(crate) fn resolve(explicit: Option<&Path>, cwd: &Path) -> Result<Self> {
        if let Some(path) = explicit {
            debug!(config = %path.display(), "config precedence: explicit --config (bypasses discovery + global)");
            return Self::load(path);
        }
        let layers = Self::layers(cwd)?;
        Ok(Self::merge(&layers.project, &layers.global))
    }

    pub(crate) fn layers(cwd: &Path) -> Result<Layers> {
        let project_path = discover(cwd);
        if let Some(path) = &project_path {
            debug!(config = %path.display(), "config precedence: nearest project .aozora.toml wins");
        } else {
            debug!("config precedence: no project .aozora.toml; defaults over global config.toml");
        }
        let project = match &project_path {
            Some(path) => Self::load(path)?,
            None => Self::default(),
        };
        let global_path = global_config_path().filter(|path| path.is_file());
        let global = match &global_path {
            Some(path) => Self::load(path)?,
            None => Self::default(),
        };
        Ok(Layers {
            project_path,
            project,
            global_path,
            global,
        })
    }

    fn merge(project: &Self, global: &Self) -> Self {
        Self {
            encoding: project.encoding.or(global.encoding),
            format: project.format.or(global.format),
            strict: project.strict.or(global.strict),
            color: project.color.or(global.color),
            lang: project.lang.clone().or_else(|| global.lang.clone()),
        }
    }

    fn load(path: &Path) -> Result<Self> {
        let text = fs::read_to_string(path)
            .with_context(|| format!("failed to read config {}", path.display()))?;
        toml::from_str(&text).with_context(|| format!("invalid config {}", path.display()))
    }
}

pub(crate) fn strict_active(flag_or_env: bool, config_strict: Option<bool>) -> bool {
    flag_or_env || config_strict.unwrap_or(false)
}

fn discover(start: &Path) -> Option<PathBuf> {
    start
        .ancestors()
        .map(|dir| dir.join(CONFIG_NAME))
        .find(|candidate| candidate.is_file())
}

fn global_config_path() -> Option<PathBuf> {
    global_config_path_from(env::var_os("XDG_CONFIG_HOME"), env::var_os("HOME"))
}

fn global_config_path_from(xdg: Option<OsString>, home: Option<OsString>) -> Option<PathBuf> {
    let base = match xdg {
        Some(dir) if !dir.is_empty() => PathBuf::from(dir),
        _ => PathBuf::from(home.filter(|home| !home.is_empty())?).join(".config"),
    };
    Some(base.join("aozora").join("config.toml"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn empty() -> ConfigFile {
        ConfigFile::default()
    }

    #[test]
    fn strict_active_is_the_boolean_or_of_flag_env_and_config() {
        assert!(strict_active(true, None), "flag/env true wins");
        assert!(
            strict_active(true, Some(false)),
            "flag/env true beats a config false"
        );
        assert!(
            strict_active(false, Some(true)),
            "config true forces strict on"
        );
        assert!(!strict_active(false, None), "nothing set -> off");
        assert!(
            !strict_active(false, Some(false)),
            "config false is a no-op -> off"
        );
    }

    #[test]
    fn merge_project_wins_each_field_it_sets() {
        let project = ConfigFile {
            strict: Some(true),
            color: Some(ColorChoice::Never),
            lang: Some("ja".to_owned()),
            ..empty()
        };
        let global = ConfigFile {
            encoding: Some(Encoding::Sjis),
            format: Some(DiagFormat::Json),
            strict: Some(false),
            color: Some(ColorChoice::Always),
            lang: Some("zh".to_owned()),
        };
        let merged = ConfigFile::merge(&project, &global);
        assert_eq!(merged.strict, Some(true));
        assert_eq!(merged.color, Some(ColorChoice::Never));
        assert_eq!(merged.lang.as_deref(), Some("ja"));
        assert_eq!(merged.encoding, Some(Encoding::Sjis));
        assert!(matches!(merged.format, Some(DiagFormat::Json)));
    }

    #[test]
    fn merge_global_fills_fields_project_leaves_unset() {
        let global = ConfigFile {
            strict: Some(true),
            color: Some(ColorChoice::Always),
            lang: Some("zh".to_owned()),
            ..empty()
        };
        let merged = ConfigFile::merge(&empty(), &global);
        assert_eq!(merged.strict, Some(true));
        assert_eq!(merged.color, Some(ColorChoice::Always));
        assert_eq!(merged.lang.as_deref(), Some("zh"));
    }

    #[test]
    fn merge_of_two_empties_stays_all_unset() {
        let merged = ConfigFile::merge(&empty(), &empty());
        assert_eq!(merged.strict, None);
        assert_eq!(merged.color, None);
        assert_eq!(merged.encoding, None);
        assert!(merged.format.is_none());
        assert_eq!(merged.lang, None);
    }

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
        let path = global_config_path_from(Some(OsString::new()), Some(OsString::from("/home/u")));
        assert_eq!(
            path,
            Some(PathBuf::from("/home/u/.config/aozora/config.toml"))
        );
    }

    #[test]
    fn global_path_absent_without_xdg_or_home() {
        assert_eq!(global_config_path_from(None, None), None);
        assert_eq!(global_config_path_from(None, Some(OsString::new())), None);
    }
}
