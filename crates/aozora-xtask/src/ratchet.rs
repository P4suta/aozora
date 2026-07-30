use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::Value;

use crate::{RatchetArgs, perf::PERF_BASELINE};

const TSV_BASELINES: &[&str] = &[
    "crates/aozora-bench/perf-baseline.tsv",
    PERF_BASELINE,
    "crates/aozora-bench/artifact-size-baseline.tsv",
];
const ALLOC_BASELINE: &str = "corpus/alloc-baseline.json";
const ALLOC_METRICS: &[&str] = &[
    "alloc_blocks_per_file",
    "alloc_blocks_total",
    "alloc_bytes_per_source_byte",
    "alloc_bytes_total",
    "tolerance",
];

pub(crate) fn check(args: &RatchetArgs) -> Result<(), String> {
    let root = workspace_root()?;
    let base = resolve_base(&root, &args.base)?;
    let mut errors = Vec::new();

    for path in TSV_BASELINES {
        if let Some(previous) = read_at_commit(&root, &base, path)? {
            let current = read_file(&root.join(path))?;
            compare_tsv(path, &previous, &current, &mut errors)?;
        }
    }
    if let Some(previous) = read_at_commit(&root, &base, ALLOC_BASELINE)? {
        let current = read_file(&root.join(ALLOC_BASELINE))?;
        compare_alloc(ALLOC_BASELINE, &previous, &current, &mut errors)?;
    }
    for path in schema_paths(&root)? {
        let relative = relative_path(&root, &path)?;
        if let Some(previous) = read_at_commit(&root, &base, &relative)? {
            let current = read_file(&path)?;
            compare_schema(&relative, &previous, &current, &mut errors)?;
        }
    }

    if errors.is_empty() {
        println!("xtask ratchet: no performance, allocation, or artifact ceiling increased");
        Ok(())
    } else {
        for error in &errors {
            eprintln!("{error}");
        }
        Err(format!(
            "baseline ratchet failed with {} error(s)",
            errors.len()
        ))
    }
}

fn resolve_base(root: &Path, requested: &str) -> Result<String, String> {
    let candidate = if requested.is_empty() || requested.chars().all(|character| character == '0') {
        "HEAD^"
    } else {
        requested
    };
    if commit_exists(root, candidate)? {
        return Ok(candidate.to_owned());
    }
    if candidate != "HEAD^" && commit_exists(root, "HEAD^")? {
        Ok("HEAD^".to_owned())
    } else {
        Err(format!("baseline commit does not exist: {candidate}"))
    }
}

fn commit_exists(root: &Path, revision: &str) -> Result<bool, String> {
    Command::new("git")
        .current_dir(root)
        .args(["cat-file", "-e", &format!("{revision}^{{commit}}")])
        .output()
        .map(|output| output.status.success())
        .map_err(|err| format!("run git cat-file: {err}"))
}

fn read_at_commit(root: &Path, base: &str, path: &str) -> Result<Option<String>, String> {
    let spec = format!("{base}:{path}");
    let exists = Command::new("git")
        .current_dir(root)
        .args(["cat-file", "-e", &spec])
        .output()
        .map_err(|err| format!("run git cat-file for {path}: {err}"))?;
    if !exists.status.success() {
        return Ok(None);
    }
    let output = Command::new("git")
        .current_dir(root)
        .args(["show", &spec])
        .output()
        .map_err(|err| format!("run git show for {path}: {err}"))?;
    if !output.status.success() {
        return Err(format!("git show {spec} failed: {}", output.status));
    }
    String::from_utf8(output.stdout)
        .map(Some)
        .map_err(|err| format!("{path} at {base} is not UTF-8: {err}"))
}

fn compare_tsv(
    path: &str,
    previous: &str,
    current: &str,
    errors: &mut Vec<String>,
) -> Result<(), String> {
    let previous = parse_tsv(path, previous)?;
    let current = parse_tsv(path, current)?;
    for (name, previous_value) in previous {
        match current.get(&name) {
            Some(current_value) if current_value > &previous_value => {
                errors.push(format!(
                    "::error title=baseline-ratchet::{path} {name} increased from {previous_value} to {current_value}"
                ));
            }
            None => errors.push(format!(
                "::error title=baseline-ratchet::{path} removed baseline {name}"
            )),
            Some(_) => {}
        }
    }
    Ok(())
}

fn parse_tsv(path: &str, text: &str) -> Result<BTreeMap<String, u64>, String> {
    text.lines()
        .enumerate()
        .map(|(index, line)| {
            let (name, value) = line
                .split_once('\t')
                .ok_or_else(|| format!("{path}:{}: expected tab-separated fields", index + 1))?;
            let value = value
                .parse()
                .map_err(|err| format!("{path}:{}: invalid value: {err}", index + 1))?;
            Ok((name.to_owned(), value))
        })
        .collect()
}

