use std::collections::{BTreeMap, BTreeSet};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use clap::{Args, Subcommand};
use serde::{Deserialize, Serialize};

#[derive(Args)]
pub(crate) struct ArtifactsArgs {
    #[command(subcommand)]
    op: ArtifactsOp,
}

#[derive(Subcommand)]
enum ArtifactsOp {
    /// Package every publishable workspace crate and check the exact archives.
    Crates {
        /// Permit a dirty worktree for local verification.
        #[arg(long)]
        allow_dirty: bool,
    },
    /// Enforce the monotonic size ceilings for distribution artifacts.
    SizeCheck(SizeCheckArgs),
}

#[derive(Args)]
struct SizeCheckArgs {
    /// Check only one named artifact from the baseline.
    #[arg(long, env = "ARTIFACT_SIZE_ONLY")]
    only: Option<String>,
    /// CLI artifact path.
    #[arg(
        long,
        env = "AOZORA_CLI_ARTIFACT",
        default_value = "target/release-ready-build/aozora"
    )]
    cli: PathBuf,
    /// Browser WASM artifact path.
    #[arg(
        long,
        env = "AOZORA_WASM_ARTIFACT",
        default_value = "crates/aozora-wasm/pkg/aozora_wasm_bg.wasm"
    )]
    wasm: PathBuf,
    /// Extism artifact path.
    #[arg(
        long,
        env = "AOZORA_EXTISM_ARTIFACT",
        default_value = "crates/aozora-extism/dist/aozora.wasm"
    )]
    extism: PathBuf,
}

#[derive(Deserialize)]
struct Metadata {
    packages: Vec<Package>,
    target_directory: PathBuf,
}

#[derive(Deserialize)]
struct Package {
    name: String,
    version: String,
    publish: Option<Vec<String>>,
    dependencies: Vec<Dependency>,
}

#[derive(Deserialize)]
struct Dependency {
    name: String,
}

#[derive(Serialize)]
struct CargoConfig {
    patch: Patch,
}

#[derive(Serialize)]
struct Patch {
    #[serde(rename = "crates-io")]
    crates_io: BTreeMap<String, LocalPatch>,
}

#[derive(Serialize)]
struct LocalPatch {
    path: PathBuf,
}

pub(crate) fn dispatch(args: &ArtifactsArgs) -> Result<(), String> {
    match &args.op {
        ArtifactsOp::Crates { allow_dirty } => package_crates(*allow_dirty),
        ArtifactsOp::SizeCheck(args) => size_check(args),
    }
}

fn package_crates(allow_dirty: bool) -> Result<(), String> {
    let root = workspace_root()?;
    let metadata = cargo_metadata(&root)?;
    let packages = publishable_packages(&metadata.packages);
    if packages.is_empty() {
        return Err("artifacts crates: no publishable workspace crates".to_owned());
    }

    let cargo = cargo_program();
    run_cargo_package(&root, &cargo, &packages, allow_dirty)?;

    let temp = tempfile::tempdir().map_err(|err| format!("create package verify dir: {err}"))?;
    let archives = extract_archives(
        &packages,
        &metadata.target_directory.join("package"),
        temp.path(),
    )?;
    verify_archives(&packages, &cargo, &metadata.target_directory, temp.path())?;
    stage_archives(&root, &archives)?;

    println!(
        "xtask artifacts crates: {} exact crate artifacts verified",
        archives.len()
    );
    Ok(())
}

