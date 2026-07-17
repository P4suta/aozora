//! Resolve the channel-aware build version once, at compile time, and inject it
//! via `rustc-env` so the `PyO3` `version()` export can read it via
//! `env!("AOZORA_VERSION_STRING")`.
//!
//! Precedence (the aozora three-way fallback):
//!   1. `AOZORA_BUILD_VERSION` (CI-authoritative — `nightly.yml` / `release.yml`
//!      set it verbatim from `xtask version --channel …`).
//!   2. In a workspace checkout (the repo `.git` exists) →
//!      `{CARGO_PKG_VERSION}-dev+g{sha}[.dirty]`, so a hand-built wheel can never
//!      be mistaken for a release. The sha is best-effort: if the `git` binary is
//!      absent (e.g. a slim container) the stamp degrades to a bare `-dev`.
//!   3. Otherwise — a packaged build with no `.git` (a `PyPI` sdist / wheel build) →
//!      the clean `{CARGO_PKG_VERSION}`. A published wheel *is* the stable
//!      channel, so it must report the bare triple, never `-dev`.
//!
//! This mirrors `aozora-wasm/build.rs` — the two binding stamps share one
//! version-string format so every driver's `version()` reports identically.
//!
//! `rerun-if-changed` is scoped to the git refs that actually move the answer, so
//! the daily edit→build loop on one commit never re-runs this script — keeping the
//! wheel's incremental builds fast.

use std::env;
use std::path::{Path, PathBuf};
use std::process::Command;

fn main() {
    let git_dir = workspace_git_dir();

    // Only HEAD/index movement changes the stamp; pure source edits on one commit
    // leave these untouched, so the script stays cached during the inner loop.
    if let Some(git) = git_dir.as_deref() {
        println!("cargo:rerun-if-changed={}", git.join("HEAD").display());
        println!("cargo:rerun-if-changed={}", git.join("index").display());
    }
    println!("cargo:rerun-if-env-changed=AOZORA_BUILD_VERSION");

    let version = resolve_version(git_dir.is_some());
    println!("cargo:rustc-env=AOZORA_VERSION_STRING={version}");
}

/// The workspace `.git` directory, if this is a repo checkout. `CARGO_MANIFEST_DIR`
/// is this crate (`crates/aozora-py`); the workspace root is two levels up.
/// A packaged crate (a `PyPI` sdist / wheel build) has no such directory.
fn workspace_git_dir() -> Option<PathBuf> {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let git = manifest.parent()?.parent()?.join(".git");
    git.exists().then_some(git)
}

fn resolve_version(is_checkout: bool) -> String {
    if let Ok(forced) = env::var("AOZORA_BUILD_VERSION") {
        let forced = forced.trim();
        if !forced.is_empty() {
            return forced.to_owned();
        }
    }

    // The base triple is the workspace version (release-plz-managed once adopted).
    let base = env!("CARGO_PKG_VERSION");
    if !is_checkout {
        // Packaged build (no `.git`): a PyPI sdist / wheel build = stable.
        return base.to_owned();
    }

    // Workspace checkout = a dev build. Best-effort sha + dirty marker.
    git_short_sha().map_or_else(
        || format!("{base}-dev"),
        |sha| {
            let dirty = if git_is_dirty() { ".dirty" } else { "" };
            format!("{base}-dev+g{sha}{dirty}")
        },
    )
}

fn git_short_sha() -> Option<String> {
    let out = Command::new("git")
        .args(["rev-parse", "--short=7", "HEAD"])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let sha = String::from_utf8(out.stdout).ok()?.trim().to_owned();
    (!sha.is_empty()).then_some(sha)
}

fn git_is_dirty() -> bool {
    // Best-effort: evaluated only when HEAD/index moved (rerun gating above), so a
    // build, then unstaged edit, then rebuild may not flip the flag — the sha still
    // pins the base commit. `git status --porcelain` prints one line per change.
    Command::new("git")
        .args(["status", "--porcelain"])
        .output()
        .is_ok_and(|o| o.status.success() && !o.stdout.is_empty())
}
