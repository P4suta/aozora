//! MSRV / toolchain pin coherence gate.
//!
//! Two authorities, deliberately holding **different** numbers:
//!
//! - `T` = `rust-toolchain.toml`'s `channel` — the DEV toolchain. Tracks
//!   latest stable; that is how we get new clippy lints.
//! - `M` = the root `Cargo.toml`'s `rust-version` — the PUBLIC CONTRACT.
//!   A measured floor; moves only when a new stable feature is needed.
//!
//! There is no rule that they match. The rule is that every *other* pin
//! follows the right one of the two, and that `M` stays far enough behind
//! `T` to honour the six-month policy. See ADR-0034 and
//! `docs/contrib/msrv.md`.
//!
//! ## Why a gate rather than care
//!
//! `dd65755`'s commit message asserted "All version pins move together".
//! It did not: three README badges were left behind and a later commit
//! had to sweep them. Nothing checked, so nothing caught it. Coupling the
//! numbers made that inevitable — one edit had to land in a dozen places
//! at once, and "a dozen places, by hand, every time" is not a policy.
//!
//! The existing `version-literal-gate` (`Justfile`) cannot cover this: it
//! requires a `v` prefix, so `Rust 1.96.0` never matches, and its scope
//! excludes the READMEs. It polices release-tag literals; this polices
//! version pins. Different facts, different gates.

use std::fmt::{self, Display, Formatter};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use regex::Regex;
use serde::Deserialize;
use serde::de::DeserializeOwned;

use crate::MsrvArgs;
use crate::MsrvOp;

/// Minimum number of releases `M` must sit behind `T`.
///
/// Rust ships every 6 weeks, so `T - 4` is 24 weeks (~5.5 months) — short
/// of the six-month promise — while `T - 5` is 30 weeks (~6.9 months).
/// Five is therefore the smallest gap that keeps the policy true, and it
/// is pure arithmetic, so the repo needs no release calendar to check it.
const MIN_RELEASES_BEHIND: u64 = 5;

/// Pins that must equal the MSRV (`Cargo.toml`'s `rust-version`).
static MSRV_PINS: &[Pin] = &[Pin {
    path: "clippy.toml",
    what: "clippy's `msrv`",
    // `msrv = "1.89.0"`
    pattern: r#"(?m)^msrv\s*=\s*"([^"]+)""#,
}];

/// Pins that must equal the toolchain channel (`rust-toolchain.toml`).
static TOOLCHAIN_PINS: &[Pin] = &[Pin {
    path: "Dockerfile",
    what: "the `FROM rust:` base tag",
    // `FROM rust:1.97.0-bookworm@sha256:…`
    pattern: r"(?m)^FROM\s+rust:([0-9]+\.[0-9]+\.[0-9]+)-",
}];

struct Pin {
    path: &'static str,
    what: &'static str,
    pattern: &'static str,
}

/// A Rust version as `(minor, patch)`. Every release since 1.0 is `1.x`,
/// so the major is not worth carrying.
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
    // `rust-version = "1.89"` is legal; treat the patch as 0.
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

pub(crate) fn dispatch(args: &MsrvArgs) -> Result<(), String> {
    match args.op {
        MsrvOp::Check => check(),
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

/// Extract the first capture group of `pattern` from `path`.
fn scrape(root: &Path, pin: &Pin) -> Result<Version, String> {
    let path = root.join(pin.path);
    let text =
        fs::read_to_string(&path).map_err(|err| format!("read {}: {err}", path.display()))?;
    let re =
        Regex::new(pin.pattern).map_err(|err| format!("bad pattern for {}: {err}", pin.path))?;
    let found = re
        .captures(&text)
        .and_then(|c| c.get(1))
        .ok_or_else(|| format!("{}: could not find {}", pin.path, pin.what))?;
    parse_version(found.as_str()).map_err(|err| format!("{}: {err}", pin.path))
}

/// Maintained docs may not name a Rust version — `docs/contrib/msrv.md`
/// is the single place that does. Matches `1.NN` only near a rust/MSRV
/// word so a crate version (`aozora = "0.4"`) or a dep pin
/// (`toml = "1.1"`) does not trip it.
static MAINTAINED_DOC_VERSION: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)(rust|msrv)[^\n]{0,20}?\b1\.(8[5-9]|9[0-9]|[1-9][0-9]{2})\b")
        .expect("static pattern")
});

/// The MSRV badge must be derived from crates.io, never written down.
/// A plain substring, not a regex — shields.io's hand-written form is
/// `/badge/rust-<ver>`, the derived one `/crates/msrv/aozora`.
const STATIC_MSRV_BADGE: &str = "img.shields.io/badge/rust-";

const READMES: &[&str] = &["README.md", "crates/aozora/README.md"];
const MSRV_PAGE: &str = "docs/contrib/msrv.md";

