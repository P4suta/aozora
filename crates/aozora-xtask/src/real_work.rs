use std::collections::BTreeMap;
use std::env::current_dir;
use std::fs::{self, File};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Command;

use clap::{Args, Subcommand, ValueEnum};
use serde::Serialize;
use sha2::{Digest, Sha256};
use walkdir::WalkDir;

#[derive(Args)]
pub(crate) struct RealWorkArgs {
    #[command(subcommand)]
    op: RealWorkOp,
}

#[derive(Subcommand)]
enum RealWorkOp {
    /// Build one OS-specific seven-engine bundle for the release lab.
    Bundle(BundleArgs),
    /// Pack the pinned rights-filtered corpus without changing its bytes.
    Corpus(CorpusArgs),
    /// Validate and pack reviewed diagnostics and visual baselines.
    Baselines(BaselineArgs),
}

#[derive(Clone, Copy, ValueEnum)]
enum Platform {
    Linux,
    Macos,
    Windows,
}

impl Platform {
    const fn executable_suffix(self) -> &'static str {
        if matches!(self, Self::Windows) {
            ".exe"
        } else {
            ""
        }
    }
}

#[derive(Args)]
struct BundleArgs {
    #[arg(long)]
    platform: Platform,
    #[arg(long, env = "GITHUB_SHA")]
    commit: String,
    #[arg(long, default_value = env!("CARGO_PKG_VERSION"))]
    version: String,
    #[arg(long, default_value_t = aozora::json::SCHEMA_VERSION)]
    schema_version: u32,
    #[arg(long)]
    release_dir: PathBuf,
    #[arg(long)]
    native_dir: PathBuf,
    #[arg(long)]
    python_dir: PathBuf,
    #[arg(long)]
    adapters_dir: PathBuf,
    #[arg(long)]
    extism_worker: PathBuf,
    #[arg(long)]
    out_dir: PathBuf,
}

#[derive(Args)]
struct CorpusArgs {
    #[arg(long)]
    checkout: PathBuf,
    #[arg(long)]
    expected_commit: String,
    #[arg(long)]
    out: PathBuf,
}

#[derive(Args)]
struct BaselineArgs {
    #[arg(long)]
    diagnostics: PathBuf,
    #[arg(long)]
    site: PathBuf,
    #[arg(long)]
    out_dir: PathBuf,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Manifest {
    schema_version: u32,
    aozora_commit: String,
    engines: BTreeMap<String, Engine>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Engine {
    command: Vec<String>,
    artifact_path: String,
    sha256: String,
    support_paths: BTreeMap<String, String>,
    expected_version: String,
    expected_schema_version: u32,
    timeout_ms: u64,
}

fn workspace_root() -> Result<PathBuf, String> {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .map(Path::to_path_buf)
        .ok_or_else(|| "xtask manifest is outside a workspace".to_owned())
}

fn validate_commit(value: &str, label: &str) -> Result<(), String> {
    if value.len() == 40
        && value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    {
        Ok(())
    } else {
        Err(format!(
            "{label} must be a lowercase 40-character commit SHA"
        ))
    }
}

fn require_file(path: &Path) -> Result<(), String> {
    if path.is_file() {
        Ok(())
    } else {
        Err(format!("required file is missing: {}", path.display()))
    }
}

fn find_one(
    root: &Path,
    predicate: impl Fn(&Path) -> bool,
    label: &str,
) -> Result<PathBuf, String> {
    let mut matches = WalkDir::new(root)
        .follow_links(false)
        .into_iter()
        .filter_map(Result::ok)
        .map(walkdir::DirEntry::into_path)
        .filter(|path| path.is_file() && predicate(path))
        .collect::<Vec<_>>();
    matches.sort();
    match matches.as_slice() {
        [path] => Ok(path.clone()),
        [] => Err(format!(
            "{label}: no matching file under {}",
            root.display()
        )),
        _ => Err(format!(
            "{label}: expected one matching file under {}, found {}",
            root.display(),
            matches.len()
        )),
    }
}

fn copy(source: &Path, destination: &Path) -> Result<(), String> {
    require_file(source)?;
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("create {}: {error}", parent.display()))?;
    }
    fs::copy(source, destination).map(|_| ()).map_err(|error| {
        format!(
            "copy {} to {}: {error}",
            source.display(),
            destination.display()
        )
    })
}

