//! Checks local release preconditions and remote settings unless offline.

use std::collections::BTreeSet;
use std::env;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::Value;

const COMMIT_SHA_HEX_LEN: usize = 40;
const TREE_SITTER_CRATE_URL: &str = "https://crates.io/api/v1/crates/tree-sitter-aozora";
const PYPI_PROJECT_URL: &str = "https://pypi.org/pypi/aozora/json";
const NPM_PACKAGE_URL: &str = "https://registry.npmjs.org/aozora-wasm";
const REGISTRY_USER_AGENT: &str =
    "aozora-xtask release-preflight (+https://github.com/P4suta/aozora)";
const CURL_OUTPUT_SINK: &str = "/dev/null";
const REGISTRY_TIMEOUT_SECONDS: &str = "30";

pub(super) struct Request<'a> {
    pub offline: bool,
    pub first_publish: bool,
    pub commit: Option<&'a str>,
}

pub(super) fn run(request: &Request<'_>) -> Result<(), String> {
    let root = workspace_root()?;
    let mut violations = Vec::new();

    check_source_integrity(&mut violations);
    check_freeze_latch(&root, &mut violations);
    if !request.offline {
        match online_context(request, &root) {
            Ok(context) => check_deployed_state(&context, request, &mut violations),
            Err(err) => violations.push(err),
        }
    }

    if violations.is_empty() {
        report_success(request);
        Ok(())
    } else {
        Err(format!(
            "release preflight found {} unmet precondition(s):\n  {}",
            violations.len(),
            violations.join("\n  "),
        ))
    }
}

fn check_source_integrity(violations: &mut Vec<String>) {
    if let Err(err) = super::ruleset::check() {
        violations.push(err);
    }
}

fn check_freeze_latch(root: &Path, violations: &mut Vec<String>) {
    if root.join(".github/RELEASE_FROZEN.md").exists() {
        violations.push(
            "publication freeze latch .github/RELEASE_FROZEN.md is still present — \
             remove it in the dedicated reviewed rearm PR before rearming (release.md)"
                .to_owned(),
        );
    }
}

struct Context {
    repo: String,
    commit: String,
}

fn online_context(request: &Request<'_>, root: &Path) -> Result<Context, String> {
    let repo = resolve_repo()?;
    if !repo.contains('/') {
        return Err(format!("resolved repository {repo:?} is not owner/name"));
    }
    let commit = resolve_commit(request, root)?;
    Ok(Context { repo, commit })
}

fn resolve_repo() -> Result<String, String> {
    if let Some(env_repo) = env::var_os("GITHUB_REPOSITORY") {
        let env_repo = env_repo.to_string_lossy().trim().to_owned();
        if !env_repo.is_empty() {
            return Ok(env_repo);
        }
    }
    let out = gh(&[
        "repo",
        "view",
        "--json",
        "nameWithOwner",
        "-q",
        ".nameWithOwner",
    ])?;
    let repo = out.trim().to_owned();
    if repo.is_empty() {
        return Err("gh repo view returned an empty nameWithOwner".to_owned());
    }
    Ok(repo)
}

fn resolve_commit(request: &Request<'_>, root: &Path) -> Result<String, String> {
    let commit = match request.commit {
        Some(commit) => commit.to_owned(),
        None => git(root, &["rev-parse", "HEAD"])?,
    };
    if commit.len() != COMMIT_SHA_HEX_LEN || !commit.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(format!("commit {commit:?} is not a 40-hex sha"));
    }
    Ok(commit)
}

fn check_deployed_state(context: &Context, request: &Request<'_>, violations: &mut Vec<String>) {
    check_release_plz_secrets(&context.repo, request.first_publish, violations);
    check_release_environment(&context.repo, violations);
    check_tag_ruleset(&context.repo, violations);
    check_release_ready_run(&context.repo, &context.commit, violations);
    check_registries(request.first_publish, violations);
}

fn check_release_plz_secrets(repo: &str, first_publish: bool, violations: &mut Vec<String>) {
    let json = match gh(&[
        "secret",
        "list",
        "--repo",
        repo,
        "--env",
        "release-plz",
        "--json",
        "name",
    ]) {
        Ok(json) => json,
        Err(err) => {
            violations.push(err);
            return;
        }
    };
    match parse_names(&json) {
        Ok(names) => audit_release_plz_secrets(&names, first_publish, violations),
        Err(err) => violations.push(err),
    }
}

