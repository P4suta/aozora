use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::{PerfArgs, PerfOp};

pub(crate) fn dispatch(args: &PerfArgs) -> Result<(), String> {
    match args.op {
        PerfOp::Check => check(),
    }
}

fn check() -> Result<(), String> {
    let root = workspace_root()?;
    let baseline_path = root.join("crates/aozora-bench/perf-baseline.tsv");
    let baseline = parse_baseline(&read_file(&baseline_path)?)?;
    if baseline.is_empty() {
        return Err(format!(
            "no performance cases in {}",
            baseline_path.display()
        ));
    }

    let cargo = cargo_program();
    let mut build = Command::new(&cargo);
    build.current_dir(&root).args([
        "build",
        "--release",
        "-p",
        "aozora-bench",
        "--example",
        "perf_gate",
    ]);
    run(&mut build, "build perf_gate")?;

    let binary = target_directory(&root)
        .join("release/examples")
        .join(format!("perf_gate{}", env::consts::EXE_SUFFIX));
    require_nonempty(&binary)?;
    let output_dir = tempfile::tempdir().map_err(|err| format!("create Callgrind dir: {err}"))?;
    let mut actual = BTreeMap::new();
    let mut errors = Vec::new();

    for (name, ceiling) in &baseline {
        let callgrind = output_dir.path().join(format!("{name}.out"));
        let mut command = Command::new("valgrind");
        command
            .args(["--quiet", "--tool=callgrind", "--collect-atstart=no"])
            .arg(format!("--callgrind-out-file={}", callgrind.display()))
            .arg(&binary)
            .arg(name)
            .stdout(Stdio::null());
        if let Err(error) = run(&mut command, &format!("Callgrind {name}")) {
            errors.push(format!("::error title=perf-gate::{error}"));
            continue;
        }
        let instructions = parse_callgrind(&read_file(&callgrind)?)?;
        println!("{name:<24} {instructions:>10} instructions (ceiling {ceiling:>10})");
        if instructions > *ceiling {
            errors.push(format!(
                "::error title=perf-gate::{name} regressed beyond its instruction ceiling"
            ));
        }
        actual.insert(name.as_str(), instructions);
    }
    check_speedups(&actual, &mut errors)?;

    if errors.is_empty() {
        Ok(())
    } else {
        for error in &errors {
            eprintln!("{error}");
        }
        Err(format!(
            "performance gate failed with {} error(s)",
            errors.len()
        ))
    }
}

fn parse_baseline(text: &str) -> Result<Vec<(String, u64)>, String> {
    text.lines()
        .enumerate()
        .map(|(index, line)| {
            let (name, instructions) = line
                .split_once('\t')
                .ok_or_else(|| format!("performance baseline line {} is invalid", index + 1))?;
            let instructions = instructions
                .parse()
                .map_err(|err| format!("invalid instruction ceiling for {name}: {err}"))?;
            Ok((name.to_owned(), instructions))
        })
        .collect()
}

fn parse_callgrind(text: &str) -> Result<u64, String> {
    text.lines()
        .find_map(|line| line.strip_prefix("summary: "))
        .ok_or_else(|| "Callgrind output has no summary".to_owned())?
        .parse()
        .map_err(|err| format!("invalid Callgrind summary: {err}"))
}

fn check_speedups(actual: &BTreeMap<&str, u64>, errors: &mut Vec<String>) -> Result<(), String> {
    let single = required_case(actual, "edit-single")?;
    let full_single = required_case(actual, "full-edit-source")?;
    if single.saturating_mul(2) >= full_single {
        errors.push(
            "::error title=perf-gate::incremental edit is less than 2x faster than full parse"
                .to_owned(),
        );
    }

    let multiple = required_case(actual, "edit-multiple")?;
    let full_multiple = required_case(actual, "full-multiple-edit-source")?;
    if multiple.saturating_mul(5) >= full_multiple.saturating_mul(4) {
        errors.push(
            "::error title=perf-gate::incremental multiple edit is less than 1.25x faster than full parse"
                .to_owned(),
        );
    }
    Ok(())
}

fn required_case(actual: &BTreeMap<&str, u64>, name: &str) -> Result<u64, String> {
    actual
        .get(name)
        .copied()
        .ok_or_else(|| format!("performance case did not produce a result: {name}"))
}

fn target_directory(root: &Path) -> PathBuf {
    env::var_os("CARGO_TARGET_DIR").map_or_else(|| root.join("target"), PathBuf::from)
}

fn cargo_program() -> PathBuf {
    env::var_os("CARGO").map_or_else(|| PathBuf::from("cargo"), PathBuf::from)
}

fn require_nonempty(path: &Path) -> Result<(), String> {
    let metadata =
        fs::metadata(path).map_err(|err| format!("inspect {}: {err}", path.display()))?;
    if metadata.len() == 0 {
        Err(format!("binary is empty: {}", path.display()))
    } else {
        Ok(())
    }
}

fn run(command: &mut Command, label: &str) -> Result<(), String> {
    let status = command
        .status()
        .map_err(|err| format!("run {label}: {err}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("{label} failed: {status}"))
    }
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
    use std::collections::BTreeMap;

    use super::{check_speedups, parse_callgrind};

    #[test]
    fn callgrind_summary_is_parsed() {
        assert_eq!(
            parse_callgrind("events: Ir\nsummary: 9012\n").unwrap(),
            9012
        );
        drop(parse_callgrind("events: Ir\n").unwrap_err());
    }

    #[test]
    fn edit_speedups_enforce_both_contracts() {
        let actual = BTreeMap::from([
            ("edit-single", 49),
            ("full-edit-source", 100),
            ("edit-multiple", 79),
            ("full-multiple-edit-source", 100),
        ]);
        let mut errors = Vec::new();
        check_speedups(&actual, &mut errors).unwrap();
        assert!(errors.is_empty());

        let regressed = BTreeMap::from([
            ("edit-single", 50),
            ("full-edit-source", 100),
            ("edit-multiple", 80),
            ("full-multiple-edit-source", 100),
        ]);
        check_speedups(&regressed, &mut errors).unwrap();
        assert_eq!(errors.len(), 2);
    }
}
