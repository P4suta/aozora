#![expect(
    clippy::expect_used,
    reason = "publish checks operate on validated workspace manifest structures"
)]

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use regex::Regex;
use serde::Deserialize;
use serde::de::DeserializeOwned;

use crate::PublishArgs;
use crate::PublishOp;

#[derive(Deserialize)]
struct RootManifest {
    workspace: WorkspaceTable,
}

#[derive(Deserialize)]
struct WorkspaceTable {
    members: Vec<String>,
    #[serde(default)]
    dependencies: BTreeMap<String, toml::Value>,
}

#[derive(Deserialize)]
struct CrateManifest {
    package: PackageTable,
    #[serde(default, rename = "dev-dependencies")]
    dev_dependencies: BTreeMap<String, toml::Value>,
}

#[derive(Deserialize)]
struct PackageTable {
    name: String,
    #[serde(default)]
    publish: Option<toml::Value>,
}

#[derive(Deserialize)]
struct JsonPackage {
    version: String,
}

#[derive(Deserialize)]
struct ReleasePlzManifest {
    #[serde(default)]
    package: Vec<ReleasePlzPackage>,
}

#[derive(Deserialize)]
struct ReleasePlzPackage {
    name: String,
    #[serde(default)]
    git_tag_enable: Option<bool>,
    #[serde(default)]
    changelog_update: Option<bool>,
    #[serde(default)]
    changelog_include: Option<Vec<String>>,
}

struct Ledger {
    publishable: Vec<String>,
    unpublishable: Vec<String>,
    release_identities: Vec<String>,
    changelog_include: Vec<String>,
    versioned_internal_dev_deps: Vec<(String, String)>,
    versioned_unpublishable_workspace_deps: Vec<String>,
    non_placeholder_package_versions: Vec<(String, String)>,
    readme_missing: Vec<String>,
    readme_relative_links: Vec<(String, String)>,
}

pub(crate) fn dispatch(args: &PublishArgs) -> Result<(), String> {
    match args.op {
        PublishOp::Check => check(),
    }
}

fn workspace_root() -> Result<PathBuf, String> {
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    let root = Path::new(manifest_dir)
        .parent()
        .and_then(Path::parent)
        .ok_or_else(|| {
            format!("could not derive workspace root from CARGO_MANIFEST_DIR={manifest_dir:?}")
        })?;
    Ok(root.to_path_buf())
}

fn read_toml<T: DeserializeOwned>(path: &Path) -> Result<T, String> {
    let text = fs::read_to_string(path).map_err(|err| format!("read {}: {err}", path.display()))?;
    toml::from_str(&text).map_err(|err| format!("parse {}: {err}", path.display()))
}

fn read_json<T: DeserializeOwned>(path: &Path) -> Result<T, String> {
    let text = fs::read_to_string(path).map_err(|err| format!("read {}: {err}", path.display()))?;
    serde_json::from_str(&text).map_err(|err| format!("parse {}: {err}", path.display()))
}

fn publish_is_disabled(value: Option<&toml::Value>) -> bool {
    match value {
        Some(toml::Value::Boolean(flag)) => !flag,
        Some(toml::Value::Array(registries)) => registries.is_empty(),
        _ => false,
    }
}

fn entry_states_version(entry: &toml::Value) -> bool {
    match entry {
        toml::Value::String(_) => true,
        toml::Value::Table(table) => table.contains_key("version"),
        _ => false,
    }
}

fn resolves_to_version(
    name: &str,
    entry: &toml::Value,
    workspace_deps: &BTreeMap<String, toml::Value>,
) -> bool {
    let inherits = entry
        .as_table()
        .and_then(|table| table.get("workspace"))
        .and_then(toml::Value::as_bool)
        == Some(true);
    if inherits {
        workspace_deps.get(name).is_some_and(entry_states_version)
    } else {
        entry_states_version(entry)
    }
}

