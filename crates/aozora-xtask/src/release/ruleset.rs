//! `xtask release check` — offline source-integrity for the release path.
//!
//! Four facts that today live only in server-side / tag-time state, checked
//! against the committed source so a drift fails at PR time (in `drift-gate`)
//! instead of silently, or only at the real tag push:
//!
//! * **Tag ruleset integrity.** `.github/rulesets/*.json` still encode their
//!   rules. An emptied `"rules": []` parses fine and protects nothing — the
//!   canonical silent gate. The tag ruleset must keep its immutability rules,
//!   and the main-branch ruleset must keep `release-ready` + `ci-success`
//!   required, or a release could merge / a tag could move unchecked.
//! * **Native-SBOM path mirror.** `release.yml` (tag time) and
//!   `release-ready.yml` (PR time) each hard-code the six per-target SBOM
//!   filenames. They must agree, or the PR-time assertion stops mirroring the
//!   tag-time expectation — exactly the B1 layout drift that would hard-fail
//!   only at tag push.
//! * **`release_always` enabled.** `release-plz.toml` must keep
//!   `release_always = true`, or the `workflow_dispatch` recovery silently skips
//!   publishing a re-qualified commit ("current commit is not from a release
//!   PR") — the DEV-106 failure that ran green while publishing nothing.
//! * **Detached-HEAD attach.** `release-plz-release` checks out a SHA (a
//!   detached HEAD); the attach-branch step (`git update-ref
//!   refs/remotes/origin/main …`) must survive, or `release-plz release` aborts
//!   at `git … @{upstream}` before publishing — DEV-104.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use regex::Regex;
use serde_json::Value;

pub(super) fn check() -> Result<(), String> {
    let root = workspace_root()?;
    let mut violations = Vec::new();
    ruleset_integrity(&root, &mut violations);
    sbom_mirror_parity(&root, &mut violations);
    release_always_enabled(&root, &mut violations);
    detached_head_attach_guard(&root, &mut violations);
    if violations.is_empty() {
        eprintln!(
            "xtask release check: tag ruleset + SBOM mirror + release_always + detached-HEAD attach intact"
        );
        Ok(())
    } else {
        Err(format!(
            "release source-integrity drift detected:\n  {}",
            violations.join("\n  "),
        ))
    }
}

// ── tag ruleset integrity ────────────────────────────────────────────────

/// Rule `type` strings present in a ruleset document.
fn rule_types(ruleset: &Value) -> BTreeSet<String> {
    ruleset
        .get("rules")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|rule| rule.get("type").and_then(Value::as_str))
        .map(str::to_owned)
        .collect()
}

/// The `required_status_checks` contexts a branch ruleset demands.
fn status_check_contexts(ruleset: &Value) -> BTreeSet<String> {
    ruleset
        .get("rules")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|rule| rule.get("type").and_then(Value::as_str) == Some("required_status_checks"))
        .filter_map(|rule| {
            rule.get("parameters")
                .and_then(|p| p.get("required_status_checks"))
                .and_then(Value::as_array)
        })
        .flatten()
        .filter_map(|check| check.get("context").and_then(Value::as_str))
        .map(str::to_owned)
        .collect()
}

fn load_ruleset(path: &Path) -> Result<Value, String> {
    let text = fs::read_to_string(path).map_err(|err| format!("read {}: {err}", path.display()))?;
    serde_json::from_str(&text).map_err(|err| format!("parse {}: {err}", path.display()))
}

/// A ruleset must keep a non-empty `rules` array (fail-on-zero — an emptied one
/// still parses) and each rule it exists to enforce.
fn check_required_rules(
    file: &str,
    required: &[&str],
    ruleset: &Value,
    violations: &mut Vec<String>,
) {
    let types = rule_types(ruleset);
    if types.is_empty() {
        violations.push(format!(
            "{file}: `rules` is empty — the ruleset parses but protects nothing"
        ));
        return;
    }
    for rule in required {
        if !types.contains(*rule) {
            violations.push(format!("{file}: missing required rule `{rule}`"));
        }
    }
}

