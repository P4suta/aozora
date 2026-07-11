//! WPT-style conformance runner.
//!
//! Walks the fixture set under
//! `crates/aozora-conformance/fixtures/render/<case>/`, reads each
//! case's `meta.toml` for its conformance metadata
//! (`feature` + `level`), runs the parser against the source, and
//! aggregates pass / fail counts by `(feature, level)`.
//!
//! ## Tier model
//!
//! Three tiers, mirroring W3C-style conformance levels:
//!
//! | Level   | Meaning                                          | Effect on `xtask conformance run` |
//! | ------- | ------------------------------------------------ | --------------------------------- |
//! | `must`  | Required for any conforming implementation.      | A failure here exits non-zero. |
//! | `should`| Recommended but not strictly required.           | A failure here logs a warning. |
//! | `may`   | Optional; implementations decide.                | Pure information, never fails. |
//!
//! The canonical implementation under test is the Rust parser
//! itself; the runner emits a `results.json` file so other
//! implementations (the tree-sitter reference grammar, third-party
//! ports) can publish their own per-case pass / fail ratio against
//! the same manifest.

use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::mem;
use std::path::{Path, PathBuf};

use aozora::json::{diagnostics, nodes, pairs};
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::ConformanceArgs;
use crate::ConformanceOp;
use crate::Implementation;
use crate::grammar;

const FIXTURE_REL: &str = "crates/aozora-conformance/fixtures/render";
const RESULTS_REL: &str = "crates/aozora-book/src/conformance-results.json";
const SPEC_VECTORS_REL: &str = "crates/aozora-conformance/spec-vectors/vectors";

/// Per-fixture S-expression snapshot for the tree-sitter implementation,
/// stored alongside the render goldens (`expected.html`, …).
const TS_GOLDEN: &str = "expected.tree-sitter.txt";
/// Published per-case pass / fail artefact for the tree-sitter run.
const TS_RESULTS_REL: &str = "crates/aozora-book/src/conformance-results-tree-sitter.json";
/// S-expression snapshot of the tree-sitter parse of every spec vector's
/// `source`. Lives at the top of `spec-vectors/` — outside the vendored
/// `vectors/` subtree that `sync-spec-vectors` / `verify-spec-vectors`
/// rewrite and diff — so it survives a re-sync untouched.
const TS_VECTORS_SNAPSHOT_REL: &str =
    "crates/aozora-conformance/spec-vectors/tree-sitter-snapshot.json";

/// Ratchet floor (G2c) for how many fixtures / spec vectors the reference
/// grammar parses with NO ERROR / MISSING nodes, bucketed by tier. Sits
/// next to the sexp snapshots. Unlike those snapshots — which pin exact
/// structure and are refreshed wholesale by `--update` — this records the
/// *count* of clean parses per tier as a floor: a routine grammar change
/// that quietly turns a must-tier fixture or a spec vector from clean to
/// ERROR drops the count and fails the run, so the loss can't hide inside a
/// large sexp diff. `--update` re-records the floor from the current run,
/// making any drop a reviewable number in the committed diff.
const ERROR_FREE_BASELINE_REL: &str =
    "crates/aozora-conformance/spec-vectors/error-free-baseline.json";
const ERROR_FREE_SCHEMA_VERSION: u32 = 1;

/// Which section of [`ERROR_FREE_BASELINE_REL`] a run owns: the fixture
/// runner writes `fixtures`, the spec-vector runner writes `vectors`.
#[derive(Debug, Clone, Copy)]
enum BaselineWhich {
    Fixtures,
    Vectors,
}

impl BaselineWhich {
    /// `(unit label, the `--update` command that re-records this section)`.
    fn labels(self) -> (&'static str, &'static str) {
        match self {
            Self::Fixtures => (
                "fixtures",
                "xtask conformance run --implementation tree-sitter --update",
            ),
            Self::Vectors => (
                "vectors",
                "xtask conformance vectors --implementation tree-sitter --update",
            ),
        }
    }

    /// Borrow this run's section out of a loaded baseline.
    fn section(self, baseline: &ErrorFreeBaseline) -> Option<&ErrorFreeSection> {
        match self {
            Self::Fixtures => baseline.fixtures.as_ref(),
            Self::Vectors => baseline.vectors.as_ref(),
        }
    }
}

/// The committed ERROR-free ratchet floor. Both sections are optional so
/// the file can be bootstrapped one runner at a time (each `--update`
/// preserves the other section).
#[derive(Debug, Default, Serialize, Deserialize)]
struct ErrorFreeBaseline {
    #[serde(rename = "schemaVersion")]
    schema_version: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    fixtures: Option<ErrorFreeSection>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    vectors: Option<ErrorFreeSection>,
}

/// Per-tier ERROR-free counts for one corpus (fixtures or spec vectors).
#[derive(Debug, Clone, Serialize, Deserialize)]
struct ErrorFreeSection {
    total: usize,
    #[serde(rename = "errorFree")]
    error_free: usize,
    #[serde(rename = "byLevel")]
    by_level: BTreeMap<String, usize>,
}

/// Project a tree-sitter [`Summary`] (whose `passed` field counts ERROR-free
/// parses) into the ratchet section.
fn error_free_section(summary: &Summary) -> ErrorFreeSection {
    ErrorFreeSection {
        total: summary.total,
        error_free: summary.passed,
        by_level: summary
            .by_level
            .iter()
            .map(|(level, ls)| (level.clone(), ls.passed))
            .collect(),
    }
}

/// Read the committed ratchet floor. A missing file is a hard error in
/// check mode — the baseline must be committed.
fn load_error_free_baseline(root: &Path) -> Result<ErrorFreeBaseline, String> {
    let path = root.join(ERROR_FREE_BASELINE_REL);
    match fs::read_to_string(&path) {
        Ok(text) => serde_json::from_str(&text)
            .map_err(|err| format!("parse {ERROR_FREE_BASELINE_REL}: {err}")),
        Err(err) if err.kind() == io::ErrorKind::NotFound => Err(format!(
            "conformance (tree-sitter): missing {ERROR_FREE_BASELINE_REL}; run \
             `xtask conformance run --implementation tree-sitter --update` and \
             `xtask conformance vectors --implementation tree-sitter --update` to create it"
        )),
        Err(err) => Err(format!("read {ERROR_FREE_BASELINE_REL}: {err}")),
    }
}

/// Rewrite one section of the ratchet floor from the current run, preserving
/// the other section (the two runners update independently).
fn write_error_free_section(
    root: &Path,
    which: BaselineWhich,
    section: ErrorFreeSection,
) -> Result<(), String> {
    let path = root.join(ERROR_FREE_BASELINE_REL);
    let mut baseline = match fs::read_to_string(&path) {
        Ok(text) => serde_json::from_str(&text)
            .map_err(|err| format!("parse {ERROR_FREE_BASELINE_REL}: {err}"))?,
        Err(err) if err.kind() == io::ErrorKind::NotFound => ErrorFreeBaseline::default(),
        Err(err) => return Err(format!("read {ERROR_FREE_BASELINE_REL}: {err}")),
    };
    baseline.schema_version = ERROR_FREE_SCHEMA_VERSION;
    match which {
        BaselineWhich::Fixtures => baseline.fixtures = Some(section),
        BaselineWhich::Vectors => baseline.vectors = Some(section),
    }
    let json = serde_json::to_string_pretty(&baseline)
        .map_err(|err| format!("serialize {ERROR_FREE_BASELINE_REL}: {err}"))?;
    fs::write(&path, format!("{json}\n")).map_err(|err| format!("write {}: {err}", path.display()))
}