fn is_repo_relative_target(target: &str) -> bool {
    let t = target.trim();
    if t.is_empty() || t.starts_with('#') || t.starts_with("//") {
        return false;
    }
    if let Some(colon) = t.find(':') {
        let scheme = &t[..colon];
        let is_scheme = scheme
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_alphabetic())
            && scheme
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'));
        if is_scheme {
            return false;
        }
    }
    true
}

fn repo_relative_link_lines(readme: &str) -> Vec<String> {
    let inline = Regex::new(r"\]\(\s*<?([^)\s>]+)").expect("static inline-link regex");
    let html =
        Regex::new(r#"(?:href|src)\s*=\s*["']([^"']+)["']"#).expect("static html-link regex");
    let reference =
        Regex::new(r"(?m)^\s*\[[^\]]+\]:\s*<?([^\s>]+)").expect("static ref-link regex");

    let mut hits = Vec::new();
    for line in readme.lines() {
        let flagged = [&inline, &html, &reference].iter().any(|re| {
            re.captures_iter(line)
                .any(|cap| is_repo_relative_target(&cap[1]))
        });
        if flagged {
            hits.push(line.trim().to_owned());
        }
    }
    hits
}

fn readme_hygiene(
    root: &Path,
    members: &[(CrateManifest, String)],
) -> (Vec<String>, Vec<(String, String)>) {
    let mut missing = Vec::new();
    let mut relative = Vec::new();
    for (member, rel) in members {
        if publish_is_disabled(member.package.publish.as_ref()) {
            continue;
        }
        match fs::read_to_string(root.join(rel).join("README.md")) {
            Err(_) => missing.push(member.package.name.clone()),
            Ok(text) => {
                for line in repo_relative_link_lines(&text) {
                    relative.push((member.package.name.clone(), line));
                }
            }
        }
    }
    (missing, relative)
}

fn distribution_versions(root: &Path) -> Result<Vec<(String, String)>, String> {
    const PACKAGE_PATHS: [&str; 3] = [
        "crates/tree-sitter-aozora/package.json",
        "editors/vscode/package.json",
        "playground/package.json",
    ];
    let versions = PACKAGE_PATHS
        .iter()
        .map(|rel| {
            let package: JsonPackage = read_json(&root.join(rel))?;
            Ok((*rel, package.version))
        })
        .collect::<Result<Vec<_>, String>>()?;
    Ok(versions
        .into_iter()
        .filter(|(_, version)| version != "0.0.0")
        .map(|(path, version)| (path.to_owned(), version))
        .collect())
}

fn load(root: &Path) -> Result<Ledger, String> {
    let manifest: RootManifest = read_toml(&root.join("Cargo.toml"))?;
    let workspace_deps = &manifest.workspace.dependencies;

    let members: Vec<(CrateManifest, String)> = manifest
        .workspace
        .members
        .iter()
        .map(|rel| {
            let path = root.join(rel).join("Cargo.toml");
            read_toml::<CrateManifest>(&path).map(|m| (m, rel.clone()))
        })
        .collect::<Result<_, _>>()?;

    let mut publishable = Vec::new();
    let mut unpublishable = Vec::new();
    for (member, _) in &members {
        if publish_is_disabled(member.package.publish.as_ref()) {
            unpublishable.push(member.package.name.clone());
        } else {
            publishable.push(member.package.name.clone());
        }
    }

    let (readme_missing, readme_relative_links) = readme_hygiene(root, &members);

    let internal: Vec<&str> = members
        .iter()
        .map(|(m, _)| m.package.name.as_str())
        .collect();

    let mut versioned_internal_dev_deps = Vec::new();
    for (member, _) in &members {
        if publish_is_disabled(member.package.publish.as_ref()) {
            continue;
        }
        for (dep_name, entry) in &member.dev_dependencies {
            if internal.contains(&dep_name.as_str())
                && resolves_to_version(dep_name, entry, workspace_deps)
            {
                versioned_internal_dev_deps.push((member.package.name.clone(), dep_name.clone()));
            }
        }
    }

    let mut versioned_unpublishable_workspace_deps = Vec::new();
    for name in &unpublishable {
        if workspace_deps.get(name).is_some_and(entry_states_version) {
            versioned_unpublishable_workspace_deps.push(name.clone());
        }
    }

    let release_plz: ReleasePlzManifest = read_toml(&root.join("release-plz.toml"))?;
    let release_identities: Vec<String> = release_plz
        .package
        .iter()
        .filter(|pkg| pkg.changelog_update == Some(true) && pkg.git_tag_enable == Some(true))
        .map(|pkg| pkg.name.clone())
        .collect();
    let changelog_include = release_plz
        .package
        .iter()
        .find_map(|pkg| pkg.changelog_include.clone())
        .unwrap_or_default();

    let non_placeholder_package_versions = distribution_versions(root)?;

    Ok(Ledger {
        publishable,
        unpublishable,
        release_identities,
        changelog_include,
        versioned_internal_dev_deps,
        versioned_unpublishable_workspace_deps,
        non_placeholder_package_versions,
        readme_missing,
        readme_relative_links,
    })
}

fn verify(ledger: &Ledger) -> Result<(), Vec<String>> {
    let mut violations = Vec::new();
    check_public_packages(ledger, &mut violations);
    let identity = release_identity(ledger, &mut violations);
    check_changelog_include(ledger, identity, &mut violations);
    check_manifest_hygiene(ledger, &mut violations);
    check_distribution_versions(ledger, &mut violations);
    check_readme_hygiene(ledger, &mut violations);

    if violations.is_empty() {
        Ok(())
    } else {
        Err(violations)
    }
}

fn check_readme_hygiene(ledger: &Ledger, violations: &mut Vec<String>) {
    if !ledger.readme_missing.is_empty() {
        violations.push(format!(
            "{} publishable crate(s) have no README.md (the crates.io page would be blank):\n    {}",
            ledger.readme_missing.len(),
            ledger.readme_missing.join("\n    "),
        ));
    }
    if !ledger.readme_relative_links.is_empty() {
        let listed: Vec<String> = ledger
            .readme_relative_links
            .iter()
            .map(|(krate, line)| format!("{krate}: {line}"))
            .collect();
        violations.push(format!(
            "{} repo-relative link(s) in a publishable crate README (they 404 on crates.io / \
             docs.rs — use absolute https URLs):\n    {}",
            ledger.readme_relative_links.len(),
            listed.join("\n    "),
        ));
    }
}

fn check_distribution_versions(ledger: &Ledger, violations: &mut Vec<String>) {
    if ledger.non_placeholder_package_versions.is_empty() {
        return;
    }
    let entries = ledger
        .non_placeholder_package_versions
        .iter()
        .map(|(path, version)| format!("{path}: {version}"))
        .collect::<Vec<_>>();
    violations.push(format!(
        "source package manifests must use the release-injected `0.0.0` placeholder:\n    {}",
        entries.join("\n    ")
    ));
}

fn check_public_packages(ledger: &Ledger, violations: &mut Vec<String>) {
    let mut actual = ledger.publishable.clone();
    actual.sort_unstable();
    let expected = [
        "aozora".to_owned(),
        "aozora-cli".to_owned(),
        "tree-sitter-aozora".to_owned(),
    ];
    if actual != expected {
        violations.push(format!(
            "publishable workspace members differ from the supported public packages:\n    {}",
            actual.join("\n    ")
        ));
    }
}

fn release_identity<'a>(ledger: &'a Ledger, violations: &mut Vec<String>) -> Option<&'a str> {
    match ledger.release_identities.as_slice() {
        [one] => {
            let identity = one.as_str();
            if !ledger.publishable.iter().any(|name| name == identity) {
                violations.push(format!(
                    "release identity `{identity}` is not a publishable workspace member"
                ));
            }
            Some(identity)
        }
        [] => {
            violations.push(
                "no release identity: release-plz.toml has no [[package]] with both \
                 `changelog_update = true` and `git_tag_enable = true`, so no `vX.Y.Z` \
                 tag would ever be cut"
                    .to_owned(),
            );
            None
        }
        many => {
            violations.push(format!(
                "{} release identities in release-plz.toml (exactly one [[package]] may set \
                 both `changelog_update` and `git_tag_enable`):\n    {}",
                many.len(),
                many.join("\n    "),
            ));
            None
        }
    }
}

