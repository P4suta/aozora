//! crates.io publish-path ledger drift gate.
//!
//! Cross-checks the workspace's publishable members against
//! `release-plz.toml`'s `changelog_include`, and enforces the manifest
//! hygiene rules the root `Cargo.toml` already states in prose:
//! path-only internal dev-deps, and no registry `version` on an entry
//! that points at a `publish = false` member.
//!
//! **Offline.** It reads manifests and never contacts a registry —
//! whether a crate actually exists on crates.io is a registry fact, and
//! wiring that into a blocking gate would make the build depend on a
//! third party's uptime (the same call `lychee.toml` documents for
//! external links). The release runbook derives that fact on demand.
//!
//! ## Why this exists
//!
//! `aozora-i18n` joined the workspace publishable by default, but
//! `release-plz.toml`'s `changelog_include` was never updated — so its
//! commits would have silently vanished from the aggregated CHANGELOG.
//! The release runbook drifted the same way, twice. A ledger that three
//! files have to agree on is a ledger a machine should check.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde::Deserialize;
use serde::de::DeserializeOwned;

use crate::PublishArgs;
use crate::PublishOp;

/// Root manifest — the workspace member list and the shared dependency
/// table that `workspace = true` entries inherit from.
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

/// A member manifest. Only the fields the ledger reasons about; serde
/// ignores the rest.
#[derive(Deserialize)]
struct CrateManifest {
    package: PackageTable,
    #[serde(default, rename = "dev-dependencies")]
    dev_dependencies: BTreeMap<String, toml::Value>,
}

#[derive(Deserialize)]
struct PackageTable {
    name: String,
    /// `publish = false`, or `publish = ["registry", …]`. Absent means
    /// "publishable to any registry".
    #[serde(default)]
    publish: Option<toml::Value>,
}

#[derive(Deserialize)]
struct JsonPackage {
    version: String,
}

/// `release-plz.toml`. `[[package]]` overrides plus whatever else the
/// file carries (`[workspace]`, `[changelog]`) — serde ignores those.
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

/// The facts the invariants are checked against. Built by [`load`]
/// (all the I/O) and consumed by [`verify`] (pure, unit-tested).
struct Ledger {
    /// Member package names with `publish != false`, in member order.
    publishable: Vec<String>,
    /// Member package names with `publish = false`.
    unpublishable: Vec<String>,
    /// `release-plz.toml` packages that claim the release identity —
    /// `changelog_update` *and* `git_tag_enable` both on. Exactly one is
    /// legal; the vector exists so a violation can name the offenders.
    release_identities: Vec<String>,
    /// The identity crate's `changelog_include`, verbatim (order and
    /// duplicates preserved — I5 inspects both).
    changelog_include: Vec<String>,
    /// `(crate, dev-dep)` where a publishable crate's internal dev-dep
    /// resolves to a registry `version`.
    versioned_internal_dev_deps: Vec<(String, String)>,
    /// `[workspace.dependencies]` entries that carry a `version` while
    /// pointing at a `publish = false` member.
    versioned_unpublishable_workspace_deps: Vec<String>,
    non_placeholder_package_versions: Vec<(String, String)>,
}

pub(crate) fn dispatch(args: &PublishArgs) -> Result<(), String> {
    match args.op {
        PublishOp::Check => check(),
    }
}

