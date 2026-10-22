//! Sets the JavaScript binding's build version through `AOZORA_VERSION_STRING`.

use std::env;
use std::path::{Path, PathBuf};
use std::process::Command;

fn main() {
    let git_dir = workspace_git_dir();

    if let Some(git) = git_dir.as_deref() {
        println!("cargo:rerun-if-changed={}", git.join("HEAD").display());
        println!("cargo:rerun-if-changed={}", git.join("index").display());
    }
    println!("cargo:rerun-if-env-changed=AOZORA_BUILD_VERSION");

    let version = resolve_version(git_dir.is_some());
    println!("cargo:rustc-env=AOZORA_VERSION_STRING={version}");
}

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

    let base = env!("CARGO_PKG_VERSION");
    if !is_checkout {
        return base.to_owned();
    }

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
    Command::new("git")
        .args(["status", "--porcelain"])
        .output()
        .is_ok_and(|o| o.status.success() && !o.stdout.is_empty())
}
