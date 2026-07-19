//! Suppression-hygiene ratchet: `#[allow(...)]` counts may only go down.
//!
//! Every `#[allow]` is a lint the compiler wanted to raise and a human
//! chose to silence. A silenced lint is invisible — nothing re-asks the
//! question once the attribute lands. This gate makes the *quantity* of
//! silencing a tracked number: a per-crate baseline the build refuses to
//! let grow, and refuses to let a reduction go unrecorded.
//!
//! ## Why exact-match both directions
//!
//! `found > baseline` fails (a new suppression slipped in). `found <
//! baseline` *also* fails, telling the author to lower the baseline to the
//! number the gate just measured. Insta-style strictness is what makes the
//! ratchet monotonic: a reduction that isn't written down is a reduction
//! the next author is free to spend back. The baseline is edited by a
//! human — this gate prints the number and never rewrites source.
//!
//! ## Why `#[allow]` and not `#[expect]`
//!
//! `#[expect]` self-expires: `unfulfilled_lint_expectations` fires the
//! moment the underlying lint stops triggering, so an obsolete `expect`
//! fails the build on its own. It needs no ratchet, and is deliberately
//! uncounted here. That is the intended gradient — `allow` (ratcheted) →
//! `expect` (self-expiring) → gone.
//!
//! ## Scope determinism
//!
//! Source files come from `git ls-files '*.rs'`, not a `crates/**` glob:
//! the glob non-deterministically sweeps up generated `fuzz/target/**`
//! files in a dirty tree, so two runs on the same commit could disagree.
//! Git-tracked scope is the one both CI and a local run share. Suppressions
//! inside `#[cfg(test)]` modules are not counted — test code is allowed to
//! be expressive (mirroring `allow-*-in-tests` in `clippy.toml`).

use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

use syn::punctuated::Punctuated;
use syn::visit::{self, Visit};
use syn::{AttrStyle, Attribute, ItemMod, Meta, Token};

use crate::scan::{tracked_rs_files, workspace_root};

/// Per-crate baseline for OUTER item-level `#[allow(...)]` lint pairs.
///
/// A pair is one `(crate, lint)`: `#[allow(a, b)]` is two. A `reason = ".."`
/// clause is not a lint and is not counted. Crates with zero outer
/// suppressions are omitted. Lower a number here when the gate tells you a
/// reduction is unrecorded — never raise one to admit a new suppression.
const OUTER_BASELINE: &[(&str, usize)] = &[("aozora-py", 1)];

/// Per-crate baseline for INNER blanket `#![allow(...)]` lint pairs — the
/// module-/crate-wide suppressions. Tracked separately from the outer
/// counts because a blanket silences an unbounded set of future sites,
/// so it deserves its own visible ledger. Same edit rule as
/// [`OUTER_BASELINE`].
const INNER_BASELINE: &[(&str, usize)] = &[
    ("aozora", 1),
    ("aozora-cli", 4),
    ("aozora-corpus", 7),
    ("aozora-extism", 1),
    ("aozora-ffi", 1),
    ("aozora-trace", 4),
    ("aozora-xtask", 15),
];

/// Source root of the `.expect(` tripwire migrated out of `just
/// strict-code`. Kept here so every count-baseline in the workspace lives
/// in one Rust asset. After the 18→3 collapse the lexer pipeline lives at
/// `crates/aozora/src/pipeline`.
const EXPECT_CRATE_SRC: &str = "crates/aozora/src/pipeline";

/// Ceiling for `.expect(`-bearing lines under [`EXPECT_CRATE_SRC`]. A
/// coarse "no new runtime state-assertions" tripwire, not a precise audit
/// — it counts matching lines across every file (test modules included),
/// exactly as the old `grep -hcE '\.expect\('` sum did. Growth means an
/// invariant was pushed to runtime instead of into the type; refactor
/// rather than raise this.
const EXPECT_BASELINE: usize = 51;