fn run(command: &mut Command, label: &str) -> Result<(), String> {
    let status = command
        .status()
        .map_err(|error| format!("start {label}: {error}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("{label} failed with {status}"))
    }
}

fn python_command() -> Result<&'static str, String> {
    ["python3", "python"]
        .into_iter()
        .find(|candidate| {
            Command::new(candidate)
                .arg("--version")
                .output()
                .is_ok_and(|output| output.status.success())
        })
        .ok_or_else(|| "Python 3 is required to unpack and exercise release artifacts".to_owned())
}

fn command_output_path(path: &Path) -> Result<PathBuf, String> {
    if path.is_absolute() {
        return Ok(path.to_path_buf());
    }
    current_dir()
        .map(|directory| directory.join(path))
        .map_err(|error| format!("resolve command output {}: {error}", path.display()))
}

fn digest(path: &Path) -> Result<String, String> {
    let mut file = File::open(path).map_err(|error| format!("open {}: {error}", path.display()))?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0_u8; 64 * 1024].into_boxed_slice();
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|error| format!("read {}: {error}", path.display()))?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

fn relative(path: &Path, root: &Path) -> Result<String, String> {
    path.strip_prefix(root)
        .map_err(|_| format!("{} is outside {}", path.display(), root.display()))
        .map(|value| value.to_string_lossy().replace('\\', "/"))
}

fn supports(paths: &[&Path], staging: &Path) -> Result<BTreeMap<String, String>, String> {
    paths
        .iter()
        .map(|path| Ok((relative(path, staging)?, digest(path)?)))
        .collect()
}

struct Identity<'a> {
    staging: &'a Path,
    version: &'a str,
    schema: u32,
}

fn engine(
    command: Vec<String>,
    artifact: &Path,
    support: &[&Path],
    identity: &Identity<'_>,
) -> Result<Engine, String> {
    Ok(Engine {
        command,
        artifact_path: relative(artifact, identity.staging)?,
        sha256: digest(artifact)?,
        support_paths: supports(support, identity.staging)?,
        expected_version: identity.version.to_owned(),
        expected_schema_version: identity.schema,
        timeout_ms: 30_000,
    })
}

fn deterministic_tar(source: &Path, output: &Path) -> Result<(), String> {
    if let Some(parent) = output.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("create {}: {error}", parent.display()))?;
    }
    let file =
        File::create(output).map_err(|error| format!("create {}: {error}", output.display()))?;
    let mut archive = tar::Builder::new(file);
    let mut files = WalkDir::new(source)
        .follow_links(false)
        .into_iter()
        .filter_map(Result::ok)
        .map(walkdir::DirEntry::into_path)
        .filter(|path| path.is_file())
        .collect::<Vec<_>>();
    files.sort();
    for path in files {
        let name = path
            .strip_prefix(source)
            .map_err(|error| format!("relative tar path: {error}"))?;
        let metadata =
            fs::metadata(&path).map_err(|error| format!("stat {}: {error}", path.display()))?;
        let mut header = tar::Header::new_gnu();
        header.set_size(metadata.len());
        header.set_uid(0);
        header.set_gid(0);
        header.set_mtime(0);
        #[cfg(unix)]
        let mode = {
            use std::os::unix::fs::PermissionsExt;
            metadata.permissions().mode()
        };
        #[cfg(not(unix))]
        let mode = 0o644;
        header.set_mode(mode);
        header.set_cksum();
        let mut input =
            File::open(&path).map_err(|error| format!("open {}: {error}", path.display()))?;
        archive
            .append_data(&mut header, name, &mut input)
            .map_err(|error| format!("append {}: {error}", path.display()))?;
    }
    archive
        .finish()
        .map_err(|error| format!("finish {}: {error}", output.display()))
}

