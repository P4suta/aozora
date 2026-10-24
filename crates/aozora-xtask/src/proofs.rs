use std::collections::BTreeSet;
use std::env;
use std::fs;
use std::iter;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use serde::Deserialize;
use sha2::{Digest as _, Sha256};

#[derive(Clone, Copy)]
pub(crate) struct Subject {
    source: &'static str,
    production: &'static str,
    rejecting: &'static [&'static str],
    output: &'static str,
}

pub(crate) const DEVELOPER: Subject = Subject {
    source: "crates/aozora-xtask/src/developer/rules.rs",
    production: "proofs::timestamps_require_a_nonfuture_check_within_the_exact_freshness_window",
    rejecting: &[
        "rejecting_controls::reject_a_future_timestamp_as_fresh",
        "rejecting_controls::reject_an_extra_second_beyond_the_freshness_window",
    ],
    output: "target/developer-proofs",
};

pub(crate) const FUZZ: Subject = Subject {
    source: "crates/aozora-xtask/src/fuzz/rules.rs",
    production: "proofs::only_a_completed_run_or_a_reproduced_input_has_a_verdict",
    rejecting: &[
        "rejecting_controls::reject_a_startup_error_as_an_input_failure",
        "rejecting_controls::reject_a_failure_without_a_retained_input",
        "rejecting_controls::reject_an_input_that_replays_successfully",
        "rejecting_controls::reject_a_replay_startup_error_as_an_input_failure",
        "rejecting_controls::reject_an_interrupted_run_as_completed",
        "rejecting_controls::reject_an_oom_artifact_as_a_crash",
    ],
    output: "target/fuzz-proofs",
};
const CHECKED_MIR: &str = "--edition=2024 -C overflow-checks=yes -C debug-assertions=yes -Z mir-opt-level=3 -Z mir-enable-passes=-Inline,-JumpThreading";
const REQUIRED_COVERS: usize = 3;

#[derive(Clone, Copy)]
enum Kind {
    Production(Subject),
    Rejecting(Subject),
}

impl Kind {
    fn names(self) -> Vec<&'static str> {
        match self {
            Self::Production(subject) => vec![subject.production],
            Self::Rejecting(subject) => subject.rejecting.to_vec(),
        }
    }

    fn rejecting(self) -> bool {
        matches!(self, Self::Rejecting(_))
    }
}

#[derive(Deserialize)]
struct Inventory {
    proof_harnesses: Vec<Harness>,
}

#[derive(Deserialize)]
struct Harness {
    pretty_name: String,
}

#[derive(Deserialize)]
struct Report {
    verification_results: Results,
}

#[derive(Deserialize)]
struct Results {
    summary: Summary,
    results: Vec<HarnessResult>,
}

#[derive(Deserialize)]
struct Summary {
    status: String,
    total_harnesses: usize,
    executed: usize,
    successful: usize,
    failed: usize,
}

#[derive(Deserialize)]
struct HarnessResult {
    harness_id: String,
    status: String,
    checks: Vec<Check>,
}

#[derive(Deserialize)]
struct Check {
    status: String,
    category: String,
}

fn validate(report: &Report, expected: &[&str], rejecting: bool) -> Result<(), String> {
    if expected.is_empty()
        || expected.iter().copied().collect::<BTreeSet<_>>().len() != expected.len()
    {
        return Err("the required proof inventory must be nonempty and unique".into());
    }
    let results = &report.verification_results;
    let summary = &results.summary;
    let names: BTreeSet<_> = results
        .results
        .iter()
        .map(|result| result.harness_id.as_str())
        .collect();
    if summary.status != "completed"
        || summary.total_harnesses != expected.len()
        || summary.executed != expected.len()
        || results.results.len() != expected.len()
        || names != expected.iter().copied().collect()
        || summary.successful != if rejecting { 0 } else { expected.len() }
        || summary.failed != if rejecting { expected.len() } else { 0 }
    {
        return Err("missing, duplicate, unexpected, or incomplete Kani harness results".into());
    }
    for result in &results.results {
        let failure = result
            .checks
            .iter()
            .filter(|check| check.status == "Failure")
            .collect::<Vec<_>>();
        let assertions = result
            .checks
            .iter()
            .filter(|check| check.category == "assertion")
            .count();
        let covers = result
            .checks
            .iter()
            .filter(|check| check.category == "cover" && check.status == "Satisfied")
            .count();
        let known = result.checks.iter().all(|check| {
            check.status == "Success"
                || check.category == "cover" && check.status == "Satisfied"
                || rejecting && check.category == "assertion" && check.status == "Failure"
        });
        let accepted = if rejecting {
            result.status == "Failure"
                && !failure.is_empty()
                && failure.iter().all(|check| check.category == "assertion")
        } else {
            result.status == "Success" && failure.is_empty() && covers == REQUIRED_COVERS
        };
        if !known || assertions == 0 || !accepted {
            return Err(format!("unverified properties in {}", result.harness_id));
        }
    }
    Ok(())
}