fn check_changelog_include(ledger: &Ledger, identity: Option<&str>, violations: &mut Vec<String>) {
    if let Some(identity) = identity {
        let missing: Vec<&str> = ledger
            .publishable
            .iter()
            .map(String::as_str)
            .filter(|name| *name != identity)
            .filter(|name| !ledger.changelog_include.iter().any(|inc| inc == name))
            .collect();
        if !missing.is_empty() {
            violations.push(format!(
                "release-plz.toml `changelog_include` is missing {} publishable crate(s):\n    {}\n\
                 -> add them to the changelog_include list in release-plz.toml (keep it sorted)",
                missing.len(),
                missing.join("\n    "),
            ));
        }

        if ledger.changelog_include.iter().any(|inc| inc == identity) {
            violations.push(format!(
                "release-plz.toml `changelog_include` contains the release identity \
                 `{identity}` itself, which would duplicate its commits in the changelog"
            ));
        }
    }

    let stray: Vec<&str> = ledger
        .changelog_include
        .iter()
        .map(String::as_str)
        .filter(|inc| !ledger.publishable.iter().any(|name| name == inc))
        .filter(|inc| Some(*inc) != identity)
        .collect();
    if !stray.is_empty() {
        violations.push(format!(
            "release-plz.toml `changelog_include` names {} crate(s) that are not publishable \
             workspace members:\n    {}\n\
             -> drop them from release-plz.toml, or check for a rename",
            stray.len(),
            stray.join("\n    "),
        ));
    }

    let mut canonical = ledger.changelog_include.clone();
    canonical.sort_unstable();
    canonical.dedup();
    if canonical != ledger.changelog_include {
        violations.push(format!(
            "release-plz.toml `changelog_include` is not sorted + deduplicated; it should read:\n    {}",
            canonical.join("\n    "),
        ));
    }
}