fn size_check(args: &SizeCheckArgs) -> Result<(), String> {
    let root = workspace_root()?;
    let baseline_path = root.join("crates/aozora-bench/artifact-size-baseline.tsv");
    let baseline = fs::read_to_string(&baseline_path)
        .map_err(|err| format!("read {}: {err}", baseline_path.display()))?;
    let artifacts = BTreeMap::from([
        ("cli", &args.cli),
        ("wasm", &args.wasm),
        ("extism", &args.extism),
    ]);
    let mut checked = 0;
    let mut errors = Vec::new();

    for (line_index, line) in baseline.lines().enumerate() {
        let (name, ceiling) = parse_size_baseline(line)
            .map_err(|err| format!("{}: {err}", line_index.saturating_add(1)))?;
        if args.only.as_deref().is_some_and(|only| only != name) {
            continue;
        }
        checked += 1;
        let Some(relative) = artifacts.get(name) else {
            errors.push(format!("{name} has no configured artifact path"));
            continue;
        };
        let path = if relative.is_absolute() {
            (*relative).clone()
        } else {
            root.join(relative)
        };
        match fs::metadata(&path) {
            Ok(metadata) if metadata.len() > 0 => {
                let actual = metadata.len();
                println!("{name:<8} {actual:>10} bytes (ceiling {ceiling:>10})");
                if actual > ceiling {
                    errors.push(format!("{name} exceeds its size ceiling"));
                }
            }
            Ok(_) | Err(_) => errors.push(format!("{name} artifact is missing")),
        }
    }

    if checked == 0 {
        errors.push("no artifact matched the requested gate".to_owned());
    }
    if errors.is_empty() {
        Ok(())
    } else {
        for error in &errors {
            eprintln!("::error title=artifact-size::{error}");
        }
        Err(format!(
            "artifact size gate failed with {} error(s)",
            errors.len()
        ))
    }
}

fn parse_size_baseline(line: &str) -> Result<(&str, u64), String> {
    let (name, ceiling) = line
        .split_once('\t')
        .ok_or_else(|| "expected tab-separated name and ceiling".to_owned())?;
    if name.is_empty() {
        return Err("artifact name is empty".to_owned());
    }
    let ceiling = ceiling
        .parse()
        .map_err(|err| format!("invalid ceiling for {name}: {err}"))?;
    Ok((name, ceiling))
}

fn run_cargo_package(
    root: &Path,
    cargo: &Path,
    packages: &[&Package],
    allow_dirty: bool,
) -> Result<(), String> {
    let mut package = Command::new(cargo);
    package
        .current_dir(root)
        .args(["package", "--locked", "--no-verify"]);
    if allow_dirty {
        package.arg("--allow-dirty");
    }
    for member in packages {
        package.args(["-p", &member.name]);
    }
    run(&mut package, "cargo package")
}

fn extract_archives(
    packages: &[&Package],
    package_dir: &Path,
    extracted: &Path,
) -> Result<Vec<PathBuf>, String> {
    let mut archives = Vec::with_capacity(packages.len());
    for member in packages {
        let archive = package_dir.join(format!("{}-{}.crate", member.name, member.version));
        require_nonempty(&archive)?;
        let mut unpack = Command::new("tar");
        unpack.args(["-xzf"]).arg(&archive).arg("-C").arg(extracted);
        run(&mut unpack, &format!("extract {}", archive.display()))?;
        archives.push(archive);
    }
    Ok(archives)
}

fn verify_archives(
    packages: &[&Package],
    cargo: &Path,
    target_directory: &Path,
    extracted: &Path,
) -> Result<(), String> {
    let cargo_dir = extracted.join(".cargo");
    fs::create_dir(&cargo_dir).map_err(|err| format!("create {}: {err}", cargo_dir.display()))?;
    let versions = packages
        .iter()
        .map(|member| (member.name.as_str(), member.version.as_str()))
        .collect::<BTreeMap<_, _>>();
    let verify_target = target_directory.join("package-verify");
    for member in packages {
        let dependencies = member
            .dependencies
            .iter()
            .filter_map(|dependency| {
                versions
                    .get(dependency.name.as_str())
                    .map(|version| (dependency.name.as_str(), *version))
            })
            .collect::<BTreeSet<_>>();
        write_patch_config(&cargo_dir, extracted, &dependencies)?;

        let manifest = extracted
            .join(format!("{}-{}", member.name, member.version))
            .join("Cargo.toml");
        let mut check = Command::new(cargo);
        check
            .current_dir(extracted)
            .env("CARGO_TARGET_DIR", &verify_target)
            .args(["check", "--all-features", "--manifest-path"])
            .arg(&manifest);
        run(&mut check, &format!("check {}", member.name))?;
    }
    Ok(())
}

