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
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::ConformanceArgs;
use crate::ConformanceOp;

const FIXTURE_REL: &str = "crates/aozora-conformance/fixtures/render";
const RESULTS_REL: &str = "crates/aozora-book/src/conformance-results.json";

pub(crate) fn dispatch(args: &ConformanceArgs) -> Result<(), String> {
    match args.op {
        ConformanceOp::Run => run(),
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
    let summary = build_summary(cases);
    print_summary(&summary);
    write_results(&root, &summary)?;

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

    let mut cases = Vec::new();
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

        let (passed, message) = match run_case(&case_dir) {
            Ok(()) => (true, None),
            Err(msg) => (false, Some(msg)),
        };
        cases.push(CaseResult {
            case: case_name,
            feature: meta.feature,
            level,
            passed,
            message,
        });
    }
    Ok(cases)
}

fn build_summary(cases: Vec<CaseResult>) -> Summary {
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
        implementation: "rust".to_owned(),
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

fn write_results(root: &Path, summary: &Summary) -> Result<(), String> {
    let results_path = root.join(RESULTS_REL);
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
    let actual_serialize = tree.serialize();

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
