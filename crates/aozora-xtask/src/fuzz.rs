use std::env;
use std::ffi::OsStr;
use std::fs::{self, OpenOptions};
use std::io::Write as _;
use std::num::NonZeroU32;
use std::path::{Path, PathBuf};
use std::process::Command;

use clap::{Args, Subcommand, ValueEnum};
use sha2::{Digest as _, Sha256};

use crate::proofs;

mod rules;

use rules::{Artifacts, FAILURE_EXIT, Outcome, artifacts, classify};

const HOST_TARGET: &str = "x86_64-unknown-linux-gnu";
const DEFAULT_SECONDS: &str = "600";
const INPUT_TIMEOUT_SECONDS: u32 = 60;
const RESULT_FILE: &str = "result.json";

#[derive(Args)]
pub(crate) struct FuzzArgs {
    #[command(subcommand)]
    command: FuzzCommand,
}

#[derive(Subcommand)]
enum FuzzCommand {
    /// Run the pinned engine and report only a retained input that reproduces.
    Soak {
        family: Family,
        target: String,
        #[arg(long, default_value = ".")]
        root: PathBuf,
        #[arg(long, default_value = DEFAULT_SECONDS)]
        seconds: NonZeroU32,
    },
    /// Replay retained inputs and refuse infrastructure failures.
    Triage {
        family: Family,
        target: String,
        #[arg(long, default_value = ".")]
        root: PathBuf,
    },
    /// Verify the complete source-bound fuzz report contract.
    Prove {
        #[arg(long, default_value = ".")]
        root: PathBuf,
    },
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum Family {
    Pipeline,
    Render,
    Encoding,
}

impl Family {
    const fn directory(self) -> &'static str {
        match self {
            Self::Pipeline => "pipeline",
            Self::Render => "render",
            Self::Encoding => "encoding",
        }
    }
}

struct Project {
    root: PathBuf,
    directory: PathBuf,
    build: PathBuf,
    target: String,
    nightly: String,
}

impl Project {
    fn read(root: &Path, family: Family, target: &str) -> Result<Self, String> {
        if target.is_empty()
            || !target
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
        {
            return Err("fuzz target must be a registered identifier".into());
        }
        let root = root.canonicalize().map_err(|error| error.to_string())?;
        let directory = root.join("crates/aozora/fuzz").join(family.directory());
        let manifest = read_toml(&directory.join("Cargo.toml"))?;
        if !manifest
            .get("bin")
            .and_then(toml::Value::as_array)
            .is_some_and(|bins| {
                bins.iter()
                    .any(|bin| bin.get("name").and_then(toml::Value::as_str) == Some(target))
            })
        {
            return Err(format!(
                "unregistered fuzz target {}/{target}",
                family.directory()
            ));
        }
        let tools = read_toml(&root.join("mise.toml"))?;
        let nightlies = tools
            .get("tools")
            .and_then(|tools| tools.get("rust"))
            .and_then(toml::Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|tool| tool.get("version").and_then(toml::Value::as_str))
            .filter(|version| version.starts_with("nightly-"))
            .collect::<Vec<_>>();
        let [nightly] = nightlies.as_slice() else {
            return Err("fuzz requires one dated nightly in mise.toml".into());
        };
        Ok(Self {
            build: root.join("target/fuzz-build").join(family.directory()),
            root,
            directory,
            target: target.into(),
            nightly: (*nightly).into(),
        })
    }

    fn executable(&self) -> PathBuf {
        self.build
            .join(HOST_TARGET)
            .join("release")
            .join(&self.target)
    }

    fn build(
        &self,
        execute: &mut impl FnMut(&mut Command) -> Result<Option<i32>, String>,
    ) -> Result<(), String> {
        let mut command = Command::new("cargo");
        command
            .current_dir(&self.directory)
            .arg(format!("+{}", self.nightly))
            .args([
                "fuzz",
                "build",
                "--fuzz-dir",
                ".",
                "--target",
                HOST_TARGET,
                "--target-dir",
            ])
            .arg(&self.build)
            .arg(&self.target);
        let exit = execute(&mut command)?;
        if exit != Some(0) {
            return Err(format!(
                "fuzz build did not complete successfully: {exit:?}"
            ));
        }
        Ok(())
    }