fn stage_archives(root: &Path, archives: &[PathBuf]) -> Result<(), String> {
    let destination = root.join("target/release-ready-build/crates");
    if destination.exists() {
        fs::remove_dir_all(&destination)
            .map_err(|err| format!("remove {}: {err}", destination.display()))?;
    }
    fs::create_dir_all(&destination)
        .map_err(|err| format!("create {}: {err}", destination.display()))?;
    for archive in archives {
        let name = archive
            .file_name()
            .ok_or_else(|| format!("archive has no file name: {}", archive.display()))?;
        fs::copy(archive, destination.join(name))
            .map_err(|err| format!("copy {}: {err}", archive.display()))?;
    }
    Ok(())
}

fn cargo_metadata(root: &Path) -> Result<Metadata, String> {
    let mut command = Command::new(cargo_program());
    command
        .current_dir(root)
        .args(["metadata", "--no-deps", "--format-version", "1"]);
    let output = output(&mut command, "cargo metadata")?;
    serde_json::from_slice(&output.stdout).map_err(|err| format!("parse cargo metadata: {err}"))
}

fn publishable_packages(packages: &[Package]) -> Vec<&Package> {
    packages
        .iter()
        .filter(|package| {
            package
                .publish
                .as_ref()
                .is_none_or(|registries| !registries.is_empty())
        })
        .collect()
}

fn write_patch_config(
    cargo_dir: &Path,
    extracted: &Path,
    dependencies: &BTreeSet<(&str, &str)>,
) -> Result<(), String> {
    let crates_io = dependencies
        .iter()
        .map(|(name, version)| {
            (
                (*name).to_owned(),
                LocalPatch {
                    path: extracted.join(format!("{name}-{version}")),
                },
            )
        })
        .collect();
    let text = toml::to_string(&CargoConfig {
        patch: Patch { crates_io },
    })
    .map_err(|err| format!("serialize cargo patch config: {err}"))?;
    let path = cargo_dir.join("config.toml");
    fs::write(&path, text).map_err(|err| format!("write {}: {err}", path.display()))
}

fn cargo_program() -> PathBuf {
    env::var_os("CARGO").map_or_else(|| PathBuf::from("cargo"), PathBuf::from)
}

fn workspace_root() -> Result<PathBuf, String> {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .map(Path::to_path_buf)
        .ok_or_else(|| "could not derive workspace root".to_owned())
}

fn require_nonempty(path: &Path) -> Result<(), String> {
    let metadata =
        fs::metadata(path).map_err(|err| format!("inspect {}: {err}", path.display()))?;
    if metadata.len() == 0 {
        Err(format!("artifact is empty: {}", path.display()))
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

fn output(command: &mut Command, label: &str) -> Result<Output, String> {
    let output = command
        .output()
        .map_err(|err| format!("run {label}: {err}"))?;
    if output.status.success() {
        Ok(output)
    } else {
        Err(format!(
            "{label} failed: {}\n{}",
            output.status,
            String::from_utf8_lossy(&output.stderr)
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::{Dependency, Package, parse_size_baseline, publishable_packages};

    fn package(name: &str, publish: Option<Vec<String>>) -> Package {
        Package {
            name: name.to_owned(),
            version: "1.0.0".to_owned(),
            publish,
            dependencies: Vec::<Dependency>::new(),
        }
    }

    #[test]
    fn empty_registry_list_disables_packaging() {
        let packages = [
            package("default", None),
            package("disabled", Some(Vec::new())),
            package("private-registry", Some(vec!["private".to_owned()])),
        ];

        let names = publishable_packages(&packages)
            .into_iter()
            .map(|package| package.name.as_str())
            .collect::<Vec<_>>();

        assert_eq!(names, ["default", "private-registry"]);
    }

    #[test]
    fn size_baseline_requires_a_numeric_tab_separated_ceiling() {
        assert_eq!(parse_size_baseline("cli\t123").unwrap(), ("cli", 123));
        drop(parse_size_baseline("cli 123").unwrap_err());
        drop(parse_size_baseline("cli\tlarge").unwrap_err());
    }
}
