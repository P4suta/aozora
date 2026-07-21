//! `xtask release preflight` — verify the rearm preconditions, fail closed.
//!
//! Two tiers. `--offline` runs only what the working tree can answer: the
//! `check` source-integrity gate, plus the publication-freeze latch, which the
//! rearm PR must already have removed (`release.md`). The default tier adds the
//! deployed state CI-green can never prove — the `release-plz` environment
//! secrets, the protected `release` environment, the server-side tag ruleset,
//! and a completed-success `release-ready` run for the exact commit — then
//! probes the three package registries to classify the first-publish residue.
//! Every check fails on an empty subject, never silently; what no API can
//! verify is printed as explicit acknowledgments against `releasing-secrets.md`.

use std::collections::BTreeSet;
use std::env;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::Value;

/// What to verify. Grouped into a struct rather than passed as loose flags so
/// the online/offline split and the first-publish acknowledgment stay named.
pub(super) struct Request<'a> {
    /// Skip every network probe — run only the repo-local checks.
    pub offline: bool,
    /// Acknowledge a known first-publish (a new crate / project the registry
    /// cannot auto-create), so preflight does not hard-stop on it.
    pub first_publish: bool,
    /// The commit being rearmed (defaults to HEAD when `None`).
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

// ── offline tier ─────────────────────────────────────────────────────────

/// Fold the offline source-integrity gate in, surfacing any failure as a
/// preflight violation rather than a separate early exit, so every unmet
/// precondition is reported together.
fn check_source_integrity(violations: &mut Vec<String>) {
    if let Err(err) = super::ruleset::check() {
        violations.push(err);
    }
}

/// The publication-freeze latch must be ABSENT at rearm — it is removed in the
/// dedicated reviewed rearm PR (`release.md`, "Publication freeze"). A present
/// latch is a hard stop: rearming through it would defeat the freeze.
fn check_freeze_latch(root: &Path, violations: &mut Vec<String>) {
    if root.join(".github/RELEASE_FROZEN.md").exists() {
        violations.push(
            "publication freeze latch .github/RELEASE_FROZEN.md is still present — \
             remove it in the dedicated reviewed rearm PR before rearming (release.md)"
                .to_owned(),
        );
    }
}

// ── online tier ──────────────────────────────────────────────────────────

/// The `owner/name` repo and 40-hex commit every deployed-state probe is scoped
/// to. Resolved once; a failure here fails the whole online tier closed.
struct Context {
    /// `owner/name`, from `GITHUB_REPOSITORY` or `gh repo view`.
    repo: String,
    /// The 40-hex commit being rearmed (request override or HEAD).
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

/// Prefer the Actions-provided `GITHUB_REPOSITORY`; fall back to `gh` so the
/// same tool works from a maintainer's checkout.
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

/// The rearm commit is the exact qualified 40-hex merge commit (`release.md`);
/// anything shorter or non-hex is rejected so a probe cannot resolve the wrong
/// commit.
fn resolve_commit(request: &Request<'_>, root: &Path) -> Result<String, String> {
    let commit = match request.commit {
        Some(commit) => commit.to_owned(),
        None => git(root, &["rev-parse", "HEAD"])?,
    };
    if commit.len() != 40 || !commit.bytes().all(|b| b.is_ascii_hexdigit()) {
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

// ── release-plz environment secrets (releasing-secrets.md §2/§3) ──────────

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

/// The App credentials must both be present, and `CARGO_REGISTRY_TOKEN` present
/// iff a first publish is in flight (steady state is tokenless OIDC). Zero
/// secrets is a failure, not a pass — an absent environment and a stripped one
/// must not read alike.
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

// ── release environment protection (releasing-secrets.md §1) ──────────────

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

/// How many reviewers the environment's `required_reviewers` rules name in
/// total; zero means the human approval gate is not enforced.
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

/// Both `main` (branch) and `v*` (tag) must be admitted. Zero admitted policies
/// is a failure, not a pass — an empty policy set is as unprotected as a
/// missing environment.
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

/// The `(name, type)` pairs a deployment-branch-policies listing admits.
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

// ── server-side tag ruleset (releasing-secrets.md §2) ─────────────────────

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
    match serde_json::from_str::<Value>(&body) {
        Ok(value) => audit_tag_ruleset(&value, violations),
        Err(err) => violations.push(format!("parse rulesets: {err}")),
    }
}

/// Whether the immutable-tag ruleset is present and active server-side.
enum TagRuleset {
    /// The rulesets listing is empty — fail-on-zero: nothing to verify.
    NoRulesets,
    /// No ruleset named like `release-tags-immutable` is applied.
    Missing,
    /// Present, but its enforcement is not `active`.
    Inactive,
    /// Present and active.
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
            "no release-tags-immutable ruleset is applied server-side (releasing-secrets.md §2)"
                .to_owned(),
        ),
        TagRuleset::Inactive => violations.push(
            "the release-tags-immutable ruleset exists but is not active \
             (releasing-secrets.md §2)"
                .to_owned(),
        ),
    }
}

/// Classify the tag ruleset from a `GET /repos/{repo}/rulesets` listing.
fn tag_ruleset_state(rulesets: &Value) -> TagRuleset {
    let Some(list) = rulesets.as_array() else {
        return TagRuleset::NoRulesets;
    };
    if list.is_empty() {
        return TagRuleset::NoRulesets;
    }
    let found = list.iter().find(|ruleset| {
        ruleset
            .get("name")
            .and_then(Value::as_str)
            .is_some_and(|name| name.contains("release-tags"))
    });
    match found {
        None => TagRuleset::Missing,
        Some(ruleset) if ruleset.get("enforcement").and_then(Value::as_str) == Some("active") => {
            TagRuleset::Active
        }
        Some(_) => TagRuleset::Inactive,
    }
}