fn audit_release_plz_secrets(
    names: &BTreeSet<String>,
    first_publish: bool,
    violations: &mut Vec<String>,
) {
    if names.is_empty() {
        violations.push(
            "release-plz environment lists zero secrets — empty must not read like present \
             (releasing-secrets.md §2)"
                .to_owned(),
        );
        return;
    }
    for required in ["RELEASE_PLZ_APP_CLIENT_ID", "RELEASE_PLZ_APP_PRIVATE_KEY"] {
        if !names.contains(required) {
            violations.push(format!(
                "release-plz environment is missing {required} (releasing-secrets.md §2)"
            ));
        }
    }
    let token = names.contains("CARGO_REGISTRY_TOKEN");
    if token && !first_publish {
        violations.push(
            "CARGO_REGISTRY_TOKEN is present on release-plz outside a first publish — \
             steady state is tokenless OIDC (releasing-secrets.md §3)"
                .to_owned(),
        );
    }
    if !token && first_publish {
        violations.push(
            "--first-publish set but CARGO_REGISTRY_TOKEN is absent from release-plz — \
             the crates.io bootstrap needs it (releasing-secrets.md §3)"
                .to_owned(),
        );
    }
}

fn check_release_environment(repo: &str, violations: &mut Vec<String>) {
    match gh_api(&[&format!("repos/{repo}/environments/release")]) {
        Err(err) => violations.push(err),
        Ok(GhApi::Missing) => violations.push(format!(
            "`release` environment does not exist for {repo} — a workflow naming it would \
             auto-create it WITHOUT protection rules (releasing-secrets.md §1)"
        )),
        Ok(GhApi::Found(body)) => {
            audit_release_reviewers(&body, violations);
            check_release_branch_policies(repo, violations);
        }
    }
}

fn audit_release_reviewers(body: &str, violations: &mut Vec<String>) {
    let env = match serde_json::from_str::<Value>(body) {
        Ok(env) => env,
        Err(err) => {
            violations.push(format!("parse release environment: {err}"));
            return;
        }
    };
    if required_reviewer_count(&env) == 0 {
        violations.push(
            "release environment requires zero reviewers — the approval gate is open \
             (releasing-secrets.md §1)"
                .to_owned(),
        );
    }
}

fn required_reviewer_count(env: &Value) -> usize {
    env.get("protection_rules")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|rule| rule.get("type").and_then(Value::as_str) == Some("required_reviewers"))
        .filter_map(|rule| rule.get("reviewers").and_then(Value::as_array))
        .flatten()
        .count()
}

fn check_release_branch_policies(repo: &str, violations: &mut Vec<String>) {
    let path = format!("repos/{repo}/environments/release/deployment-branch-policies");
    let body = match gh_api(&[&path]) {
        Ok(GhApi::Found(body)) => body,
        Ok(GhApi::Missing) => {
            violations.push(format!(
                "release environment exposes no deployment-branch policies for {repo} \
                 (releasing-secrets.md §1)"
            ));
            return;
        }
        Err(err) => {
            violations.push(err);
            return;
        }
    };
    match serde_json::from_str::<Value>(&body) {
        Ok(value) => audit_branch_policies(&value, violations),
        Err(err) => violations.push(format!("parse deployment-branch policies: {err}")),
    }
}

fn audit_branch_policies(body: &Value, violations: &mut Vec<String>) {
    let policies = admitted_branch_policies(body);
    if policies.is_empty() {
        violations.push(
            "release environment admits zero deployment branches or tags — main and v* must \
             both be allowed (releasing-secrets.md §1)"
                .to_owned(),
        );
        return;
    }
    for (name, kind) in [("main", "branch"), ("v*", "tag")] {
        if !policies.contains(&(name.to_owned(), kind.to_owned())) {
            violations.push(format!(
                "release environment does not admit {name} ({kind}) (releasing-secrets.md §1)"
            ));
        }
    }
}

fn admitted_branch_policies(body: &Value) -> BTreeSet<(String, String)> {
    body.get("branch_policies")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|policy| {
            let name = policy.get("name").and_then(Value::as_str)?;
            let kind = policy.get("type").and_then(Value::as_str)?;
            Some((name.to_owned(), kind.to_owned()))
        })
        .collect()
}

