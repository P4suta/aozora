#![expect(
    clippy::expect_used,
    reason = "release ruleset patterns and workflow fixtures are validated constants"
)]

//! Checks release rules and workflow consistency offline.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use regex::Regex;
use serde_json::Value;

#[expect(
    clippy::print_stderr,
    reason = "the command reports results and diagnostics to developers"
)]
pub(super) fn check() -> Result<(), String> {
    let root = workspace_root()?;
    let mut violations = Vec::new();
    ruleset_integrity(&root, &mut violations);
    release_inputs_integrity(&root, &mut violations);
    sbom_mirror_parity(&root, &mut violations);
    release_always_enabled(&root, &mut violations);
    detached_head_attach_guard(&root, &mut violations);
    if violations.is_empty() {
        eprintln!("xtask release check: release source-integrity intact");
        Ok(())
    } else {
        Err(format!(
            "release source-integrity drift detected:\n  {}",
            violations.join("\n  "),
        ))
    }
}

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
    let required_rules: &[(&str, &[&str])] = &[
        (
            "release-tags.json",
            &["deletion", "non_fast_forward", "update"],
        ),
        (
            "main-branch.json",
            &[
                "deletion",
                "non_fast_forward",
                "required_linear_history",
                "required_signatures",
                "pull_request",
                "required_status_checks",
            ],
        ),
    ];
    for (file, required) in required_rules {
        let path = root.join(".github/rulesets").join(file);
        match load_ruleset(&path) {
            Err(err) => violations.push(err),
            Ok(ruleset) => {
                check_required_rules(file, required, &ruleset, violations);
                if *file == "main-branch.json" {
                    check_main_status_checks(&ruleset, violations);
                }
            }
        }
    }
}

fn check_main_status_checks(ruleset: &Value, violations: &mut Vec<String>) {
    if status_check_contexts(ruleset) != BTreeSet::from(["required".to_owned()]) {
        violations
            .push("main-branch.json: required status checks must be exactly `required`".to_owned());
    }
}

fn release_input_violations(text: &str) -> Vec<String> {
    let repository = Regex::new(r"^[A-Za-z0-9](?:[A-Za-z0-9-]*[A-Za-z0-9])?/[A-Za-z0-9_.-]+$")
        .expect("static repository regex");
    let sha = Regex::new(r"^[0-9a-f]{40}$").expect("static SHA regex");
    let mut violations = Vec::new();
    for key in [
        "AOZORA_CORPUS_REPOSITORY",
        "AOZORA_CORPUS_SHA",
        "AOZORA_SPEC_REPOSITORY",
        "AOZORA_SPEC_SHA",
    ] {
        let prefix = format!("{key}=");
        let matches: Vec<_> = text
            .lines()
            .filter_map(|line| line.strip_prefix(&prefix))
            .collect();
        if matches.len() != 1 {
            violations.push(format!(
                "release-inputs.env: `{key}` must occur exactly once"
            ));
            continue;
        }
        let value = matches[0];
        let valid = if key.ends_with("_REPOSITORY") {
            repository.is_match(value)
        } else {
            sha.is_match(value)
        };
        if !valid {
            violations.push(format!("release-inputs.env: `{key}` has an invalid value"));
        }
    }
    violations
}

fn release_inputs_integrity(root: &Path, violations: &mut Vec<String>) {
    match fs::read_to_string(root.join(".github/release-inputs.env")) {
        Err(err) => violations.push(format!("read .github/release-inputs.env: {err}")),
        Ok(text) => violations.extend(release_input_violations(&text)),
    }
}

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
                    { "context": "required" }
                ]}
            }]
        });
        let contexts = status_check_contexts(&ruleset);
        assert_eq!(contexts, BTreeSet::from(["required".to_owned()]));
    }

    #[test]
    fn main_status_checks_reject_missing_or_stale_contexts() {
        for checks in [
            serde_json::json!([]),
            serde_json::json!([
                { "context": "release-ready" },
                { "context": "ci-success" },
                { "context": "codeql (actions)" },
                { "context": "dependency review" }
            ]),
            serde_json::json!([{ "context": "required" }, { "context": "ci-success" }]),
            serde_json::json!([{ "context": "required" }]),
        ] {
            let ruleset = serde_json::json!({
                "rules": [{
                    "type": "required_status_checks",
                    "parameters": { "required_status_checks": checks }
                }]
            });
            let mut violations = Vec::new();
            check_main_status_checks(&ruleset, &mut violations);
            assert_eq!(
                violations.is_empty(),
                checks == serde_json::json!([{ "context": "required" }]),
                "unexpected result for {checks}: {violations:?}"
            );
        }
    }

    #[test]
    fn tag_immutability_requires_every_rule() {
        let required = ["deletion", "non_fast_forward", "update"];
        let ruleset = serde_json::json!({
            "rules": required.map(|kind| serde_json::json!({ "type": kind }))
        });
        let mut violations = Vec::new();
        check_required_rules("release-tags.json", &required, &ruleset, &mut violations);
        assert_eq!(violations, Vec::<String>::new());

        for missing in required {
            let mut broken = ruleset.clone();
            broken["rules"]
                .as_array_mut()
                .expect("rules array")
                .retain(|rule| rule["type"] != missing);
            let mut violations = Vec::new();
            check_required_rules("release-tags.json", &required, &broken, &mut violations);
            assert!(
                violations
                    .iter()
                    .any(|violation| violation.contains(missing)),
                "missing {missing} must be rejected: {violations:?}"
            );
        }
    }

    #[test]
    fn release_inputs_require_unique_typed_values() {
        let valid = "\
AOZORA_CORPUS_REPOSITORY=P4suta/aozorabunko_text\n\
AOZORA_CORPUS_SHA=b1ec9a7fa46de8dd5acc33378428c899e86bfb32\n\
AOZORA_SPEC_REPOSITORY=P4suta/aozora-notation-spec\n\
AOZORA_SPEC_SHA=a5473069f219000f7306a012e79e3ea393406680\n";
        assert_eq!(release_input_violations(valid), Vec::<String>::new());

        let duplicate = format!("{valid}AOZORA_SPEC_SHA={}", "0".repeat(40));
        assert!(
            release_input_violations(&duplicate)
                .iter()
                .any(|violation| violation.contains("exactly once"))
        );

        let mutable = valid
            .replace("P4suta/aozora-notation-spec", "main")
            .replace("a5473069f219000f7306a012e79e3ea393406680", "release");
        let violations = release_input_violations(&mutable);
        assert_eq!(violations.len(), 2, "repository and SHA are both rejected");
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
        assert!(detached_head_violation("ref: main\n").is_none());
    }

    #[test]
    fn live_release_source_integrity_holds() {
        check().expect("release source-integrity must hold on the live tree");
    }
}