/// Enforce the ratchet: every tier's ERROR-free count (and the total) must
/// be at least the committed floor. A drop fails the run; an improvement
/// passes but nudges toward re-baselining.
fn check_error_free_ratchet(
    root: &Path,
    which: BaselineWhich,
    summary: &Summary,
) -> Result<(), String> {
    let (unit, update_cmd) = which.labels();
    let current = error_free_section(summary);
    let file = load_error_free_baseline(root)?;
    let Some(baseline) = which.section(&file) else {
        return Err(format!(
            "conformance (tree-sitter): {ERROR_FREE_BASELINE_REL} has no `{unit}` section; \
             run `{update_cmd}` to record the ERROR-free ratchet baseline"
        ));
    };
    let regressions = error_free_regressions(&current, baseline);
    if !regressions.is_empty() {
        return Err(format!(
            "conformance (tree-sitter): {unit} ERROR-free ratchet regressed ({}); a construct \
             that used to parse without ERROR / MISSING nodes no longer does. Fix the grammar so \
             it parses cleanly again, or — if the loss is intentional — re-baseline with \
             `{update_cmd}` (the count drop is then reviewable in the committed diff).",
            regressions.join(", "),
        ));
    }
    if current.error_free > baseline.error_free {
        eprintln!(
            "  NOTE: {unit} ERROR-free improved {} -> {}; run `{update_cmd}` to ratchet the floor up.",
            baseline.error_free, current.error_free,
        );
    }
    Ok(())
}

/// Pure ratchet comparison. Returns the labels of the tiers (and/or the
/// aggregate `total`) whose ERROR-free count fell below the committed floor;
/// an empty vector means no regression. A tier the baseline records but the
/// current run does not is treated as `0` (a regression against any positive
/// floor).
fn error_free_regressions(current: &ErrorFreeSection, baseline: &ErrorFreeSection) -> Vec<String> {
    let mut regressions = Vec::new();
    for (level, &floor) in &baseline.by_level {
        let cur = current.by_level.get(level).copied().unwrap_or(0);
        if cur < floor {
            regressions.push(format!("{level} {cur} < {floor}"));
        }
    }
    if current.error_free < baseline.error_free {
        regressions.push(format!(
            "total {} < {}",
            current.error_free, baseline.error_free
        ));
    }
    regressions
}