fn check_tag_ruleset(repo: &str, violations: &mut Vec<String>) {
    let body = match gh_api(&[&format!("repos/{repo}/rulesets")]) {
        Ok(GhApi::Found(body)) => body,
        Ok(GhApi::Missing) => {
            violations.push(format!(
                "{repo} exposes no rulesets endpoint (releasing-secrets.md §2)"
            ));
            return;
        }
        Err(err) => {
            violations.push(err);
            return;
        }
    };
    let rulesets = serde_json::from_str::<Value>(&body)
        .map_err(|err| format!("parse rulesets: {err}"))
        .and_then(|listing| {
            tag_ruleset_details(&listing, |id| {
                let path = format!("repos/{repo}/rulesets/{id}");
                match gh_api(&[&path])? {
                    GhApi::Found(body) => serde_json::from_str(&body)
                        .map_err(|err| format!("parse ruleset {id}: {err}")),
                    GhApi::Missing => Err(format!("ruleset {id} disappeared from {repo}")),
                }
            })
        });
    match rulesets {
        Ok(value) => audit_tag_ruleset(&value, violations),
        Err(err) => violations.push(err),
    }
}

fn tag_ruleset_details(
    listing: &Value,
    mut read: impl FnMut(u64) -> Result<Value, String>,
) -> Result<Value, String> {
    let list = listing
        .as_array()
        .ok_or_else(|| "rulesets: expected a JSON array".to_owned())?;
    list.iter()
        .map(|ruleset| {
            if ruleset.get("target").and_then(Value::as_str) != Some("tag") {
                return Ok(ruleset.clone());
            }
            let id = ruleset
                .get("id")
                .and_then(Value::as_u64)
                .ok_or_else(|| "tag ruleset has no numeric id".to_owned())?;
            read(id)
        })
        .collect::<Result<Vec<_>, _>>()
        .map(Value::Array)
}

enum TagRuleset {
    NoRulesets,
    Missing,
    Inactive,
    Active,
}

fn audit_tag_ruleset(rulesets: &Value, violations: &mut Vec<String>) {
    match tag_ruleset_state(rulesets) {
        TagRuleset::Active => {}
        TagRuleset::NoRulesets => violations.push(
            "repository returns zero rulesets — the v* tag-immutability protection is \
             unverifiable (releasing-secrets.md §2)"
                .to_owned(),
        ),
        TagRuleset::Missing => violations.push(
            "no tag ruleset includes refs/tags/v* server-side (releasing-secrets.md §2)".to_owned(),
        ),
        TagRuleset::Inactive => violations.push(
            "the ruleset for refs/tags/v* exists but is not active \
             (releasing-secrets.md §2)"
                .to_owned(),
        ),
    }
}

fn tag_ruleset_state(rulesets: &Value) -> TagRuleset {
    let Some(list) = rulesets.as_array() else {
        return TagRuleset::NoRulesets;
    };
    if list.is_empty() {
        return TagRuleset::NoRulesets;
    }
    let found = list.iter().find(|ruleset| {
        ruleset.get("target").and_then(Value::as_str) == Some("tag")
            && ruleset
                .pointer("/conditions/ref_name/include")
                .and_then(Value::as_array)
                .is_some_and(|includes| includes.iter().any(|include| include == "refs/tags/v*"))
    });
    match found {
        None => TagRuleset::Missing,
        Some(ruleset) if ruleset.get("enforcement").and_then(Value::as_str) == Some("active") => {
            TagRuleset::Active
        }
        Some(_) => TagRuleset::Inactive,
    }
}

fn check_release_ready_run(repo: &str, commit: &str, violations: &mut Vec<String>) {
    let json = match gh(&[
        "run",
        "list",
        "--repo",
        repo,
        "--workflow",
        "release-ready.yml",
        "--commit",
        commit,
        "--json",
        "status,conclusion",
    ]) {
        Ok(json) => json,
        Err(err) => {
            violations.push(err);
            return;
        }
    };
    match successful_run_count(&json) {
        Ok(0) => violations.push(format!(
            "no completed+success release-ready run for {commit} — the publish authority is \
             unproven (release.md; ADR-0042)"
        )),
        Ok(_) => {}
        Err(err) => violations.push(err),
    }
}

fn successful_run_count(json: &str) -> Result<usize, String> {
    let value: Value =
        serde_json::from_str(json).map_err(|err| format!("parse release-ready runs: {err}"))?;
    let runs = value
        .as_array()
        .ok_or_else(|| "release-ready runs: expected a JSON array".to_owned())?;
    Ok(runs
        .iter()
        .filter(|run| {
            run.get("status").and_then(Value::as_str) == Some("completed")
                && run.get("conclusion").and_then(Value::as_str) == Some("success")
        })
        .count())
}

