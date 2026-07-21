//! `xtask release check` — offline source-integrity for the release path.
//!
//! Two facts that today live only in server-side / tag-time state, checked
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
    if violations.is_empty() {
        eprintln!("xtask release check: tag ruleset + native-SBOM path mirror intact");
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
    fn live_release_source_integrity_holds() {
        // The integration guard: the committed rulesets and the two SBOM
        // filename lists must actually agree.
        check().expect("release source-integrity must hold on the live tree");
    }
}
