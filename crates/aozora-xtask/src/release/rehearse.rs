//! Dispatches publisher rehearsals and reports commands for monitoring their results.

use std::env;
use std::path::{Path, PathBuf};
use std::process::Command;

const DEFAULT_BRANCH: &str = "main";

struct Rehearsable {
    workflow: &'static str,
    dry_run_inputs: &'static [&'static str],
}

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

fn dispatch_inputs(publisher: &Rehearsable, commit: &str) -> Vec<String> {
    let mut inputs = vec![format!("commit={commit}")];
    inputs.extend(publisher.dry_run_inputs.iter().copied().map(str::to_owned));
    inputs
}

#[expect(
    clippy::print_stderr,
    reason = "the command reports results and diagnostics to developers"
)]
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

fn is_full_sha(commit: &str) -> bool {
    const FULL_COMMIT_HEX_DIGITS: usize = 40;
    commit.len() == FULL_COMMIT_HEX_DIGITS && commit.bytes().all(|byte| byte.is_ascii_hexdigit())
}

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