/// `xtask lint suppressions` — per-crate `#[allow]` counts may not exceed
/// (nor silently undershoot) their recorded baselines, and the pipeline
/// `.expect(` count may not grow.
pub(crate) fn check() -> Result<(), String> {
    let root = workspace_root()?;

    let mut outer: BTreeMap<String, usize> = BTreeMap::new();
    let mut inner: BTreeMap<String, usize> = BTreeMap::new();
    let mut scanned = 0usize;
    for rel in tracked_rs_files(&root)? {
        if is_excluded(&rel) {
            continue;
        }
        let (o, i) = count_file(&root.join(&rel))?;
        scanned += 1;
        if o > 0 {
            *outer.entry(crate_of(&rel)).or_default() += o;
        }
        if i > 0 {
            *inner.entry(crate_of(&rel)).or_default() += i;
        }
    }

    // Always print the live table so a human can read the current numbers
    // straight off it when a baseline needs editing.
    eprint_table(&outer, &inner, scanned);

    let mut errs = compare("outer", &outer, OUTER_BASELINE);
    errs.extend(compare("inner", &inner, INNER_BASELINE));

    let expect_count = count_expect_lines(&root)?;
    if expect_count > EXPECT_BASELINE {
        errs.push(format!(
            ".expect( count in {EXPECT_CRATE_SRC} grew: baseline {EXPECT_BASELINE}, \
             found {expect_count} — lift the invariant into the type, do not push \
             it to runtime"
        ));
    }

    if !errs.is_empty() {
        for e in &errs {
            eprintln!("    {e}");
        }
        return Err(format!(
            "{} suppression-baseline violation(s) — see the per-crate table above",
            errs.len()
        ));
    }
    eprintln!(
        "xtask lint suppressions: clean — {} outer + {} inner #[allow] pairs, \
         .expect( lines {expect_count}/{EXPECT_BASELINE}",
        total(&outer),
        total(&inner),
    );
    Ok(())
}

/// Files whose suppressions are out of scope: anything generated
/// (`/target/`), the build-script surface (`build.rs`), and the dev-only
/// harness trees (`tests/`, `benches/`, `examples/`, `fuzz/`). Matched on
/// path components so a leading segment counts the same as an interior
/// one.
fn is_excluded(rel: &Path) -> bool {
    if rel.file_name().and_then(|n| n.to_str()) == Some("build.rs") {
        return true;
    }
    rel.components().any(|c| {
        matches!(
            c.as_os_str().to_str(),
            Some("target" | "tests" | "benches" | "examples" | "fuzz")
        )
    })
}

/// The crate a repo-relative path belongs to. `crates/<name>/…` → `<name>`;
/// anything else keys under its first path component so a stray file can
/// never drop out of the ledger unnoticed.
fn crate_of(rel: &Path) -> String {
    let comps: Vec<&str> = rel
        .components()
        .filter_map(|c| c.as_os_str().to_str())
        .collect();
    match comps.as_slice() {
        ["crates", name, ..] => (*name).to_owned(),
        [first, ..] => (*first).to_owned(),
        [] => "(unknown)".to_owned(),
    }
}

/// Parse one file and return its (outer, inner) `#[allow]` pair counts,
/// with `#[cfg(test)]` module bodies skipped.
fn count_file(path: &Path) -> Result<(usize, usize), String> {
    let text = fs::read_to_string(path).map_err(|e| format!("read {}: {e}", path.display()))?;
    let ast = syn::parse_file(&text).map_err(|e| format!("parse {}: {e}", path.display()))?;
    let mut counter = AllowCounter::default();
    counter.visit_file(&ast);
    if let Some(err) = counter.errors.into_iter().next() {
        return Err(format!("{}: {err}", path.display()));
    }
    Ok((counter.outer, counter.inner))
}

/// Syntax-tree visitor counting `#[allow]` (outer) and `#![allow]` (inner)
/// lint pairs, skipping the contents of any `#[cfg(test)]` module.
#[derive(Default)]
struct AllowCounter {
    outer: usize,
    inner: usize,
    errors: Vec<String>,
}

impl<'ast> Visit<'ast> for AllowCounter {
    fn visit_item_mod(&mut self, node: &'ast ItemMod) {
        // Do not descend into (nor count the attributes of) a test module.
        if node.attrs.iter().any(cfg_mentions_test) {
            return;
        }
        visit::visit_item_mod(self, node);
    }

