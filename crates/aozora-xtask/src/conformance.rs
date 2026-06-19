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
use std::mem;
use std::path::{Path, PathBuf};

use aozora::wire::{serialize_diagnostics, serialize_nodes, serialize_pairs};
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::ConformanceArgs;
use crate::ConformanceOp;

const FIXTURE_REL: &str = "crates/aozora-conformance/fixtures/render";
const RESULTS_REL: &str = "crates/aozora-book/src/conformance-results.json";
const SPEC_VECTORS_REL: &str = "crates/aozora-conformance/spec-vectors/vectors";

pub(crate) fn dispatch(args: &ConformanceArgs) -> Result<(), String> {
    match args.op {
        ConformanceOp::Run => run(),
        ConformanceOp::Vectors => run_vectors(),
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

    if vector
        .expected
        .serialize
        .as_ref()
        .is_some_and(|expected| tree.serialize() != *expected)
    {
        mismatches.push("serialize".to_owned());
    }
    if let Some(expected) = &vector.expected.nodes {
        let actual = Value::Array(wire_data(&serialize_nodes(&tree))?);
        mismatches.extend((actual != *expected).then(|| "nodes".to_owned()));
    }
    if let Some(expected) = &vector.expected.pairs {
        let actual = Value::Array(wire_data(&serialize_pairs(&tree))?);
        mismatches.extend((actual != *expected).then(|| "pairs".to_owned()));
    }
    if let Some(expected) = &vector.expected.diagnostics {
        let actual = normalized_actual_diagnostics(&serialize_diagnostics(tree.diagnostics()))?;
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
        let summary = build_summary(cases);
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
        let summary = build_summary(cases);
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
        let summary = build_summary(Vec::new());
        assert_eq!(summary.total, 0, "no cases");
        assert_eq!(summary.passed, 0, "no passes");
        assert_eq!(summary.failed, 0, "no fails");
        assert!(summary.by_level.is_empty(), "no level buckets");
    }

    #[test]
    fn wire_data_extracts_data_array() {
        let json = r#"{ "schema_version": 1, "data": [1, 2, 3] }"#;
        let items = wire_data(json).expect("valid envelope");
        assert_eq!(items.len(), 3, "three data items");
        assert_eq!(items[0], Value::from(1), "first item preserved");
    }

    #[test]
    fn wire_data_rejects_missing_data() {
        let err = wire_data(r#"{ "schema_version": 1 }"#).expect_err("no data array");
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
            "schema_version": 1,
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
            "schema_version": 1,
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
        let expected_serialize = doc.parse().serialize();
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
}