fn ruleset_integrity(root: &Path, violations: &mut Vec<String>) {
    // Each ruleset must keep a non-empty `rules` array (fail-on-zero — an
    // emptied one still parses), plus the specific rules it exists to enforce.
    let required_rules: &[(&str, &[&str])] = &[
        (
            "release-tags.json",
            &["deletion", "non_fast_forward", "update"],
        ),
        (
            "main-branch.json",
            &["pull_request", "required_status_checks"],
        ),
        ("require-signed-commits.json", &[]),
    ];
    for (file, required) in required_rules {
        let path = root.join(".github/rulesets").join(file);
        match load_ruleset(&path) {
            Err(err) => violations.push(err),
            Ok(ruleset) => check_required_rules(file, required, &ruleset, violations),
        }
    }

    // The two release gates must stay required on main, or a release could
    // merge without release-ready / ci-success ever having to be green.
    if let Ok(ruleset) = load_ruleset(&root.join(".github/rulesets/main-branch.json")) {
        let contexts = status_check_contexts(&ruleset);
        for required in ["release-ready", "ci-success"] {
            if !contexts.contains(required) {
                violations.push(format!(
                    "main-branch.json: `{required}` is not a required status check"
                ));
            }
        }
    }
}

// ── native-SBOM path mirror ──────────────────────────────────────────────

/// The per-target native SBOM filenames named inside a bash array in a
/// workflow `run:` block. `marker` is the array opener (`expected=(` /
/// `native_sboms=(`); scanning stops at the closing `)`.
fn native_sboms_in(text: &str, marker: &str) -> BTreeSet<String> {
    let sbom = Regex::new(r"aozora-v\$\{version\}-[A-Za-z0-9_.-]+-(?:cli|ffi)\.cdx\.json")
        .expect("static native-SBOM regex");
    let mut in_block = false;
    let mut found = BTreeSet::new();
    for line in text.lines() {
        if !in_block {
            if line.contains(marker) {
                in_block = true;
            }
            continue;
        }
        if line.trim() == ")" {
            break;
        }
        if let Some(hit) = sbom.find(line) {
            found.insert(hit.as_str().to_owned());
        }
    }
    found
}

fn sbom_mirror_parity(root: &Path, violations: &mut Vec<String>) {
    let tag_time = root.join(".github/workflows/release.yml");
    let pr_time = root.join(".github/workflows/release-ready.yml");
    let (Ok(tag_text), Ok(pr_text)) = (fs::read_to_string(&tag_time), fs::read_to_string(&pr_time))
    else {
        violations.push("could not read release.yml / release-ready.yml".to_owned());
        return;
    };
    let expected = native_sboms_in(&tag_text, "expected=(");
    let mirror = native_sboms_in(&pr_text, "native_sboms=(");

    // Fail-on-zero: an extraction that finds nothing means the array moved or
    // was renamed, and the gate would otherwise pass vacuously.
    if expected.is_empty() {
        violations.push("release.yml: found no native SBOMs in `expected=(…)`".to_owned());
    }
    if mirror.is_empty() {
        violations
            .push("release-ready.yml: found no native SBOMs in `native_sboms=(…)`".to_owned());
    }
    if expected.is_empty() || mirror.is_empty() {
        return;
    }
    for missing in expected.difference(&mirror) {
        violations.push(format!(
            "release-ready.yml PR-time mirror is missing `{missing}` that release.yml expects at tag time"
        ));
    }
    for extra in mirror.difference(&expected) {
        violations.push(format!(
            "release-ready.yml PR-time mirror asserts `{extra}` that release.yml does not expect"
        ));
    }
}

// ── release_always ───────────────────────────────────────────────────────

/// `release-plz release` runs only on `workflow_dispatch` (the recovery / fan-in
/// path). With `release_always = false` it publishes only from a release-PR
/// merge commit and silently skips a re-qualified recovery commit — it must be
/// `true`. Pure over the file text so the branches are unit-testable.
fn release_always_violation(text: &str) -> Option<String> {
    let re = Regex::new(r"(?m)^\s*release_always\s*=\s*(true|false)\b")
        .expect("static release_always regex");
    match re.captures(text).map(|c| c[1].to_owned()).as_deref() {
        None => Some(
            "release-plz.toml: no `release_always = <bool>` — the key moved or was renamed, \
             so this gate is inert"
                .to_owned(),
        ),
        Some("false") => Some(
            "release-plz.toml: `release_always = false` makes a recovery dispatch skip \
             publishing (\"current commit is not from a release PR\"); it must be true"
                .to_owned(),
        ),
        Some(_) => None,
    }
}