fn workspace_root() -> Result<PathBuf, String> {
    // The xtask binary lives under <workspace>/crates/aozora-xtask;
    // resolve the workspace root by stripping two directory levels
    // from CARGO_MANIFEST_DIR so this works from any cwd.
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

/// Whether a `publish` value means "do not publish". `false` and an
/// empty registry list both do; anything else is publishable.
fn publish_is_disabled(value: Option<&toml::Value>) -> bool {
    match value {
        Some(toml::Value::Boolean(flag)) => !flag,
        Some(toml::Value::Array(registries)) => registries.is_empty(),
        _ => false,
    }
}

/// Whether a dependency entry states a registry `version` on its own,
/// without following `workspace = true`.
fn entry_states_version(entry: &toml::Value) -> bool {
    match entry {
        // `foo = "1.0"` — the bare-version shorthand.
        toml::Value::String(_) => true,
        toml::Value::Table(table) => table.contains_key("version"),
        _ => false,
    }
}

/// Whether a dependency resolves to a registry `version` — i.e. whether
/// cargo would keep it in the published manifest. `workspace = true`
/// inherits, so the shared entry decides: `aozora-corpus` is declared
/// path-only at the workspace level, which makes
/// `aozora-corpus = { workspace = true }` version-free and therefore
/// legal as a dev-dep.
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

fn load(root: &Path) -> Result<Ledger, String> {
    let manifest: RootManifest = read_toml(&root.join("Cargo.toml"))?;
    let workspace_deps = &manifest.workspace.dependencies;

    // Pass 1: read every member manifest. Member *paths* are not member
    // *names* (`crates/tree-sitter-aozora` → `tree-sitter-aozora`), so
    // the name has to come out of each manifest.
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

    // An "internal" dependency is one whose name is a workspace member.
    let internal: Vec<&str> = members
        .iter()
        .map(|(m, _)| m.package.name.as_str())
        .collect();

    let mut versioned_internal_dev_deps = Vec::new();
    for (member, _) in &members {
        if publish_is_disabled(member.package.publish.as_ref()) {
            // A `publish = false` crate has no published manifest, so
            // cargo never checks its dev-deps against the registry.
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

    let package_paths = [
        "crates/tree-sitter-aozora/package.json",
        "editors/vscode/package.json",
        "playground/package.json",
    ];
    let non_placeholder_package_versions = package_paths
        .iter()
        .map(|rel| {
            let package: JsonPackage = read_json(&root.join(rel))?;
            Ok((*rel, package.version))
        })
        .collect::<Result<Vec<_>, String>>()?
        .into_iter()
        .filter(|(_, version)| version != "0.0.0")
        .map(|(path, version)| (path.to_owned(), version))
        .collect();

    Ok(Ledger {
        publishable,
        unpublishable,
        release_identities,
        changelog_include,
        versioned_internal_dev_deps,
        versioned_unpublishable_workspace_deps,
        non_placeholder_package_versions,
    })
}

/// Check every invariant, collecting all violations rather than
/// stopping at the first — one run should tell you everything that is
/// wrong.
fn verify(ledger: &Ledger) -> Result<(), Vec<String>> {
    let mut violations = Vec::new();
    check_public_packages(ledger, &mut violations);
    // The identity is resolved first because `changelog_include` hangs
    // off it: without knowing which crate is the umbrella, "everyone
    // else must be folded in" has no subject.
    let identity = release_identity(ledger, &mut violations);
    check_changelog_include(ledger, identity, &mut violations);
    check_manifest_hygiene(ledger, &mut violations);
    check_distribution_versions(ledger, &mut violations);

    if violations.is_empty() {
        Ok(())
    } else {
        Err(violations)
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

/// I4 — exactly one release identity. Zero means no `vX.Y.Z` tag is
/// ever cut (and the whole downstream chain stays silent); two means two
/// tags race for the same version.
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

/// I1 / I2 / I3 / I5 — the aggregated changelog's crate list.
fn check_changelog_include(ledger: &Ledger, identity: Option<&str>, violations: &mut Vec<String>) {
    if let Some(identity) = identity {
        // I1 — every publishable crate but the identity folds its
        // commits into the one aggregated changelog.
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

        // I3 — the identity must not fold itself in.
        if ledger.changelog_include.iter().any(|inc| inc == identity) {
            violations.push(format!(
                "release-plz.toml `changelog_include` contains the release identity \
                 `{identity}` itself, which would duplicate its commits in the changelog"
            ));
        }
    }

    // I2 — nothing in the list may be stale (renamed, deleted, or
    // turned `publish = false`).
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

    // I5 — sorted + unique. Not cosmetic: an unsorted list is one a
    // human appends to blindly, which is how a duplicate gets in.
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

/// I6 / I7 — the publish-path rules the root `Cargo.toml` states in
/// prose, made executable.
fn check_manifest_hygiene(ledger: &Ledger, violations: &mut Vec<String>) {
    // I6 — a versioned internal dev-dep must already exist on crates.io
    // at publish time, which blocks a first publish and creates
    // unpublishable dev-dep cycles.
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

    // I7 — a `publish = false` member has no registry version, so a
    // version pin here goes stale on the lockstep bump and breaks
    // `cargo update`.
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
            // Counts are derived, never written down — a hard-coded
            // total is exactly the kind of fact this gate exists to
            // stop anyone from maintaining by hand.
            let folded = ledger.changelog_include.len();
            let expected = ledger.publishable.len().saturating_sub(1);
            eprintln!(
                "xtask publish check: {folded}/{expected} publishable crates folded into the \
                 aggregated changelog, {} publish=false skipped",
                ledger.unpublishable.len(),
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

    /// A ledger that satisfies every invariant, for tests to perturb.
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
        // The `aozora-lsp` → `aozora-fmt` first-publish block.
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
        // `aozora-corpus` is publish=false and therefore path-only at
        // the workspace level — inheriting it is legal.
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
    fn the_live_ledger_loads_and_passes() {
        // The integration test: parse the real manifests, not fixtures.
        // This is what fails in CI when someone adds a crate and forgets
        // release-plz.toml.
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
