//! `xtask release rehearse` — dry-run the tag-driven publishers before the
//! real tag.
//!
//! Cutting the `v*` tag is irreversible, yet the `qualify` jobs that gate each
//! publisher never run until that tag pushes — so a break in them surfaces only
//! after the point of no return. This fires each publisher's `dry_run` dispatch
//! (`qualify` runs, `publish` stays skipped) against the qualified commit while
//! the tag can still be withheld, and fails on zero dispatches so a rehearsal
//! that exercised nothing can never read the same as one that passed.
//!
//! Only the publishers whose `qualify` verifies the commit's retained artifacts
//! *without* resolving a tag can be rehearsed pre-tag: `publish-pypi.yml` and
//! `publish-npm.yml` leave `RELEASE_TAG` empty on dispatch. `release.yml` and
//! `publish-extism-wasm.yml` resolve the tag with `git rev-parse` in `qualify`,
//! which cannot succeed before the push, so they are inherently tag-only and
//! are reported as deliberately skipped rather than silently dropped.
//!
//! `gh workflow run` returns no run id, and a dispatched run's head SHA is the
//! branch tip rather than the input `commit`, so there is no race-free way to
//! correlate and block on the run this process just started. The rehearsal
//! therefore confirms each dispatch was accepted and hands the maintainer the
//! exact `gh run watch` command to require green — the fail-on-zero gate is the
//! automated half; watching `qualify` to completion is the manual half.

use std::env;
use std::path::{Path, PathBuf};
use std::process::Command;

/// The branch the dispatches target; a rehearsal runs against the release line.
const DEFAULT_BRANCH: &str = "main";

/// A publisher whose `qualify` job runs under `workflow_dispatch` without a
/// pre-existing tag, so its dry run rehearses the tag-push path in advance.
struct Rehearsable {
    /// The workflow file under `.github/workflows/`.
    workflow: &'static str,
    /// The `workflow_dispatch` inputs, besides the per-run `commit`, that select
    /// the no-upload dry run — each declared verbatim by that workflow.
    dry_run_inputs: &'static [&'static str],
}

/// The publishers whose `qualify` verifies retained artifacts against the
/// commit alone (`RELEASE_TAG` empty on dispatch), so a dry run needs no tag.
const REHEARSABLE: &[Rehearsable] = &[
    Rehearsable {
        workflow: "publish-pypi.yml",
        dry_run_inputs: &["dry_run=true"],
    },
    Rehearsable {
        workflow: "publish-npm.yml",
        dry_run_inputs: &["dry_run=true"],
    },
];

/// The publishers whose `qualify` resolves the release tag with `git rev-parse`
/// and so cannot run before the tag exists. Named, not omitted, so the report
/// distinguishes "deliberately tag-only" from "forgotten".
const TAG_ONLY: &[&str] = &["release.yml", "publish-extism-wasm.yml"];

pub(super) fn run(commit: Option<&str>) -> Result<(), String> {
    let root = workspace_root()?;
    let commit = resolve_commit(&root, commit)?;
    let repo = resolve_repo(&root)?;

    let mut fired: Vec<&'static str> = Vec::new();
    let mut failures: Vec<String> = Vec::new();
    for publisher in REHEARSABLE {
        match fire(&repo, publisher, &commit) {
            Ok(()) => fired.push(publisher.workflow),
            Err(err) => failures.push(err),
        }
    }

    // Fail-on-zero: a rehearsal that dispatched nothing exercised nothing, so it
    // proves nothing — it must never render the same as a clean pass.
    if fired.is_empty() {
        let detail = if failures.is_empty() {
            String::new()
        } else {
            format!(":\n  {}", failures.join("\n  "))
        };
        return Err(format!(
            "rehearsal fired ZERO dry-run dispatches — nothing was exercised{detail}"
        ));
    }
    if !failures.is_empty() {
        return Err(format!(
            "rehearsal dispatch failed for {} of {} publishers:\n  {}",
            failures.len(),
            REHEARSABLE.len(),
            failures.join("\n  "),
        ));
    }

    report_success(&repo, &commit, &fired);
    Ok(())
}

