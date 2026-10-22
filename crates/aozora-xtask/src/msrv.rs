#![expect(
    clippy::expect_used,
    reason = "the maintained MSRV regex is a valid static pattern"
)]

//! Checks Rust version pins and the supported version policy.
//! ADR-0034 separates the development toolchain from the supported version floor.

use std::fmt::{self, Display, Formatter};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use regex::Regex;
use serde::Deserialize;
use serde::de::DeserializeOwned;

use crate::MsrvArgs;
use crate::MsrvOp;

/// Rust's six-week release cadence needs this gap to meet ADR-0034.
const MIN_RELEASES_BEHIND: u64 = 5;

static MSRV_PINS: &[Pin] = &[Pin {
    path: "clippy.toml",
    what: "clippy's `msrv`",
    pattern: r#"(?m)^msrv\s*=\s*"([^"]+)""#,
}];

const WORKFLOW_DEV_RUST_PIN: &str =
    r#"(?m)(?:rust@|toolchain:\s*["']?)([0-9]+\.[0-9]+\.[0-9]+)(?:["']|\b)"#;

static TOOLCHAIN_PINS: &[Pin] = &[
    Pin {
        path: "mise.toml",
        what: "mise's stable Rust tool",
        pattern: r#"(?m)^\s*\{\s*version\s*=\s*"([0-9]+\.[0-9]+\.[0-9]+)""#,
    },
    Pin {
        path: ".github/workflows/ci.yml",
        what: "CI workflow's development Rust pin",
        pattern: WORKFLOW_DEV_RUST_PIN,
    },
    Pin {
        path: ".github/workflows/release-ready.yml",
        what: "release-ready workflow's development Rust pin",
        pattern: WORKFLOW_DEV_RUST_PIN,
    },
    Pin {
        path: ".github/workflows/release-plz.yml",
        what: "release-plz workflow's development Rust pin",
        pattern: WORKFLOW_DEV_RUST_PIN,
    },
];

struct Pin {
    path: &'static str,
    what: &'static str,
    pattern: &'static str,
}

struct PinExpectation<'a> {
    version: Version,
    authority: &'a str,
    follows: &'a str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Version {
    minor: u64,
    patch: u64,
}

impl Display for Version {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(f, "1.{}.{}", self.minor, self.patch)
    }
}

fn parse_version(text: &str) -> Result<Version, String> {
    let mut parts = text.trim().split('.');
    let major: u64 = parts
        .next()
        .and_then(|p| p.parse().ok())
        .ok_or_else(|| format!("not a Rust version: {text:?}"))?;
    if major != 1 {
        return Err(format!("expected a 1.x Rust version, got {text:?}"));
    }
    let minor = parts
        .next()
        .and_then(|p| p.parse().ok())
        .ok_or_else(|| format!("missing minor version in {text:?}"))?;
    let patch = parts.next().map_or(Ok(0), |p| {
        p.parse::<u64>()
            .map_err(|_| format!("bad patch version in {text:?}"))
    })?;
    Ok(Version { minor, patch })
}

#[derive(Deserialize)]
struct RootManifest {
    workspace: WorkspaceTable,
}

#[derive(Deserialize)]
struct WorkspaceTable {
    package: WorkspacePackage,
    members: Vec<String>,
}

#[derive(Deserialize)]
struct WorkspacePackage {
    #[serde(rename = "rust-version")]
    rust_version: String,
}

#[derive(Deserialize)]
struct ToolchainFile {
    toolchain: ToolchainTable,
}

#[derive(Deserialize)]
struct ToolchainTable {
    channel: String,
}

#[derive(Deserialize)]
struct MemberManifest {
    package: Option<MemberPackage>,
}