fn release_always_enabled(root: &Path, violations: &mut Vec<String>) {
    match fs::read_to_string(root.join("release-plz.toml")) {
        Err(err) => violations.push(format!("read release-plz.toml: {err}")),
        Ok(text) => violations.extend(release_always_violation(&text)),
    }
}

// ── detached-HEAD attach ─────────────────────────────────────────────────

/// Pure over release-plz.yml text: if `release-plz-release` checks out a SHA
/// (detached HEAD) but the attach-branch step's load-bearing
/// `git update-ref refs/remotes/origin/main` is gone, `release-plz release`
/// aborts at `git … @{upstream}`.
fn detached_head_violation(release_plz_yml: &str) -> Option<String> {
    let checks_out_sha = release_plz_yml.contains("ref: ${{ env.QUALIFIED_SHA }}");
    let attaches = release_plz_yml.contains("update-ref refs/remotes/origin/main");
    (checks_out_sha && !attaches).then(|| {
        "release-plz.yml: release-plz-release checks out a SHA (detached HEAD) but no longer \
         attaches a tracking branch (`git update-ref refs/remotes/origin/main`); `release-plz \
         release` will abort at `git … @{upstream}` — DEV-104"
            .to_owned()
    })
}

fn detached_head_attach_guard(root: &Path, violations: &mut Vec<String>) {
    match fs::read_to_string(root.join(".github/workflows/release-plz.yml")) {
        Err(err) => violations.push(format!("read release-plz.yml: {err}")),
        Ok(text) => violations.extend(detached_head_violation(&text)),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rule_types_reads_every_rule() {
        let ruleset = serde_json::json!({
            "rules": [{ "type": "deletion" }, { "type": "update" }]
        });
        let types = rule_types(&ruleset);
        assert!(types.contains("deletion") && types.contains("update"));
    }

    #[test]
    fn rule_types_of_an_emptied_ruleset_is_empty() {
        assert!(rule_types(&serde_json::json!({ "rules": [] })).is_empty());
        assert!(rule_types(&serde_json::json!({})).is_empty());
    }

    #[test]
    fn status_check_contexts_are_extracted() {
        let ruleset = serde_json::json!({
            "rules": [{
                "type": "required_status_checks",
                "parameters": { "required_status_checks": [
                    { "context": "ci-success" },
                    { "context": "release-ready" }
                ]}
            }]
        });
        let contexts = status_check_contexts(&ruleset);
        assert!(contexts.contains("ci-success") && contexts.contains("release-ready"));
    }

    #[test]
    fn native_sboms_extracts_only_the_in_block_sboms() {
        let text = "\
            other=(\n  \"aozora-v${version}-decoy-cli.cdx.json\"\n)\n\
            native_sboms=(\n\
            \x20 \"aozora-v${version}-x86_64-unknown-linux-gnu-cli.cdx.json\"\n\
            \x20 \"aozora-v${version}-x86_64-unknown-linux-gnu-ffi.cdx.json\"\n\
            \x20 aozora-go.tar.gz\n\
            )\n";
        let found = native_sboms_in(text, "native_sboms=(");
        assert_eq!(found.len(), 2, "only the two in-block SBOMs: {found:?}");
        assert!(found.iter().all(|f| f.contains("linux-gnu")));
    }

    #[test]
    fn release_always_true_passes_false_and_missing_fail() {
        assert!(release_always_violation("[workspace]\nrelease_always = true\n").is_none());
        assert!(
            release_always_violation("release_always = false\n")
                .expect("false is a violation")
                .contains("must be true")
        );
        assert!(
            release_always_violation("publish = true\n")
                .expect("a missing key is a violation")
                .contains("moved or was renamed")
        );
    }

    #[test]
    fn detached_head_attach_must_survive_a_sha_checkout() {
        let with =
            "ref: ${{ env.QUALIFIED_SHA }}\n  git update-ref refs/remotes/origin/main HEAD\n";
        assert!(detached_head_violation(with).is_none());
        let without = "ref: ${{ env.QUALIFIED_SHA }}\n  git switch -C main\n";
        assert!(
            detached_head_violation(without)
                .expect("a SHA checkout without the attach is a violation")
                .contains("@{upstream}")
        );
        // A workflow that never checks out a SHA is not subject to this.
        assert!(detached_head_violation("ref: main\n").is_none());
    }

    #[test]
    fn live_release_source_integrity_holds() {
        // The integration guard: the committed rulesets and the two SBOM
        // filename lists must actually agree.
        check().expect("release source-integrity must hold on the live tree");
    }
}