/// Fire one publisher's dry-run dispatch and confirm GitHub accepted it. A
/// non-zero exit (bad input, missing workflow, unauthenticated `gh`) is a
/// failure — acceptance is the most this step can assert offline.
fn fire(repo: &str, publisher: &Rehearsable, commit: &str) -> Result<(), String> {
    let mut args = vec![
        "workflow".to_owned(),
        "run".to_owned(),
        publisher.workflow.to_owned(),
        "--repo".to_owned(),
        repo.to_owned(),
        "--ref".to_owned(),
        DEFAULT_BRANCH.to_owned(),
    ];
    for input in dispatch_inputs(publisher, commit) {
        args.push("-f".to_owned());
        args.push(input);
    }
    let output = Command::new("gh")
        .args(&args)
        .output()
        .map_err(|err| format!("spawn `gh workflow run {}`: {err}", publisher.workflow))?;
    if output.status.success() {
        Ok(())
    } else {
        Err(format!(
            "gh workflow run {} was rejected: {}",
            publisher.workflow,
            String::from_utf8_lossy(&output.stderr).trim(),
        ))
    }
}

/// Build the `-f key=value` inputs for `gh workflow run`: the required `commit`
/// pinned to the rehearsed SHA, then each dry-run selector the workflow
/// declares. Pure so the table-to-inputs mapping is tested without a dispatch.
fn dispatch_inputs(publisher: &Rehearsable, commit: &str) -> Vec<String> {
    let mut inputs = vec![format!("commit={commit}")];
    inputs.extend(publisher.dry_run_inputs.iter().copied().map(str::to_owned));
    inputs
}

/// Report what fired, how to require each `qualify` green, and which publishers
/// are inherently tag-only. Prose only — the gate already fired in [`run`].
fn report_success(repo: &str, commit: &str, fired: &[&str]) {
    eprintln!(
        "xtask release rehearse: fired {} dry-run dispatch(es) for commit {commit}:",
        fired.len(),
    );
    for workflow in fired {
        eprintln!("  - {workflow} (dry_run=true: qualify runs, publish stays skipped)");
    }
    eprintln!("require each qualify job green before cutting the tag:");
    for workflow in fired {
        eprintln!(
            "  gh run watch --repo {repo} --exit-status \
             \"$(gh run list --repo {repo} --workflow {workflow} --branch {DEFAULT_BRANCH} \
             --event workflow_dispatch --limit 1 --json databaseId --jq '.[0].databaseId')\""
        );
    }
    eprintln!(
        "NOT rehearsable before the tag (their qualify resolves the tag with \
         `git rev-parse`, which cannot succeed pre-push) — verify after the tag exists:"
    );
    for workflow in TAG_ONLY {
        eprintln!("  - {workflow}");
    }
    eprintln!(
        "post-tag steps: docs/contrib/release.md \"Cutting a release\" (6-7) and its retry \
         recipe; the OIDC/credential posture is docs/contrib/releasing-secrets.md \
         \"Security model\".",
    );
}

/// The commit to rehearse: the explicit argument, else `HEAD`. Validated to a
/// full SHA here so a typo fails loud locally, not deep inside a dispatched
/// runner's checkout.
fn resolve_commit(root: &Path, commit: Option<&str>) -> Result<String, String> {
    let commit = match commit {
        Some(commit) => commit.to_owned(),
        None => git(root, &["rev-parse", "HEAD"])?,
    };
    if is_full_sha(&commit) {
        Ok(commit)
    } else {
        Err(format!("commit is not a 40-hex SHA: {commit:?}"))
    }
}

/// A full-length hex commit id, as the publishers' `qualify` steps require.
fn is_full_sha(commit: &str) -> bool {
    commit.len() == 40 && commit.bytes().all(|byte| byte.is_ascii_hexdigit())
}