enum Presence {
    New,
    Existing,
}

struct Registry<'a> {
    label: &'a str,
    url: &'a str,
    presence: Presence,
    fix: &'a str,
}

fn check_registries(first_publish: bool, violations: &mut Vec<String>) {
    let registries = [
        Registry {
            label: "crates.io crate tree-sitter-aozora",
            url: TREE_SITTER_CRATE_URL,
            presence: Presence::New,
            fix: "a new crate needs the CARGO_REGISTRY_TOKEN bootstrap before it can publish, \
                  so it can never silently proceed (releasing-secrets.md §3)",
        },
        Registry {
            label: "PyPI project aozora",
            url: PYPI_PROJECT_URL,
            presence: Presence::New,
            fix: "the pending publisher creates it on first OIDC upload (releasing-secrets.md §4)",
        },
        Registry {
            label: "npm package aozora-wasm",
            url: NPM_PACKAGE_URL,
            presence: Presence::Existing,
            fix: "the package is expected to already exist (releasing-secrets.md §5)",
        },
    ];
    for registry in &registries {
        check_registry(registry, first_publish, violations);
    }
}

fn check_registry(registry: &Registry<'_>, first_publish: bool, violations: &mut Vec<String>) {
    match http_status(registry.url) {
        Err(err) => violations.push(err),
        Ok(RegistryState::Exists) => {}
        Ok(RegistryState::Absent) => {
            if absence_is_violation(&registry.presence, first_publish) {
                violations.push(format!(
                    "{} not found ({}) — {}",
                    registry.label, registry.url, registry.fix
                ));
            }
        }
    }
}

fn absence_is_violation(presence: &Presence, first_publish: bool) -> bool {
    match presence {
        Presence::Existing => true,
        Presence::New => !first_publish,
    }
}

#[derive(Debug)]
enum RegistryState {
    Exists,
    Absent,
}

fn http_status(url: &str) -> Result<RegistryState, String> {
    let output = Command::new("curl")
        .args([
            "-sS",
            "-A",
            REGISTRY_USER_AGENT,
            "-o",
            CURL_OUTPUT_SINK,
            "-w",
            "%{http_code}",
            "--max-time",
            REGISTRY_TIMEOUT_SECONDS,
            url,
        ])
        .output()
        .map_err(|err| format!("run curl {url}: {err}"))?;
    if !output.status.success() {
        return Err(format!(
            "curl {url} failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    classify_http_status(&String::from_utf8_lossy(&output.stdout))
}

fn classify_http_status(code: &str) -> Result<RegistryState, String> {
    match code.trim() {
        "200" => Ok(RegistryState::Exists),
        "404" => Ok(RegistryState::Absent),
        other => Err(format!(
            "unexpected HTTP status {other:?} from a registry probe"
        )),
    }
}

#[expect(
    clippy::print_stderr,
    reason = "the command reports results and diagnostics to developers"
)]
fn report_success(request: &Request<'_>) {
    eprintln!("release preflight: every verifiable precondition holds");
    eprintln!("  - source-integrity gate intact (tag ruleset + native-SBOM path mirror)");
    eprintln!("  - publication freeze latch absent (rearm-ready)");
    if request.offline {
        eprintln!("  (offline: deployed-state probes skipped)");
        return;
    }
    eprintln!(
        "  - release-plz secrets, protected release environment, tag ruleset, release-ready \
         proof, and registry existence verified"
    );
    print_residue();
}

#[expect(
    clippy::print_stderr,
    reason = "the command reports results and diagnostics to developers"
)]
fn print_residue() {
    eprintln!("MANUAL RESIDUE — verify by hand; no API confirms these (releasing-secrets.md):");
    eprintln!(
        "  1. retired publish-crates.yml trusted publishers were deleted and re-added \
         against release-plz.yml (§3)"
    );
    eprintln!(
        "  2. the App-ID v* tag-creation lock was applied LAST, only after a v* tag was \
         seen cut (§2)"
    );
    eprintln!(
        "  3. the App-token tag push fans out to the downstream publishers — the default \
         token would not trigger them (§2; release.md)"
    );
}

enum GhApi {
    Found(String),
    Missing,
}