pub(crate) fn dispatch(args: &ConformanceArgs) -> Result<(), String> {
    match &args.op {
        ConformanceOp::Run(run_args) => match run_args.implementation {
            Implementation::Rust => run(),
            Implementation::TreeSitter => run_tree_sitter(run_args.update),
        },
        ConformanceOp::Vectors(vec_args) => match vec_args.implementation {
            Implementation::Rust => run_vectors(),
            Implementation::TreeSitter => run_vectors_tree_sitter(vec_args.update),
        },
        ConformanceOp::Grammar(grammar_args) => {
            // `--update` regenerates the committed parser; `--check` (or no
            // flag, the default) runs the drift gate. clap's `conflicts_with`
            // rejects both at once; the tuple reads both flags so neither is
            // a dead field.
            let regenerate = matches!((grammar_args.update, grammar_args.check), (true, _));
            grammar::dispatch(regenerate)
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
enum Level {
    Must,
    Should,
    May,
}

impl Level {
    fn parse(s: &str) -> Result<Self, String> {
        match s {
            "must" => Ok(Self::Must),
            "should" => Ok(Self::Should),
            "may" => Ok(Self::May),
            _ => Err(format!(
                "unknown conformance level {s:?} (expected must / should / may)"
            )),
        }
    }
}

#[derive(Deserialize)]
struct Meta {
    feature: String,
    level: String,
}

#[derive(Debug, Serialize)]
struct CaseResult {
    case: String,
    feature: String,
    level: Level,
    passed: bool,
    message: Option<String>,
}

#[derive(Debug, Serialize)]
struct Summary {
    implementation: String,
    total: usize,
    passed: usize,
    failed: usize,
    by_level: BTreeMap<String, LevelSummary>,
    cases: Vec<CaseResult>,
}

#[derive(Debug, Serialize, Default)]
struct LevelSummary {
    total: usize,
    passed: usize,
    failed: usize,
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

fn run() -> Result<(), String> {
    let root = workspace_root()?;
    let cases = collect_cases(&root)?;
    let summary = build_summary(cases, "rust");
    print_summary(&summary);
    write_results(&root, &summary, RESULTS_REL)?;

    let must_failed = summary.by_level.get("must").map_or(0, |s| s.failed);
    if must_failed > 0 {
        let results_path = root.join(RESULTS_REL);
        return Err(format!(
            "conformance: {must_failed} `must`-tier case(s) failed (see {} for detail)",
            results_path.display()
        ));
    }
    Ok(())
}

fn collect_cases(root: &Path) -> Result<Vec<CaseResult>, String> {
    let fixtures_dir = root.join(FIXTURE_REL);
    let mut entries: Vec<_> = fs::read_dir(&fixtures_dir)
        .map_err(|err| format!("read_dir {}: {err}", fixtures_dir.display()))?
        .filter_map(Result::ok)
        .filter(|e| e.path().is_dir())
        .collect();
    entries.sort_by_key(fs::DirEntry::file_name);

    // Rayon's `par_iter()` preserves the input order on `collect()`,
    // so the alphabetised fixture sequence above carries through to
    // the resulting `Vec<CaseResult>`. `run_case` reads the fixture
    // files and parses them through the aozora pipeline; both are
    // pure with no shared mutable state, so the parallelisation is
    // safe by construction.
    entries
        .par_iter()
        .map(|entry| -> Result<CaseResult, String> {
            let case_dir = entry.path();
            let case_name = case_dir
                .file_name()
                .and_then(|s| s.to_str())
                .ok_or_else(|| format!("non-utf8 fixture name {}", case_dir.display()))?
                .to_owned();

            let meta_path = case_dir.join("meta.toml");
            let meta_str = fs::read_to_string(&meta_path)
                .map_err(|err| format!("read {}: {err}", meta_path.display()))?;
            let meta: Meta = toml::from_str(&meta_str)
                .map_err(|err| format!("parse {}: {err}", meta_path.display()))?;
            let level = Level::parse(&meta.level)?;

            let (passed, message) = match run_case(&case_dir) {
                Ok(()) => (true, None),
                Err(msg) => (false, Some(msg)),
            };
            Ok(CaseResult {
                case: case_name,
                feature: meta.feature,
                level,
                passed,
                message,
            })
        })
        .collect()
}

fn build_summary(cases: Vec<CaseResult>, implementation: &str) -> Summary {
    let mut by_level: BTreeMap<Level, LevelSummary> = BTreeMap::new();
    for case in &cases {
        let bucket = by_level.entry(case.level).or_default();
        bucket.total += 1;
        if case.passed {
            bucket.passed += 1;
        } else {
            bucket.failed += 1;
        }
    }

    let total = cases.len();
    let passed = cases.iter().filter(|c| c.passed).count();
    let failed = total - passed;

    Summary {
        implementation: implementation.to_owned(),
        total,
        passed,
        failed,
        by_level: by_level
            .into_iter()
            .map(|(level, ls)| (level_slug(level).to_owned(), ls))
            .collect(),
        cases,
    }
}

fn level_slug(level: Level) -> &'static str {
    match level {
        Level::Must => "must",
        Level::Should => "should",
        Level::May => "may",
    }
}

fn write_results(root: &Path, summary: &Summary, rel: &str) -> Result<(), String> {
    let results_path = root.join(rel);
    if let Some(parent) = results_path.parent() {
        fs::create_dir_all(parent)
            .map_err(|err| format!("create_dir_all {}: {err}", parent.display()))?;
    }
    let json =
        serde_json::to_string_pretty(summary).map_err(|err| format!("serialize summary: {err}"))?;
    fs::write(&results_path, json)
        .map_err(|err| format!("write {}: {err}", results_path.display()))?;
    eprintln!(
        "xtask conformance run: wrote results to {}",
        results_path.display()
    );
    Ok(())
}

fn run_case(dir: &Path) -> Result<(), String> {
    let source_path = dir.join("source.txt");
    let source = fs::read_to_string(&source_path)
        .map_err(|err| format!("read {}: {err}", source_path.display()))?;
    let doc = aozora::Document::new(source);
    let tree = doc.parse();

    let actual_html = tree.to_html();
    let actual_serialize = tree.to_source();

    let expected_html = fs::read_to_string(dir.join("expected.html"))
        .map_err(|err| format!("read {}/expected.html: {err}", dir.display()))?;
    let expected_serialize = fs::read_to_string(dir.join("expected.serialize.txt"))
        .map_err(|err| format!("read {}/expected.serialize.txt: {err}", dir.display()))?;

    if actual_html != expected_html {
        return Err("HTML output drift vs expected.html".to_owned());
    }
    if actual_serialize != expected_serialize {
        return Err("serialize output drift vs expected.serialize.txt".to_owned());
    }
    Ok(())
}

fn print_summary(summary: &Summary) {
    eprintln!(
        "xtask conformance: {} / {} passed (impl={})",
        summary.passed, summary.total, summary.implementation,
    );
    for (level, ls) in &summary.by_level {
        eprintln!(
            "  {level:6} {passed:3} / {total:3} pass ({failed} fail)",
            level = level,
            passed = ls.passed,
            total = ls.total,
            failed = ls.failed,
        );
    }
    for case in &summary.cases {
        if !case.passed {
            eprintln!(
                "  FAIL [{level:?} {feature}] {case}: {msg}",
                level = case.level,
                feature = case.feature,
                case = case.case,
                msg = case.message.as_deref().unwrap_or("(no message)"),
            );
        }
    }
}

// ────────────────────────────────────────────────────────────────────
// tree-sitter reference grammar
// ────────────────────────────────────────────────────────────────────
//
// The reference grammar lives in `crates/tree-sitter-aozora` (built
// from its `grammar.js` into the committed `src/parser.c`). It is a
// *syntactic skeleton* — it classifies bracket structure but cannot
// render HTML, so the byte-equality comparison the Rust path uses does
// not apply. Two orthogonal signals replace it:
//
//   1. A **per-tier pass rate** (issue #82's ask): a fixture "passes"
//      when the grammar parses it without ERROR / MISSING nodes. This
//      is a coverage measurement, printed per level — it never fails
//      the gate. Constructs the grammar does not model (stateful
//      container pairing, forward bouten, unclosed brackets) honestly
//      count as non-passing.
//   2. A **snapshot drift gate**: each fixture's `root.to_sexp()` is
//      pinned to `expected.tree-sitter.txt`. `to_sexp()` carries node
//      kinds / fields with no byte offsets, so it is deterministic and
//      only changes when the grammar's structure changes — exactly the
//      drift we want surfaced. ANY mismatch fails, tier-independent: a
//      snapshot is a fingerprint, and the rust path's must/should/may
//      leniency (which models *partial conformance*) does not apply to
//      a fingerprint, where every change is a regression-or-intentional
//      -update worth a human's eyes. `--update` regenerates the
//      snapshots after an intentional grammar change.

/// Parse `source` with the reference grammar, returning its
/// S-expression and whether the parse contains any ERROR / MISSING
/// node. `root.has_error()` already subsumes both.
fn tree_sitter_parse(source: &str) -> Result<(String, bool), String> {
    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(&tree_sitter_aozora::LANGUAGE.into())
        .map_err(|err| format!("set tree-sitter language: {err}"))?;
    let tree = parser
        .parse(source, None)
        .ok_or_else(|| "tree-sitter parse returned None".to_owned())?;
    let root = tree.root_node();
    Ok((root.to_sexp(), root.has_error()))
}

#[allow(
    clippy::too_many_lines,
    reason = "one linear pass: read fixtures, parse each, then either refresh \
              the sexp goldens + ERROR-free baseline or gate on drift + the \
              ratchet — splitting the flow would fragment a single narrative"
)]
fn run_tree_sitter(update: bool) -> Result<(), String> {
    let root = workspace_root()?;
    let fixtures_dir = root.join(FIXTURE_REL);
    let mut entries: Vec<_> = fs::read_dir(&fixtures_dir)
        .map_err(|err| format!("read_dir {}: {err}", fixtures_dir.display()))?
        .filter_map(Result::ok)
        .filter(|entry| entry.path().is_dir())
        .collect();
    entries.sort_by_key(fs::DirEntry::file_name);

    // A plain sequential walk: `tree_sitter::Parser` is not shareable
    // across rayon worker threads, and parsing 64 tiny fixtures is a
    // sub-millisecond affair, so the rust path's `par_iter` buys nothing
    // here.
    let mut cases = Vec::with_capacity(entries.len());
    let mut drifts = Vec::new();
    let mut missing = Vec::new();
    let mut written = 0usize;

    for entry in &entries {
        let case_dir = entry.path();
        let case_name = case_dir
            .file_name()
            .and_then(|s| s.to_str())
            .ok_or_else(|| format!("non-utf8 fixture name {}", case_dir.display()))?
            .to_owned();

        let meta_path = case_dir.join("meta.toml");
        let meta_str = fs::read_to_string(&meta_path)
            .map_err(|err| format!("read {}: {err}", meta_path.display()))?;
        let meta: Meta = toml::from_str(&meta_str)
            .map_err(|err| format!("parse {}: {err}", meta_path.display()))?;
        let level = Level::parse(&meta.level)?;

        let source_path = case_dir.join("source.txt");
        let source = fs::read_to_string(&source_path)
            .map_err(|err| format!("read {}: {err}", source_path.display()))?;
        let (sexp, has_error) =
            tree_sitter_parse(&source).map_err(|err| format!("{case_name}: {err}"))?;

        let golden_path = case_dir.join(TS_GOLDEN);
        if update {
            fs::write(&golden_path, &sexp)
                .map_err(|err| format!("write {}: {err}", golden_path.display()))?;
            written += 1;
        } else {
            match fs::read_to_string(&golden_path) {
                Ok(golden) if golden == sexp => {}
                Ok(_) => drifts.push(case_name.clone()),
                Err(err) if err.kind() == io::ErrorKind::NotFound => {
                    missing.push(case_name.clone());
                }
                Err(err) => return Err(format!("read {}: {err}", golden_path.display())),
            }
        }

        cases.push(CaseResult {
            case: case_name,
            feature: meta.feature,
            level,
            passed: !has_error,
            message: has_error.then(|| "grammar produced ERROR / MISSING node(s)".to_owned()),
        });
    }

    let summary = build_summary(cases, "tree-sitter");
    print_ts_summary(&summary, &drifts, "fixtures");

    if update {
        write_results(&root, &summary, TS_RESULTS_REL)?;
        write_error_free_section(&root, BaselineWhich::Fixtures, error_free_section(&summary))?;
        eprintln!(
            "xtask conformance run (tree-sitter): wrote {written} snapshot(s) + results + \
             ERROR-free baseline"
        );
        return Ok(());
    }

    if !missing.is_empty() {
        return Err(format!(
            "conformance (tree-sitter): {n} fixture(s) missing {TS_GOLDEN} ({cases}); \
             run `xtask conformance run --implementation tree-sitter --update` to create them",
            n = missing.len(),
            cases = missing.join(", "),
        ));
    }
    if !drifts.is_empty() {
        return Err(format!(
            "conformance (tree-sitter): {n} fixture(s) drifted from {TS_GOLDEN} ({cases}); \
             if the grammar change is intentional, run \
             `xtask conformance run --implementation tree-sitter --update`",
            n = drifts.len(),
            cases = drifts.join(", "),
        ));
    }
    check_error_free_ratchet(&root, BaselineWhich::Fixtures, &summary)?;
    Ok(())
}

/// Print the per-tier pass rate (coverage) and any snapshot drift (the
/// gate). The pass rate is informational; only `drifts` fails the run.
fn print_ts_summary(summary: &Summary, drifts: &[String], unit: &str) {
    eprintln!(
        "xtask conformance (tree-sitter): {} / {} {unit} parse without ERROR nodes",
        summary.passed, summary.total,
    );
    for (level, ls) in &summary.by_level {
        eprintln!(
            "  {level:6} {passed:3} / {total:3} clean ({uncovered} with ERROR)",
            level = level,
            passed = ls.passed,
            total = ls.total,
            uncovered = ls.failed,
        );
    }
    if !drifts.is_empty() {
        eprintln!("  DRIFT: {} {unit} diverge from snapshot:", drifts.len());
        for case in drifts {
            eprintln!("    - {case}");
        }
    }
}

/// One spec vector's tree-sitter parse, pinned in the snapshot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct TsVectorEntry {
    name: String,
    /// The grammar produced an ERROR / MISSING node (a non-pass).
    error: bool,
    /// `root.to_sexp()` — the structural fingerprint that gates drift.
    sexp: String,
}