/// The `OWNER/REPO` to dispatch against: `$GITHUB_REPOSITORY` when running under
/// Actions, else whatever `gh` resolves from the checkout. Explicit so a
/// rehearsal inside CI targets the same repo the eventual tag push will.
fn resolve_repo(root: &Path) -> Result<String, String> {
    if let Some(value) = env::var_os("GITHUB_REPOSITORY") {
        let repo = value.to_string_lossy().trim().to_owned();
        if !repo.is_empty() {
            return Ok(repo);
        }
    }
    let output = Command::new("gh")
        .current_dir(root)
        .args([
            "repo",
            "view",
            "--json",
            "nameWithOwner",
            "--jq",
            ".nameWithOwner",
        ])
        .output()
        .map_err(|err| format!("spawn `gh repo view`: {err}"))?;
    if !output.status.success() {
        return Err(format!(
            "gh repo view failed (set $GITHUB_REPOSITORY or run inside the checkout): {}",
            String::from_utf8_lossy(&output.stderr).trim(),
        ));
    }
    let repo = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    if repo.is_empty() {
        return Err("gh repo view returned an empty OWNER/REPO".to_owned());
    }
    Ok(repo)
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

fn workspace_root() -> Result<PathBuf, String> {
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    Path::new(manifest_dir)
        .parent()
        .and_then(Path::parent)
        .map(Path::to_path_buf)
        .ok_or_else(|| format!("could not derive workspace root from {manifest_dir:?}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dispatch_inputs_pin_the_commit_then_select_the_dry_run() {
        for publisher in REHEARSABLE {
            let inputs = dispatch_inputs(publisher, "cafef00d");
            assert_eq!(
                inputs.first().map(String::as_str),
                Some("commit=cafef00d"),
                "{} must pin the commit first",
                publisher.workflow,
            );
            assert!(
                inputs.iter().any(|input| input == "dry_run=true"),
                "{} must select the dry run",
                publisher.workflow,
            );
        }
    }

    #[test]
    fn every_rehearsable_declares_a_dry_run_selector() {
        // Fail-on-zero at the table level: a publisher with no dry-run input
        // would dispatch a *real* publish, so the set must never be empty and
        // each entry must carry a selector.
        assert!(!REHEARSABLE.is_empty(), "rehearsable set must not be empty");
        for publisher in REHEARSABLE {
            assert!(
                !publisher.dry_run_inputs.is_empty(),
                "{} has no dry-run selector",
                publisher.workflow,
            );
        }
    }

    #[test]
    fn rehearsable_and_tag_only_are_disjoint_and_named() {
        assert!(
            !TAG_ONLY.is_empty(),
            "tag-only set must be named, not empty"
        );
        for publisher in REHEARSABLE {
            assert!(
                !TAG_ONLY.contains(&publisher.workflow),
                "{} cannot be both rehearsable and tag-only",
                publisher.workflow,
            );
        }
    }

    #[test]
    fn is_full_sha_accepts_only_a_40_hex_id() {
        assert!(is_full_sha(&"a".repeat(40)));
        assert!(is_full_sha("0123456789abcdef0123456789ABCDEF01234567"));
        assert!(!is_full_sha("abc"), "too short");
        assert!(!is_full_sha(&"a".repeat(41)), "too long");
        assert!(!is_full_sha(&"g".repeat(40)), "non-hex");
    }

    #[test]
    fn resolve_commit_rejects_a_malformed_argument() {
        let root = workspace_root().expect("workspace root");
        let err = resolve_commit(&root, Some("not-a-sha")).expect_err("must reject a bad SHA");
        assert!(err.contains("40-hex"), "{err}");
    }

    #[test]
    fn named_workflows_exist_on_disk() {
        // Guards against a workflow rename silently emptying a set: every named
        // file must actually live under `.github/workflows/`.
        let dir = workspace_root()
            .expect("workspace root")
            .join(".github/workflows");
        let named = REHEARSABLE
            .iter()
            .map(|publisher| publisher.workflow)
            .chain(TAG_ONLY.iter().copied());
        for workflow in named {
            assert!(
                dir.join(workflow).is_file(),
                "{workflow} is named but missing under .github/workflows/",
            );
        }
    }
}