#[derive(Deserialize)]
struct MemberPackage {
    name: Option<String>,
    publish: Option<Publish>,
    #[serde(rename = "rust-version")]
    rust_version: Option<RustVersion>,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum Publish {
    Flag(bool),
    Registries(Vec<String>),
}

#[derive(Deserialize)]
#[serde(untagged)]
enum RustVersion {
    Inherited { workspace: bool },
    Literal(String),
}

fn is_published(package: &MemberPackage) -> bool {
    match &package.publish {
        None | Some(Publish::Flag(true)) => true,
        Some(Publish::Flag(false)) => false,
        Some(Publish::Registries(registries)) => !registries.is_empty(),
    }
}

fn member_msrv_violation(member: &str, package: &MemberPackage) -> Option<String> {
    let name = package.name.as_deref().unwrap_or(member);
    match &package.rust_version {
        Some(RustVersion::Inherited { workspace: true }) => None,
        Some(RustVersion::Inherited { workspace: false }) => Some(format!(
            "{member}/Cargo.toml: published crate `{name}` sets \
             `rust-version.workspace = false`; it must inherit the workspace MSRV \
             with `rust-version.workspace = true`"
        )),
        Some(RustVersion::Literal(v)) => Some(format!(
            "{member}/Cargo.toml: published crate `{name}` hardcodes \
             `rust-version = {v:?}`; it must inherit the workspace MSRV with \
             `rust-version.workspace = true` so the contract stays single-source"
        )),
        None => Some(format!(
            "{member}/Cargo.toml: published crate `{name}` declares no \
             `rust-version`; it must inherit the workspace MSRV with \
             `rust-version.workspace = true`"
        )),
    }
}

pub(crate) fn dispatch(args: &MsrvArgs) -> Result<(), String> {
    match args.op {
        MsrvOp::Check => check(),
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

fn scrape_text(text: &str, pin: &Pin) -> Result<Vec<Version>, String> {
    let re =
        Regex::new(pin.pattern).map_err(|err| format!("bad pattern for {}: {err}", pin.path))?;
    let mut found = Vec::new();
    for captures in re.captures_iter(text) {
        let value = captures
            .get(1)
            .ok_or_else(|| format!("{}: {} pattern has no capture", pin.path, pin.what))?;
        found.push(parse_version(value.as_str()).map_err(|err| format!("{}: {err}", pin.path))?);
    }
    if found.is_empty() {
        return Err(format!("{}: could not find {}", pin.path, pin.what));
    }
    Ok(found)
}

fn scrape(root: &Path, pin: &Pin) -> Result<Vec<Version>, String> {
    let path = root.join(pin.path);
    let text =
        fs::read_to_string(&path).map_err(|err| format!("read {}: {err}", path.display()))?;
    scrape_text(&text, pin)
}

fn pin_violations(pin: &Pin, found: &[Version], expected: &PinExpectation<'_>) -> Vec<String> {
    let expected_version = expected.version;
    found
        .iter()
        .filter(|version| **version != expected_version)
        .map(|version| {
            format!(
                "{}: {} is {version}, but {} is {expected_version}\n\
                 -> {}",
                pin.path, pin.what, expected.authority, expected.follows,
            )
        })
        .collect()
}

static MAINTAINED_DOC_VERSION: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)(rust|msrv)[^\n]{0,20}?\b1\.(8[5-9]|9[0-9]|[1-9][0-9]{2})\b")
        .expect("static pattern")
});

const STATIC_MSRV_BADGE: &str = "img.shields.io/badge/rust-";
const DERIVED_MSRV_BADGE: &str = "https://img.shields.io/crates/msrv/aozora";

const READMES: &[&str] = &["README.md", "crates/aozora/README.md"];
const MSRV_PAGE: &str = "docs/contrib/msrv.md";

const MAINTAINED_DOCS: &[&str] = &["docs/contrib"];

fn check_members_inherit_msrv(
    root: &Path,
    members: &[String],
    violations: &mut Vec<String>,
) -> Result<(), String> {
    for member in members {
        if member.contains('*') {
            return Err(format!(
                "workspace member glob {member:?} is unsupported here — this gate \
                 must enumerate every member or it silently skips one"
            ));
        }
        let manifest: MemberManifest = read_toml(&root.join(member).join("Cargo.toml"))?;
        let Some(package) = manifest.package else {
            continue;
        };
        if is_published(&package)
            && let Some(violation) = member_msrv_violation(member, &package)
        {
            violations.push(violation);
        }
    }
    Ok(())
}

fn check_docs(root: &Path, violations: &mut Vec<String>) -> Result<(), String> {
    let mut offenders = Vec::new();
    if !root.join(MSRV_PAGE).exists() {
        return Err(format!(
            "{MSRV_PAGE}: not found — the page every other doc defers to is \
             gone, so this gate is exempting nothing"
        ));
    }
    for dir in MAINTAINED_DOCS {
        let src = root.join(dir);
        if !src.exists() {
            return Err(format!(
                "{dir}: not found — MAINTAINED_DOCS is stale, so this gate is \
                 checking nothing"
            ));
        }
        for entry in walkdir::WalkDir::new(&src) {
            let entry = entry.map_err(|err| format!("walk {dir}: {err}"))?;
            let path = entry.path();
            if path.extension().is_none_or(|ext| ext != "md") {
                continue;
            }
            let rel = path
                .strip_prefix(root)
                .map_err(|err| format!("strip_prefix {}: {err}", path.display()))?;
            if rel == Path::new(MSRV_PAGE) {
                continue;
            }
            let text = fs::read_to_string(path)
                .map_err(|err| format!("read {}: {err}", path.display()))?;
            for (n, line) in text.lines().enumerate() {
                if MAINTAINED_DOC_VERSION.is_match(line) {
                    offenders.push(format!("{}:{}: {}", rel.display(), n + 1, line.trim()));
                }
            }
        }
    }
    if !offenders.is_empty() {
        violations.push(format!(
            "{} maintained doc line(s) name a Rust version outside {MSRV_PAGE}:\n    {}\n\
             -> link to the MSRV policy page instead; it is the one place the number lives",
            offenders.len(),
            offenders.join("\n    "),
        ));
    }
    Ok(())
}