    fn engine(&self, artifacts: &Path) -> Command {
        let mut command = Command::new(self.executable());
        command
            .current_dir(&self.directory)
            .arg(format!("-artifact_prefix={}/", artifacts.display()))
            .arg(format!("-error_exitcode={FAILURE_EXIT}"))
            .arg("-timeout_exitcode=78")
            .arg(format!("-timeout={INPUT_TIMEOUT_SECONDS}"));
        command
    }
}

fn read_toml(path: &Path) -> Result<toml::Value, String> {
    let text =
        fs::read_to_string(path).map_err(|error| format!("read {}: {error}", path.display()))?;
    toml::from_str(&text).map_err(|error| format!("parse {}: {error}", path.display()))
}

fn execute(command: &mut Command) -> Result<Option<i32>, String> {
    command
        .status()
        .map(|status| status.code())
        .map_err(|error| format!("start {command:?}: {error}"))
}

fn digest(path: &Path) -> Result<String, String> {
    fs::read(path)
        .map(|bytes| format!("{:x}", Sha256::digest(bytes)))
        .map_err(|error| format!("hash {}: {error}", path.display()))
}

#[expect(
    clippy::filetype_is_file,
    reason = "only regular fuzz inputs are admissible; directories, symlinks, and devices must fail"
)]
fn inputs(directory: &Path) -> Result<Vec<PathBuf>, String> {
    let mut result = Vec::new();
    for entry in fs::read_dir(directory).map_err(|error| format!("read inputs: {error}"))? {
        let entry = entry.map_err(|error| error.to_string())?;
        if !entry
            .file_type()
            .map_err(|error| error.to_string())?
            .is_file()
        {
            return Err("fuzz inputs must be regular files, without symbolic links".into());
        }
        result.push(entry.path());
    }
    result.sort();
    Ok(result)
}

fn archive_inputs(directory: &Path) -> Result<Vec<PathBuf>, String> {
    let mut retained = inputs(directory)?;
    retained.retain(|path| path.file_name() != Some(OsStr::new(RESULT_FILE)));
    Ok(retained)
}

#[derive(Debug)]
struct SoakReport {
    outcome: Outcome,
    directory: PathBuf,
}

fn soak(
    project: &Project,
    seconds: NonZeroU32,
    execute: &mut impl FnMut(&mut Command) -> Result<Option<i32>, String>,
) -> Result<SoakReport, String> {
    project.build(execute)?;
    let executable_hash = digest(&project.executable())?;
    let reports = project.root.join("target/fuzz-reports");
    fs::create_dir_all(&reports).map_err(|error| error.to_string())?;
    let directory = tempfile::Builder::new()
        .prefix("run-")
        .tempdir_in(reports)
        .map_err(|error| error.to_string())?;
    let mut command = project.engine(directory.path());
    let corpus = project.directory.join("corpus").join(&project.target);
    fs::create_dir_all(&corpus).map_err(|error| format!("create corpus: {error}"))?;
    command
        .arg(format!("-max_total_time={seconds}"))
        .arg(corpus);
    let exit = execute(&mut command)?;
    let retained = inputs(directory.path())?;
    let name = if let [input] = retained.as_slice() {
        input
            .file_name()
            .map(OsStr::as_encoded_bytes)
            .unwrap_or_default()
    } else {
        &[]
    };
    let kind = artifacts(retained.len(), name);
    let mut input_hash = None;
    let replay = if let (Some(FAILURE_EXIT), Artifacts::FailureInput, [input]) =
        (exit, kind, retained.as_slice())
    {
        let before = digest(input)?;
        let replay = execute(project.engine(directory.path()).arg("-runs=1").arg(input))?;
        if digest(input)? != before {
            return Err("fuzz input changed during replay".into());
        }
        if inputs(directory.path())? != retained {
            return Err("fuzz artifact inventory changed during replay".into());
        }
        input_hash = Some(before);
        replay
    } else {
        None
    };
    if digest(&project.executable())? != executable_hash {
        return Err("fuzz executable changed during verification".into());
    }
    let outcome = classify(exit, kind, replay);
    let directory = if outcome == Outcome::Clean {
        directory.path().to_owned()
    } else {
        let directory = directory.keep();
        let report = serde_json::json!({
            "schema": 1, "target": project.target, "binary_sha256": executable_hash,
            "exit": exit, "replay_exit": replay, "input_sha256": input_hash,
            "reproduced_input_failure": outcome == Outcome::InputFailure,
        });
        let report_file = directory.join(RESULT_FILE);
        OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&report_file)
            .map_err(|error| {
                format!(
                    "retain fuzz report without overwriting {}: {error}",
                    report_file.display()
                )
            })?
            .write_all(&serde_json::to_vec_pretty(&report).map_err(|error| error.to_string())?)
            .map_err(|error| format!("write fuzz report: {error}"))?;
        directory
    };
    Ok(SoakReport { outcome, directory })
}