/// The committed tree-sitter snapshot over the whole spec-vector corpus.
#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
struct TsVectorSnapshot {
    #[serde(rename = "schemaVersion")]
    schema_version: u32,
    implementation: String,
    vectors: Vec<TsVectorEntry>,
}

/// Run the reference grammar over every spec vector's `source` (#242).
///
/// Mirrors `run_tree_sitter` but over the specification corpus: a
/// per-tier pass rate report (no ERROR nodes) plus a single
/// S-expression snapshot gate (`TS_VECTORS_SNAPSHOT_REL`). `--update`
/// regenerates the snapshot after an intentional grammar change.
#[allow(
    clippy::too_many_lines,
    reason = "one linear pass: read every spec vector, parse each, then either \
              refresh the sexp snapshot + ERROR-free baseline or gate on drift + \
              the ratchet — splitting the flow would fragment a single narrative"
)]
fn run_vectors_tree_sitter(update: bool) -> Result<(), String> {
    let root = workspace_root()?;
    let vectors_dir = root.join(SPEC_VECTORS_REL);
    let mut entries: Vec<_> = fs::read_dir(&vectors_dir)
        .map_err(|err| format!("read_dir {}: {err}", vectors_dir.display()))?
        .filter_map(Result::ok)
        .filter(|entry| entry.path().is_dir())
        .collect();
    entries.sort_by_key(fs::DirEntry::file_name);

    let mut cases = Vec::with_capacity(entries.len());
    let mut snapshot_vectors = Vec::with_capacity(entries.len());

    for entry in &entries {
        let vector_path = entry.path().join("vector.json");
        let raw = fs::read_to_string(&vector_path)
            .map_err(|err| format!("read {}: {err}", vector_path.display()))?;
        let vector: Vector = serde_json::from_str(&raw)
            .map_err(|err| format!("parse {}: {err}", vector_path.display()))?;
        let level = Level::parse(&vector.meta.level)?;
        let (sexp, has_error) =
            tree_sitter_parse(&vector.source).map_err(|err| format!("{}: {err}", vector.name))?;

        cases.push(CaseResult {
            case: vector.name.clone(),
            feature: vector.meta.feature.clone(),
            level,
            passed: !has_error,
            message: has_error.then(|| "grammar produced ERROR / MISSING node(s)".to_owned()),
        });
        snapshot_vectors.push(TsVectorEntry {
            name: vector.name,
            error: has_error,
            sexp,
        });
    }

    // Stable order regardless of read_dir: sort the snapshot by name.
    snapshot_vectors.sort_by(|a, b| a.name.cmp(&b.name));
    let snapshot = TsVectorSnapshot {
        schema_version: 1,
        implementation: "tree-sitter".to_owned(),
        vectors: snapshot_vectors,
    };
    let summary = build_summary(cases, "tree-sitter");
    let snapshot_path = root.join(TS_VECTORS_SNAPSHOT_REL);

    if update {
        let json = serde_json::to_string_pretty(&snapshot)
            .map_err(|err| format!("serialize snapshot: {err}"))?;
        fs::write(&snapshot_path, format!("{json}\n"))
            .map_err(|err| format!("write {}: {err}", snapshot_path.display()))?;
        write_error_free_section(&root, BaselineWhich::Vectors, error_free_section(&summary))?;
        print_ts_summary(&summary, &[], "spec vectors");
        eprintln!(
            "xtask conformance vectors (tree-sitter): wrote {} vector snapshot to {} + \
             ERROR-free baseline",
            snapshot.vectors.len(),
            snapshot_path.display()
        );
        return Ok(());
    }

    let committed = match fs::read_to_string(&snapshot_path) {
        Ok(text) => text,
        Err(err) if err.kind() == io::ErrorKind::NotFound => {
            return Err(format!(
                "conformance vectors (tree-sitter): missing {TS_VECTORS_SNAPSHOT_REL}; run \
                 `xtask conformance vectors --implementation tree-sitter --update` to create it"
            ));
        }
        Err(err) => return Err(format!("read {}: {err}", snapshot_path.display())),
    };
    let prev: TsVectorSnapshot = serde_json::from_str(&committed)
        .map_err(|err| format!("parse {}: {err}", snapshot_path.display()))?;

    let drifts = snapshot_drifts(&prev, &snapshot);
    print_ts_summary(&summary, &drifts, "spec vectors");
    if !drifts.is_empty() {
        return Err(format!(
            "conformance vectors (tree-sitter): {n} vector(s) drifted from {TS_VECTORS_SNAPSHOT_REL} \
             ({cases}); if the grammar change is intentional, run \
             `xtask conformance vectors --implementation tree-sitter --update`",
            n = drifts.len(),
            cases = drifts.join(", "),
        ));
    }
    check_error_free_ratchet(&root, BaselineWhich::Vectors, &summary)?;
    Ok(())
}