fn compare_alloc(
    path: &str,
    previous: &str,
    current: &str,
    errors: &mut Vec<String>,
) -> Result<(), String> {
    let previous = parse_json(path, previous)?;
    let current = parse_json(path, current)?;
    for metric in ALLOC_METRICS {
        compare_json_metric(path, metric, (&previous, &current), errors)?;
    }
    if let Some(contract) = previous.get("contract").and_then(Value::as_object) {
        for operation in contract.keys() {
            for metric in ["blocks", "bytes"] {
                compare_json_metric(
                    path,
                    &format!("contract.{operation}.{metric}"),
                    (&previous, &current),
                    errors,
                )?;
            }
        }
    }
    Ok(())
}

fn compare_json_metric(
    path: &str,
    metric: &str,
    values: (&Value, &Value),
    errors: &mut Vec<String>,
) -> Result<(), String> {
    let previous = number_at(values.0, metric)
        .ok_or_else(|| format!("{path}: previous metric is missing or non-numeric: {metric}"))?;
    let current = number_at(values.1, metric)
        .ok_or_else(|| format!("{path}: current metric is missing or non-numeric: {metric}"))?;
    if current > previous {
        errors.push(format!(
            "::error title=baseline-ratchet::{path} {metric} increased from {previous} to {current}"
        ));
    }
    Ok(())
}

fn number_at(value: &Value, path: &str) -> Option<f64> {
    path.split('.')
        .try_fold(value, |cursor, component| cursor.get(component))
        .and_then(Value::as_f64)
}

fn compare_schema(
    path: &str,
    previous: &str,
    current: &str,
    errors: &mut Vec<String>,
) -> Result<(), String> {
    let mut previous = parse_json(path, previous)?;
    let mut current = parse_json(path, current)?;
    let previous_version = schema_version(&previous, path)?;
    let current_version = schema_version(&current, path)?;
    if current_version < previous_version {
        errors.push(format!(
            "::error title=wire-schema::{path} schema version decreased from {previous_version} to {current_version}"
        ));
    }
    remove_schema_version(&mut previous, path)?;
    remove_schema_version(&mut current, path)?;
    if previous != current && current_version <= previous_version {
        errors.push(format!(
            "::error title=wire-schema::{path} changed without increasing schema version {current_version}"
        ));
    }
    Ok(())
}

fn schema_version(value: &Value, path: &str) -> Result<u64, String> {
    value
        .pointer("/properties/schemaVersion/const")
        .and_then(Value::as_u64)
        .ok_or_else(|| format!("{path}: schemaVersion.const is missing or non-numeric"))
}

fn remove_schema_version(value: &mut Value, path: &str) -> Result<(), String> {
    value
        .pointer_mut("/properties/schemaVersion")
        .and_then(Value::as_object_mut)
        .and_then(|version| version.remove("const"))
        .map(|_| ())
        .ok_or_else(|| format!("{path}: schemaVersion.const is missing"))
}

fn parse_json(path: &str, text: &str) -> Result<Value, String> {
    serde_json::from_str(text).map_err(|err| format!("parse {path}: {err}"))
}

fn schema_paths(root: &Path) -> Result<Vec<PathBuf>, String> {
    let directory = root.join("crates/aozora-conformance/json");
    let mut paths = fs::read_dir(&directory)
        .map_err(|err| format!("read {}: {err}", directory.display()))?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("schema-"))
                && path
                    .extension()
                    .is_some_and(|extension| extension.eq_ignore_ascii_case("json"))
        })
        .collect::<Vec<_>>();
    paths.sort();
    if paths.is_empty() {
        Err(format!("no wire schemas found in {}", directory.display()))
    } else {
        Ok(paths)
    }
}

fn relative_path(root: &Path, path: &Path) -> Result<String, String> {
    path.strip_prefix(root)
        .map_err(|err| format!("{} is outside {}: {err}", path.display(), root.display()))?
        .to_str()
        .map(str::to_owned)
        .ok_or_else(|| format!("path is not UTF-8: {}", path.display()))
}

fn read_file(path: &Path) -> Result<String, String> {
    fs::read_to_string(path).map_err(|err| format!("read {}: {err}", path.display()))
}

fn workspace_root() -> Result<PathBuf, String> {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .map(Path::to_path_buf)
        .ok_or_else(|| "could not derive workspace root".to_owned())
}

#[cfg(test)]
mod tests {
    use super::{compare_schema, compare_tsv};

    #[test]
    fn tsv_ratchet_rejects_increases_and_removals() {
        let mut errors = Vec::new();
        compare_tsv(
            "baseline.tsv",
            "parse\t10\nrender\t20\n",
            "parse\t11\n",
            &mut errors,
        )
        .unwrap();
        assert_eq!(errors.len(), 2);
    }

    #[test]
    fn schema_changes_require_a_version_increase() {
        let previous = r#"{"properties":{"schemaVersion":{"const":1},"value":{"type":"string"}}}"#;
        let current = r#"{"properties":{"schemaVersion":{"const":1},"value":{"type":"number"}}}"#;
        let mut errors = Vec::new();
        compare_schema("schema.json", previous, current, &mut errors).unwrap();
        assert_eq!(errors.len(), 1);

        let bumped = r#"{"properties":{"schemaVersion":{"const":2},"value":{"type":"number"}}}"#;
        errors.clear();
        compare_schema("schema.json", previous, bumped, &mut errors).unwrap();
        assert!(errors.is_empty());
    }
}