fn github_output(outcome: Outcome, directory: &Path) -> Result<(), String> {
    let Some(output) = env::var_os("GITHUB_OUTPUT") else {
        return Ok(());
    };
    let directory = directory
        .to_str()
        .ok_or("artifact directory is not UTF-8")?;
    if directory.contains(['\n', '\r']) {
        return Err("artifact directory contains an output delimiter".into());
    }
    let mut output = OpenOptions::new()
        .append(true)
        .open(output)
        .map_err(|error| error.to_string())?;
    let crashed = outcome == Outcome::InputFailure;
    let kind = if crashed { "crash" } else { "incomplete" };
    writeln!(
        output,
        "crashed={crashed}\nkind={kind}\nartifact_directory={directory}"
    )
    .map_err(|error| error.to_string())
}

#[expect(
    clippy::print_stdout,
    reason = "the command reports the completed replay count"
)]
fn triage(
    project: &Project,
    execute: &mut impl FnMut(&mut Command) -> Result<Option<i32>, String>,
) -> Result<(), String> {
    let directory = project.directory.join("artifacts").join(&project.target);
    let retained = archive_inputs(&directory)?;
    if retained.is_empty() {
        return Err("no retained fuzz inputs to replay".into());
    }
    project.build(execute)?;
    for input in &retained {
        let exit = execute(project.engine(&directory).arg("-runs=1").arg(input))?;
        match exit {
            Some(0) => (),
            Some(FAILURE_EXIT) => return Err(format!("input still fails: {}", input.display())),
            _ => {
                return Err(format!(
                    "input replay is incomplete: {} ({exit:?})",
                    input.display()
                ));
            }
        }
    }
    println!("Replayed {} retained inputs successfully", retained.len());
    Ok(())
}

