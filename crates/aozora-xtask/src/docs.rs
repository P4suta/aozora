//! Dangling-reference gate for the docs a human is told to read.
//!
//! CI workflows and the Justfile point people at pages by path — in a
//! `::notice::` a maintainer reads on a real run, or in a comment the
//! next contributor reads. Those paths are strings. Nothing has ever
//! checked that they resolve.
//!
//! **Offline.** Only repo-relative `docs/**.md` paths are checked; an
//! external URL is somebody else's uptime, and a blocking gate must not
//! depend on it.
//!
//! ## Why this exists
//!
//! Deleting or moving a page does not break the reference to it — it
//! makes the reference wrong while everything stays green. Measured in
//! #525: `.github/workflows/fuzz.yml` cited `docs/fuzz-workflow.md`, which
//! #550 deleted because the Justfile documented it next to the fuzz recipes.
//! That is the case this gate catches, and the reason it exists.
//!
//! It is necessary and not sufficient. Content can move out of a page
//! that still exists, leaving a line that resolves and misleads — no
//! path check sees that, and no link checker would either. So read the
//! whole line, not just the path, when this gate is green.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::{fs, io};

use regex::Regex;

use crate::scan::workspace_root;

/// Trees whose prose names doc pages a reader is expected to open.
///
/// `docs/` itself is deliberately absent: a page cross-linking a sibling
/// is ordinary Markdown, and the `docs/adr/` half of it must never be
/// checked — an accepted ADR is a dated record, never edited, so a path
/// inside one is history. ADR-0009 names
/// `crates/aozora-book/src/getting-started/install.md` and is *correct*
/// to, because that is where the pin lived when the decision was made.
const SOURCES: &[&str] = &[".github/workflows", "Justfile"];

/// The one Justfile recipe whose `docs/…` strings are inputs, not
/// directions. `ci-fast-selftest` feeds invented paths to
/// `scripts/ci-classify.sh` and asserts the categories that come back —
/// `docs/x.md` sits beside `crates/x/src/a.rs`, and both must *not*
/// exist or the test would be asserting against real files.
///
/// Scoped to the recipe rather than to the paths: a rule that skips
/// `docs/x.md` is a plaster that the next invented path defeats.
const SELFTEST_RECIPE: &str = "ci-fast-selftest";

/// Strip [`SELFTEST_RECIPE`]'s body — from its target line to the next
/// unindented line — so its fixtures are not read as directions.
///
/// Errors when the recipe is gone: an exclusion whose subject has left
/// is an exclusion nobody rechecks, and this gate exists because of
/// exactly that failure mode.
fn strip_selftest(text: &str) -> Result<String, String> {
    let head = format!("{SELFTEST_RECIPE}:");
    if !text.lines().any(|l| l.starts_with(&head)) {
        return Err(format!(
            "Justfile: recipe `{SELFTEST_RECIPE}` not found — \
             SELFTEST_RECIPE is stale, so this gate is excluding nothing"
        ));
    }
    let mut out = String::with_capacity(text.len());
    let mut skipping = false;
    for line in text.lines() {
        if line.starts_with(&head) {
            skipping = true;
            continue;
        }
        if skipping && !line.is_empty() && !line.starts_with([' ', '\t']) {
            skipping = false;
        }
        if !skipping {
            out.push_str(line);
            out.push('\n');
        }
    }
    Ok(out)
}

/// Collect every repo-relative `docs/**.md` path named in `text`.
///
/// A path preceded by `/` belongs to a URL — `github.com/ossf/scorecard/
/// blob/main/docs/checks.md` is scorecard's page, not ours — so the
/// pattern anchors on a boundary that a URL's slash cannot satisfy.
fn cited_paths(text: &str, re: &Regex) -> BTreeSet<String> {
    text.lines()
        .flat_map(|line| {
            re.captures_iter(line)
                .filter_map(|c| c.name("path").map(|m| m.as_str().to_owned()))
                .collect::<Vec<_>>()
        })
        .collect()
}

