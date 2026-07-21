//! `xtask release rearm-decision` — the version-standstill accept/reject rule.
//!
//! Lifted verbatim out of `release-plz.yml`'s inline "Detect release commit"
//! step, whose `workflow_dispatch` branch (a rearm that does NOT bump the
//! version) had never run in production. As Rust it is unit-tested and the
//! workflow calls the binary, so the untested inline branch ceases to exist.
//!
//! The rule: a `push` publishes iff it bumped the version. A
//! `workflow_dispatch` (a rearm recovery) publishes even without a bump — main
//! may already sit at the target version — UNLESS that version is already
//! tagged, which would mean re-publishing a cut release.

use std::env;
use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde::Deserialize;

#[derive(Debug, PartialEq, Eq)]
pub(super) enum Decision {
    /// Publish this commit.
    Release,
    /// Not a release commit — downstream publish steps stay skipped.
    Skip,
}

#[derive(Debug, PartialEq, Eq)]
pub(super) enum Rejected {
    /// The target version is already tagged; a rearm must not re-publish it.
    AlreadyReleased,
}

/// The observed facts a decision turns on.
pub(super) struct Facts {
    /// HEAD's diff against its parent bumped the workspace version.
    pub version_changed: bool,
    /// A `v<version>` tag already exists for the target version.
    pub tag_exists: bool,
}

/// The pure decision. Kept free of I/O so every branch — including the rearm
/// one the workflow could not exercise — is unit-tested.
pub(super) fn decide(event: &str, facts: &Facts) -> Result<Decision, Rejected> {
    if event == "workflow_dispatch" {
        if !facts.version_changed && facts.tag_exists {
            Err(Rejected::AlreadyReleased)
        } else {
            Ok(Decision::Release)
        }
    } else if facts.version_changed {
        Ok(Decision::Release)
    } else {
        Ok(Decision::Skip)
    }
}

pub(super) fn run(event: &str, version_changed: &str, commit: &str) -> Result<(), String> {
    // Guard the commit format and that it is actually checked out, exactly as
    // the inline step did — a malformed or mismatched SHA must fail loud.
    if commit.len() != 40 || !commit.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(format!("QUALIFIED_SHA is not a 40-hex commit: {commit:?}"));
    }
    let root = workspace_root()?;
    let head = git(&root, &["rev-parse", "HEAD"])?;
    if head != commit {
        return Err(format!(
            "HEAD ({head}) is not the qualified commit ({commit})"
        ));
    }

    let version = workspace_version(&root)?;
    let tag = format!("v{version}");
    let facts = Facts {
        version_changed: version_changed == "true",
        tag_exists: git_tag_exists(&root, &tag)?,
    };

    match decide(event, &facts) {
        Ok(Decision::Release) => emit("release=true"),
        Ok(Decision::Skip) => emit("release=false"),
        Err(Rejected::AlreadyReleased) => Err(format!(
            "::error title=release commit::{tag} is already released; nothing to rearm"
        )),
    }
}

/// Write a `key=value` line to `$GITHUB_OUTPUT` when running under Actions,
/// else to stdout — so the workflow reads the decision without having to
/// capture (and strip) cargo's build chatter.
fn emit(line: &str) -> Result<(), String> {
    if let Some(path) = env::var_os("GITHUB_OUTPUT") {
        let mut file = fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .map_err(|err| format!("open $GITHUB_OUTPUT: {err}"))?;
        writeln!(file, "{line}").map_err(|err| format!("write $GITHUB_OUTPUT: {err}"))
    } else {
        println!("{line}");
        Ok(())
    }
}

fn workspace_root() -> Result<PathBuf, String> {
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    Path::new(manifest_dir)
        .parent()
        .and_then(Path::parent)
        .map(Path::to_path_buf)
        .ok_or_else(|| format!("could not derive workspace root from {manifest_dir:?}"))
}