pub(crate) fn dispatch(args: FuzzArgs) -> Result<(), String> {
    if !cfg!(target_os = "linux") {
        return Err("fuzz commands require the pinned Linux target; use the bounded Linux project runner on owner machines".into());
    }
    match args.command {
        FuzzCommand::Prove { root } => proofs::verify(&root, proofs::FUZZ),
        FuzzCommand::Triage {
            family,
            target,
            root,
        } => triage(&Project::read(&root, family, &target)?, &mut execute),
        FuzzCommand::Soak {
            family,
            target,
            root,
            seconds,
        } => {
            let report = soak(
                &Project::read(&root, family, &target)?,
                seconds,
                &mut execute,
            )?;
            match report.outcome {
                Outcome::Clean => Ok(()),
                Outcome::InputFailure | Outcome::Incomplete => {
                    github_output(report.outcome, &report.directory)?;
                    Err(format!(
                        "fuzz result {:?}; retained evidence at {}",
                        report.outcome,
                        report.directory.display()
                    ))
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{FAILURE_EXIT, Family, HOST_TARGET, NonZeroU32, Outcome, Project, soak};
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::process::Command;

    fn fixture() -> (tempfile::TempDir, Project) {
        let root = tempfile::tempdir().unwrap();
        let directory = root.path().join("crates/aozora/fuzz/render");
        fs::create_dir_all(directory.join("corpus/fixture")).unwrap();
        fs::write(directory.join("Cargo.toml"), "[[bin]]\nname = 'fixture'\n").unwrap();
        fs::write(
            root.path().join("mise.toml"),
            "[tools]\nrust = [{version = '1.99.1'}, {version = 'nightly-2026-07-15'}]\n",
        )
        .unwrap();
        let project = Project::read(root.path(), Family::Render, "fixture").unwrap();
        (root, project)
    }

    fn artifacts(command: &Command) -> PathBuf {
        command
            .get_args()
            .find_map(|argument| argument.to_str()?.strip_prefix("-artifact_prefix="))
            .map(PathBuf::from)
            .unwrap()
    }

    fn build(project: &Project, command: &Command) {
        assert_eq!(command.get_program(), "cargo");
        let arguments = command
            .get_args()
            .map(|argument| argument.to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        assert_eq!(
            &arguments[..7],
            &[
                "+nightly-2026-07-15",
                "fuzz",
                "build",
                "--fuzz-dir",
                ".",
                "--target",
                HOST_TARGET
            ]
        );
        assert_eq!(command.get_current_dir(), Some(project.directory.as_path()));
        let executable = project.executable();
        fs::create_dir_all(executable.parent().unwrap()).unwrap();
        fs::write(executable, b"compiled engine identity").unwrap();
    }

    fn scenario(exit: Option<i32>, count: usize, replay: Option<i32>) -> Outcome {
        let (_root, project) = fixture();
        let stale = project.directory.join("artifacts/fixture");
        fs::create_dir_all(&stale).unwrap();
        fs::write(stale.join("crash-stale"), b"old input").unwrap();
        let mut calls = 0;
        let result = soak(&project, NonZeroU32::new(1).unwrap(), &mut |command| {
            calls += 1;
            match calls {
                1 => {
                    build(&project, command);
                    Ok(Some(0))
                }
                2 => {
                    let directory = artifacts(command);
                    assert!(directory.starts_with(project.root.join("target/fuzz-reports")));
                    assert_ne!(directory, stale);
                    assert!(
                        command
                            .get_args()
                            .any(|argument| argument == "-error_exitcode=77")
                    );
                    for index in 0..count {
                        fs::write(directory.join(format!("crash-{index}")), b"current input")
                            .unwrap();
                    }
                    Ok(exit)
                }
                3 => {
                    assert!(command.get_args().any(|argument| argument == "-runs=1"));
                    assert!(command.get_args().any(|argument| {
                        Path::new(argument)
                            .file_name()
                            .is_some_and(|name| name == "crash-0")
                    }));
                    Ok(replay)
                }
                _ => panic!("unexpected engine invocation"),
            }
        })
        .unwrap();
        assert_eq!(
            calls,
            if exit == Some(FAILURE_EXIT) && count == 1 {
                3
            } else {
                2
            }
        );
        assert_eq!(result.directory.exists(), result.outcome != Outcome::Clean);
        assert_eq!(fs::read(stale.join("crash-stale")).unwrap(), b"old input");
        result.outcome
    }

    #[test]
    fn startup_and_ambiguous_results_never_become_crash_reports() {
        assert_eq!(scenario(Some(1), 0, None), Outcome::Incomplete);
        assert_eq!(
            scenario(Some(1), 1, Some(FAILURE_EXIT)),
            Outcome::Incomplete
        );
        assert_eq!(scenario(None, 0, None), Outcome::Incomplete);
        assert_eq!(scenario(Some(FAILURE_EXIT), 0, None), Outcome::Incomplete);
        assert_eq!(scenario(Some(0), 2, None), Outcome::Incomplete);
        assert_eq!(
            scenario(Some(FAILURE_EXIT), 2, Some(FAILURE_EXIT)),
            Outcome::Incomplete
        );
    }

    #[test]
    fn the_exact_retained_input_must_reproduce_in_the_same_engine() {
        assert_eq!(
            scenario(Some(FAILURE_EXIT), 1, Some(FAILURE_EXIT)),
            Outcome::InputFailure
        );
        for replay in [Some(0), Some(1), None] {
            assert_eq!(scenario(Some(FAILURE_EXIT), 1, replay), Outcome::Incomplete);
        }
        assert_eq!(scenario(Some(0), 0, None), Outcome::Clean);
    }

    #[test]
    fn a_failed_build_stops_before_running_or_creating_report_artifacts() {
        let (_root, project) = fixture();
        let mut calls = 0;
        let error = soak(&project, NonZeroU32::new(1).unwrap(), &mut |_| {
            calls += 1;
            Ok(Some(1))
        })
        .err()
        .unwrap();
        assert_eq!(calls, 1);
        assert!(error.contains("fuzz build did not complete"));
        assert!(!project.root.join("target/fuzz-reports").exists());
    }

    #[test]
    fn unregistered_targets_and_path_traversal_fail_before_building() {
        let (root, _) = fixture();
        for target in ["missing", "../fixture", "fixture/other", ""] {
            assert!(Project::read(root.path(), Family::Render, target).is_err());
        }
    }

    #[cfg(unix)]
    #[test]
    fn directories_and_symbolic_links_cannot_enter_the_input_inventory() {
        use std::os::unix::fs::symlink;

        for (link, name) in [
            (false, "crash-other"),
            (true, "crash-other"),
            (false, super::RESULT_FILE),
            (true, super::RESULT_FILE),
        ] {
            let directory = tempfile::tempdir().unwrap();
            let input = directory.path().join("crash-input");
            fs::write(&input, b"original bytes").unwrap();
            let other = directory.path().join(name);
            if link {
                symlink(&input, &other).unwrap();
            } else {
                fs::create_dir_all(&other).unwrap();
            }
            assert!(
                super::inputs(directory.path())
                    .unwrap_err()
                    .contains("regular files")
            );
            assert_eq!(fs::read(input).unwrap(), b"original bytes");
        }
    }

    #[test]
    fn replay_artifact_changes_invalidate_a_failure_report() {
        let (_root, project) = fixture();
        let mut calls = 0;
        let result = soak(&project, NonZeroU32::new(1).unwrap(), &mut |command| {
            calls += 1;
            match calls {
                1 => {
                    build(&project, command);
                    Ok(Some(0))
                }
                2 => {
                    fs::write(artifacts(command).join("crash-input"), b"original").unwrap();
                    Ok(Some(FAILURE_EXIT))
                }
                3 => {
                    fs::write(artifacts(command).join("oom-input"), b"original").unwrap();
                    Ok(Some(FAILURE_EXIT))
                }
                _ => panic!("unexpected replay"),
            }
        });
        assert!(result.unwrap_err().contains("artifact inventory changed"));
    }

    #[test]
    fn input_and_executable_mutation_invalidate_the_replay() {
        for change_binary in [false, true] {
            let (_root, project) = fixture();
            let mut calls = 0;
            let error = soak(&project, NonZeroU32::new(1).unwrap(), &mut |command| {
                calls += 1;
                if calls == 1 {
                    build(&project, command);
                    return Ok(Some(0));
                }
                let input = artifacts(command).join("crash-input");
                if calls == 2 {
                    fs::write(&input, b"original").unwrap();
                } else if change_binary {
                    fs::write(project.executable(), b"changed executable").unwrap();
                } else {
                    fs::write(&input, b"changed input").unwrap();
                }
                Ok(Some(FAILURE_EXIT))
            })
            .err()
            .unwrap();
            assert!(error.contains("changed during"));
        }
    }

    #[test]
    fn triage_refuses_startup_failures_and_missing_inputs() {
        let (_root, project) = fixture();
        let directory = project.directory.join("artifacts/fixture");
        fs::create_dir_all(&directory).unwrap();
        fs::write(directory.join(super::RESULT_FILE), b"{}").unwrap();
        assert!(
            super::triage(&project, &mut |_| panic!(
                "empty input must stop before building"
            ))
            .is_err()
        );
        fs::write(directory.join("crash-input"), b"input").unwrap();
        for exit in [Some(0), Some(1), Some(FAILURE_EXIT), None] {
            let mut calls = 0;
            let result = super::triage(&project, &mut |command| {
                calls += 1;
                if calls == 1 {
                    build(&project, command);
                    Ok(Some(0))
                } else {
                    Ok(exit)
                }
            });
            assert_eq!(calls, 2);
            assert_eq!(result.is_ok(), exit == Some(0));
            if exit == Some(1) || exit.is_none() {
                assert!(result.unwrap_err().contains("incomplete"));
            }
        }
    }

    #[test]
    fn an_oom_artifact_stays_incomplete_without_replay() {
        let (_root, project) = fixture();
        let mut calls = 0;
        let result = soak(&project, NonZeroU32::new(1).unwrap(), &mut |command| {
            calls += 1;
            match calls {
                1 => {
                    build(&project, command);
                    Ok(Some(0))
                }
                2 => {
                    fs::write(artifacts(command).join("oom-input"), b"input").unwrap();
                    Ok(Some(FAILURE_EXIT))
                }
                _ => panic!("unclassified input must not trigger a replay"),
            }
        })
        .unwrap();
        assert_eq!(result.outcome, Outcome::Incomplete);
        assert_eq!(calls, 2);
    }

    #[test]
    fn unexpected_engine_metadata_cannot_disappear_or_be_overwritten() {
        let (_root, project) = fixture();
        let mut calls = 0;
        let mut evidence = None;
        let result = soak(&project, NonZeroU32::new(1).unwrap(), &mut |command| {
            calls += 1;
            if calls == 1 {
                build(&project, command);
                return Ok(Some(0));
            }
            let path = artifacts(command).join(super::RESULT_FILE);
            fs::write(&path, b"unexpected engine bytes").unwrap();
            evidence = Some(path);
            Ok(Some(0))
        });
        assert!(result.unwrap_err().contains("without overwriting"));
        assert_eq!(
            fs::read(evidence.unwrap()).unwrap(),
            b"unexpected engine bytes"
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    #[ignore = "requires the pinned Linux nightly and cargo-fuzz; the release gate runs this control"]
    fn native_engine_contract() {
        let root = tempfile::tempdir().unwrap();
        let directory = root.path().join("crates/aozora/fuzz/render");
        fs::create_dir_all(directory.join("fuzz_targets")).unwrap();
        for (name, contents) in [
            (
                "Cargo.toml",
                include_str!("../tests/fixtures/fuzz-report/Cargo.toml"),
            ),
            (
                "Cargo.lock",
                include_str!("../tests/fixtures/fuzz-report/Cargo.lock"),
            ),
            (
                "fuzz_targets/crash.rs",
                include_str!("../tests/fixtures/fuzz-report/fuzz_targets/crash.rs"),
            ),
            (
                "fuzz_targets/clean.rs",
                include_str!("../tests/fixtures/fuzz-report/fuzz_targets/clean.rs"),
            ),
        ] {
            fs::write(directory.join(name), contents).unwrap();
        }
        fs::write(
            root.path().join("mise.toml"),
            include_str!("../../../mise.toml"),
        )
        .unwrap();
        fs::create_dir_all(directory.join("corpus/crash")).unwrap();
        fs::write(directory.join("corpus/crash/trigger"), b"boom").unwrap();
        let crash = Project::read(root.path(), Family::Render, "crash").unwrap();
        let report = soak(&crash, NonZeroU32::new(1).unwrap(), &mut super::execute).unwrap();
        assert_eq!(report.outcome, Outcome::InputFailure);
        let retained = super::archive_inputs(&report.directory).unwrap();
        let [input] = retained.as_slice() else {
            panic!("one actual engine input required")
        };
        assert_eq!(fs::read(input).unwrap(), b"boom");
        let manifest: serde_json::Value =
            serde_json::from_slice(&fs::read(report.directory.join(super::RESULT_FILE)).unwrap())
                .unwrap();
        assert_eq!(manifest["exit"], FAILURE_EXIT);
        assert_eq!(manifest["replay_exit"], FAILURE_EXIT);
        assert_eq!(manifest["reproduced_input_failure"], true);
        let clean = Project::read(root.path(), Family::Render, "clean").unwrap();
        assert_eq!(
            soak(&clean, NonZeroU32::new(1).unwrap(), &mut super::execute)
                .unwrap()
                .outcome,
            Outcome::Clean
        );
        fs::remove_file(directory.join("fuzz_targets/clean.rs")).unwrap();
        assert!(
            soak(&clean, NonZeroU32::new(1).unwrap(), &mut super::execute)
                .unwrap_err()
                .contains("fuzz build did not complete")
        );
    }
}