/// Names whose tree-sitter parse changed between the committed snapshot
/// and a fresh run — including vectors added (`(new)`) or removed
/// (`(removed)`) from the corpus.
fn snapshot_drifts(prev: &TsVectorSnapshot, current: &TsVectorSnapshot) -> Vec<String> {
    let index = |snap: &TsVectorSnapshot| -> BTreeMap<String, (bool, String)> {
        snap.vectors
            .iter()
            .map(|v| (v.name.clone(), (v.error, v.sexp.clone())))
            .collect()
    };
    let prev_index = index(prev);
    let cur_index = index(current);

    let mut drifts = Vec::new();
    for (name, cur) in &cur_index {
        match prev_index.get(name) {
            Some(old) if old == cur => {}
            Some(_) => drifts.push(name.clone()),
            None => drifts.push(format!("{name} (new)")),
        }
    }
    for name in prev_index.keys() {
        if !cur_index.contains_key(name) {
            drifts.push(format!("{name} (removed)"));
        }
    }
    drifts.sort();
    drifts
}

// ────────────────────────────────────────────────────────────────────
// Specification conformance vectors
// ────────────────────────────────────────────────────────────────────
//
// The sibling `aozora-notation-spec` repository owns the conformance
// corpus; its vectors are vendored under
// `crates/aozora-conformance/spec-vectors/` by `just sync-spec-vectors`.
// Unlike the fixture runner above (which compares the parser against its
// OWN golden output), this runner holds the parser to the
// SPECIFICATION's expectations: it parses each vector's `source` and
// diffs the projections the spec pins in `expected`, governed by
// `meta.level` per `spec-vectors/RUNNER.md` (`must` fails, `should` /
// `may` warn). The `html` projection is a reference rendering (spec §8,
// informative) and only ever warns.

#[derive(Deserialize)]
struct Vector {
    name: String,
    meta: VectorMeta,
    source: String,
    expected: VectorExpected,
}

#[derive(Deserialize)]
struct VectorMeta {
    feature: String,
    level: String,
}

#[derive(Deserialize)]
struct VectorExpected {
    #[serde(default)]
    html: Option<String>,
    #[serde(default)]
    serialize: Option<String>,
    #[serde(default)]
    nodes: Option<Value>,
    #[serde(default)]
    pairs: Option<Value>,
    #[serde(default)]
    diagnostics: Option<Vec<ExpectedDiagnostic>>,
}

#[derive(Deserialize)]
struct ExpectedDiagnostic {
    code: String,
    severity: String,
    #[serde(default)]
    span: Option<SpanCmp>,
}

#[derive(Deserialize, Clone, Copy, PartialEq, Eq)]
struct SpanCmp {
    start: u64,
    end: u64,
}

struct ActualDiagnostic {
    code: String,
    severity: String,
    span: SpanCmp,
}

fn run_vectors() -> Result<(), String> {
    let root = workspace_root()?;
    let vectors_dir = root.join(SPEC_VECTORS_REL);
    let mut entries: Vec<_> = fs::read_dir(&vectors_dir)
        .map_err(|err| format!("read_dir {}: {err}", vectors_dir.display()))?
        .filter_map(Result::ok)
        .filter(|entry| entry.path().is_dir())
        .collect();
    entries.sort_by_key(fs::DirEntry::file_name);

    let mut total = 0usize;
    let mut must_failed = 0usize;
    let mut should_warned = 0usize;
    let mut html_warned = 0usize;

    for entry in &entries {
        let vector_path = entry.path().join("vector.json");
        let raw = fs::read_to_string(&vector_path)
            .map_err(|err| format!("read {}: {err}", vector_path.display()))?;
        let vector: Vector = serde_json::from_str(&raw)
            .map_err(|err| format!("parse {}: {err}", vector_path.display()))?;
        let level = Level::parse(&vector.meta.level)?;
        total += 1;

        let (normative, html) = compare_vector(&vector)?;

        // Informative reference rendering (spec §8): a mismatch never
        // fails the gate, regardless of level.
        if html.is_some() {
            html_warned += 1;
            eprintln!(
                "  WARN  [html   {feature}] {name}: html diverges from the spec example",
                feature = vector.meta.feature,
                name = vector.name,
            );
        }

        if normative.is_empty() {
            continue;
        }
        let detail = normative.join(", ");
        match level {
            Level::Must => {
                must_failed += 1;
                eprintln!(
                    "  FAIL  [must   {feature}] {name}: {detail}",
                    feature = vector.meta.feature,
                    name = vector.name,
                );
            }
            Level::Should => {
                should_warned += 1;
                eprintln!(
                    "  WARN  [should {feature}] {name}: {detail}",
                    feature = vector.meta.feature,
                    name = vector.name,
                );
            }
            Level::May => eprintln!(
                "  INFO  [may    {feature}] {name}: {detail}",
                feature = vector.meta.feature,
                name = vector.name,
            ),
        }
    }

    eprintln!(
        "xtask conformance vectors: {total} vector(s) — {must_failed} must-fail, \
         {should_warned} should-warn, {html_warned} html-warn"
    );

    if must_failed > 0 {
        return Err(format!(
            "conformance vectors: {must_failed} `must`-tier vector(s) diverge from the specification"
        ));
    }
    Ok(())
}