fn read_sources(root: &Path) -> Result<Vec<(PathBuf, String)>, String> {
    let mut out = Vec::new();
    for src in SOURCES {
        let path = root.join(src);
        if !path.exists() {
            return Err(format!(
                "{src}: not found — SOURCES is stale, so this gate is \
                 checking nothing"
            ));
        }
        if path.is_file() {
            let text = fs::read_to_string(&path).map_err(|e| format!("read {src}: {e}"))?;
            out.push((PathBuf::from(src), strip_selftest(&text)?));
            continue;
        }
        let entries = fs::read_dir(&path).map_err(|e| format!("read_dir {src}: {e}"))?;
        let mut files: Vec<PathBuf> = entries
            .collect::<io::Result<Vec<_>>>()
            .map_err(|e| format!("walk {src}: {e}"))?
            .into_iter()
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|x| x == "yml" || x == "yaml"))
            .collect();
        files.sort();
        for file in files {
            let text =
                fs::read_to_string(&file).map_err(|e| format!("read {}: {e}", file.display()))?;
            let rel = file
                .strip_prefix(root)
                .map_err(|e| format!("strip_prefix {}: {e}", file.display()))?;
            out.push((rel.to_path_buf(), text));
        }
    }
    Ok(out)
}

/// `xtask docs check` — every `docs/**.md` a workflow or the Justfile
/// names must exist.
pub(crate) fn check() -> Result<(), String> {
    let root = workspace_root()?;
    let re = Regex::new(r"(?:^|[^/\w])(?P<path>docs/[A-Za-z0-9_./-]+\.md)")
        .map_err(|e| format!("compile pattern: {e}"))?;

    let mut dangling = Vec::new();
    let mut checked = 0usize;
    for (rel, text) in read_sources(&root)? {
        for path in cited_paths(&text, &re) {
            checked += 1;
            if !root.join(&path).is_file() {
                dangling.push(format!("{}: {path}", rel.display()));
            }
        }
    }

    if !dangling.is_empty() {
        for d in &dangling {
            eprintln!("    {d}");
        }
        return Err(format!(
            "{} dangling doc reference(s) — CI tells a reader to open a page \
             that is not there",
            dangling.len()
        ));
    }
    eprintln!("xtask docs check: {checked} doc references resolve");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn re() -> Regex {
        Regex::new(r"(?:^|[^/\w])(?P<path>docs/[A-Za-z0-9_./-]+\.md)").expect("pattern compiles")
    }

    #[test]
    fn finds_a_bare_path() {
        let found = cited_paths("echo \"See docs/contrib/release.md.\"", &re());
        assert!(found.contains("docs/contrib/release.md"));
    }

    #[test]
    fn finds_a_path_in_backticks() {
        let found = cited_paths(
            "# harnesses (`docs/fuzz-workflow.md`, seven targets)",
            &re(),
        );
        assert!(found.contains("docs/fuzz-workflow.md"));
    }

    /// The one false positive worth naming: scorecard's own `docs/checks.md`
    /// is reached through a URL and is not ours to resolve. A gate that
    /// flags it teaches people to ignore the gate.
    #[test]
    fn ignores_a_path_inside_a_url() {
        let found = cited_paths(
            "#   https://github.com/ossf/scorecard/blob/main/docs/checks.md#fuzzing",
            &re(),
        );
        assert!(found.is_empty(), "matched inside a URL: {found:?}");
    }

    #[test]
    fn strip_selftest_drops_the_recipe_body_and_nothing_after() {
        let just = "\
lint:
    echo docs/contrib/release.md

ci-fast-selftest:
    check \"play + docs\" \"play book\" \"docs/x.md\"
    check \"docs + code\" \"code book\" \"docs/adr/0017-x.md\"

doc:
    echo docs/contrib/msrv.md
";
        let out = strip_selftest(just).expect("recipe present");
        assert!(!out.contains("docs/x.md"), "fixture survived: {out}");
        assert!(!out.contains("0017-x.md"), "fixture survived: {out}");
        assert!(
            out.contains("docs/contrib/release.md"),
            "ate the recipe before"
        );
        assert!(out.contains("docs/contrib/msrv.md"), "ate the recipe after");
    }

    /// The exclusion is only sound while its subject exists. Same shape as
    /// the `SOURCES` check — an exclusion that quietly covers nothing is
    /// how the next invented path gets read as a direction.
    #[test]
    fn strip_selftest_errors_when_the_recipe_is_gone() {
        let err = strip_selftest("lint:\n    echo hi\n").expect_err("must not pass silently");
        assert!(err.contains("SELFTEST_RECIPE is stale"), "{err}");
    }

    #[test]
    fn the_repo_has_no_dangling_doc_references() {
        check().expect("every docs/**.md named by CI or the Justfile resolves");
    }
}