/// Prose we maintain, and therefore prose that must not restate the MSRV.
///
/// `docs/adr/` is deliberately absent. An accepted ADR is a dated record,
/// never edited (`docs/ADR_INDEX.md`), so the Rust version inside one is
/// history rather than a fact anybody keeps current — ADR-0031 says "rust
/// 1.96.0" and is *correct* to, because that is what it was. Scanning
/// them would turn every honest record into a violation.
const MAINTAINED_DOCS: &[&str] = &["docs/contrib"];

fn check_docs(root: &Path, violations: &mut Vec<String>) -> Result<(), String> {
    let mut offenders = Vec::new();
    // The exemption below is only sound while its subject exists. Without
    // this, deleting the page turns `MSRV_PAGE` into a dead constant and
    // leaves the gate green — the same silent pass the directory check
    // guards against.
    if !root.join(MSRV_PAGE).exists() {
        return Err(format!(
            "{MSRV_PAGE}: not found — the page every other doc defers to is \
             gone, so this gate is exempting nothing"
        ));
    }
    for dir in MAINTAINED_DOCS {
        let src = root.join(dir);
        // A missing path is an error, not an absence of violations. The
        // gate that silently passes when its subject disappears is the
        // gate that is not there.
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
             -> use `https://img.shields.io/crates/msrv/aozora`, which reads the published \
             crate's rust-version and cannot go stale",
            offenders.len(),
            offenders.join("\n    "),
        ));
    }
    Ok(())
}

fn check() -> Result<(), String> {
    let root = workspace_root()?;

    let manifest: RootManifest = read_toml(&root.join("Cargo.toml"))?;
    let msrv = parse_version(&manifest.workspace.package.rust_version)
        .map_err(|err| format!("Cargo.toml rust-version: {err}"))?;

    let toolchain_file: ToolchainFile = read_toml(&root.join("rust-toolchain.toml"))?;
    let channel = parse_version(&toolchain_file.toolchain.channel)
        .map_err(|err| format!("rust-toolchain.toml channel: {err}"))?;

    let mut violations = Vec::new();

    // I1 — every MSRV pin equals the contract.
    for pin in MSRV_PINS {
        let found = scrape(&root, pin)?;
        if found != msrv {
            violations.push(format!(
                "{}: {} is {found}, but Cargo.toml's rust-version is {msrv}\n\
                 -> the MSRV is the contract; this pin follows it",
                pin.path, pin.what,
            ));
        }
    }

    // I2 — every toolchain pin equals the channel.
    for pin in TOOLCHAIN_PINS {
        let found = scrape(&root, pin)?;
        if found != channel {
            violations.push(format!(
                "{}: {} is {found}, but rust-toolchain.toml's channel is {channel}\n\
                 -> this follows the DEV channel, not the MSRV; re-resolve the digest with \
                 `docker buildx imagetools inspect rust:{channel}-bookworm`",
                pin.path, pin.what,
            ));
        }
    }

    // I3 — the contract cannot ask for more than we develop on.
    //
    // Usually cargo gets there first: with `M > T` it refuses to build
    // xtask at all ("requires rustc <M>"), so this branch never runs. It
    // is still load-bearing twice over. It keeps I4's subtraction below
    // from underflowing, and it IS reachable whenever the running
    // toolchain is neither `T` nor selected by rust-toolchain.toml — a
    // `RUSTUP_TOOLCHAIN` override (which the mise host lane sets) can
    // satisfy `M` while `T` in the file does not. Do not delete this as
    // dead.
    if msrv.minor > channel.minor {
        violations.push(format!(
            "MSRV {msrv} is newer than the toolchain channel {channel}; the contract cannot \
             exceed what we build with"
        ));
    // I4 — the six-month rule.
    } else if channel.minor - msrv.minor < MIN_RELEASES_BEHIND {
        violations.push(format!(
            "MSRV {msrv} is only {} release(s) behind the channel {channel}; the policy is at \
             least {MIN_RELEASES_BEHIND} (~6 months of Rust's 6-week cadence)\n\
             -> either the MSRV bump is unjustified, or the policy changed and \
             contrib/msrv.md + ADR-0034 need to say so",
            channel.minor - msrv.minor,
        ));
    }

    // I5 / I6 — the number lives in exactly one page, and the badge is derived.
    check_docs(&root, &mut violations)?;
    check_badges(&root, &mut violations)?;

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
        // `rust-version = "1.89"` is legal cargo.
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
        // 6-week cadence: 4 releases = 24 weeks (~5.5 months, short of the
        // promise); 5 = 30 weeks (~6.9 months, honours it).
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
        // The blind spot that let the READMEs drift was over-narrow
        // matching; the opposite failure is matching everything.
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
            !"https://img.shields.io/crates/msrv/aozora".contains(STATIC_MSRV_BADGE),
            "the derived badge is the fix, not a violation"
        );
    }

    #[test]
    fn the_live_pins_are_coherent() {
        // The integration test: read the real files. This is what fails in
        // CI when someone bumps one pin and forgets its siblings.
        check().expect("the committed pins must satisfy every invariant");
    }
}