#[allow(
    clippy::too_many_lines,
    reason = "bundle assembly keeps discovery, extraction, and the seven manifest entries in release order"
)]
fn bundle(args: &BundleArgs) -> Result<(), String> {
    validate_commit(&args.commit, "commit")?;
    if args.version.is_empty() || args.schema_version == 0 {
        return Err("version and schema-version must be non-empty".to_owned());
    }
    let python = python_command()?;
    let staging = args.out_dir.join("staging");
    if staging.exists() {
        fs::remove_dir_all(&staging)
            .map_err(|error| format!("remove {}: {error}", staging.display()))?;
    }
    fs::create_dir_all(&staging)
        .map_err(|error| format!("create {}: {error}", staging.display()))?;
    let artifacts = staging.join("artifacts");
    let packages = staging.join("packages");
    let workers = staging.join("workers");
    fs::create_dir_all(&artifacts).map_err(|error| error.to_string())?;
    fs::create_dir_all(&packages).map_err(|error| error.to_string())?;
    fs::create_dir_all(&workers).map_err(|error| error.to_string())?;

    let npm_source = find_one(
        &args.release_dir,
        |path| path.extension().is_some_and(|ext| ext == "tgz"),
        "npm artifact",
    )?;
    let crate_name = format!("aozora-{}.crate", args.version);
    let crate_source = find_one(
        &args.release_dir,
        |path| {
            path.file_name()
                .is_some_and(|name| name == crate_name.as_str())
        },
        "crate artifact",
    )?;
    let extism_source = find_one(
        &args.release_dir,
        |path| path.file_name().is_some_and(|name| name == "aozora.wasm"),
        "Extism artifact",
    )?;
    let go_source = find_one(
        &args.release_dir,
        |path| {
            path.file_name()
                .is_some_and(|name| name == "aozora-go.tar.gz")
        },
        "Go artifact",
    )?;
    let native_source = find_one(
        &args.native_dir,
        |path| {
            path.file_name().is_some_and(|name| {
                let name = name.to_string_lossy();
                name.starts_with("aozora-v")
                    && (name.ends_with(".tar.gz") || name.ends_with(".zip"))
            })
        },
        "native artifact",
    )?;
    let wheel_source = find_one(
        &args.python_dir,
        |path| path.extension().is_some_and(|ext| ext == "whl"),
        "Python wheel",
    )?;

    let npm = artifacts.join(npm_source.file_name().ok_or("npm artifact has no name")?);
    let crate_artifact = artifacts.join(&crate_name);
    let extism = artifacts.join("aozora-extism.wasm");
    let go = artifacts.join("aozora-go.tar.gz");
    let native = artifacts.join(
        native_source
            .file_name()
            .ok_or("native artifact has no name")?,
    );
    let wheel = artifacts.join(wheel_source.file_name().ok_or("wheel has no name")?);
    copy(&npm_source, &npm)?;
    copy(&crate_source, &crate_artifact)?;
    copy(&extism_source, &extism)?;
    copy(&go_source, &go)?;
    copy(&native_source, &native)?;
    copy(&wheel_source, &wheel)?;

    let npm_dir = packages.join("npm");
    let native_dir = packages.join("native");
    let python_dir = packages.join("python");
    let go_dir = packages.join("go");
    let rust_build = tempfile::tempdir()
        .map_err(|error| format!("create packaged-crate build directory: {error}"))?;
    let rust_dir = rust_build.path().join("source");
    for directory in [&npm_dir, &native_dir, &python_dir, &go_dir, &rust_dir] {
        fs::create_dir_all(directory)
            .map_err(|error| format!("create {}: {error}", directory.display()))?;
    }
    run(
        Command::new("tar")
            .args(["-xzf"])
            .arg(&npm)
            .arg("-C")
            .arg(&npm_dir),
        "extract npm artifact",
    )?;
    run(
        Command::new("tar")
            .args(["-xf"])
            .arg(&native)
            .arg("-C")
            .arg(&native_dir),
        "extract native artifact",
    )?;
    run(
        Command::new("tar")
            .args(["-xzf"])
            .arg(&go)
            .arg("-C")
            .arg(&go_dir),
        "extract Go artifact",
    )?;
    run(
        Command::new("tar")
            .args(["-xzf"])
            .arg(&crate_artifact)
            .arg("-C")
            .arg(&rust_dir),
        "extract Rust crate",
    )?;
    run(
        Command::new(python)
            .args(["-m", "zipfile", "-e"])
            .arg(&wheel)
            .arg(&python_dir),
        "extract Python wheel",
    )?;

    let suffix = args.platform.executable_suffix();
    let rust_worker = workers.join(format!("aozora-real-work-rust{suffix}"));
    let extism_worker = workers.join(format!("aozora-real-work-extism{suffix}"));
    let go_worker = workers.join(format!("aozora-real-work-go{suffix}"));
    let rust_manifest = find_one(
        &rust_dir,
        |path| path.file_name().is_some_and(|name| name == "Cargo.toml"),
        "Rust crate manifest",
    )?;
    let rust_target = rust_build.path().join("target");
    run(
        Command::new("cargo")
            .args(["build", "--release", "--locked", "--manifest-path"])
            .arg(&rust_manifest)
            .args(["--example", "real_work_worker", "--features", "json"])
            .arg("--target-dir")
            .arg(&rust_target),
        "build worker from the packaged Rust crate",
    )?;
    copy(
        &rust_target
            .join("release")
            .join("examples")
            .join(format!("real_work_worker{suffix}")),
        &rust_worker,
    )?;
    copy(&args.extism_worker, &extism_worker)?;
    let go_module = go_dir.join("aozora-go");
    let go_worker_output = command_output_path(&go_worker)?;
    run(
        Command::new("go")
            .current_dir(&go_module)
            .args(["build", "-trimpath", "-buildvcs=false", "-o"])
            .arg(&go_worker_output)
            .arg("./cmd/aozora-real-work-worker"),
        "build Go release worker",
    )?;

    let wasm_adapter = workers.join("release-wasm.ts");
    let python_adapter = workers.join("release-python.py");
    let ffi_adapter = workers.join("release-ffi.py");
    copy(&args.adapters_dir.join("release-wasm.ts"), &wasm_adapter)?;
    copy(
        &args.adapters_dir.join("release-python.py"),
        &python_adapter,
    )?;
    copy(&args.adapters_dir.join("release-ffi.py"), &ffi_adapter)?;

    let cli = find_one(
        &native_dir,
        |path| {
            path.file_name()
                .is_some_and(|name| name == format!("aozora{suffix}").as_str())
        },
        "native CLI",
    )?;
    let library = find_one(
        &native_dir,
        |path| {
            path.file_name().is_some_and(|name| {
                matches!(
                    name.to_string_lossy().as_ref(),
                    "libaozora_ffi.so" | "libaozora_ffi.dylib" | "aozora_ffi.dll"
                )
            })
        },
        "FFI dynamic library",
    )?;
    let npm_package = npm_dir.join("package");

    let candidate = |path: &Path| -> Result<String, String> {
        Ok(format!("_release/candidates/{}", relative(path, &staging)?))
    };
    let identity = Identity {
        staging: &staging,
        version: &args.version,
        schema: args.schema_version,
    };
    let mut engines = BTreeMap::new();
    engines.insert(
        "wasm".to_owned(),
        engine(
            vec![
                "bun".to_owned(),
                "run".to_owned(),
                candidate(&wasm_adapter)?,
                candidate(&npm_package)?,
            ],
            &npm,
            &[&wasm_adapter],
            &identity,
        )?,
    );
    engines.insert(
        "rust".to_owned(),
        engine(
            vec![candidate(&rust_worker)?],
            &crate_artifact,
            &[&rust_worker],
            &identity,
        )?,
    );
    engines.insert(
        "cli".to_owned(),
        engine(
            vec![candidate(&cli)?, "lab-worker".to_owned()],
            &native,
            &[&cli],
            &identity,
        )?,
    );
    engines.insert(
        "ffi".to_owned(),
        engine(
            vec![
                python.to_owned(),
                candidate(&ffi_adapter)?,
                candidate(&library)?,
            ],
            &native,
            &[&ffi_adapter, &library],
            &identity,
        )?,
    );
    engines.insert(
        "extism".to_owned(),
        engine(
            vec![candidate(&extism_worker)?, candidate(&extism)?],
            &extism,
            &[&extism_worker],
            &identity,
        )?,
    );
    engines.insert(
        "python".to_owned(),
        engine(
            vec![
                python.to_owned(),
                candidate(&python_adapter)?,
                candidate(&python_dir)?,
            ],
            &wheel,
            &[&python_adapter],
            &identity,
        )?,
    );
    engines.insert(
        "go".to_owned(),
        engine(vec![candidate(&go_worker)?], &go, &[&go_worker], &identity)?,
    );
    let manifest = Manifest {
        schema_version: 1,
        aozora_commit: args.commit.clone(),
        engines,
    };
    let mut bytes = serde_json::to_vec_pretty(&manifest).map_err(|error| error.to_string())?;
    bytes.push(b'\n');
    fs::write(staging.join("artifacts.json"), bytes).map_err(|error| error.to_string())?;
    deterministic_tar(&staging, &args.out_dir.join("bundle.tar"))?;
    println!(
        "real-work bundle assembled for {}",
        match args.platform {
            Platform::Linux => "linux",
            Platform::Macos => "macos",
            Platform::Windows => "windows",
        }
    );
    Ok(())
}

