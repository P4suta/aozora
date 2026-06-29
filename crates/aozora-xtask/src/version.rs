//! `xtask version --channel <dev|nightly|stable> [--date YYYYMMDD]` — print the
//! canonical channel-aware version string. This is the single source of the
//! *format*: `nightly.yml` / `release.yml` export the result as
//! `AOZORA_BUILD_VERSION` so the `aozora-buildstamp` build.rs stamps it verbatim.
//!
//! ```text
//!   dev     → 0.4.1-dev+g<sha>
//!   nightly → 0.4.1-nightly.<date>+g<sha>
//!   stable  → 0.4.1                          (clean; the release tag itself)
//! ```
//!
//! The base `X.Y.Z` triple is the workspace version: xtask is a workspace member
//! with `version.workspace = true`, so `CARGO_PKG_VERSION` is authoritative — no
//! manifest parsing needed (unlike find-my-files, whose xtask is a separate
//! workspace and must read `engine/Cargo.toml`). The git sha is resolved at call
//! time; when `.git`/`git` is absent the metadata is simply omitted.
//!
//! Release *bumping* is NOT here — release-plz owns the version/tag/CHANGELOG.
//! This subcommand only formats a build identity for the dev/nightly/stable lanes.

use std::process::Command;

use clap::{Args, ValueEnum};

#[derive(Args)]
pub(crate) struct VersionArgs {
    /// Release channel: `dev` (local checkout), `nightly` (scheduled main
    /// build), or `stable` (a tagged release).
    #[arg(long, value_enum)]
    channel: Channel,
    /// Build date `YYYYMMDD` — required for `--channel nightly`.
    #[arg(long)]
    date: Option<String>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, ValueEnum)]
pub(crate) enum Channel {
    Dev,
    Nightly,
    Stable,
}

pub(crate) fn dispatch(args: &VersionArgs) -> Result<(), String> {
    let base = env!("CARGO_PKG_VERSION");
    let sha = git_short_sha();
    println!(
        "{}",
        compute(base, args.channel, args.date.as_deref(), sha.as_deref())?
    );
    Ok(())
}

/// Pure formatter — unit-tested without touching git or the filesystem.
fn compute(
    base: &str,
    channel: Channel,
    date: Option<&str>,
    sha: Option<&str>,
) -> Result<String, String> {
    let meta = sha.map_or_else(String::new, |s| format!("+g{s}"));
    Ok(match channel {
        Channel::Stable => base.to_owned(),
        Channel::Dev => format!("{base}-dev{meta}"),
        Channel::Nightly => {
            let date = date.ok_or("--date YYYYMMDD is required for the nightly channel")?;
            if date.len() != 8 || !date.bytes().all(|b| b.is_ascii_digit()) {
                return Err(format!("--date must be 8 digits (YYYYMMDD), got '{date}'"));
            }
            format!("{base}-nightly.{date}{meta}")
        }
    })
}

fn git_short_sha() -> Option<String> {
    let out = Command::new("git")
        .args(["rev-parse", "--short=7", "HEAD"])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let sha = String::from_utf8(out.stdout).ok()?.trim().to_owned();
    (!sha.is_empty()).then_some(sha)
}

#[cfg(test)]
mod tests {
    use super::{Channel, compute};

    #[test]
    fn stable_is_the_clean_base() {
        assert_eq!(
            compute("0.4.1", Channel::Stable, None, Some("abc1234")).unwrap(),
            "0.4.1"
        );
    }

    #[test]
    fn dev_carries_channel_and_sha() {
        assert_eq!(
            compute("0.4.1", Channel::Dev, None, Some("abc1234")).unwrap(),
            "0.4.1-dev+gabc1234"
        );
    }

    #[test]
    fn dev_without_sha_drops_metadata() {
        assert_eq!(
            compute("0.4.1", Channel::Dev, None, None).unwrap(),
            "0.4.1-dev"
        );
    }

    #[test]
    fn nightly_embeds_date_and_sha() {
        assert_eq!(
            compute("0.4.1", Channel::Nightly, Some("20260629"), Some("abc1234")).unwrap(),
            "0.4.1-nightly.20260629+gabc1234"
        );
    }

    #[test]
    fn nightly_requires_a_date() {
        compute("0.4.1", Channel::Nightly, None, Some("abc1234")).unwrap_err();
    }

    #[test]
    fn nightly_rejects_a_malformed_date() {
        for bad in ["2026-06-29", "20260", "2026062x", ""] {
            assert!(
                compute("0.4.1", Channel::Nightly, Some(bad), None).is_err(),
                "{bad} should be rejected"
            );
        }
    }
}
