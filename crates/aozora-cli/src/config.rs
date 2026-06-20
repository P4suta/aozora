//! `.aozora.toml` discovery and loading.
//!
//! Settings resolve **flag > env > config > default**: clap handles the
//! flag-and-env half (the document flags carry `env = "AOZORA_*"`), and
//! the caller folds in the config value with `Option::or` before falling
//! back to the built-in default. This module owns only the file half:
//! finding the nearest `.aozora.toml` and deserializing it.
//!
//! `serde` + `toml` only — no new external crate (ADR-0013). Unknown
//! keys are rejected (`deny_unknown_fields`) so a mistyped setting fails
//! loudly instead of being silently ignored.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::Deserialize;

use crate::Encoding;
use crate::diagnostics_render::DiagFormat;

/// The file searched for, walking up from the working directory.
const CONFIG_NAME: &str = ".aozora.toml";

/// A deserialized `.aozora.toml`. Every field is optional: a present
/// value becomes the default for that setting, still overridable by a
/// flag or environment variable.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
pub(crate) struct ConfigFile {
    pub encoding: Option<Encoding>,
    pub diagnostic_format: Option<DiagFormat>,
    pub strict: Option<bool>,
}

impl ConfigFile {
    /// Resolve the effective config: an explicit `--config PATH` (a hard
    /// error if unreadable or malformed), else the nearest `.aozora.toml`
    /// walking up from `cwd`, else the all-default config.
    pub(crate) fn resolve(explicit: Option<&Path>, cwd: &Path) -> Result<Self> {
        if let Some(path) = explicit {
            return Self::load(path);
        }
        discover(cwd).map_or_else(|| Ok(Self::default()), |path| Self::load(&path))
    }

    fn load(path: &Path) -> Result<Self> {
        let text = fs::read_to_string(path)
            .with_context(|| format!("failed to read config {}", path.display()))?;
        toml::from_str(&text).with_context(|| format!("invalid config {}", path.display()))
    }
}

/// Walk up from `start` to the filesystem root, returning the first
/// directory that holds a `.aozora.toml`. The git / rustfmt idiom:
/// project-local config without having to locate the repo root.
fn discover(start: &Path) -> Option<PathBuf> {
    start
        .ancestors()
        .map(|dir| dir.join(CONFIG_NAME))
        .find(|candidate| candidate.is_file())
}