    fn visit_attribute(&mut self, attr: &'ast Attribute) {
        if !attr.path().is_ident("allow") {
            return;
        }
        match count_lint_pairs(attr) {
            Ok(0) => {}
            Ok(pairs) => match attr.style {
                AttrStyle::Outer => self.outer += pairs,
                AttrStyle::Inner(_) => self.inner += pairs,
            },
            Err(e) => self.errors.push(e),
        }
    }
}

/// Number of lint idents inside an `#[allow(...)]` — the comma-separated
/// entries minus any `reason = "…"` clause.
fn count_lint_pairs(attr: &Attribute) -> Result<usize, String> {
    let Meta::List(list) = &attr.meta else {
        return Ok(0); // `#[allow]` with no list silences nothing.
    };
    let nested = list
        .parse_args_with(Punctuated::<Meta, Token![,]>::parse_terminated)
        .map_err(|e| format!("parse `allow(...)` list: {e}"))?;
    Ok(nested.iter().filter(|m| !is_reason(m)).count())
}

/// Whether a nested meta is the `reason = "…"` clause (not a lint).
fn is_reason(meta: &Meta) -> bool {
    matches!(meta, Meta::NameValue(nv) if nv.path.is_ident("reason"))
}

/// Whether a `#[cfg(...)]` attribute names `test` in its predicate —
/// directly (`cfg(test)`) or nested (`cfg(all(test, unix))`). A
/// `cfg(feature = "test-utils")` does not match: only the bare `test`
/// ident does.
fn cfg_mentions_test(attr: &Attribute) -> bool {
    attr.path().is_ident("cfg")
        && attr
            .parse_args::<Meta>()
            .is_ok_and(|meta| meta_mentions_test(&meta))
}

fn meta_mentions_test(meta: &Meta) -> bool {
    match meta {
        Meta::Path(path) => path.is_ident("test"),
        Meta::List(list) => list
            .parse_args_with(Punctuated::<Meta, Token![,]>::parse_terminated)
            .is_ok_and(|nested| nested.iter().any(meta_mentions_test)),
        Meta::NameValue(_) => false,
    }
}

/// Count `.expect(`-bearing lines under [`EXPECT_CRATE_SRC`] (line-count
/// semantics, matching the old `grep -c` sum). Errors when the directory
/// is gone, so a rename cannot silently defang the tripwire.
fn count_expect_lines(root: &Path) -> Result<usize, String> {
    let dir = root.join(EXPECT_CRATE_SRC);
    if !dir.is_dir() {
        return Err(format!(
            "{EXPECT_CRATE_SRC} not found — the .expect( tripwire is watching nothing"
        ));
    }
    let mut count = 0usize;
    for entry in walkdir::WalkDir::new(&dir) {
        let entry = entry.map_err(|e| format!("walk {}: {e}", dir.display()))?;
        let path = entry.path();
        // Directories never carry a `.rs` extension, so the extension gate
        // below is the only file-vs-dir discriminator we need; a stray
        // `.rs`-suffixed directory would surface loudly at `read_to_string`.
        if path.extension().and_then(|x| x.to_str()) != Some("rs") {
            continue;
        }
        let text = fs::read_to_string(path).map_err(|e| format!("read {}: {e}", path.display()))?;
        count += text.lines().filter(|l| l.contains(".expect(")).count();
    }
    Ok(count)
}

/// Exact-match a found map against a baseline in BOTH directions.
fn compare(kind: &str, found: &BTreeMap<String, usize>, baseline: &[(&str, usize)]) -> Vec<String> {
    let base: BTreeMap<&str, usize> = baseline.iter().copied().collect();
    let mut keys: BTreeSet<&str> = BTreeSet::new();
    keys.extend(found.keys().map(String::as_str));
    keys.extend(base.keys().copied());

    let mut errs = Vec::new();
    for k in keys {
        let f = found.get(k).copied().unwrap_or(0);
        let b = base.get(k).copied().unwrap_or(0);
        match f.cmp(&b) {
            Ordering::Greater => {
                errs.push(format!(
                    "{kind} suppressions grew in {k}: baseline {b}, found {f}"
                ));
            }
            Ordering::Less => errs.push(format!(
                "you reduced {kind} suppressions in {k}: lower its baseline from {b} to {f}"
            )),
            Ordering::Equal => {}
        }
    }
    errs
}