fn check_manifest_hygiene(ledger: &Ledger, violations: &mut Vec<String>) {
    if !ledger.versioned_internal_dev_deps.is_empty() {
        let listed: Vec<String> = ledger
            .versioned_internal_dev_deps
            .iter()
            .map(|(krate, dep)| format!("{krate}: dev-dependency `{dep}`"))
            .collect();
        violations.push(format!(
            "{} publishable crate(s) carry a versioned internal dev-dependency:\n    {}\n\
             -> declare it path-only at the use site (`{{ path = \"../<crate>\" }}`) so cargo \
             strips it from the published manifest; see the [workspace.dependencies] note in \
             the root Cargo.toml",
            ledger.versioned_internal_dev_deps.len(),
            listed.join("\n    "),
        ));
    }

    if !ledger.versioned_unpublishable_workspace_deps.is_empty() {
        violations.push(format!(
            "{} [workspace.dependencies] entr(y/ies) pin a `version` on a `publish = false` \
             member:\n    {}\n\
             -> make them path-only (no version); the crate is never released, so the pin can \
             only go stale",
            ledger.versioned_unpublishable_workspace_deps.len(),
            ledger.versioned_unpublishable_workspace_deps.join("\n    "),
        ));
    }
}

fn check() -> Result<(), String> {
    let root = workspace_root()?;
    let ledger = load(&root)?;
    match verify(&ledger) {
        Ok(()) => {
            let folded = ledger.changelog_include.len();
            let expected = ledger.publishable.len().saturating_sub(1);
            eprintln!(
                "xtask publish check: {folded}/{expected} publishable crates folded into the \
                 aggregated changelog, {} publish=false skipped; {} README(s) link-checked",
                ledger.unpublishable.len(),
                ledger.publishable.len(),
            );
            Ok(())
        }
        Err(violations) => Err(format!(
            "publish ledger drift detected:\n  {}",
            violations.join("\n  "),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn table(src: &str) -> toml::Value {
        toml::from_str::<toml::Value>(src).expect("test fixture must be valid TOML")
    }

    fn healthy() -> Ledger {
        Ledger {
            publishable: vec![
                "aozora".to_owned(),
                "aozora-cli".to_owned(),
                "tree-sitter-aozora".to_owned(),
            ],
            unpublishable: vec!["aozora-xtask".to_owned()],
            release_identities: vec!["aozora".to_owned()],
            changelog_include: vec!["aozora-cli".to_owned(), "tree-sitter-aozora".to_owned()],
            versioned_internal_dev_deps: Vec::new(),
            versioned_unpublishable_workspace_deps: Vec::new(),
            non_placeholder_package_versions: Vec::new(),
            readme_missing: Vec::new(),
            readme_relative_links: Vec::new(),
        }
    }

    #[test]
    fn verify_accepts_a_healthy_ledger() {
        assert_eq!(verify(&healthy()), Ok(()), "healthy ledger must pass");
    }

    #[test]
    fn verify_flags_a_publishable_crate_missing_from_changelog_include() {
        let mut ledger = healthy();
        ledger
            .changelog_include
            .retain(|name| name != "tree-sitter-aozora");
        let err = verify(&ledger).expect_err("missing crate must be flagged");
        assert!(
            err.iter()
                .any(|v| v.contains("tree-sitter-aozora") && v.contains("missing")),
            "must name the missing crate: {err:?}"
        );
    }

    #[test]
    fn verify_flags_a_stray_changelog_include_entry() {
        let mut ledger = healthy();
        ledger
            .changelog_include
            .push("aozora-renamed-away".to_owned());
        let err = verify(&ledger).expect_err("stray entry must be flagged");
        assert!(
            err.iter().any(|v| v.contains("aozora-renamed-away")),
            "must name the stray entry: {err:?}"
        );
    }

    #[test]
    fn verify_rejects_the_umbrella_including_itself() {
        let mut ledger = healthy();
        ledger.changelog_include.insert(0, "aozora".to_owned());
        let err = verify(&ledger).expect_err("self-inclusion must be flagged");
        assert!(
            err.iter().any(|v| v.contains("itself")),
            "must call out self-inclusion: {err:?}"
        );
    }

    #[test]
    fn verify_rejects_zero_release_identities() {
        let mut ledger = healthy();
        ledger.release_identities.clear();
        let err = verify(&ledger).expect_err("a missing identity must be flagged");
        assert!(
            err.iter().any(|v| v.contains("no release identity")),
            "must call out the missing identity: {err:?}"
        );
    }

    #[test]
    fn verify_rejects_two_release_identities() {
        let mut ledger = healthy();
        ledger.release_identities.push("aozora-fmt".to_owned());
        let err = verify(&ledger).expect_err("two identities must be flagged");
        assert!(
            err.iter().any(|v| v.contains("2 release identities")),
            "must count the identities: {err:?}"
        );
    }

    #[test]
    fn verify_rejects_an_identity_that_is_not_publishable() {
        let mut ledger = healthy();
        ledger.release_identities = vec!["aozora-xtask".to_owned()];
        let err = verify(&ledger).expect_err("unpublishable identity must be flagged");
        assert!(
            err.iter().any(|v| v.contains("not a publishable")),
            "must say the identity is unpublishable: {err:?}"
        );
    }

    #[test]
    fn verify_flags_an_unsorted_changelog_include() {
        let mut ledger = healthy();
        ledger.changelog_include.reverse();
        let err = verify(&ledger).expect_err("unsorted list must be flagged");
        assert!(
            err.iter().any(|v| v.contains("sorted")),
            "must call out the ordering: {err:?}"
        );
    }

    #[test]
    fn verify_flags_a_duplicated_changelog_include_entry() {
        let mut ledger = healthy();
        ledger.changelog_include.push("aozora-i18n".to_owned());
        let err = verify(&ledger).expect_err("duplicate must be flagged");
        assert!(
            err.iter().any(|v| v.contains("sorted")),
            "must call out the duplicate: {err:?}"
        );
    }

    #[test]
    fn verify_flags_a_versioned_internal_dev_dep() {
        let mut ledger = healthy();
        ledger
            .versioned_internal_dev_deps
            .push(("aozora-lsp".to_owned(), "aozora-fmt".to_owned()));
        let err = verify(&ledger).expect_err("versioned internal dev-dep must be flagged");
        assert!(
            err.iter()
                .any(|v| v.contains("aozora-lsp") && v.contains("path-only")),
            "must name the crate and the fix: {err:?}"
        );
    }

    #[test]
    fn verify_flags_a_versioned_unpublishable_workspace_dep() {
        let mut ledger = healthy();
        ledger
            .versioned_unpublishable_workspace_deps
            .push("aozora-trace".to_owned());
        let err = verify(&ledger).expect_err("versioned publish=false dep must be flagged");
        assert!(
            err.iter().any(|v| v.contains("aozora-trace")),
            "must name the entry: {err:?}"
        );
    }

    #[test]
    fn verify_reports_every_violation_at_once() {
        let mut ledger = healthy();
        ledger
            .changelog_include
            .retain(|name| name != "tree-sitter-aozora");
        ledger
            .versioned_internal_dev_deps
            .push(("aozora-lsp".to_owned(), "aozora-fmt".to_owned()));
        let err = verify(&ledger).expect_err("both faults must be flagged");
        assert_eq!(err.len(), 2, "one run must surface both: {err:?}");
    }

    #[test]
    fn verify_rejects_a_handwritten_distribution_version() {
        let mut ledger = healthy();
        ledger
            .non_placeholder_package_versions
            .push(("editors/vscode/package.json".to_owned(), "1.2.3".to_owned()));
        let err = verify(&ledger).expect_err("handwritten distribution version must be flagged");
        assert!(err.iter().any(|v| v.contains("0.0.0")), "{err:?}");
    }

    #[test]
    fn publish_is_disabled_reads_both_spellings() {
        assert!(
            publish_is_disabled(Some(&table("v = false")["v"])),
            "`publish = false` disables publishing"
        );
        assert!(
            publish_is_disabled(Some(&table("v = []")["v"])),
            "an empty registry list disables publishing"
        );
        assert!(
            !publish_is_disabled(Some(&table("v = true")["v"])),
            "`publish = true` publishes"
        );
        assert!(
            !publish_is_disabled(Some(&table(r#"v = ["crates-io"]"#)["v"])),
            "a non-empty registry list publishes"
        );
        assert!(
            !publish_is_disabled(None),
            "an absent key publishes — that is cargo's default"
        );
    }

    #[test]
    fn entry_states_version_reads_both_dependency_spellings() {
        assert!(
            entry_states_version(&table(r#"d = "1.0""#)["d"]),
            "the bare-version shorthand states a version"
        );
        assert!(
            entry_states_version(&table(r#"d = { version = "1.0", path = "../x" }"#)["d"]),
            "an explicit version states a version"
        );
        assert!(
            !entry_states_version(&table(r#"d = { path = "../x" }"#)["d"]),
            "path-only states no version"
        );
    }

    #[test]
    fn workspace_true_inherits_the_shared_entry_version() {
        let mut workspace_deps = BTreeMap::new();
        workspace_deps.insert(
            "aozora-fmt".to_owned(),
            table(r#"d = { version = "1.2.3", path = "crates/aozora-fmt" }"#)["d"].clone(),
        );
        workspace_deps.insert(
            "aozora-corpus".to_owned(),
            table(r#"d = { path = "crates/aozora-corpus" }"#)["d"].clone(),
        );
        let inherit = table("d = { workspace = true }")["d"].clone();

        assert!(
            resolves_to_version("aozora-fmt", &inherit, &workspace_deps),
            "inheriting a versioned entry resolves to a version"
        );
        assert!(
            !resolves_to_version("aozora-corpus", &inherit, &workspace_deps),
            "inheriting a path-only entry resolves to no version"
        );
    }

    #[test]
    fn is_repo_relative_target_classifies_targets() {
        for absolute in [
            "https://example.com/x",
            "http://example.com",
            "mailto:a@b.example",
            "tel:+123",
            "data:text/plain,hi",
            "#in-page-anchor",
            "//cdn.example.com/x",
        ] {
            assert!(
                !is_repo_relative_target(absolute),
                "{absolute} must be treated as absolute/anchor"
            );
        }
        for relative in [
            "CONTRIBUTING.md",
            "./docs/x.md",
            "../sibling/y.md",
            "docs/guide.md",
            "/site-absolute/z",
            "LICENSE",
        ] {
            assert!(
                is_repo_relative_target(relative),
                "{relative} must be flagged repo-relative"
            );
        }
    }

    #[test]
    fn repo_relative_link_lines_catches_the_forms_the_prefix_scan_missed() {
        let readme = "\
            See [contributing](CONTRIBUTING.md) and [home](https://aozora.example).\n\
            An <a href=\"docs/guide.md\">html link</a> and an ![img](../logo.png).\n\
            A clean [spec][s] line.\n\
            \n\
            [s]: https://spec.example/v1\n\
            [old]: legacy/notes.md\n";
        let hits = repo_relative_link_lines(readme);
        assert!(
            hits.iter().any(|l| l.contains("CONTRIBUTING.md")),
            "bare inline relative link must be caught: {hits:?}"
        );
        assert!(
            hits.iter().any(|l| l.contains("docs/guide.md")),
            "html href relative link must be caught: {hits:?}"
        );
        assert!(
            hits.iter().any(|l| l.contains("../logo.png")),
            "relative image src must be caught: {hits:?}"
        );
        assert!(
            hits.iter().any(|l| l.contains("legacy/notes.md")),
            "reference-definition relative target must be caught: {hits:?}"
        );
        assert!(
            !hits.iter().any(|l| l.contains("clean [spec][s]")),
            "a line whose only link resolves absolutely must not be flagged: {hits:?}"
        );
    }

    #[test]
    fn verify_flags_a_missing_readme() {
        let mut ledger = healthy();
        ledger.readme_missing.push("aozora-cli".to_owned());
        let err = verify(&ledger).expect_err("a missing README must be flagged");
        assert!(
            err.iter()
                .any(|v| v.contains("aozora-cli") && v.contains("README")),
            "must name the crate and README: {err:?}"
        );
    }

    #[test]
    fn verify_flags_a_repo_relative_readme_link() {
        let mut ledger = healthy();
        ledger
            .readme_relative_links
            .push(("aozora".to_owned(), "[x](CONTRIBUTING.md)".to_owned()));
        let err = verify(&ledger).expect_err("a repo-relative README link must be flagged");
        assert!(
            err.iter()
                .any(|v| v.contains("CONTRIBUTING.md") && v.contains("404")),
            "must show the offending link and why: {err:?}"
        );
    }

    #[test]
    fn the_live_ledger_loads_and_passes() {
        let root = workspace_root().expect("workspace root");
        let ledger = load(&root).expect("the live manifests must parse");
        assert!(
            ledger.publishable.len() > 1,
            "the workspace must have publishable members"
        );
        assert_eq!(
            verify(&ledger),
            Ok(()),
            "the committed ledger must satisfy every invariant"
        );
    }
}