// ── release-ready proof for the commit (release.md; ADR-0042) ─────────────

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

/// How many runs in a `gh run list --json status,conclusion` listing are both
/// `completed` and `success`. Zero is the fail-on-zero case the caller rejects.
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

// ── registry existence — first-publish residue (releasing-secrets.md §3-5) ─

/// Whether a package is expected to already exist in steady state, which
/// governs when its absence is a violation versus an acknowledged first publish.
enum Presence {
    /// Not yet on the registry; only a first publish creates it.
    New,
    /// Expected to already exist; absence is always a failure.
    Existing,
}

/// One registry existence probe: a plain HTTPS GET whose 404 means "not yet".
struct Registry<'a> {
    /// Human label for the message.
    label: &'a str,
    /// The URL whose HTTP status classifies existence.
    url: &'a str,
    /// Steady-state expectation.
    presence: Presence,
    /// How to resolve an unexpected absence.
    fix: &'a str,
}

fn check_registries(first_publish: bool, violations: &mut Vec<String>) {
    let registries = [
        Registry {
            label: "crates.io crate tree-sitter-aozora",
            url: "https://crates.io/api/v1/crates/tree-sitter-aozora",
            presence: Presence::New,
            fix: "a new crate needs the CARGO_REGISTRY_TOKEN bootstrap before it can publish, \
                  so it can never silently proceed (releasing-secrets.md §3)",
        },
        Registry {
            label: "PyPI project aozora",
            url: "https://pypi.org/pypi/aozora/json",
            presence: Presence::New,
            fix: "the pending publisher creates it on first OIDC upload (releasing-secrets.md §4)",
        },
        Registry {
            label: "npm package aozora-wasm",
            url: "https://registry.npmjs.org/aozora-wasm",
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

/// A first publish excuses only a not-yet-created package; an existing package
/// missing is always a failure, and a new one missing outside a first publish
/// must never proceed silently.
fn absence_is_violation(presence: &Presence, first_publish: bool) -> bool {
    match presence {
        Presence::Existing => true,
        Presence::New => !first_publish,
    }
}

/// Whether a registry package exists, from an HTTP status.
#[derive(Debug)]
enum RegistryState {
    /// 200 — the package exists.
    Exists,
    /// 404 — the package does not exist yet.
    Absent,
}

fn http_status(url: &str) -> Result<RegistryState, String> {
    // crates.io's data-access policy REQUIRES a descriptive User-Agent and
    // rejects the default `curl/x.y` one with 403 — which would otherwise fail
    // every online preflight closed on a spurious "unexpected HTTP status".
    // Identify the tool for all three registries.
    let output = Command::new("curl")
        .args([
            "-sS",
            "-A",
            "aozora-xtask release-preflight (+https://github.com/P4suta/aozora)",
            "-o",
            "/dev/null",
            "-w",
            "%{http_code}",
            "--max-time",
            "30",
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

/// Map an HTTP status body to existence. Only 200 and 404 are expected; any
/// other status fails closed rather than being guessed at.
fn classify_http_status(code: &str) -> Result<RegistryState, String> {
    match code.trim() {
        "200" => Ok(RegistryState::Exists),
        "404" => Ok(RegistryState::Absent),
        other => Err(format!(
            "unexpected HTTP status {other:?} from a registry probe"
        )),
    }
}

// ── success reporting + irreducible residue ──────────────────────────────

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

/// The residue no API can verify — printed as named acknowledgments so it is
/// impossible to conflate "verified" with "unverifiable".
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

// ── process helpers ──────────────────────────────────────────────────────

/// A `gh api` call that may legitimately 404 (an absent environment / ruleset),
/// which must be classified rather than mistaken for a transport failure.
enum GhApi {
    /// The response body.
    Found(String),
    /// The resource returned 404.
    Missing,
}

/// Run a `gh` subcommand, failing loud on any non-zero exit.
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

/// Run `gh api <args>`, distinguishing a 404 (a verifiable "absent") from a
/// real failure so an unprotected resource is caught, not swallowed.
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

/// The names in a `gh secret list --json name` listing.
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
    fn tag_ruleset_state_classifies_presence_and_enforcement() {
        let active =
            serde_json::json!([{ "name": "release-tags-immutable", "enforcement": "active" }]);
        assert!(matches!(tag_ruleset_state(&active), TagRuleset::Active));

        let disabled =
            serde_json::json!([{ "name": "release-tags-immutable", "enforcement": "disabled" }]);
        assert!(matches!(tag_ruleset_state(&disabled), TagRuleset::Inactive));

        let other = serde_json::json!([{ "name": "main-branch", "enforcement": "active" }]);
        assert!(matches!(tag_ruleset_state(&other), TagRuleset::Missing));

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

        // Steady state: both App creds, no token -> clean.
        let mut steady = Vec::new();
        audit_release_plz_secrets(&names, false, &mut steady);
        assert!(steady.is_empty(), "steady state must be clean: {steady:?}");

        // A stray token outside a first publish is a violation.
        names.insert("CARGO_REGISTRY_TOKEN".to_owned());
        let mut stray = Vec::new();
        audit_release_plz_secrets(&names, false, &mut stray);
        assert!(
            stray.iter().any(|m| m.contains("CARGO_REGISTRY_TOKEN")),
            "stray token must be flagged: {stray:?}"
        );

        // First publish without the token is also a violation.
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