fn gh(args: &[&str]) -> Result<String, String> {
    let output = Command::new("gh")
        .args(args)
        .output()
        .map_err(|err| format!("run gh {args:?}: {err}"))?;
    if !output.status.success() {
        return Err(format!(
            "gh {args:?} failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

fn gh_api(args: &[&str]) -> Result<GhApi, String> {
    let output = Command::new("gh")
        .arg("api")
        .args(args)
        .output()
        .map_err(|err| format!("run gh api {args:?}: {err}"))?;
    if output.status.success() {
        return Ok(GhApi::Found(
            String::from_utf8_lossy(&output.stdout).into_owned(),
        ));
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    if stderr.contains("404") || stderr.contains("Not Found") {
        Ok(GhApi::Missing)
    } else {
        Err(format!("gh api {args:?} failed: {}", stderr.trim()))
    }
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

fn parse_names(json: &str) -> Result<BTreeSet<String>, String> {
    let value: Value =
        serde_json::from_str(json).map_err(|err| format!("parse secret list: {err}"))?;
    let array = value
        .as_array()
        .ok_or_else(|| "secret list: expected a JSON array".to_owned())?;
    Ok(array
        .iter()
        .filter_map(|entry| entry.get("name").and_then(Value::as_str))
        .map(str::to_owned)
        .collect())
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
    fn http_status_maps_only_200_and_404() {
        assert!(matches!(
            classify_http_status("200"),
            Ok(RegistryState::Exists)
        ));
        assert!(matches!(
            classify_http_status(" 404\n"),
            Ok(RegistryState::Absent)
        ));
        classify_http_status("500").expect_err("a 5xx must not be guessed at");
        classify_http_status("").expect_err("an empty status must fail closed");
    }

    #[test]
    fn a_new_package_absence_is_excused_only_by_first_publish() {
        assert!(absence_is_violation(&Presence::New, false));
        assert!(!absence_is_violation(&Presence::New, true));
    }

    #[test]
    fn an_existing_package_absence_is_always_a_violation() {
        assert!(absence_is_violation(&Presence::Existing, false));
        assert!(absence_is_violation(&Presence::Existing, true));
    }

    #[test]
    fn required_reviewers_are_summed_across_rules() {
        let env = serde_json::json!({
            "protection_rules": [
                { "type": "required_reviewers", "reviewers": [ {"type":"User"}, {"type":"User"} ] },
                { "type": "wait_timer", "wait_timer": 0 }
            ]
        });
        assert_eq!(required_reviewer_count(&env), 2);
    }

    #[test]
    fn required_reviewers_is_zero_when_the_rule_is_absent() {
        let env = serde_json::json!({ "protection_rules": [ { "type": "branch_policy" } ] });
        assert_eq!(required_reviewer_count(&env), 0);
        assert_eq!(required_reviewer_count(&serde_json::json!({})), 0);
    }

    #[test]
    fn branch_policies_extract_name_and_kind() {
        let body = serde_json::json!({
            "branch_policies": [
                { "name": "main", "type": "branch" },
                { "name": "v*", "type": "tag" }
            ]
        });
        let policies = admitted_branch_policies(&body);
        assert!(policies.contains(&("main".to_owned(), "branch".to_owned())));
        assert!(policies.contains(&("v*".to_owned(), "tag".to_owned())));
    }

    #[test]
    fn tag_ruleset_details_fetches_conditions_missing_from_the_listing() {
        const MAIN_ID: u64 = 24_049_042;
        const TAG_ID: u64 = 24_049_043;
        let listing = serde_json::json!([
            { "id": MAIN_ID, "name": "main", "target": "branch", "enforcement": "active" },
            { "id": TAG_ID, "name": "release tags", "target": "tag", "enforcement": "active" }
        ]);
        let mut requested = Vec::new();
        let details = tag_ruleset_details(&listing, |id| {
            requested.push(id);
            serde_json::from_str(include_str!(
                "../../../../.github/rulesets/release-tags.json"
            ))
            .map_err(|err| err.to_string())
        })
        .expect("tag details load");
        assert_eq!(requested, [TAG_ID]);
        assert!(matches!(tag_ruleset_state(&details), TagRuleset::Active));
    }

    #[test]
    fn tag_ruleset_details_propagates_failures_and_rejects_invalid_listings() {
        const TAG_ID: u64 = 24_049_043;
        let listing = serde_json::json!([{ "id": TAG_ID, "target": "tag" }]);
        let error = tag_ruleset_details(&listing, |_| Err("detail unavailable".to_owned()))
            .expect_err("a failed detail fetch must fail closed");
        assert_eq!(error, "detail unavailable");

        for invalid in [
            serde_json::json!({}),
            serde_json::json!([{ "target": "tag" }]),
            serde_json::json!([{ "target": "tag", "id": "not numeric" }]),
        ] {
            tag_ruleset_details(&invalid, |_| Ok(Value::Null))
                .expect_err("invalid input must fail closed");
        }
    }

    #[test]
    fn tag_ruleset_state_classifies_presence_and_enforcement() {
        let active = serde_json::json!([{
            "name": "release tags",
            "target": "tag",
            "enforcement": "active",
            "conditions": { "ref_name": { "include": ["refs/tags/v*"], "exclude": [] } }
        }]);
        assert!(matches!(tag_ruleset_state(&active), TagRuleset::Active));

        let mut renamed = active.clone();
        renamed[0]["name"] = serde_json::json!("immutable versions");
        renamed[0]["conditions"]["ref_name"]["include"] =
            serde_json::json!(["refs/tags/preview-*", "refs/tags/v*"]);
        assert!(matches!(tag_ruleset_state(&renamed), TagRuleset::Active));

        let mut disabled = active.clone();
        disabled[0]["enforcement"] = serde_json::json!("disabled");
        assert!(matches!(tag_ruleset_state(&disabled), TagRuleset::Inactive));

        let mut branch = active.clone();
        branch[0]["name"] = serde_json::json!("release-tags-immutable");
        branch[0]["target"] = serde_json::json!("branch");
        assert!(matches!(tag_ruleset_state(&branch), TagRuleset::Missing));

        let mut other_tags = active.clone();
        other_tags[0]["name"] = serde_json::json!("release-tags-immutable");
        other_tags[0]["conditions"]["ref_name"]["include"] =
            serde_json::json!(["refs/tags/test-*"]);
        assert!(matches!(
            tag_ruleset_state(&other_tags),
            TagRuleset::Missing
        ));

        let mut no_conditions = active;
        no_conditions[0]["conditions"] = Value::Null;
        assert!(matches!(
            tag_ruleset_state(&no_conditions),
            TagRuleset::Missing
        ));

        assert!(matches!(
            tag_ruleset_state(&serde_json::json!([])),
            TagRuleset::NoRulesets
        ));
    }

    #[test]
    fn successful_run_count_requires_completed_and_success() {
        let json = r#"[
            {"status":"completed","conclusion":"success"},
            {"status":"completed","conclusion":"failure"},
            {"status":"in_progress","conclusion":null}
        ]"#;
        assert_eq!(successful_run_count(json).expect("valid json"), 1);
        assert_eq!(successful_run_count("[]").expect("empty is zero"), 0);
        successful_run_count("not json").expect_err("garbage must fail, not read as zero");
    }

    #[test]
    fn parse_names_reads_the_name_field() {
        let names = parse_names(r#"[{"name":"A"},{"name":"B"}]"#).expect("valid json");
        assert!(names.contains("A") && names.contains("B"));
        parse_names("{}").expect_err("a non-array must fail");
    }

    #[test]
    fn release_plz_secret_audit_enforces_the_token_biconditional() {
        let mut names = BTreeSet::new();
        names.insert("RELEASE_PLZ_APP_CLIENT_ID".to_owned());
        names.insert("RELEASE_PLZ_APP_PRIVATE_KEY".to_owned());

        let mut steady = Vec::new();
        audit_release_plz_secrets(&names, false, &mut steady);
        assert!(steady.is_empty(), "steady state must be clean: {steady:?}");

        names.insert("CARGO_REGISTRY_TOKEN".to_owned());
        let mut stray = Vec::new();
        audit_release_plz_secrets(&names, false, &mut stray);
        assert!(
            stray.iter().any(|m| m.contains("CARGO_REGISTRY_TOKEN")),
            "stray token must be flagged: {stray:?}"
        );

        names.remove("CARGO_REGISTRY_TOKEN");
        let mut bootstrap = Vec::new();
        audit_release_plz_secrets(&names, true, &mut bootstrap);
        assert!(
            bootstrap.iter().any(|m| m.contains("CARGO_REGISTRY_TOKEN")),
            "first publish needs the token: {bootstrap:?}"
        );
    }

    #[test]
    fn zero_secrets_is_a_failure_not_a_pass() {
        let mut violations = Vec::new();
        audit_release_plz_secrets(&BTreeSet::new(), false, &mut violations);
        assert!(!violations.is_empty(), "empty secret set must fail on zero");
    }
}