fn eprint_table(outer: &BTreeMap<String, usize>, inner: &BTreeMap<String, usize>, scanned: usize) {
    let mut keys: BTreeSet<&str> = BTreeSet::new();
    keys.extend(outer.keys().map(String::as_str));
    keys.extend(inner.keys().map(String::as_str));
    eprintln!(
        "xtask lint suppressions: #[allow] pairs per crate \
         (cfg(test) modules excluded), {scanned} files scanned"
    );
    for k in keys {
        eprintln!(
            "    {:<22} outer={:<3} inner={}",
            k,
            outer.get(k).copied().unwrap_or(0),
            inner.get(k).copied().unwrap_or(0),
        );
    }
}

fn total(map: &BTreeMap<String, usize>) -> usize {
    map.values().sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn counts(src: &str) -> AllowCounter {
        let ast = syn::parse_file(src).expect("fixture parses");
        let mut counter = AllowCounter::default();
        counter.visit_file(&ast);
        assert!(
            counter.errors.is_empty(),
            "visitor errored: {:?}",
            counter.errors
        );
        counter
    }

    /// Guards the determinism rule: a fixture path under a generated
    /// `target/` tree (and the other harness/build exclusions) must never
    /// enter the ledger.
    #[test]
    fn generated_and_harness_paths_are_excluded() {
        assert!(is_excluded(Path::new(
            "crates/aozora/target/debug/build/foo-abc/out/gen.rs"
        )));
        assert!(is_excluded(Path::new("crates/aozora-cli/tests/cli.rs")));
        assert!(is_excluded(Path::new(
            "crates/aozora-bench/benches/throughput.rs"
        )));
        assert!(is_excluded(Path::new("crates/aozora/examples/parse.rs")));
        assert!(is_excluded(Path::new(
            "crates/aozora-ffi/fuzz/fuzz_targets/roundtrip.rs"
        )));
        assert!(is_excluded(Path::new("crates/aozora/build.rs")));
        assert!(!is_excluded(Path::new("crates/aozora/src/lib.rs")));
    }

    #[test]
    fn crate_of_reads_the_second_path_component() {
        assert_eq!(
            crate_of(Path::new("crates/aozora-cli/src/main.rs")),
            "aozora-cli"
        );
    }

    #[test]
    fn outer_and_inner_pairs_ignore_reason_and_test_modules() {
        let c = counts(
            r#"
#![allow(clippy::a, clippy::b, reason = "two inner pairs; the reason clause is not a lint")]
#[allow(dead_code, reason = "one outer pair")]
fn shipped() {}
#[cfg(test)]
mod tests {
    #[allow(clippy::must_not_count, reason = "inside a test module — skipped")]
    fn t() {}
}
"#,
        );
        assert_eq!(c.inner, 2, "two inner lint pairs, reason excluded");
        assert_eq!(
            c.outer, 1,
            "one outer pair; the test-module allow is skipped"
        );
    }

    #[test]
    fn nested_cfg_all_test_module_is_skipped() {
        // One source line (the attrs live behind `\n` escapes), so the
        // reasonless `#[allow]` here exercises test-module skipping without
        // tripping strict-code's line-anchored bare-allow text scan.
        let c =
            counts("#[cfg(all(test, unix))]\nmod t {\n    #[allow(dead_code)]\n    fn f() {}\n}\n");
        assert_eq!((c.outer, c.inner), (0, 0));
    }

    #[test]
    fn expect_attribute_is_never_counted() {
        let c = counts("#[expect(dead_code)]\nfn f() {}\n");
        assert_eq!((c.outer, c.inner), (0, 0));
    }

    /// Self-check mirroring `docs.rs`: the live tree must sit exactly on
    /// the recorded baseline, in both directions.
    #[test]
    fn the_repo_is_on_the_suppression_baseline() {
        check().expect("every crate's #[allow] count matches its recorded baseline");
    }
}