fn corpus(args: &CorpusArgs) -> Result<(), String> {
    validate_commit(&args.expected_commit, "expected-commit")?;
    let output = Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(&args.checkout)
        .output()
        .map_err(|error| format!("read corpus commit: {error}"))?;
    if !output.status.success()
        || String::from_utf8_lossy(&output.stdout).trim() != args.expected_commit
    {
        return Err("rights corpus checkout does not match expected commit".to_owned());
    }
    require_file(&args.checkout.join("corpus/manifest.json"))?;
    deterministic_tar(&args.checkout.join("corpus"), &args.out)
}

fn baselines(args: &BaselineArgs) -> Result<(), String> {
    require_file(&args.diagnostics)?;
    require_file(&args.site.join("index.html"))?;
    fs::create_dir_all(&args.out_dir).map_err(|error| error.to_string())?;
    copy(
        &args.diagnostics,
        &args.out_dir.join("diagnostics-baseline.json"),
    )?;
    deterministic_tar(&args.site, &args.out_dir.join("site.tar"))
}

pub(crate) fn dispatch(args: &RealWorkArgs) -> Result<(), String> {
    let _root = workspace_root()?;
    match &args.op {
        RealWorkOp::Bundle(args) => bundle(args),
        RealWorkOp::Corpus(args) => corpus(args),
        RealWorkOp::Baselines(args) => baselines(args),
    }
}

#[cfg(test)]
mod tests {
    use std::env::current_dir;
    use std::path::Path;

    use super::{command_output_path, validate_commit};

    #[test]
    fn immutable_inputs_require_full_lowercase_commits() -> Result<(), String> {
        validate_commit(&"a".repeat(40), "commit")?;
        assert!(validate_commit(&"A".repeat(40), "commit").is_err());
        assert!(validate_commit("main", "commit").is_err());
        Ok(())
    }

    #[test]
    fn command_outputs_are_resolved_before_changing_working_directory() -> Result<(), String> {
        let relative = Path::new("target/real-work/linux/staging/workers/aozora-real-work-go");
        let expected = current_dir()
            .map_err(|error| error.to_string())?
            .join(relative);

        assert_eq!(command_output_path(relative)?, expected);
        assert_eq!(command_output_path(&expected)?, expected);
        Ok(())
    }
}