fn git(root: &Path, args: &[&str]) -> Result<String, String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .output()
        .map_err(|err| format!("run git {args:?}: {err}"))?;
    if !output.status.success() {
        return Err(format!(
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

/// Whether `tag` resolves — mirrors `git rev-parse -q --verify refs/tags/…`,
/// which exits non-zero (not an error) when the tag is absent.
fn git_tag_exists(root: &Path, tag: &str) -> Result<bool, String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["rev-parse", "-q", "--verify", &format!("refs/tags/{tag}")])
        .output()
        .map_err(|err| format!("run git rev-parse for {tag}: {err}"))?;
    Ok(output.status.success())
}

fn workspace_version(root: &Path) -> Result<String, String> {
    #[derive(Deserialize)]
    struct Root {
        workspace: Workspace,
    }
    #[derive(Deserialize)]
    struct Workspace {
        package: Package,
    }
    #[derive(Deserialize)]
    struct Package {
        version: String,
    }
    let path = root.join("Cargo.toml");
    let text =
        fs::read_to_string(&path).map_err(|err| format!("read {}: {err}", path.display()))?;
    let root: Root =
        toml::from_str(&text).map_err(|err| format!("parse {}: {err}", path.display()))?;
    Ok(root.workspace.package.version)
}

#[cfg(test)]
mod tests {
    use super::*;

    const BUMPED: Facts = Facts {
        version_changed: true,
        tag_exists: false,
    };
    const STANDSTILL_UNTAGGED: Facts = Facts {
        version_changed: false,
        tag_exists: false,
    };
    const STANDSTILL_TAGGED: Facts = Facts {
        version_changed: false,
        tag_exists: true,
    };
    const BUMPED_OVER_OLD_TAG: Facts = Facts {
        version_changed: true,
        tag_exists: true,
    };

    #[test]
    fn push_publishes_only_on_a_version_bump() {
        assert_eq!(decide("push", &BUMPED), Ok(Decision::Release));
        assert_eq!(decide("push", &STANDSTILL_UNTAGGED), Ok(Decision::Skip));
        // A stale tag is irrelevant to a push — the bump alone decides.
        assert_eq!(decide("push", &BUMPED_OVER_OLD_TAG), Ok(Decision::Release));
        assert_eq!(decide("push", &STANDSTILL_TAGGED), Ok(Decision::Skip));
    }

    #[test]
    fn dispatch_rearms_without_a_bump_when_untagged() {
        // The path release-plz.yml could never exercise: version standstill,
        // no tag yet -> the rearm is accepted.
        assert_eq!(
            decide("workflow_dispatch", &STANDSTILL_UNTAGGED),
            Ok(Decision::Release)
        );
    }

    #[test]
    fn dispatch_rejects_a_standstill_rearm_of_an_already_tagged_version() {
        assert_eq!(
            decide("workflow_dispatch", &STANDSTILL_TAGGED),
            Err(Rejected::AlreadyReleased)
        );
    }

    #[test]
    fn dispatch_with_a_bump_always_releases() {
        // A dispatch that did bump releases whether or not an (older) tag
        // exists — the guard only blocks a standstill re-publish.
        assert_eq!(decide("workflow_dispatch", &BUMPED), Ok(Decision::Release));
        assert_eq!(
            decide("workflow_dispatch", &BUMPED_OVER_OLD_TAG),
            Ok(Decision::Release)
        );
    }

    #[test]
    fn run_rejects_a_malformed_commit() {
        let err = run("push", "false", "not-a-sha").expect_err("malformed SHA must fail");
        assert!(err.contains("40-hex"), "{err}");
    }

    #[test]
    fn workspace_version_reads_the_live_manifest() {
        let root = workspace_root().expect("workspace root");
        let version = workspace_version(&root).expect("live Cargo.toml must parse");
        assert!(
            version.split('.').count() == 3,
            "expected a semver-ish version, got {version:?}"
        );
    }
}