fn command(kani: &Path) -> Result<Command, String> {
    let directory = kani.parent().ok_or("verifier directory is unavailable")?;
    let inherited = env::var_os("PATH").unwrap_or_default();
    let search =
        env::join_paths(iter::once(directory.to_owned()).chain(env::split_paths(&inherited)))
            .map_err(|error| format!("construct pinned verifier search path: {error}"))?;
    let mut command = Command::new(kani);
    command.env("PATH", search);
    Ok(command)
}

fn invoke(kani: &Path, source: &Path, output: &Path, arguments: &[&str]) -> Result<Output, String> {
    command(kani)?
        .env("RUSTFLAGS", CHECKED_MIR)
        .arg(source)
        .arg("--target-dir")
        .arg(output)
        .args(["-Z", "unstable-options", "--keep-temps"])
        .args(arguments)
        .output()
        .map_err(|error| format!("start pinned Kani: {error}"))
}

fn verifier(root: &Path) -> Result<(PathBuf, String), String> {
    let config = fs::read_to_string(root.join("mise.toml"))
        .map_err(|error| format!("read verifier pin: {error}"))?;
    let config: toml::Value =
        toml::from_str(&config).map_err(|error| format!("parse verifier pin: {error}"))?;
    let version = config
        .get("tools")
        .and_then(|tools| tools.get("cargo:kani-verifier"))
        .and_then(|tool| tool.get("version"))
        .and_then(toml::Value::as_str)
        .ok_or("missing exact Kani verifier pin")?
        .to_owned();
    if version.split('.').count() != 3
        || !version
            .split('.')
            .all(|part| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit()))
    {
        return Err("Kani requires an exact numeric release pin".into());
    }
    let home =
        env::var_os("HOME").ok_or("HOME is required to locate the pinned Kani installation")?;
    let kani = PathBuf::from(home).join(format!(".kani/kani-{version}/bin/kani-driver"));
    let output = command(&kani)?
        .arg("--version")
        .output()
        .map_err(|error| format!("read Kani version; run pinned `cargo kani setup`: {error}"))?;
    if !output.status.success()
        || !String::from_utf8_lossy(&output.stdout)
            .split_whitespace()
            .any(|word| word == version)
    {
        return Err(format!(
            "Kani version check for {} expected {version}, exited with {}: stdout={:?}; stderr={:?}",
            kani.display(),
            output.status,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    Ok((kani, version))
}

fn batch(kani: &Path, source: &Path, directory: &Path, kind: Kind) -> Result<(), String> {
    let names = kind.names();
    let rejecting = kind.rejecting();
    let report = directory.join(if rejecting {
        "rejecting.json"
    } else {
        "production.json"
    });
    let report_argument = report.to_str().ok_or("Kani report path is not UTF-8")?;
    let mut arguments = vec![
        "--output-format",
        "terse",
        "--exact",
        "--jobs",
        "1",
        "--harness-timeout",
        "60s",
        "--export-json",
        report_argument,
    ];
    for name in &names {
        arguments.extend(["--harness", name]);
    }
    arguments.extend(["--cbmc-args", "--object-bits", "12"]);
    let output = invoke(kani, source, directory, &arguments)?;
    let bytes = fs::read(&report)
        .map_err(|error| format!("incomplete Kani report {}: {error}", report.display()))?;
    let decoded: Report =
        serde_json::from_slice(&bytes).map_err(|error| format!("decode Kani report: {error}"))?;
    validate(&decoded, &names, rejecting)?;
    if output.status.success() == rejecting {
        return Err(format!("unexpected Kani exit status: {}", output.status));
    }
    Ok(())
}

#[expect(
    clippy::print_stdout,
    reason = "the proof command reports its exact source identity and retained evidence"
)]
pub(crate) fn verify(root: &Path, subject: Subject) -> Result<(), String> {
    if !cfg!(target_os = "linux") {
        return Err("proofs require the pinned Linux verifier; use the bounded Linux project runner on owner machines".into());
    }
    let root = root
        .canonicalize()
        .map_err(|error| format!("proof root: {error}"))?;
    let source = root.join(subject.source);
    let before = fs::read(&source).map_err(|error| format!("read proof source: {error}"))?;
    let (kani, version) = verifier(&root)?;
    let output_root = root.join(subject.output);
    fs::create_dir_all(&output_root).map_err(|error| format!("create proof output: {error}"))?;
    let directory = tempfile::Builder::new()
        .prefix("run-")
        .tempdir_in(output_root)
        .map_err(|error| format!("isolate proof output: {error}"))?
        .keep();
    let compiled = invoke(&kani, &source, &directory, &["--only-codegen"])?;
    if !compiled.status.success() {
        return Err(format!(
            "compile proof inventory exited with {}: stdout={:?}; stderr={:?}",
            compiled.status,
            String::from_utf8_lossy(&compiled.stdout),
            String::from_utf8_lossy(&compiled.stderr)
        ));
    }
    let metadata = fs::read(directory.join("rules.kani-metadata.json"))
        .map_err(|error| format!("read compiled proof inventory: {error}"))?;
    let inventory: Inventory = serde_json::from_slice(&metadata)
        .map_err(|error| format!("decode proof inventory: {error}"))?;
    let expected: BTreeSet<_> = iter::once(subject.production)
        .chain(subject.rejecting.iter().copied())
        .collect();
    let declared: BTreeSet<_> = inventory
        .proof_harnesses
        .iter()
        .map(|harness| harness.pretty_name.as_str())
        .collect();
    if declared != expected || inventory.proof_harnesses.len() != expected.len() {
        return Err("the compiled proof inventory differs from the complete declared production and rejection inventory".into());
    }
    batch(&kani, &source, &directory, Kind::Production(subject))?;
    batch(&kani, &source, &directory, Kind::Rejecting(subject))?;
    if fs::read(&source).map_err(|error| format!("recheck proof source: {error}"))? != before {
        return Err("the production source changed during verification".into());
    }
    let digest = format!("{:x}", Sha256::digest(&before));
    let tool_directory = kani.parent().ok_or("verifier directory is unavailable")?;
    let mut verifier_digests = serde_json::Map::new();
    for name in [
        "kani-driver",
        "kani-compiler",
        "cbmc",
        "goto-cc",
        "goto-instrument",
    ] {
        let bytes = fs::read(tool_directory.join(name))
            .map_err(|error| format!("read {name} identity: {error}"))?;
        verifier_digests.insert(name.into(), format!("{:x}", Sha256::digest(bytes)).into());
    }
    let receipt = serde_json::json!({
        "source": subject.source, "source_sha256": digest, "kani_version": version,
        "verifier_sha256": verifier_digests, "rustflags": CHECKED_MIR,
        "production": subject.production, "rejecting": subject.rejecting,
        "harness_timeout_seconds": 60, "object_bits": 12, "completed": true
    });
    fs::write(
        directory.join("verified.json"),
        serde_json::to_vec_pretty(&receipt).map_err(|error| error.to_string())?,
    )
    .map_err(|error| format!("retain proof identity: {error}"))?;
    println!(
        "Verified rules sha256:{digest}; retained evidence at {}",
        directory.display()
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{Report, validate};

    fn report(rejecting: bool) -> Report {
        let mut checks = vec![serde_json::json!({
            "status": if rejecting { "Failure" } else { "Success" },
            "category": "assertion"
        })];
        if !rejecting {
            checks.extend(
                (0..super::REQUIRED_COVERS)
                    .map(|_| serde_json::json!({"status": "Satisfied", "category": "cover"})),
            );
        }
        serde_json::from_value(serde_json::json!({
            "verification_results": {
                "summary": {
                    "status": "completed", "total_harnesses": 1, "executed": 1,
                    "successful": usize::from(!rejecting), "failed": usize::from(rejecting)
                },
                "results": [{
                    "harness_id": "required",
                    "status": if rejecting { "Failure" } else { "Success" },
                    "checks": checks
                }]
            }
        }))
        .expect("complete report fixture")
    }

    #[test]
    fn complete_reports_require_reachable_outcomes_and_assertion_specific_rejection() {
        validate(&report(false), &["required"], false).expect("complete production evidence");
        validate(&report(true), &["required"], true).expect("complete rejecting evidence");
        let mut missing_cover = report(false);
        missing_cover.verification_results.results[0].checks.pop();
        assert!(validate(&missing_cover, &["required"], false).is_err());
        let mut pending = report(false);
        pending.verification_results.results[0].checks[0].status = "Undetermined".into();
        assert!(validate(&pending, &["required"], false).is_err());
        let mut tool_error = report(true);
        tool_error.verification_results.results[0].checks[0].category =
            "unsupported_construct".into();
        assert!(validate(&tool_error, &["required"], true).is_err());
        assert!(validate(&report(false), &[], false).is_err());
        assert!(validate(&report(false), &["required", "required"], false).is_err());
        assert!(validate(&report(false), &["different"], false).is_err());
        assert!(validate(&report(false), &["required"], true).is_err());
        assert!(validate(&report(true), &["required"], false).is_err());
    }

    #[test]
    fn incomplete_or_tool_error_reports_cannot_replace_a_proof_or_assertion_rejection() {
        let incomplete: Report = serde_json::from_value(serde_json::json!({
            "verification_results": {"summary": {"status":"completed", "total_harnesses":1, "executed":0, "successful":0, "failed":0}, "results":[]}
        })).expect("report fixture");
        assert!(validate(&incomplete, &["required"], false).is_err());
        let tool_error: Report = serde_json::from_value(serde_json::json!({
            "verification_results": {"summary": {"status":"completed", "total_harnesses":1, "executed":1, "successful":0, "failed":1}, "results":[{"harness_id":"required", "status":"Failure", "checks":[{"status":"Failure", "category":"unsupported_construct"}]}]}
        })).expect("report fixture");
        assert!(validate(&tool_error, &["required"], true).is_err());
    }
}