fn check_badges(root: &Path, violations: &mut Vec<String>) -> Result<(), String> {
    let mut offenders = Vec::new();
    for rel in READMES {
        let path = root.join(rel);
        let text =
            fs::read_to_string(&path).map_err(|err| format!("read {}: {err}", path.display()))?;
        for (n, line) in text.lines().enumerate() {
            if line.contains(STATIC_MSRV_BADGE) {
                offenders.push(format!("{rel}:{}", n + 1));
            }
        }
    }
    if !offenders.is_empty() {
        violations.push(format!(
            "{} README(s) carry a hand-written MSRV badge:\n    {}\n\
             -> use `{DERIVED_MSRV_BADGE}`, which reads the published \
             crate's rust-version and cannot go stale",
            offenders.len(),
            offenders.join("\n    "),
        ));
    }
    Ok(())
}

#[expect(
    clippy::print_stderr,
    reason = "the command reports results and diagnostics to developers"
)]
fn check() -> Result<(), String> {
    let root = workspace_root()?;

    let manifest: RootManifest = read_toml(&root.join("Cargo.toml"))?;
    let msrv = parse_version(&manifest.workspace.package.rust_version)
        .map_err(|err| format!("Cargo.toml rust-version: {err}"))?;

    let toolchain_file: ToolchainFile = read_toml(&root.join("rust-toolchain.toml"))?;
    let channel = parse_version(&toolchain_file.toolchain.channel)
        .map_err(|err| format!("rust-toolchain.toml channel: {err}"))?;

    let mut violations = Vec::new();

    for pin in MSRV_PINS {
        let found = scrape(&root, pin)?;
        violations.extend(pin_violations(
            pin,
            &found,
            &PinExpectation {
                version: msrv,
                authority: "Cargo.toml's rust-version",
                follows: "the MSRV is the contract; this pin follows it",
            },
        ));
    }

    for pin in TOOLCHAIN_PINS {
        let found = scrape(&root, pin)?;
        violations.extend(pin_violations(
            pin,
            &found,
            &PinExpectation {
                version: channel,
                authority: "rust-toolchain.toml's channel",
                follows: "this follows the DEV channel, not the MSRV",
            },
        ));
    }

    if msrv.minor > channel.minor {
        violations.push(format!(
            "MSRV {msrv} is newer than the toolchain channel {channel}; the contract cannot \
             exceed what we build with"
        ));
    } else if channel.minor - msrv.minor < MIN_RELEASES_BEHIND {
        violations.push(format!(
            "MSRV {msrv} is only {} release(s) behind the channel {channel}; the policy is at \
             least {MIN_RELEASES_BEHIND} (~6 months of Rust's 6-week cadence)\n\
             -> either the MSRV bump is unjustified, or the policy changed and \
             contrib/msrv.md + ADR-0034 need to say so",
            channel.minor - msrv.minor,
        ));
    }

    check_docs(&root, &mut violations)?;
    check_badges(&root, &mut violations)?;

    check_members_inherit_msrv(&root, &manifest.workspace.members, &mut violations)?;

    if violations.is_empty() {
        eprintln!(
            "xtask msrv check: contract {msrv}, dev channel {channel} ({} releases of headroom)",
            channel.minor - msrv.minor,
        );
        Ok(())
    } else {
        Err(format!(
            "MSRV / toolchain pin drift detected:\n  {}",
            violations.join("\n  "),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_version_reads_two_and_three_component_forms() {
        assert_eq!(
            parse_version("1.89.0").expect("three components"),
            Version {
                minor: 89,
                patch: 0
            },
        );
        assert_eq!(
            parse_version("1.89").expect("two components"),
            Version {
                minor: 89,
                patch: 0
            },
        );
    }

    #[test]
    fn parse_version_rejects_non_rust_versions() {
        assert!(parse_version("2.0.0").is_err(), "no Rust 2.x exists");
        assert!(
            parse_version("stable").is_err(),
            "a channel is not a version"
        );
        assert!(parse_version("").is_err(), "empty is not a version");
    }

    #[test]
    fn version_displays_canonically() {
        let v = parse_version("1.89").expect("parse");
        assert_eq!(v.to_string(), "1.89.0", "display normalises the patch");
    }

    #[test]
    fn six_month_rule_arithmetic_matches_the_documented_reasoning() {
        assert_eq!(
            MIN_RELEASES_BEHIND, 5,
            "5 releases is the smallest gap that clears six months"
        );
    }

    #[test]
    fn maintained_doc_pattern_matches_a_pinned_rust_version() {
        assert!(
            MAINTAINED_DOC_VERSION.is_match("aozora pins **Rust 1.96.0** as its MSRV"),
            "must catch the prose that motivated this gate"
        );
        assert!(
            MAINTAINED_DOC_VERSION.is_match("the MSRV is 1.89"),
            "must catch a bare MSRV mention"
        );
    }

    #[test]
    fn maintained_doc_pattern_ignores_unrelated_versions() {
        assert!(
            !MAINTAINED_DOC_VERSION.is_match(r#"aozora = "0.4""#),
            "a crate version is not a Rust version"
        );
        assert!(
            !MAINTAINED_DOC_VERSION.is_match(r#"toml = "1.1""#),
            "a dep pin is not a Rust version"
        );
        assert!(
            !MAINTAINED_DOC_VERSION.is_match("Keep a Changelog 1.1.0"),
            "an unrelated 1.x is not a Rust version"
        );
    }

    #[test]
    fn badge_pattern_matches_the_static_form_only() {
        assert!(
            "https://img.shields.io/badge/rust-1.96-orange".contains(STATIC_MSRV_BADGE),
            "must catch the hand-written badge"
        );
        assert!(
            !DERIVED_MSRV_BADGE.contains(STATIC_MSRV_BADGE),
            "the derived badge is the fix, not a violation"
        );
    }

    #[test]
    fn workflow_pin_validation_catches_a_stale_second_match() {
        let pin = Pin {
            path: "workflow.yml",
            what: "development Rust pin",
            pattern: WORKFLOW_DEV_RUST_PIN,
        };
        let expected = parse_version("1.99.1").expect("current toolchain");
        let found = scrape_text(
            "install_args: rust@1.99.1 just\ntoolchain: \"1.96.0\"\ntoolchain: 1.99.1\ntoolchain: ${{ steps.msrv.outputs.version }}\n",
            &pin,
        )
        .expect("workflow pins");
        let violations = pin_violations(
            &pin,
            &found,
            &PinExpectation {
                version: expected,
                authority: "rust-toolchain.toml's channel",
                follows: "this follows the DEV channel, not the MSRV",
            },
        );

        assert_eq!(found.len(), 3, "dynamic MSRV toolchain is excluded");
        assert_eq!(violations.len(), 1, "only the stale second pin differs");
        assert!(violations[0].contains("1.96.0"));
    }

    #[test]
    fn the_live_pins_are_coherent() {
        check().expect("the committed pins must satisfy every invariant");
    }

    fn member(toml: &str) -> MemberPackage {
        toml::from_str::<MemberManifest>(toml)
            .expect("valid toml")
            .package
            .expect("has a [package]")
    }

    #[test]
    fn is_published_reads_cargos_publish_key() {
        assert!(
            is_published(&member("[package]\nname = \"x\"\n")),
            "an absent publish key is publishable"
        );
        assert!(is_published(&member(
            "[package]\nname = \"x\"\npublish = true\n"
        )));
        assert!(!is_published(&member(
            "[package]\nname = \"x\"\npublish = false\n"
        )));
        assert!(is_published(&member(
            "[package]\nname = \"x\"\npublish = [\"crates-io\"]\n"
        )));
        assert!(
            !is_published(&member("[package]\nname = \"x\"\npublish = []\n")),
            "an empty registry list is unpublishable"
        );
    }

    #[test]
    fn published_member_must_inherit_the_workspace_rust_version() {
        assert!(
            member_msrv_violation(
                "crates/x",
                &member("[package]\nname = \"x\"\nrust-version.workspace = true\n"),
            )
            .is_none(),
            "inheriting the workspace MSRV is compliant"
        );

        let literal = member_msrv_violation(
            "crates/x",
            &member("[package]\nname = \"x\"\nrust-version = \"1.89.0\"\n"),
        )
        .expect("a literal rust-version is a violation");
        assert!(literal.contains("hardcodes"), "{literal}");

        let missing = member_msrv_violation("crates/x", &member("[package]\nname = \"x\"\n"))
            .expect("a missing rust-version is a violation");
        assert!(missing.contains("declares no"), "{missing}");

        let optout = member_msrv_violation(
            "crates/x",
            &member("[package]\nname = \"x\"\nrust-version.workspace = false\n"),
        )
        .expect("opting out is a violation");
        assert!(optout.contains("workspace = false"), "{optout}");
    }
}