/// Parse one vector's source and diff each projection the spec pins.
///
/// Returns `(normative_mismatches, html_mismatch)`: the first drives
/// pass / fail by level, the second is always informative.
fn compare_vector(vector: &Vector) -> Result<(Vec<String>, Option<String>), String> {
    let doc = aozora::Document::new(vector.source.clone());
    let tree = doc.parse();
    let mut mismatches = Vec::new();

    if let Some(expected) = vector.expected.serialize.as_ref() {
        // The canonical serialize of the source matches the pinned golden.
        if tree.to_source() != *expected {
            mismatches.push("serialize".to_owned());
        }
        // Contract unification (#190): the golden is its own fixed point —
        // re-serialising the canonical form returns it unchanged. Folds the
        // "source-exact" check into the single canonical-idempotence
        // invariant, so a golden that is idempotent yet differs from the
        // canonical form of its source is unrepresentable.
        if aozora::Document::new(expected.clone()).parse().to_source() != *expected {
            mismatches.push("serialize-fixed-point".to_owned());
        }
    }
    if let Some(expected) = &vector.expected.nodes {
        let actual = Value::Array(wire_data(&nodes(&tree))?);
        mismatches.extend((actual != *expected).then(|| "nodes".to_owned()));
    }
    if let Some(expected) = &vector.expected.pairs {
        let actual = Value::Array(wire_data(&pairs(&tree))?);
        mismatches.extend((actual != *expected).then(|| "pairs".to_owned()));
    }
    if let Some(expected) = &vector.expected.diagnostics {
        let actual = normalized_actual_diagnostics(&diagnostics(tree.diagnostics()))?;
        mismatches
            .extend((!diagnostics_match(expected, &actual)).then(|| "diagnostics".to_owned()));
    }

    let html = vector
        .expected
        .html
        .as_ref()
        .and_then(|expected| (tree.to_html() != *expected).then(|| "html".to_owned()));

    Ok((mismatches, html))
}

/// Pull the `data` array out of a `{ schema_version, data }` wire
/// envelope, taking ownership of the items.
fn wire_data(json: &str) -> Result<Vec<Value>, String> {
    let mut value: Value =
        serde_json::from_str(json).map_err(|err| format!("parse wire envelope: {err}"))?;
    match value.get_mut("data") {
        Some(Value::Array(items)) => Ok(mem::take(items)),
        _ => Err("wire envelope missing `data` array".to_owned()),
    }
}

/// Project the parser's diagnostic wire entries into the spec's
/// `{ code, severity, span }` shape.
///
/// Strips the pipeline-internal sanity checks (not part of the spec's
/// diagnostic contract) and rewrites the `snake_case` `kind` to the
/// kebab-case `code` of §9 — mirrors
/// `aozora-notation-spec/tools/import_vectors.py::map_diagnostic`.
fn normalized_actual_diagnostics(wire_json: &str) -> Result<Vec<ActualDiagnostic>, String> {
    let mut out = Vec::new();
    for entry in wire_data(wire_json)? {
        if entry.get("source").and_then(Value::as_str) == Some("internal") {
            continue;
        }
        let kind = entry
            .get("kind")
            .and_then(Value::as_str)
            .ok_or("diagnostic wire entry missing `kind`")?;
        let severity = entry
            .get("severity")
            .and_then(Value::as_str)
            .ok_or("diagnostic wire entry missing `severity`")?;
        let span = entry
            .get("span")
            .ok_or("diagnostic wire entry missing `span`")?;
        out.push(ActualDiagnostic {
            code: kind.replace('_', "-"),
            severity: severity.to_owned(),
            span: span_from_value(span)?,
        });
    }
    Ok(out)
}

fn span_from_value(value: &Value) -> Result<SpanCmp, String> {
    let start = value
        .get("start")
        .and_then(Value::as_u64)
        .ok_or("span missing `start`")?;
    let end = value
        .get("end")
        .and_then(Value::as_u64)
        .ok_or("span missing `end`")?;
    Ok(SpanCmp { start, end })
}

/// Order-sensitive equality per `RUNNER.md`: every diagnostic matches on
/// `code` and `severity`, and on `span` only when the spec pins one.
fn diagnostics_match(expected: &[ExpectedDiagnostic], actual: &[ActualDiagnostic]) -> bool {
    expected.len() == actual.len()
        && expected.iter().zip(actual).all(|(want, got)| {
            want.code == got.code
                && want.severity == got.severity
                && want.span.is_none_or(|span| span == got.span)
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn case(level: Level, passed: bool) -> CaseResult {
        CaseResult {
            case: "c".to_owned(),
            feature: "f".to_owned(),
            level,
            passed,
            message: None,
        }
    }

    #[test]
    fn level_parse_accepts_each_tier() {
        assert_eq!(Level::parse("must").expect("must parses"), Level::Must);
        assert_eq!(
            Level::parse("should").expect("should parses"),
            Level::Should
        );
        assert_eq!(Level::parse("may").expect("may parses"), Level::May);
    }

    #[test]
    fn level_parse_rejects_unknown() {
        let err = Level::parse("required").expect_err("unknown tier rejected");
        assert!(err.contains("required"), "error names bad input: {err}");
        assert!(
            err.contains("must / should / may"),
            "error lists tiers: {err}"
        );
    }

    #[test]
    fn level_slug_round_trips_with_parse() {
        for level in [Level::Must, Level::Should, Level::May] {
            let slug = level_slug(level);
            assert_eq!(Level::parse(slug).expect("slug parses"), level);
        }
    }

    #[test]
    fn level_ordering_is_must_then_should_then_may() {
        assert!(Level::Must < Level::Should, "must precedes should");
        assert!(Level::Should < Level::May, "should precedes may");
    }

    #[test]
    fn build_summary_counts_pass_fail_overall() {
        let cases = vec![
            case(Level::Must, true),
            case(Level::Must, false),
            case(Level::Should, true),
        ];
        let summary = build_summary(cases, "rust");
        assert_eq!(summary.total, 3, "total cases");
        assert_eq!(summary.passed, 2, "passed cases");
        assert_eq!(summary.failed, 1, "failed cases");
        assert_eq!(summary.implementation, "rust", "rust is impl under test");
    }

    #[test]
    fn build_summary_buckets_by_level_slug() {
        let cases = vec![
            case(Level::Must, true),
            case(Level::Must, false),
            case(Level::May, true),
        ];
        let summary = build_summary(cases, "rust");
        let must = summary.by_level.get("must").expect("must bucket present");
        assert_eq!(must.total, 2, "two must cases");
        assert_eq!(must.passed, 1, "one must pass");
        assert_eq!(must.failed, 1, "one must fail");
        let may = summary.by_level.get("may").expect("may bucket present");
        assert_eq!(may.total, 1, "one may case");
        assert!(
            !summary.by_level.contains_key("should"),
            "no should cases → no bucket"
        );
    }

    #[test]
    fn build_summary_empty_is_all_zero() {
        let summary = build_summary(Vec::new(), "rust");
        assert_eq!(summary.total, 0, "no cases");
        assert_eq!(summary.passed, 0, "no passes");
        assert_eq!(summary.failed, 0, "no fails");
        assert!(summary.by_level.is_empty(), "no level buckets");
    }

    // ── ERROR-free ratchet (G2c) ────────────────────────────────────

    fn efree(total: usize, must: usize, should: usize, may: usize) -> ErrorFreeSection {
        let by_level = [("must", must), ("should", should), ("may", may)]
            .into_iter()
            .map(|(k, v)| (k.to_owned(), v))
            .collect();
        ErrorFreeSection {
            total,
            error_free: must + should + may,
            by_level,
        }
    }

    #[test]
    fn error_free_section_projects_summary_clean_counts() {
        // A tree-sitter summary's `passed` field IS its ERROR-free count.
        let summary = build_summary(
            vec![
                case(Level::Must, true),
                case(Level::Must, false),
                case(Level::Should, true),
            ],
            "tree-sitter",
        );
        let sec = error_free_section(&summary);
        assert_eq!(sec.total, 3, "total cases");
        assert_eq!(sec.error_free, 2, "two parsed clean");
        assert_eq!(sec.by_level.get("must").copied(), Some(1), "one must clean");
        assert_eq!(
            sec.by_level.get("should").copied(),
            Some(1),
            "one should clean"
        );
    }

    #[test]
    fn ratchet_passes_when_counts_hold_or_improve() {
        let floor = efree(127, 21, 36, 3);
        assert!(
            error_free_regressions(&efree(127, 21, 36, 3), &floor).is_empty(),
            "equal to the floor is not a regression"
        );
        assert!(
            error_free_regressions(&efree(127, 25, 40, 3), &floor).is_empty(),
            "improving every tier is not a regression (the floor is a minimum)"
        );
    }

    #[test]
    fn ratchet_flags_a_dropped_tier() {
        let floor = efree(127, 21, 36, 3);
        // One must-tier fixture went from clean to ERROR.
        let regressions = error_free_regressions(&efree(127, 20, 36, 3), &floor);
        assert!(
            regressions.iter().any(|m| m.contains("must 20 < 21")),
            "names the tier and the drop: {regressions:?}"
        );
    }

    #[test]
    fn ratchet_flags_dropped_total_and_missing_tier() {
        let floor = efree(127, 21, 36, 3); // error_free == 60
        // A run that lost the whole `must` tier: absent tier counts as 0.
        let by_level = [("should", 36), ("may", 3)]
            .into_iter()
            .map(|(k, v)| (k.to_owned(), v))
            .collect();
        let current = ErrorFreeSection {
            total: 127,
            error_free: 39,
            by_level,
        };
        let regressions = error_free_regressions(&current, &floor);
        assert!(
            regressions.iter().any(|m| m.contains("must 0 < 21")),
            "an absent baseline tier is a regression: {regressions:?}"
        );
        assert!(
            regressions.iter().any(|m| m.contains("total 39 < 60")),
            "the aggregate drop is flagged too: {regressions:?}"
        );
    }

    #[test]
    fn baseline_which_selects_section_and_labels() {
        let file = ErrorFreeBaseline {
            schema_version: ERROR_FREE_SCHEMA_VERSION,
            fixtures: Some(efree(1, 1, 0, 0)),
            vectors: None,
        };
        assert!(
            BaselineWhich::Fixtures.section(&file).is_some(),
            "fixtures section present"
        );
        assert!(
            BaselineWhich::Vectors.section(&file).is_none(),
            "vectors section absent"
        );
        let (unit, cmd) = BaselineWhich::Vectors.labels();
        assert_eq!(unit, "vectors", "vectors unit label");
        assert!(
            cmd.contains("conformance vectors"),
            "update command targets the vectors runner: {cmd}"
        );
    }

    #[test]
    fn wire_data_extracts_data_array() {
        let json = r#"{ "schemaVersion": 2, "data": [1, 2, 3] }"#;
        let items = wire_data(json).expect("valid envelope");
        assert_eq!(items.len(), 3, "three data items");
        assert_eq!(items[0], Value::from(1), "first item preserved");
    }

    #[test]
    fn wire_data_rejects_missing_data() {
        let err = wire_data(r#"{ "schemaVersion": 2 }"#).expect_err("no data array");
        assert!(err.contains("data"), "error mentions missing data: {err}");
    }

    #[test]
    fn wire_data_rejects_invalid_json() {
        let err = wire_data("not json").expect_err("invalid json");
        assert!(err.contains("parse"), "error mentions parse failure: {err}");
    }

    fn err_of<T>(result: Result<T, String>) -> String {
        match result {
            Ok(_) => panic!("expected an Err"),
            Err(e) => e,
        }
    }

    #[test]
    fn span_from_value_parses_start_end() {
        let v = serde_json::json!({ "start": 4, "end": 9 });
        // `SpanCmp` has no `Debug`, so compare fields rather than the struct.
        let span = span_from_value(&v).unwrap_or_else(|_| panic!("valid span"));
        assert_eq!(span.start, 4, "start parsed");
        assert_eq!(span.end, 9, "end parsed");
    }

    #[test]
    fn span_from_value_requires_both_bounds() {
        let only_start = serde_json::json!({ "start": 4 });
        assert!(
            err_of(span_from_value(&only_start)).contains("end"),
            "error names the missing end bound"
        );
        let only_end = serde_json::json!({ "end": 9 });
        assert!(
            err_of(span_from_value(&only_end)).contains("start"),
            "error names the missing start bound"
        );
    }

    #[test]
    fn normalized_actual_diagnostics_kebabs_kind_and_drops_internal() {
        let wire = r#"{
            "schemaVersion": 2,
            "data": [
                { "kind": "source_contains_pua", "severity": "warning", "source": "library", "span": { "start": 0, "end": 1 } },
                { "kind": "self_check", "severity": "error", "source": "internal", "span": { "start": 2, "end": 3 } }
            ]
        }"#;
        let out = normalized_actual_diagnostics(wire).unwrap_or_else(|_| panic!("valid wire"));
        assert_eq!(out.len(), 1, "internal-source diagnostic stripped");
        assert_eq!(
            out[0].code, "source-contains-pua",
            "snake_case → kebab-case code"
        );
        assert_eq!(out[0].severity, "warning", "severity preserved");
        assert_eq!(out[0].span.start, 0, "span start parsed");
        assert_eq!(out[0].span.end, 1, "span end parsed");
    }

    #[test]
    fn normalized_actual_diagnostics_requires_kind() {
        let wire = r#"{
            "schemaVersion": 2,
            "data": [ { "severity": "warning", "source": "library", "span": { "start": 0, "end": 1 } } ]
        }"#;
        let err = err_of(normalized_actual_diagnostics(wire));
        assert!(err.contains("kind"), "error names missing kind: {err}");
    }

    #[test]
    fn diagnostics_match_requires_equal_length() {
        let expected = vec![ExpectedDiagnostic {
            code: "a".to_owned(),
            severity: "error".to_owned(),
            span: None,
        }];
        let actual: Vec<ActualDiagnostic> = Vec::new();
        assert!(
            !diagnostics_match(&expected, &actual),
            "length mismatch → no match"
        );
    }

    #[test]
    fn diagnostics_match_ignores_span_when_unpinned() {
        let expected = vec![ExpectedDiagnostic {
            code: "a".to_owned(),
            severity: "error".to_owned(),
            span: None,
        }];
        let actual = vec![ActualDiagnostic {
            code: "a".to_owned(),
            severity: "error".to_owned(),
            span: SpanCmp { start: 7, end: 9 },
        }];
        assert!(
            diagnostics_match(&expected, &actual),
            "unpinned span: code+severity match is enough"
        );
    }

    #[test]
    fn diagnostics_match_enforces_pinned_span() {
        let expected = vec![ExpectedDiagnostic {
            code: "a".to_owned(),
            severity: "error".to_owned(),
            span: Some(SpanCmp { start: 1, end: 2 }),
        }];
        let matching = vec![ActualDiagnostic {
            code: "a".to_owned(),
            severity: "error".to_owned(),
            span: SpanCmp { start: 1, end: 2 },
        }];
        assert!(
            diagnostics_match(&expected, &matching),
            "pinned span matches exactly"
        );
        let diverging = vec![ActualDiagnostic {
            code: "a".to_owned(),
            severity: "error".to_owned(),
            span: SpanCmp { start: 1, end: 3 },
        }];
        assert!(
            !diagnostics_match(&expected, &diverging),
            "pinned span mismatch → no match"
        );
    }

    #[test]
    fn diagnostics_match_compares_code_and_severity() {
        let expected = vec![ExpectedDiagnostic {
            code: "a".to_owned(),
            severity: "error".to_owned(),
            span: None,
        }];
        let wrong_severity = vec![ActualDiagnostic {
            code: "a".to_owned(),
            severity: "warning".to_owned(),
            span: SpanCmp { start: 0, end: 0 },
        }];
        assert!(
            !diagnostics_match(&expected, &wrong_severity),
            "severity must match"
        );
    }

    fn vector(source: &str, expected: VectorExpected) -> Vector {
        Vector {
            name: "v".to_owned(),
            meta: VectorMeta {
                feature: "f".to_owned(),
                level: "must".to_owned(),
            },
            source: source.to_owned(),
            expected,
        }
    }

    fn empty_expected() -> VectorExpected {
        VectorExpected {
            html: None,
            serialize: None,
            nodes: None,
            pairs: None,
            diagnostics: None,
        }
    }

    #[test]
    fn compare_vector_no_expectations_is_no_mismatch() {
        let v = vector("plain text", empty_expected());
        let (normative, html) = compare_vector(&v).expect("compare runs");
        assert!(normative.is_empty(), "nothing pinned → no normative diff");
        assert!(html.is_none(), "no html pinned → no html diff");
    }

    #[test]
    fn compare_vector_matching_serialize_passes() {
        // Parse once to learn the parser's own serialize output, then pin it.
        let doc = aozora::Document::new("plain text".to_owned());
        let expected_serialize = doc.parse().to_source();
        let mut exp = empty_expected();
        exp.serialize = Some(expected_serialize);
        let v = vector("plain text", exp);
        let (normative, _) = compare_vector(&v).expect("compare runs");
        assert!(
            normative.is_empty(),
            "pinned serialize equals parser output: {normative:?}"
        );
    }

    #[test]
    fn compare_vector_serialize_drift_is_reported() {
        let mut exp = empty_expected();
        exp.serialize = Some("definitely-not-the-real-serialization".to_owned());
        let v = vector("plain text", exp);
        let (normative, _) = compare_vector(&v).expect("compare runs");
        assert!(
            normative.contains(&"serialize".to_owned()),
            "serialize drift flagged: {normative:?}"
        );
    }

    #[test]
    fn compare_vector_html_drift_is_informative_only() {
        let mut exp = empty_expected();
        exp.html = Some("<not-the-real-html/>".to_owned());
        let v = vector("plain text", exp);
        let (normative, html) = compare_vector(&v).expect("compare runs");
        assert!(normative.is_empty(), "html is not normative: {normative:?}");
        assert_eq!(
            html,
            Some("html".to_owned()),
            "html mismatch surfaces as an informative diff"
        );
    }

    #[test]
    fn tree_sitter_parse_recognises_explicit_ruby() {
        let (sexp, has_error) =
            tree_sitter_parse("｜青空《あおぞら》").expect("grammar compiled in");
        assert!(!has_error, "well-formed ruby parses without error: {sexp}");
        assert!(
            sexp.contains("explicit_ruby"),
            "grammar classifies the ruby span: {sexp}"
        );
    }

    #[test]
    fn tree_sitter_parse_plain_text_is_clean() {
        let (sexp, has_error) = tree_sitter_parse("こんにちは世界").expect("grammar compiled in");
        assert!(!has_error, "plain text never errors: {sexp}");
    }

    #[test]
    fn tree_sitter_parse_flags_unmatched_opener() {
        // `※` opens a gaiji marker that must be followed by a `［＃…］`
        // slug; bare text after it leaves the grammar in an error state.
        let (sexp, has_error) = tree_sitter_parse("※ただの文字").expect("grammar compiled in");
        assert!(
            has_error,
            "an unmatched gaiji opener is an honest non-pass: {sexp}"
        );
    }

    #[test]
    fn tree_sitter_parse_is_deterministic() {
        let first = tree_sitter_parse("青空《あおぞら》").expect("grammar compiled in");
        let second = tree_sitter_parse("青空《あおぞら》").expect("grammar compiled in");
        assert_eq!(first.0, second.0, "to_sexp is stable for a fixed input");
    }

    #[test]
    fn build_summary_labels_tree_sitter_impl() {
        let summary = build_summary(vec![case(Level::Must, true)], "tree-sitter");
        assert_eq!(
            summary.implementation, "tree-sitter",
            "implementation label threads through"
        );
        assert_eq!(summary.passed, 1, "the clean case counts as a pass");
    }

    fn ts_entry(name: &str, error: bool, sexp: &str) -> TsVectorEntry {
        TsVectorEntry {
            name: name.to_owned(),
            error,
            sexp: sexp.to_owned(),
        }
    }

    fn ts_snapshot(vectors: Vec<TsVectorEntry>) -> TsVectorSnapshot {
        TsVectorSnapshot {
            schema_version: 1,
            implementation: "tree-sitter".to_owned(),
            vectors,
        }
    }

    #[test]
    fn snapshot_drifts_empty_when_identical() {
        let snap = ts_snapshot(vec![ts_entry("a", false, "(document)")]);
        let same = ts_snapshot(vec![ts_entry("a", false, "(document)")]);
        assert!(
            snapshot_drifts(&snap, &same).is_empty(),
            "identical snapshots do not drift"
        );
    }

    #[test]
    fn snapshot_drifts_flags_change_add_and_remove() {
        let prev = ts_snapshot(vec![
            ts_entry("a", false, "(document)"),
            ts_entry("b", false, "(document (text))"),
            ts_entry("gone", false, "(document)"),
        ]);
        let current = ts_snapshot(vec![
            ts_entry("a", false, "(document)"),
            ts_entry("b", true, "(document (ERROR))"),
            ts_entry("fresh", false, "(document)"),
        ]);
        let drifts = snapshot_drifts(&prev, &current);
        assert!(
            drifts.contains(&"b".to_owned()),
            "changed entry: {drifts:?}"
        );
        assert!(
            drifts.contains(&"fresh (new)".to_owned()),
            "added vector: {drifts:?}"
        );
        assert!(
            drifts.contains(&"gone (removed)".to_owned()),
            "removed vector: {drifts:?}"
        );
        assert!(
            !drifts.iter().any(|d| d.starts_with('a')),
            "unchanged entry is not flagged: {drifts:?}"
        );
    }
}
